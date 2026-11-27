//! myapi Rust 版（对照 web/app.py：单进程面板 + API + 静态）
mod api;
mod cli;
mod adapters;
mod ccswitch_import;
mod gateway;
mod gateway_logs;
mod protocol;
mod protocol_stream;
pub mod core {
    pub mod ir;
    pub mod config;
    pub mod cpa_client;
    pub mod remote;
    pub mod probe;
}

use axum::{extract::{Query, State}, response::IntoResponse, routing::{get, post}, Json, Router};
use serde_json::{json, Value};
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
    /// 响应缓存（相同请求返回缓存，省 token）
    pub cache: gateway::ResponseCacheStore,
}

impl S {
    /// 构造应用状态（main 和集成测试共用；测试传 tempdir 做 root）
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            health: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            pins: gateway::new_pin_store(),
            rates: gateway::new_rate_store(),
            cache: gateway::new_response_cache(),
        }
    }
}

fn root() -> PathBuf {
    // MYAPI_ROOT 环境变量可覆盖（Python 假测试框架用：隔离到临时目录，不污染真实 rs/config/）
    std::env::var("MYAPI_ROOT").map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

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

/// GET /api/deploy/nginx-conf：生成 Nginx 高可用配置（学 new-api 多机部署）
async fn get_nginx_conf(State(s): State<S>) -> axum::response::Response {
    let deploy = read_gateway_section(&s.root, "deploy");
    let conf = gateway::generate_nginx_conf(&deploy);
    axum::Json(serde_json::json!({"ok": true, "conf": conf})).into_response()
}

/// POST /api/deploy/sync-db：SQLite 主→备同步（scp gateway-logs.db）
/// 入参 {sourcePath, targetHost, targetPath} → shell scp。学前端 SqliteSync 配置。
async fn sync_db_handler(Json(body): Json<serde_json::Value>) -> axum::response::Response {
    let source = body.get("sourcePath").and_then(|v| v.as_str()).unwrap_or("");
    let target_host = body.get("targetHost").and_then(|v| v.as_str()).unwrap_or("");
    let target_path = body.get("targetPath").and_then(|v| v.as_str()).unwrap_or("");
    if source.is_empty() || target_host.is_empty() || target_path.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"ok": false, "error": "缺少 sourcePath/targetHost/targetPath"}))).into_response();
    }
    // scp 主库 → 备节点（需主节点对本备节点配了 SSH 免密）
    let output = tokio::process::Command::new("scp")
        .arg("-o").arg("StrictHostKeyChecking=no")
        .arg("-o").arg("ConnectTimeout=10")
        .arg(source)
        .arg(format!("{}:{}", target_host, target_path))
        .output()
        .await;
    match output {
        Ok(o) if o.status.success() => {
            axum::Json(serde_json::json!({"ok": true, "lastSync": chrono_now_iso(), "msg": "已同步到备用节点"})).into_response()
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            axum::Json(serde_json::json!({"ok": false, "error": format!("scp 失败: {}", err)})).into_response()
        }
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": format!("scp 启动失败: {}", e)})).into_response(),
    }
}

fn chrono_now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{:02}:{:02}:{:02}Z", h, m, s)
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

// ---- 多设备身份区分：tokens 增删改查（gateway.yaml auth.api_keys，支持 {name,key}）----
fn gateway_yaml_path(root: &PathBuf) -> PathBuf { root.join("config").join("gateway.yaml") }

/// 读 auth.api_keys（AuthKey Plain/Named → [{name, key}]）
fn read_auth_keys(root: &PathBuf) -> Vec<Value> {
    let path = gateway_yaml_path(root);
    let doc = api::read_yaml_json(&path);
    let arr = doc.pointer("/auth/api_keys").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    arr.iter().map(|k| {
        if let Some(name) = k.get("name").and_then(|v| v.as_str()) {
            let key = k.get("key").and_then(|v| v.as_str()).unwrap_or("");
            json!({"name": name, "key": key})
        } else {
            let key = k.as_str().unwrap_or("");
            json!({"name": key, "key": key})
        }
    }).collect()
}

/// 写 auth.api_keys 回 gateway.yaml（保留其他段）
fn write_auth_keys(root: &PathBuf, keys: &[Value]) -> std::io::Result<()> {
    let path = gateway_yaml_path(root);
    let mut doc = api::read_yaml_json(&path);
    if !doc.is_object() { doc = json!({}); }
    if let Some(obj) = doc.as_object_mut() {
        let arr: Vec<Value> = keys.iter().map(|k| {
            json!({"name": k.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                   "key": k.get("key").and_then(|v| v.as_str()).unwrap_or("")})
        }).collect();
        let auth = obj.entry("auth").or_insert_with(|| json!({}));
        if let Some(a) = auth.as_object_mut() {
            a.insert("api_keys".into(), json!(arr));
        }
    }
    api::write_yaml_json(&path, &doc)
}

async fn get_tokens(State(s): State<S>) -> axum::response::Response {
    axum::Json(json!({"ok": true, "tokens": read_auth_keys(&s.root)})).into_response()
}

async fn post_token(State(s): State<S>, Json(p): Json<Value>) -> axum::response::Response {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let key = p.get("key").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() || key.is_empty() {
        return axum::Json(json!({"ok": false, "error": "name 和 key 必填"})).into_response();
    }
    let mut keys = read_auth_keys(&s.root);
    if keys.iter().any(|k| k.get("name").and_then(|v| v.as_str()) == Some(name)) {
        return axum::Json(json!({"ok": false, "error": format!("设备 {} 已存在", name)})).into_response();
    }
    keys.push(json!({"name": name, "key": key}));
    match write_auth_keys(&s.root, &keys) {
        Ok(_) => axum::Json(json!({"ok": true})).into_response(),
        Err(e) => axum::Json(json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

async fn put_token(State(s): State<S>, Json(p): Json<Value>) -> axum::response::Response {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let key = p.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let mut keys = read_auth_keys(&s.root);
    let mut found = false;
    for k in keys.iter_mut() {
        if k.get("name").and_then(|v| v.as_str()) == Some(name) {
            k["key"] = json!(key);
            found = true;
        }
    }
    if !found {
        return axum::Json(json!({"ok": false, "error": format!("设备 {} 不存在", name)})).into_response();
    }
    match write_auth_keys(&s.root, &keys) {
        Ok(_) => axum::Json(json!({"ok": true})).into_response(),
        Err(e) => axum::Json(json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

async fn delete_token(State(s): State<S>, Json(p): Json<Value>) -> axum::response::Response {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let keys: Vec<Value> = read_auth_keys(&s.root).into_iter()
        .filter(|k| k.get("name").and_then(|v| v.as_str()) != Some(name)).collect();
    match write_auth_keys(&s.root, &keys) {
        Ok(_) => axum::Json(json!({"ok": true})).into_response(),
        Err(e) => axum::Json(json!({"ok": false, "error": e.to_string()})).into_response(),
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
    if !matches!(cmd, "check" | "models" | "list" | "apply" | "doctor" | "sync" | "probe" | "import-ccswitch") { return false; }
    let cfg = args.iter().position(|a| a == "--config")
        .and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    // --host h1,h2：中心下发选目标（只处理这些 host 的实例）
    let hosts_csv = args.iter().position(|a| a == "--host")
        .and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let only_hosts: Vec<String> = if hosts_csv.is_empty() { vec![] }
        else { hosts_csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() };
    // --db：cc-switch db 路径（import-ccswitch 用）
    let cc_db = args.iter().position(|a| a == "--db")
        .and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let out = if cmd == "check" { crate::cli::check_hosts(&cfg, &only_hosts).await }
        else if cmd == "models" { crate::cli::models(&cfg).await }
        else if cmd == "apply" { crate::cli::apply_hosts(&cfg, false, &[], &only_hosts).await }
        else if cmd == "doctor" { crate::cli::doctor(&cfg).await }
        else if cmd == "probe" { crate::cli::probe(&cfg, &hosts_csv) }
        else if cmd == "import-ccswitch" {
            // 一次性导入 cc-switch 的 provider 资产 → gateway.yaml
            // 用法：myapi import-ccswitch --db <cc-switch.db路径> [--config gateway.yaml路径]
            let yaml_path = if cfg.is_empty() { root().join("config").join("gateway.yaml") } else { PathBuf::from(&cfg) };
            if cc_db.is_empty() {
                serde_json::json!({"ok": false, "error": "缺少 --db 参数（cc-switch 的 cc-switch.db 路径）"})
            } else {
                match crate::ccswitch_import::import_from_db(&cc_db) {
                    Ok(providers) => {
                        let n = providers.len();
                        match crate::ccswitch_import::merge_into_yaml(&yaml_path, &providers) {
                            Ok(added) => serde_json::json!({"ok": true,
                                "imported": n, "added": added, "skipped": n - added,
                                "note": format!("已导入 {} 个 provider 到 {}（去重后新增 {} 个）", n, yaml_path.display(), added)}),
                            Err(e) => serde_json::json!({"ok": false, "error": e}),
                        }
                    }
                    Err(e) => serde_json::json!({"ok": false, "error": e}),
                }
            }
        }
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
    gateway::forward_chat(&s.root, &s.health, &s.pins, &s.rates, auth, payload, &s.cache).await
}

/// POST /v1/embeddings：复用路由+failover，走 /embeddings（无流式）
async fn gateway_embeddings(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    gateway::forward_embeddings(&s.root, &s.health, &s.pins, &s.rates, auth, payload, &s.cache).await
}

/// POST /v1/messages：Anthropic 请求 → 转 chat → 路由 → 响应转回 Anthropic（学 cc-switch 接管 Claude）
async fn gateway_messages(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let chat_body = match protocol::anthropic_to_openai(&payload) {
        Ok(b) => b,
        Err(e) => return proto_error(400, &format!("Anthropic 转换失败: {e}")),
    };
    let resp = gateway::forward_chat(&s.root, &s.health, &s.pins, &s.rates, auth, chat_body, &s.cache).await;
    if stream {
        return proto_stream_reply(resp, protocol_stream::AnthropicSseState::new()).await;
    }
    proto_reply(resp, |chat| protocol::openai_to_anthropic(chat)).await
}

/// POST /v1/responses：Responses 请求 → 转 chat → 路由 → 响应转回 Responses（学 cc-switch 接管 Codex）
async fn gateway_responses(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let chat_body = match protocol::responses_to_chat(&payload) {
        Ok(b) => b,
        Err(e) => return proto_error(400, &format!("Responses 转换失败: {e}")),
    };
    let resp = gateway::forward_chat(&s.root, &s.health, &s.pins, &s.rates, auth, chat_body, &s.cache).await;
    if stream {
        return proto_stream_reply(resp, protocol_stream::ResponsesSseState::new()).await;
    }
    proto_reply(resp, |chat| protocol::chat_to_responses(chat)).await
}

/// 流式响应：读上游 chat SSE 流 → 逐行转换 → 目标协议 SSE 流（学 cc-switch streaming 管道）
async fn proto_stream_reply<S: protocol_stream::SseTransformer + Send + 'static>(
    resp: axum::response::Response,
    mut state: S,
) -> axum::response::Response {
    let status = resp.status();
    if !status.is_success() {
        let (parts, body) = resp.into_parts();
        let bytes = http_body_util::BodyExt::collect(body).await.map(|b| b.to_bytes()).unwrap_or_default();
        let mut r = axum::response::Response::builder().status(status);
        if let Some(ct) = parts.headers.get("content-type") { r = r.header("content-type", ct); }
        return r.body(axum::body::Body::from(bytes)).unwrap_or_else(|_| proto_error(502, "错误响应构造失败"));
    }
    let (parts, body) = resp.into_parts();
    use http_body_util::BodyExt;
    let mut chunks = body.into_data_stream();
    let mut buffer = String::new();
    let mut finished = false;
    let stream = futures_util::stream::poll_fn(move |cx| -> std::task::Poll<Option<Result<bytes::Bytes, std::convert::Infallible>>> {
        loop {
            if finished { return std::task::Poll::Ready(None); }
            if let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        let out = state.finish();
                        finished = true;
                        if !out.is_empty() {
                            return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                        }
                        return std::task::Poll::Ready(None);
                    }
                    let out = state.push(data);
                    if !out.is_empty() {
                        return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                    }
                }
                continue;
            }
            match futures_util::StreamExt::poll_next_unpin(&mut chunks, cx) {
                std::task::Poll::Ready(Some(Ok(b))) => {
                    buffer.push_str(&String::from_utf8_lossy(&b));
                }
                std::task::Poll::Ready(Some(Err(_))) | std::task::Poll::Ready(None) => {
                    let out = state.finish();
                    finished = true;
                    if !out.is_empty() {
                        return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                    }
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    });
    let mut r = axum::response::Response::builder().status(200).header("content-type", "text/event-stream");
    for (k, v) in parts.headers.iter() {
        if k != "content-type" && k != "content-length" { r = r.header(k, v); }
    }
    r.body(axum::body::Body::from_stream(stream)).unwrap_or_else(|_| proto_error(502, "流式响应构造失败"))
}

/// 读 chat 响应 body → 转目标协议 → 返回；错误时透传原始 body
async fn proto_reply<F>(resp: axum::response::Response, f: F) -> axum::response::Response
where F: Fn(&Value) -> Result<Value, String> {
    let status = resp.status();
    let (parts, body) = resp.into_parts();
    let bytes = http_body_util::BodyExt::collect(body).await.map(|b| b.to_bytes()).unwrap_or_default();
    if status.is_success() {
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => match f(&v) {
                Ok(converted) => {
                    let mut r = axum::response::Response::builder().status(status);
                    if let Some(ct) = parts.headers.get("content-type") { r = r.header("content-type", ct); }
                    return r.body(axum::body::Body::from(serde_json::to_vec(&converted).unwrap_or_default()))
                        .unwrap_or_else(|_| proto_error(500, "响应构造失败"));
                }
                Err(e) => return proto_error(502, &e),
            },
            Err(e) => return proto_error(502, &format!("响应解析失败: {e}")),
        }
    }
    let mut r = axum::response::Response::builder().status(status);
    if let Some(ct) = parts.headers.get("content-type") { r = r.header("content-type", ct); }
    r.body(axum::body::Body::from(bytes)).unwrap_or_else(|_| proto_error(502, "错误响应构造失败"))
}

fn proto_error(code: u16, msg: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(serde_json::json!({"error": {"message": msg, "type": "api_error"}})),
    ).into_response()
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

/// POST /api/providers/fetch-models：拉上游模型列表（学 new-api FetchUpstreamModels）
/// 入参 {base_url, api_key, headers?} → GET 上游 /models → 返回 {models: [id...]}
async fn fetch_upstream_models_handler(
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let base_url = body.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
    let api_key = body.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
    let headers: Vec<(String, String)> = body.get("headers")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    if base_url.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"ok": false, "error": "缺少 base_url"}))).into_response();
    }
    match gateway::fetch_upstream_models(base_url, api_key, &headers, "openai_chat").await {
        Ok(models) => axum::Json(serde_json::json!({"ok": true, "models": models})).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({"ok": false, "error": e}))).into_response(),
    }
}

/// POST /api/providers/upstream-models：纯手动嗅探上游模型 + 差异检测（学 new-api DetectChannelUpstreamModelUpdates）
/// 入参 {provider, channel_id, ignored?} → 读 gateway.yaml 拿 base_url/key/headers/local 模型
/// → GET {base}/models → diff（add=上游新、remove=本地消失、ignored=被规则忽略）
/// 返回 {ok, add, remove, ignored, upstream_count, local_count}（只检测不写库）
async fn detect_upstream_models_handler(
    State(s): State<S>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = body.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
    if provider.is_empty() || channel_id.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"ok": false, "error": "缺少 provider 或 channel_id"}))).into_response();
    }
    let ignored: Vec<String> = body.get("ignored").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let config = gateway::load_config(&s.root);
    let p = match config.providers.iter().find(|p| p.name == provider) {
        Some(p) => p,
        None => return (axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"ok": false, "error": format!("供应商不存在: {}", provider)}))).into_response(),
    };
    let c = match p.channels.iter().find(|c| c.id == channel_id) {
        Some(c) => c,
        None => return (axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"ok": false, "error": format!("通道不存在: {}", channel_id)}))).into_response(),
    };
    let base = p.base_url.trim_end_matches('/').to_string();
    let api_key = gateway::resolve_key(p, c).unwrap_or_default();
    let headers: Vec<(String, String)> = p.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    // 按协议拼嗅探 URL（学 new-api fetchChannelUpstreamModelIDs）：取通道第一个非空协议，默认 openai
    let protocol = c.models.iter().find_map(|m| m.protocol.clone()).unwrap_or_else(|| "openai_chat".to_string());
    let upstream = match gateway::fetch_upstream_models(&base, &api_key, &headers, &protocol).await {
        Ok(m) => m,
        Err(e) => return (axum::http::StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({"ok": false, "error": e}))).into_response(),
    };
    let local: Vec<String> = c.models.iter().map(|m| m.name.clone()).collect();
    let (add, remove, ignored_hit) = gateway::diff_upstream_models(&upstream, &local, &ignored);
    axum::Json(serde_json::json!({
        "ok": true,
        "add": add,
        "remove": remove,
        "ignored": ignored_hit,
        "upstream_count": upstream.len(),
        "local_count": local.len(),
    })).into_response()
}

/// POST /api/providers/apply-upstream-models：纯手动应用上游模型变更（学 new-api ApplyChannelUpstreamModelUpdates）
/// 入参 {provider, channel_id, add: [{name, alias?, protocol?}, ...], remove: [model_name, ...]}
/// → 只写勾选的（前端 diff 对话框里用户勾选后传）→ 写回 gateway.yaml（热加载即时生效）
/// 返回 {ok, providers}
async fn apply_upstream_models_handler(
    State(s): State<S>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = body.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
    if provider.is_empty() || channel_id.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"ok": false, "error": "缺少 provider 或 channel_id"}))).into_response();
    }
    let add = body.get("add").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let remove: Vec<String> = body.get("remove").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    match gateway::apply_upstream_models(&s.root, provider, channel_id, &add, &remove) {
        Ok(providers) => axum::Json(serde_json::json!({"ok": true, "providers": providers})).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"ok": false, "error": e}))).into_response(),
    }
}

/// POST /api/channels/test：通道测试（学 new-api TestChannel）真发一个 chat 请求验证
/// 入参 {provider, channel_id} → 用 gateway.yaml 的 test 配置（默认模型+测试对话）→ 返回 {ok, status, latency_ms, content/error}
async fn test_channel_handler(
    State(s): State<S>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let channel_id = body.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
    if provider.is_empty() || channel_id.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"ok": false, "error": "缺少 provider 或 channel_id"}))).into_response();
    }
    let result = gateway::test_channel(&s.root, provider, channel_id).await;
    axum::Json(result).into_response()
}

/// 构建应用 Router（main 和集成测试共用；集成测试用 oneshot 真打端点）
pub fn build_router(state: S) -> Router {
    Router::new()
        .route("/api/config", get(get_config).post(post_config))
        .route("/api/tokens", get(get_tokens).post(post_token).put(put_token).delete(delete_token))
        // ---- 网关配置：读写 gateway.yaml（热加载即时生效）----
        .route("/api/providers", get(get_providers).post(post_providers))
        .route("/api/providers/fetch-models", post(fetch_upstream_models_handler))
        .route("/api/providers/upstream-models", post(detect_upstream_models_handler))
        .route("/api/providers/apply-upstream-models", post(apply_upstream_models_handler))
        .route("/api/channels/test", post(test_channel_handler))
        .route("/api/routing", get(get_routing).post(post_routing))
        .route("/api/proxypools", get(get_proxypools).post(post_proxypools))
        .route("/api/mcp", get(get_mcp).put(put_mcp))
        .route("/api/deploy/instances", get(get_deploy_instances).put(put_deploy_instances))
        .route("/api/deploy/nginx-conf", get(get_nginx_conf))
        .route("/api/deploy/sync-db", post(sync_db_handler))
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
        .route("/v1/messages", post(gateway_messages))
        .route("/v1/responses", post(gateway_responses))
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
        .fallback_service(ServeDir::new(root().join("web").join("dist")))
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
mod tests;