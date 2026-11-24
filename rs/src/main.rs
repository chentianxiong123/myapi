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

use axum::{extract::State, response::IntoResponse, routing::{get, post}, Json, Router};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct S {
    root: PathBuf,
    /// 网关运行时健康度（被动统计，学 LiteLLM cooldown）
    health: gateway::HealthMap,
    /// 黏性路由 pin 表（学 LiteLLM affinity，省 token）
    pins: gateway::PinStore,
    /// 限流计数（滑动窗口，防打爆上游）
    rates: gateway::RateStore,
}

fn root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

/// gateway.yaml 路径（网关配置，热加载：网关每请求重读）
fn gateway_yaml() -> PathBuf { root().join("config").join("gateway.yaml") }

/// 读 gateway.yaml 某一段（providers/routing/proxypools…），缺段返回空
fn read_gateway_section(section: &str) -> Value {
    let doc = api::read_yaml_json(&gateway_yaml());
    doc.get(section).cloned().unwrap_or(Value::Null)
}

/// 写 gateway.yaml 某一段（保留其他段）
fn write_gateway_section(section: &str, val: Value) -> std::io::Result<()> {
    let mut doc = api::read_yaml_json(&gateway_yaml());
    if !doc.is_object() {
        doc = serde_json::json!({});
    }
    if let Some(obj) = doc.as_object_mut() {
        obj.insert(section.to_string(), val);
    }
    api::write_yaml_json(&gateway_yaml(), &doc)
}

/// ---- 网关配置：供应商 / 路由 / 代理池（读写 gateway.yaml，网关热加载即时生效）----
/// GET/POST 通用：段名 → handler。前端 Providers/Routing/ProxyPools 页真实数据源。
macro_rules! gateway_section_handler {
    ($get:ident, $post:ident, $section:literal) => {
        async fn $get() -> axum::Json<Value> {
            let v = read_gateway_section($section);
            let arr = if v.is_null() { Value::Array(vec![]) } else { v };
            axum::Json(serde_json::json!({ $section: arr }))
        }
        async fn $post(Json(payload): Json<Value>) -> axum::response::Response {
            let val = payload.get($section).cloned().unwrap_or(Value::Null);
            if val.is_null() {
                return axum::Json(serde_json::json!({"ok": false, "error": format!("缺 {} 段", $section)})).into_response();
            }
            match write_gateway_section($section, val) {
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
async fn get_mcp() -> axum::Json<Value> {
    let v = read_gateway_section("mcp");
    if v.is_null() {
        axum::Json(serde_json::json!({"tools": [], "servers": []}))
    } else {
        axum::Json(v)
    }
}
async fn put_mcp(Json(payload): Json<Value>) -> axum::response::Response {
    // payload = {tools, servers}，整段存 mcp
    match write_gateway_section("mcp", payload) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

/// ---- 部署实例（存 gateway.yaml deploy 段：{instances, entry, sync}）----
async fn get_deploy_instances() -> axum::Json<Value> {
    let v = read_gateway_section("deploy");
    if v.is_null() {
        axum::Json(serde_json::json!({"instances": [], "entry": {}, "sync": {}}))
    } else {
        axum::Json(v)
    }
}
async fn put_deploy_instances(Json(payload): Json<Value>) -> axum::response::Response {
    match write_gateway_section("deploy", payload) {
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

/// GET /health：健康检查（探活，容器/k8s 探针/负载均衡用）
async fn health() -> axum::Json<Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

/// GET /v1/health/upstream：主动探测上游通道可达性（学 LiteLLM /health ping deployment）
async fn gateway_health_upstream(State(s): State<S>) -> axum::Json<Value> {
    let data = gateway::probe_upstream(&s.root).await;
    axum::Json(serde_json::json!({"object": "list", "data": data}))
}

#[tokio::main]
async fn main() {
    if maybe_cli(&std::env::args().skip(1).collect::<Vec<_>>()).await { return; }
    let state = S {
        root: root(),
        health: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        pins: gateway::new_pin_store(),
        rates: gateway::new_rate_store(),
    };
    // 网关请求日志表（幂等）
    if let Err(e) = gateway_logs::init_table(&state.root) {
        eprintln!("[gateway-logs] 建表失败: {}", e);
    }
    let app = Router::new()
        .route("/api/config", get(get_config).post(post_config))
        // ---- 网关配置：读写 gateway.yaml（热加载即时生效）----
        .route("/api/providers", get(get_providers).post(post_providers))
        .route("/api/routing", get(get_routing).post(post_routing))
        .route("/api/proxypools", get(get_proxypools).post(post_proxypools))
        .route("/api/mcp", get(get_mcp).put(put_mcp))
        .route("/api/deploy/instances", get(get_deploy_instances).put(put_deploy_instances))
        .route("/api/sync/config", get(get_sync_config))
        .route("/api/sync/diff", post(post_sync_diff))
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
        .route("/health", get(health))
        .route("/v1/health/upstream", get(gateway_health_upstream))
        .fallback_service(ServeDir::new(root().join("web").join("static")))
        .layer(CorsLayer::permissive()) // 跨域：前端/浏览器调网关（个人网关内网用，permissive）
        .with_state(state);
    let addr = format!("127.0.0.1:{}", api::gateway_cfg().panel_port);
    println!("myapi-rs → http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(&addr).await.expect("bind 失败"), app)
        .await.expect("serve");
}
