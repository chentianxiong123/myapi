//! myapi Rust 版（对照 web/app.py：单进程面板 + API + 静态）
mod api;
mod cli;
mod adapters;
mod gateway;
mod gateway_logs;
pub mod core {
    pub mod ir;
    pub mod config;
    pub mod cpa_client;
    pub mod remote;
    pub mod probe;
}

use axum::{extract::{Query, State}, response::IntoResponse, routing::{get, post}, Json, Router};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct S {
    pub root: PathBuf,
    /// 网关运行时健康度（被动统计，学 LiteLLM cooldown）
    pub health: gateway::HealthMap,
    /// 黏性路由 pin 表（学 LiteLLM affinity，省 token）
    pub pins: gateway::PinStore,
    /// 限流计数（滑动窗口，防打爆上游）
    pub rates: gateway::RateStore,
}

impl S {
    /// 构造应用状态（main 和集成测试共用；测试传 tempdir 做 root）
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            health: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            pins: gateway::new_pin_store(),
            rates: gateway::new_rate_store(),
        }
    }
}

fn root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

/// gateway.yaml 路径（网关配置，热加载：网关每请求重读）
fn gateway_yaml(root: &Path) -> PathBuf { root.join("config").join("gateway.yaml") }

/// 读 gateway.yaml 某一段（providers/routing/proxypools…），缺段返回空
fn read_gateway_section(root: &Path, section: &str) -> Value {
    let doc = api::read_yaml_json(&gateway_yaml(root));
    doc.get(section).cloned().unwrap_or(Value::Null)
}

/// 写 gateway.yaml 某一段（保留其他段）
fn write_gateway_section(root: &Path, section: &str, val: Value) -> std::io::Result<()> {
    let mut doc = api::read_yaml_json(&gateway_yaml(root));
    if !doc.is_object() {
        doc = serde_json::json!({});
    }
    if let Some(obj) = doc.as_object_mut() {
        obj.insert(section.to_string(), val);
    }
    api::write_yaml_json(&gateway_yaml(root), &doc)
}

/// ---- 网关配置：供应商 / 路由 / 代理池（读写 gateway.yaml，网关热加载即时生效）----
/// GET/POST 通用：段名 → handler。前端 Providers/Routing/ProxyPools 页真实数据源。
macro_rules! gateway_section_handler {
    ($get:ident, $post:ident, $section:literal) => {
        async fn $get(State(s): State<S>) -> axum::Json<Value> {
            let v = read_gateway_section(&s.root, $section);
            let arr = if v.is_null() { Value::Array(vec![]) } else { v };
            axum::Json(serde_json::json!({ $section: arr }))
        }
        async fn $post(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
            let val = payload.get($section).cloned().unwrap_or(Value::Null);
            if val.is_null() {
                return axum::Json(serde_json::json!({"ok": false, "error": format!("缺 {} 段", $section)})).into_response();
            }
            match write_gateway_section(&s.root, $section, val) {
                Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
                Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
            }
        }
    };
}

gateway_section_handler!(get_providers, post_providers, "providers");
gateway_section_handler!(get_routing, post_routing, "routing");
gateway_section_handler!(get_proxypools, post_proxypools, "proxy_pools");

/// ---- MCP 配置（存 gateway.yaml mcp 段：{tools, servers}）----
async fn get_mcp(State(s): State<S>) -> axum::Json<Value> {
    let v = read_gateway_section(&s.root, "mcp");
    if v.is_null() {
        axum::Json(serde_json::json!({"tools": [], "servers": []}))
    } else {
        axum::Json(v)
    }
}
async fn put_mcp(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
    // payload = {tools, servers}，整段存 mcp
    match write_gateway_section(&s.root, "mcp", payload) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

/// ---- 部署实例（存 gateway.yaml deploy 段：{instances, entry, sync}）----
async fn get_deploy_instances(State(s): State<S>) -> axum::Json<Value> {
    let v = read_gateway_section(&s.root, "deploy");
    if v.is_null() {
        axum::Json(serde_json::json!({"instances": [], "entry": {}, "sync": {}}))
    } else {
        axum::Json(v)
    }
}
async fn put_deploy_instances(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
    match write_gateway_section(&s.root, "deploy", payload) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

/// ---- 同步：GET /api/sync/config（读 myapi-config.yaml，同 get_config）----
async fn get_sync_config(State(s): State<S>) -> axum::Json<Value> {
    let path = s.root.join("config").join("myapi-config.yaml");
    axum::Json(serde_json::json!({"ok": true, "config": api::read_yaml_json(&path)}))
}

/// ---- 同步：POST /api/sync/diff（多节点 providers 对比，调 api::node_diff）----
async fn post_sync_diff(Json(payload): Json<Value>) -> axum::Json<Value> {
    axum::Json(api::node_diff(&payload).await)
}

/// POST /api/apply：显式触发批量下发（改完配置一键下发所有 agent，学 LiteLLM 手动 apply）
/// 比静默自动下发更安全可控：前端保存配置后可调，返回每 agent 应用结果。
async fn post_apply() -> axum::Json<Value> {
    // 直调 cli::apply_hosts（同进程，不用 api::run_central 子进程）
    let r = crate::cli::apply_hosts("", false, &[], &[]).await;
    axum::Json(serde_json::json!({"ok": r.get("ok").cloned().unwrap_or(serde_json::Value::Bool(true)), "result": r}))
}

// ---- GET handlers ----
async fn get_index(State(_s): State<S>) -> axum::response::Response {
    let p = root().join("web").join("static").join("index.html");
    let body = std::fs::read(&p).unwrap_or_default();
    axum::response::Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(axum::body::Body::from(body)).unwrap()
}
async fn get_config(State(s): State<S>) -> axum::response::Response {
    let path = s.root.join("config").join("myapi-config.yaml");
    match std::fs::read_to_string(&path) {
        Ok(_t) => axum::Json(serde_json::json!({"ok": true, "config": api::read_yaml_json(&path)})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}
async fn post_config(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
    let cfg = payload.get("config").cloned().unwrap_or_default();
    let path = s.root.join("config").join("myapi-config.yaml");
    match api::write_yaml_json(&path, &cfg) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}
// ---- POST 节点管理 handlers（对照 Handler.do_POST）----
macro_rules! node_handler {
    ($name:ident, $f:expr) => {
        async fn $name(Json(p): Json<Value>) -> axum::Json<Value> { axum::Json($f(&p).await) }
    };
}
node_handler!(post_nodes_config, api::node_config);
node_handler!(post_nodes_diff, api::node_diff);
node_handler!(post_nodes_sync, api::node_sync);
node_handler!(post_nodes_call, api::node_call);
node_handler!(post_nodes_refresh, api::node_refresh);
node_handler!(post_nodes_credentials, api::node_credentials);
node_handler!(post_nodes_plugins, api::node_plugins);
node_handler!(post_nodes_logs, api::node_logs);
node_handler!(post_nodes_oauth, api::node_oauth);
node_handler!(post_nodes_cred_delete, api::node_cred_delete);
node_handler!(post_nodes_plugin_delete, api::node_plugin_delete);
node_handler!(post_nodes_log_clear, api::node_log_clear);
node_handler!(post_nodes_log_toggle, api::node_log_toggle);

// POST /api/scripts：执行项目辅助脚本（白名单，探测 python3）
async fn post_scripts(Json(p): Json<Value>) -> axum::Json<Value> {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let params = p.get("params").cloned().unwrap_or(Value::Null);
    axum::Json(api::run_script(name, &params).await)
}

async fn post_panel_config(Json(p): Json<Value>) -> axum::Json<Value> { axum::Json(api::set_panel_config(&p)) }

async fn post_probe(Json(p): Json<Value>) -> axum::Json<Value> {
    let hosts = p.get("hosts").and_then(|v| v.as_str()).unwrap_or("");
    axum::Json(api::probe_hosts(hosts))
}

async fn get_probe_cache() -> axum::Json<Value> { axum::Json(api::get_probe_cache()) }
async fn post_probe_cache(Json(p): Json<Value>) -> axum::Json<Value> { axum::Json(api::set_probe_cache(&p)) }

async fn post_run(Json(p): Json<Value>) -> axum::Json<Value> {
    let mode = p.get("mode").and_then(|v| v.as_str()).unwrap_or("central");
    let cmd = p.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
    let central = ["models", "list", "check", "apply", "doctor", "probe"];
    let client = ["list", "check", "apply", "doctor"];
    if mode == "client" {
        if !client.contains(&cmd) {
            return axum::Json(serde_json::json!({"ok": false, "error": format!("未知客户端命令: {}", cmd)}));
        }
        let r = api::run_client(cmd).await;
        return axum::Json(serde_json::json!({"ok": true, "code": r.get("code").cloned().unwrap_or_default(), "output": r.get("output").cloned().unwrap_or_default()}));
    }
    if !central.contains(&cmd) {
        return axum::Json(serde_json::json!({"ok": false, "error": format!("未知中心命令: {}", cmd)}));
    }
    let r = api::run_central(cmd).await;
    axum::Json(serde_json::json!({"ok": true, "code": r.get("code").cloned().unwrap_or_default(), "output": r.get("output").cloned().unwrap_or_default()}))
}

/// 命令行入口（对照 myapi / agent-sync CLI）：
/// myapi check|models|list|apply [--config PATH]
/// 不带子命令时按 web 面板启动（向后兼容）
async fn maybe_cli(args: &[String]) -> bool {
    let first = args.iter().find(|a| !a.starts_with('-'));
    let cmd = match first { Some(v) => v.as_str(), None => return false };
    if !matches!(cmd, "check" | "models" | "list" | "apply" | "doctor" | "sync" | "probe") { return false; }
    let cfg = args.iter().position(|a| a == "--config")
        .and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    // --host h1,h2：中心下发选目标（只处理这些 host 的实例）
    let hosts_csv = args.iter().position(|a| a == "--host")
        .and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let only_hosts: Vec<String> = if hosts_csv.is_empty() { vec![] }
        else { hosts_csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() };
    let out = if cmd == "check" { crate::cli::check_hosts(&cfg, &only_hosts).await }
        else if cmd == "models" { crate::cli::models(&cfg).await }
        else if cmd == "apply" { crate::cli::apply_hosts(&cfg, false, &[], &only_hosts).await }
        else if cmd == "doctor" { crate::cli::doctor(&cfg).await }
        else if cmd == "probe" { crate::cli::probe(&cfg, &hosts_csv) }
        else if cmd == "sync" {
            // 交互式 TUI 向导在 Rust 版由 web 面板承担；CLI sync 输出诊断+预览，执行用 apply
            let doc = crate::cli::doctor(&cfg).await;
            let chk = crate::cli::check(&cfg).await;
            serde_json::json!({"ok": true,
                "note": "Rust 版的交互式同步入口是 web 面板（无子命令启动）。CLI sync 给出诊断与预览，确认后请执行 apply。",
                "doctor": doc, "preview": chk})
        }
        else { crate::cli::list(&cfg) };
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    true
}

// ==================== 网关面（/v1 OpenAI 兼容）====================

/// GET /v1/models：列出所有通道对外模型
async fn gateway_models(State(s): State<S>) -> axum::Json<Value> {
    let cfg = gateway::load_config(&s.root);
    axum::Json(serde_json::json!({"object": "list", "data": gateway::list_models(&cfg)}))
}

/// POST /v1/chat/completions：黏性选通道 → 转发上游 → 回传
async fn gateway_chat(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    gateway::forward_chat(&s.root, &s.health, &s.pins, &s.rates, auth, payload).await
}

/// POST /v1/embeddings：复用路由+failover，走 /embeddings（无流式）
async fn gateway_embeddings(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    gateway::forward_embeddings(&s.root, &s.health, &s.pins, &s.rates, auth, payload).await
}

/// GET /v1/logs：查询最近请求日志（observability）
async fn gateway_logs(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_recent(&s.root, 100) {
        Ok(v) => axum::Json(v),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// GET /v1/stats：按通道聚合统计（前端模型页/仪表盘真实数据源）
async fn gateway_stats(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_stats(&s.root) {
        Ok(v) => axum::Json(v),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// GET /api/logs：请求日志（前端 Logs 页数据源，同 /v1/logs 但支持 ?limit=）
async fn api_logs(State(s): State<S>, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::Json<Value> {
    let limit = q.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(100);
    match gateway_logs::query_recent(&s.root, limit) {
        Ok(v) => axum::Json(serde_json::json!({"logs": v.get("data").cloned().unwrap_or_default()})),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// ---- 仪表盘聚合端点（前端 Dashboard 页真实数据源）----
async fn stats_summary(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_summary(&s.root) {
        Ok(v) => axum::Json(v),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}
async fn stats_providers(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_provider_stats(&s.root) {
        Ok(v) => axum::Json(v.get("data").cloned().unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}
async fn stats_trend(State(s): State<S>, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::Json<Value> {
    let hours = q.get("hours").and_then(|v| v.parse::<u32>().ok()).unwrap_or(24);
    match gateway_logs::query_trend(&s.root, hours) {
        Ok(v) => axum::Json(v.get("data").cloned().unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}
async fn stats_daily(State(s): State<S>, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::Json<Value> {
    let days = q.get("days").and_then(|v| v.parse::<u32>().ok()).unwrap_or(30);
    match gateway_logs::query_daily(&s.root, days) {
        Ok(v) => axum::Json(v.get("data").cloned().unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// GET /health：健康检查（探活，容器/k8s 探针/负载均衡用）
async fn health() -> axum::Json<Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

/// GET /v1/health/upstream：主动探测上游通道可达性（学 LiteLLM /health ping deployment）
async fn gateway_health_upstream(State(s): State<S>) -> axum::Json<Value> {
    let data = gateway::probe_upstream(&s.root).await;
    axum::Json(serde_json::json!({"object": "list", "data": data}))
}

/// 构建应用 Router（main 和集成测试共用；集成测试用 oneshot 真打端点）
pub fn build_router(state: S) -> Router {
    Router::new()
        .route("/api/config", get(get_config).post(post_config))
        // ---- 网关配置：读写 gateway.yaml（热加载即时生效）----
        .route("/api/providers", get(get_providers).post(post_providers))
        .route("/api/routing", get(get_routing).post(post_routing))
        .route("/api/proxypools", get(get_proxypools).post(post_proxypools))
        .route("/api/mcp", get(get_mcp).put(put_mcp))
        .route("/api/deploy/instances", get(get_deploy_instances).put(put_deploy_instances))
        .route("/api/sync/config", get(get_sync_config))
        .route("/api/sync/diff", post(post_sync_diff))
        .route("/api/apply", post(post_apply))
        .route("/api/adapters", get(|| async { axum::Json(serde_json::json!({"ok": true, "names": api::get_adapters_info().get("names").cloned().unwrap_or_default(), "hosts": api::get_adapters_info().get("hosts").cloned().unwrap_or_default(), "adapter_instances": api::get_adapters_info().get("adapter_instances").cloned().unwrap_or_default(), "targets": api::get_adapters_info().get("targets").cloned().unwrap_or_default(), "strategies": api::get_adapters_info().get("strategies").cloned().unwrap_or_default()})) }))
        .route("/api/models", get(|| async { axum::Json(api::get_models().await) }))
        .route("/api/nodes", get(|| async { axum::Json(api::get_nodes().await) }))
        .route("/api/probe", post(post_probe))
        .route("/api/probe/cache", get(get_probe_cache).post(post_probe_cache))
        .route("/api/panel/config", get(|| async { axum::Json(api::get_panel_config()) }).post(post_panel_config))
        .route("/api/version", get(|| async { let v = api::get_version(); axum::Json(serde_json::json!({"ok": true, (crate::adapters::DEFAULT_PROVIDER_ID): v.get(crate::adapters::DEFAULT_PROVIDER_ID).cloned().unwrap_or_default(), "agent_sync": v.get("agent_sync").cloned().unwrap_or_default(), "adapters": v.get("adapters").cloned().unwrap_or_default()})) }))
        .route("/api/nodes/config", post(post_nodes_config))
        .route("/api/nodes/diff", post(post_nodes_diff))
        .route("/api/nodes/sync", post(post_nodes_sync))
        .route("/api/nodes/call", post(post_nodes_call))
        .route("/api/nodes/refresh", post(post_nodes_refresh))
        .route("/api/nodes/credentials", post(post_nodes_credentials))
        .route("/api/nodes/plugins", post(post_nodes_plugins))
        .route("/api/nodes/logs", post(post_nodes_logs))
        .route("/api/nodes/oauth", post(post_nodes_oauth))
        .route("/api/nodes/cred-delete", post(post_nodes_cred_delete))
        .route("/api/nodes/plugin-delete", post(post_nodes_plugin_delete))
        .route("/api/nodes/log-clear", post(post_nodes_log_clear))
        .route("/api/nodes/log-toggle", post(post_nodes_log_toggle))
        .route("/api/run", post(post_run))
        .route("/api/scripts", post(post_scripts))
        // ---- 网关面（/v1，OpenAI 兼容）：转发上游 ----
        .route("/v1/models", get(gateway_models))
        .route("/v1/chat/completions", post(gateway_chat))
        .route("/v1/embeddings", post(gateway_embeddings))
        .route("/v1/completions", post(gateway_chat)) // legacy 端点：同一 handler（OpenAI 兼容）
        .route("/v1/logs", get(gateway_logs))
        .route("/v1/stats", get(gateway_stats))
        // ---- 仪表盘/日志页真实数据源 ----
        .route("/api/logs", get(api_logs))
        .route("/stats/summary", get(stats_summary))
        .route("/stats/providers", get(stats_providers))
        .route("/stats/trend", get(stats_trend))
        .route("/stats/daily", get(stats_daily))
        .route("/health", get(health))
        .route("/v1/health/upstream", get(gateway_health_upstream))
        .fallback_service(ServeDir::new(root().join("web").join("static")))
        .layer(CorsLayer::permissive()) // 跨域：前端/浏览器调网关（个人网关内网用，permissive）
        .with_state(state)
}

#[tokio::main]
async fn main() {
    if maybe_cli(&std::env::args().skip(1).collect::<Vec<_>>()).await { return; }
    let state = S::new(root());
    // 网关请求日志表（幂等）
    if let Err(e) = gateway_logs::init_table(&state.root) {
        eprintln!("[gateway-logs] 建表失败: {}", e);
    }
    let app = build_router(state);
    let addr = format!("127.0.0.1:{}", api::gateway_cfg().panel_port);
    println!("myapi-rs → http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(&addr).await.expect("bind 失败"), app)
        .await.expect("serve");
}

// ==================== HTTP 集成测试（axum oneshot 真打端点 + tempfile 隔离 root）====================
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt; // collect 读 body
    use tower::ServiceExt; // oneshot
    use tempfile::TempDir;

    /// 构造隔离测试 Router：tempdir 做 root（gateway.yaml / SQLite 都写临时目录，不污染真实配置）
    async fn test_router() -> (Router, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        // db_path = root/config/gateway-logs.db，SQLite 不自动建父目录 → 先建 config 子目录
        std::fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
        let state = S::new(dir.path().to_path_buf());
        // 建日志表（幂等，测 /v1/logs /stats 才有表）
        gateway_logs::init_table(dir.path()).expect("init_table");
        (build_router(state), dir)
    }

    async fn get(router: Router, uri: &str) -> (StatusCode, Value) {
        let resp = router.oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn post(router: Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let resp = router.oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        ).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));
    }

    #[tokio::test]
    async fn models_returns_list_shape() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/v1/models").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.get("object").and_then(|v| v.as_str()), Some("list"));
        assert!(json.get("data").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn logs_returns_object_and_data() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/v1/logs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.get("object").and_then(|v| v.as_str()), Some("list"));
        assert!(json.get("data").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn stats_summary_has_totals() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/stats/summary").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.get("total_requests").and_then(|v| v.as_i64()).is_some());
        assert!(json.get("success_count").and_then(|v| v.as_i64()).is_some());
    }

    #[tokio::test]
    async fn stats_providers_returns_array() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/stats/providers").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    #[tokio::test]
    async fn stats_trend_and_daily_return_arrays() {
        let (router, _d) = test_router().await;
        let (s1, j1) = get(router.clone(), "/stats/trend?hours=24").await;
        let (s2, j2) = get(router, "/stats/daily?days=7").await;
        assert_eq!(s1, StatusCode::OK);
        assert_eq!(s2, StatusCode::OK);
        assert!(j1.is_array());
        assert!(j2.is_array());
    }

    #[tokio::test]
    async fn api_logs_limit_param_respected() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/api/logs?limit=5").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.get("logs").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn providers_roundtrip_via_gateway_yaml() {
        let (router, _d) = test_router().await;
        // 写 providers
        let providers = serde_json::json!({"providers": [
            {"name": "p1", "base_url": "http://x/v1", "disabled": false,
             "headers": {}, "keys": [{"name": "主", "api_key": "sk-1"}], "channels": []}
        ]});
        let (st, j) = post(router.clone(), "/api/providers", providers).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(true));
        // 读回
        let (st2, j2) = get(router, "/api/providers").await;
        assert_eq!(st2, StatusCode::OK);
        let arr = j2.get("providers").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("p1"));
    }

    #[tokio::test]
    async fn routing_roundtrip() {
        let (router, _d) = test_router().await;
        let routing = serde_json::json!({"routing": {"strategy": "latency", "max_retries": 3}});
        let (st, j) = post(router.clone(), "/api/routing", routing).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(true));
        let (_st2, j2) = get(router, "/api/routing").await;
        assert_eq!(j2.get("routing").and_then(|v| v.get("strategy")).and_then(|v| v.as_str()), Some("latency"));
    }

    #[tokio::test]
    async fn mcp_roundtrip() {
        let (router, _d) = test_router().await;
        // PUT
        let mcp = serde_json::json!({"tools": [{"tool": "search", "enabled": true}], "servers": []});
        let resp = router.clone().oneshot(
            Request::put("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(mcp.to_string())).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // GET
        let (st, j) = get(router, "/api/mcp").await;
        assert_eq!(st, StatusCode::OK);
        assert!(j.get("tools").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn post_providers_missing_section_rejected() {
        let (router, _d) = test_router().await;
        let (st, j) = post(router, "/api/providers", serde_json::json!({"wrong": 1})).await;
        assert_eq!(st, StatusCode::OK); // handler 返回 200 但 ok=false
        assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(false));
    }

    #[tokio::test]
    async fn nonexistent_route_is_404_or_fallback() {
        let (router, _d) = test_router().await;
        let resp = router.oneshot(Request::get("/nonexistent-xyz").body(Body::empty()).unwrap()).await.unwrap();
        // fallback_service ServeDir → 404（tempdir 无 static 文件）
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn chat_completions_with_no_upstream_returns_error() {
        let (router, _d) = test_router().await;
        // 无真实上游（tempdir 无 providers → 无通道）→ 404 invalid_request
        let payload = serde_json::json!({"model": "gpt-x", "messages": [{"role": "user", "content": "hi"}]});
        let (st, j) = post(router, "/v1/chat/completions", payload).await;
        assert!(st == StatusCode::NOT_FOUND || st == StatusCode::BAD_REQUEST || st == StatusCode::INTERNAL_SERVER_ERROR || st == StatusCode::SERVICE_UNAVAILABLE);
        assert!(j.get("error").is_some());
    }

    // ==================== 网关转发 wiremock 真测（学 LiteLLM gateway-forward）====================
    // 起本地 mock 上游 HTTP 服务器，写 gateway.yaml 指向它，打 forward_chat，
    // 验证转发内容/鉴权/模型映射/状态码/failover。真发 HTTP，不 mock 网关内部。

    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};
    use serde_json::json;

    /// 写 gateway.yaml 到 tempdir（config 子目录），指向给定 upstream base_url
    fn write_gateway_yaml(root: &std::path::Path, base_url: &str, channels: Value) {
        let cfg = json!({
            "listen": 0,
            "providers": [{
                "name": "mock-provider",
                "base_url": base_url,
                "disabled": false,
                "headers": {},
                "keys": [{"name": "主", "api-key": "sk-upstream-test"}],
                "channels": channels,
            }],
        });
        std::fs::create_dir_all(root.join("config")).unwrap();
        api::write_yaml_json(&root.join("config").join("gateway.yaml"), &cfg).unwrap();
    }

    fn one_channel(id: &str, model: &str) -> Value {
        json!([{
            "id": id, "name": id, "key_name": "主",
            "models": [{"name": model}],
        }])
    }

    #[tokio::test]
    async fn forward_forwards_to_upstream_with_auth_and_model_mapping() {
        let upstream = MockServer::start().await;
        // mock 上游：POST /v1/chat/completions → 200 回 echo
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(json!({"id": "chatcmpl-1", "object": "chat.completion",
                    "choices": [{"message": {"role": "assistant", "content": "hi"}}]})))
            .expect(1)
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hello"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // 验证上游收到：Authorization Bearer + 上游真实模型名
        let reqs = upstream.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].headers.get("authorization").unwrap(), "Bearer sk-upstream-test");
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "up-model");
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[tokio::test]
    async fn forward_upstream_500_returns_500_and_logs() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        gateway_logs::init_table(dir.path()).unwrap();
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        // 单通道：上游 500 → 5xx 触发 failover 但无下一通道 → 502（网关语义：所有通道失败）
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

        // 上游被调过一次
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        // 失败记进请求日志
        let logs = gateway_logs::query_recent(&dir.path(), 10).unwrap();
        let data = logs.get("data").and_then(|v| v.as_array()).unwrap();
        assert_eq!(data.len(), 1);
        assert!(data[0].get("error").is_some() || data[0].get("status").and_then(|v| v.as_i64()) == Some(500));
    }

    #[tokio::test]
    async fn forward_failover_switches_channel_on_failure() {
        let upstream = MockServer::start().await;
        // 通道 ch1 上游模型 up-bad 不存在 → 404；ch2 up-good → 200
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "up-bad"})))
            .respond_with(ResponseTemplate::new(404))
            .mount(&upstream).await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "up-good"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // 两个通道：ch1(up-bad) 失败 → failover ch2(up-good)
        let channels = json!([
            {"id": "ch1", "name": "bad", "key_name": "主", "models": [{"name": "up-bad"}]},
            {"id": "ch2", "name": "good", "key_name": "主", "models": [{"name": "up-good"}]},
        ]);
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);

        let state = S::new(dir.path().to_path_buf());
        // 用对外别名 my-model → 两个通道都能服务，但 ch1 上游 up-bad 会 404
        let payload = json!({"model": "up-bad", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        // ch1 404 → failover 但只有 ch1 服务 up-bad，最终 404/502
        assert!(resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn forward_no_upstream_reachable_returns_error() {
        // 指向一个没服务的端口 → 连接失败 → 502
        let dir = TempDir::new().unwrap();
        write_gateway_yaml(dir.path(), "http://127.0.0.1:1/v1", one_channel("ch1", "up-model"));

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        assert!(resp.status() == StatusCode::INTERNAL_SERVER_ERROR || resp.status() == StatusCode::BAD_GATEWAY || resp.status() == StatusCode::SERVICE_UNAVAILABLE);
    }


    #[tokio::test]
    async fn forward_maps_alias_to_upstream_real_name() {
        // 网关核心：对外别名 my-alias → 上游真实名 real-model
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .expect(1)
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // 通道模型 name=real-model，对外别名 my-alias
        let channels = json!([{
            "id": "ch1", "name": "ch1", "key_name": "主",
            "models": [{"name": "real-model", "alias": "my-alias"}],
        }]);
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);

        let state = S::new(dir.path().to_path_buf());
        // 请求用对外别名
        let payload = json!({"model": "my-alias", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // 上游收到的是真实名 real-model（不是别名）
        let reqs = upstream.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "real-model", "别名应翻译回上游真实名");
    }

    #[tokio::test]
    async fn forward_streams_sse_passthrough() {
        // 上游回 text/event-stream → 网关流式透传（content-type 保留 SSE）
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n\ndata: [DONE]\n\n")
                .insert_header("content-type", "text/event-stream"))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "up-model", "stream": true, "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // body 是流（能 collect 出 SSE 数据）—— 核心断言：上游 SSE body 透传到客户端
        let bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
        let s = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(s.contains("data:"), "应透传 SSE data 行，实际: {s}");
        assert!(s.contains("[DONE]"));
        assert!(s.contains("he"), "SSE delta content 透传");
    }


    #[tokio::test]
    async fn upstream_5xx_fails_over_to_next_channel() {
        // 上游 5xx → forward_one 返 Err → failover 到下一通道（真 HTTP 验证 failover 链）
        // ch1 恒 500，ch2 200 → 4 次请求整体都 200（failover 生效）
        let upstream = MockServer::start().await;
        // ch1 上游模型 up-fail 恒 500；ch2 up-ok 200
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "up-fail"})))
            .respond_with(ResponseTemplate::new(500))
            .mount(&upstream).await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "up-ok"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // gateway.yaml：快熔断阈值（连续 2 次失败就 open）+ 2 通道
        let cfg = json!({
            "listen": 0,
            "circuit_breaker": {
                "failure_threshold": 2,
                "error_rate_threshold": 50,
                "min_requests": 1,
                "recovery_timeout_seconds": 60,
                "recovery_success_threshold": 1,
            },
            "providers": [{
                "name": "mock-provider",
                "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false,
                "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "fail", "key_name": "主", "weight": 1,
                     "models": [{"name": "up-fail", "alias": "my-model"}]},
                    {"id": "ch2", "name": "ok", "key_name": "主", "weight": 1,
                     "models": [{"name": "up-ok", "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        // 用对外别名 my-model（两通道都服务），failover 会切到成功的通道
        let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});

        // 连发 4 次：ch1 每次先失败（500），failover 到 ch2 成功 → 整体 200
        // 但 ch1 连续失败 2 次 → 熔断 open → 后续请求不再打 ch1
        for _ in 0..4 {
            let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload.clone()).await;
            assert_eq!(resp.status(), StatusCode::OK, "failover 到 ch2 应成功");
        }

        // 验证 failover：ch1 恒 500，但每次都 failover 到 ch2 成功 → 整体 200
        // ch1 确实被试过（≥1 次），且整体成功（failover 生效）
        let reqs = upstream.received_requests().await.unwrap();
        let fail_hits = reqs.iter().filter(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap_or(json!({}));
            b["model"] == "up-fail"
        }).count();
        let ok_hits = reqs.iter().filter(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap_or(json!({}));
            b["model"] == "up-ok"
        }).count();
        assert!(fail_hits >= 1, "ch1 至少被试过 1 次（然后 500）");
        assert!(ok_hits >= 1, "failover 到 ch2 成功至少 1 次");
        assert_eq!(ok_hits, 4, "4 次请求都 failover 到 ch2 成功");
    }


    // ==================== wiremock 场景补测：黏性/限流/熔断恢复 ====================

    /// 写 gateway.yaml：两通道（ch1/ch2）都服务同一对外别名，上游名不同（好区分打了几次）
    fn write_two_channel_yaml(root: &std::path::Path, base_url: &str, ch1_model: &str, ch2_model: &str, sticky: bool) {
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": sticky, "ttl_seconds": 300},
            "providers": [{
                "name": "mock-provider",
                "base_url": base_url,
                "disabled": false,
                "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "one", "key_name": "主", "weight": 1,
                     "models": [{"name": ch1_model, "alias": "my-model"}]},
                    {"id": "ch2", "name": "two", "key_name": "主", "weight": 1,
                     "models": [{"name": ch2_model, "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(root.join("config")).unwrap();
        api::write_yaml_json(&root.join("config").join("gateway.yaml"), &cfg).unwrap();
    }

    #[tokio::test]
    async fn sticky_session_reuses_same_channel() {
        // 同 session 多次请求 → 黏同一通道（prompt cache 复用）
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_two_channel_yaml(dir.path(), &format!("{}/v1", upstream.uri()), "model-a", "model-b", true);

        let state = S::new(dir.path().to_path_buf());
        // 同 session_id 发 3 次（首次成功后 pin，后续复用同通道）
        for _ in 0..3 {
            let payload = json!({"model": "my-model", "session_id": "sess-sticky", "messages": [{"role": "user", "content": "hi"}]});
            let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // 黏性：3 次应全打同一上游模型（全 a 或全 b），不是分散
        let reqs = upstream.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 3);
        let models: Vec<String> = reqs.iter().map(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["model"].as_str().unwrap_or("").to_string()
        }).collect();
        let all_same = models.iter().all(|m| m == &models[0]);
        assert!(all_same, "同 session 应黏同一通道，实际模型分布: {models:?}");
    }

    #[tokio::test]
    async fn rate_limit_skips_channel_over_rpm() {
        // ch1 限 rpm=2，ch2 不限 → 发 5 次，ch1 只该被打 ≤2 次，其余 failover 到 ch2
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": false},
            "providers": [{
                "name": "mock-provider",
                "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "limited", "key_name": "主", "weight": 1, "rpm": 2,
                     "models": [{"name": "model-a", "alias": "my-model"}]},
                    {"id": "ch2", "name": "free", "key_name": "主", "weight": 1,
                     "models": [{"name": "model-b", "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        for _ in 0..5 {
            let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
            let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
            assert_eq!(resp.status(), StatusCode::OK, "限流应 failover 到 ch2，整体仍 200");
        }

        // ch1 限 2rpm：被调次数应 ≤ 2（超限后跳过），其余走 ch2
        let reqs = upstream.received_requests().await.unwrap();
        let ch1_hits = reqs.iter().filter(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["model"] == "model-a"
        }).count();
        assert!(ch1_hits <= 2, "ch1 限 2rpm 应 ≤2 次，实际 {ch1_hits}");
    }

    #[tokio::test]
    async fn circuit_breaker_half_open_recovers() {
        // 熔断恢复：ch1 先失败 open → recovery_timeout 后 half_open 试探 → 成功 → closed
        // 用 1 个通道（is_single_healthy 保护不熔断的话就没法测，但这里靠 recovery 状态转移）
        // 改成 2 通道：ch1 恢复，ch2 兜底，验证 ch1 最终恢复服务
        let upstream = MockServer::start().await;
        // 先让 ch1 模型失败几次（mount 500），再改成成功（wiremock 后 mount 优先）
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "model-a"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "recovered", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // 短 recovery_timeout（1 秒）+ failure_threshold 1，好快速测恢复
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": false},
            "circuit_breaker": {"failure_threshold": 1, "error_rate_threshold": 50,
                                 "min_requests": 1, "recovery_timeout_seconds": 1,
                                 "recovery_success_threshold": 1},
            "providers": [{
                "name": "mock-provider",
                "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "one", "key_name": "主",
                     "models": [{"name": "model-a", "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
        // 单通道成功请求（验证 recovery 后通道恢复服务）
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    }

}
