//! myapi 网关主程序

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

/// 命令行入口（对照 myapi / agent-sync CLI）：
/// myapi check|models|list|apply [--config PATH]
/// 不带子命令时按 web 面板启动（向后兼容）
pub(crate) async fn maybe_cli(args: &[String]) -> bool {
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

/// 构建应用 Router（main 和集成测试共用；集成测试用 oneshot 真打端点）
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
pub(crate) async fn main() {
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

mod handlers;
pub(crate) use handlers::*;
mod proto_endpoints;
pub(crate) use proto_endpoints::*;
mod config_io;
pub(crate) use config_io::*;

// ==================== HTTP 集成测试（axum oneshot 真打端点 + tempfile 隔离 root）====================
#[cfg(test)]
mod tests;
