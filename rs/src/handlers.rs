//! handlers（精确拆分）
use super::*;

/// ---- MCP 配置（存 gateway.yaml mcp 段：{tools, servers}）----
pub(crate) async fn get_mcp(State(s): State<S>) -> axum::Json<Value> {
    let v = read_gateway_section(&s.root, "mcp");
    if v.is_null() {
        axum::Json(serde_json::json!({"tools": [], "servers": []}))
    } else {
        axum::Json(v)
    }
}

pub(crate) async fn put_mcp(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
    // payload = {tools, servers}，整段存 mcp
    match write_gateway_section(&s.root, "mcp", payload) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

/// ---- 部署实例（存 gateway.yaml deploy 段：{instances, entry, sync}）----
pub(crate) async fn get_deploy_instances(State(s): State<S>) -> axum::Json<Value> {
    let v = read_gateway_section(&s.root, "deploy");
    if v.is_null() {
        axum::Json(serde_json::json!({"instances": [], "entry": {}, "sync": {}}))
    } else {
        axum::Json(v)
    }
}

pub(crate) async fn put_deploy_instances(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
    match write_gateway_section(&s.root, "deploy", payload) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

/// GET /api/deploy/nginx-conf：生成 Nginx 高可用配置（学 new-api 多机部署）
pub(crate) async fn get_nginx_conf(State(s): State<S>) -> axum::response::Response {
    let deploy = read_gateway_section(&s.root, "deploy");
    let conf = gateway::generate_nginx_conf(&deploy);
    axum::Json(serde_json::json!({"ok": true, "conf": conf})).into_response()
}

/// POST /api/deploy/sync-db：SQLite 主→备同步（scp gateway-logs.db）
/// 入参 {sourcePath, targetHost, targetPath} → shell scp。学前端 SqliteSync 配置。
pub(crate) async fn sync_db_handler(Json(body): Json<serde_json::Value>) -> axum::response::Response {
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

/// ---- 同步：GET /api/sync/config（读 myapi-config.yaml，同 get_config）----
pub(crate) async fn get_sync_config(State(s): State<S>) -> axum::Json<Value> {
    let path = s.root.join("config").join("myapi-config.yaml");
    axum::Json(serde_json::json!({"ok": true, "config": api::read_yaml_json(&path)}))
}

/// ---- 同步：POST /api/sync/diff（多节点 providers 对比，调 api::node_diff）----
pub(crate) async fn post_sync_diff(Json(payload): Json<Value>) -> axum::Json<Value> {
    axum::Json(api::node_diff(&payload).await)
}

/// POST /api/apply：显式触发批量下发（改完配置一键下发所有 agent，学 LiteLLM 手动 apply）
/// 比静默自动下发更安全可控：前端保存配置后可调，返回每 agent 应用结果。
pub(crate) async fn post_apply() -> axum::Json<Value> {
    // 直调 cli::apply_hosts（同进程，不用 api::run_central 子进程）
    let r = crate::cli::apply_hosts("", false, &[], &[]).await;
    axum::Json(serde_json::json!({"ok": r.get("ok").cloned().unwrap_or(serde_json::Value::Bool(true)), "result": r}))
}

pub(crate) async fn get_index(State(_s): State<S>) -> axum::response::Response {
    let p = root().join("web").join("static").join("index.html");
    let body = std::fs::read(&p).unwrap_or_default();
    axum::response::Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(axum::body::Body::from(body)).unwrap()
}

pub(crate) async fn get_config(State(s): State<S>) -> axum::response::Response {
    let path = s.root.join("config").join("myapi-config.yaml");
    match std::fs::read_to_string(&path) {
        Ok(_t) => axum::Json(serde_json::json!({"ok": true, "config": api::read_yaml_json(&path)})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

pub(crate) async fn post_config(State(s): State<S>, Json(payload): Json<Value>) -> axum::response::Response {
    let cfg = payload.get("config").cloned().unwrap_or_default();
    let path = s.root.join("config").join("myapi-config.yaml");
    match api::write_yaml_json(&path, &cfg) {
        Ok(_) => axum::Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

pub(crate) async fn get_tokens(State(s): State<S>) -> axum::response::Response {
    axum::Json(json!({"ok": true, "tokens": read_auth_keys(&s.root)})).into_response()
}

pub(crate) async fn post_token(State(s): State<S>, Json(p): Json<Value>) -> axum::response::Response {
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

pub(crate) async fn put_token(State(s): State<S>, Json(p): Json<Value>) -> axum::response::Response {
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

pub(crate) async fn delete_token(State(s): State<S>, Json(p): Json<Value>) -> axum::response::Response {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let keys: Vec<Value> = read_auth_keys(&s.root).into_iter()
        .filter(|k| k.get("name").and_then(|v| v.as_str()) != Some(name)).collect();
    match write_auth_keys(&s.root, &keys) {
        Ok(_) => axum::Json(json!({"ok": true})).into_response(),
        Err(e) => axum::Json(json!({"ok": false, "error": e.to_string()})).into_response(),
    }
}

pub(crate) async fn post_scripts(Json(p): Json<Value>) -> axum::Json<Value> {
    let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let params = p.get("params").cloned().unwrap_or(Value::Null);
    axum::Json(api::run_script(name, &params).await)
}

pub(crate) async fn post_panel_config(Json(p): Json<Value>) -> axum::Json<Value> { axum::Json(api::set_panel_config(&p)) }

pub(crate) async fn post_probe(Json(p): Json<Value>) -> axum::Json<Value> {
    let hosts = p.get("hosts").and_then(|v| v.as_str()).unwrap_or("");
    axum::Json(api::probe_hosts(hosts))
}

pub(crate) async fn get_probe_cache() -> axum::Json<Value> { axum::Json(api::get_probe_cache()) }

pub(crate) async fn post_probe_cache(Json(p): Json<Value>) -> axum::Json<Value> { axum::Json(api::set_probe_cache(&p)) }

pub(crate) async fn post_run(Json(p): Json<Value>) -> axum::Json<Value> {
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

/// GET /v1/models：列出所有通道对外模型
pub(crate) async fn gateway_models(State(s): State<S>) -> axum::Json<Value> {
    let cfg = gateway::load_config(&s.root);
    axum::Json(serde_json::json!({"object": "list", "data": gateway::list_models(&cfg)}))
}

/// POST /v1/chat/completions：黏性选通道 → 转发上游 → 回传
pub(crate) async fn gateway_chat(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    gateway::forward_chat(&s.root, &s.health, &s.pins, &s.rates, auth, payload, &s.cache).await
}

/// POST /v1/embeddings：复用路由+failover，走 /embeddings（无流式）
pub(crate) async fn gateway_embeddings(
    State(s): State<S>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let auth = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    gateway::forward_embeddings(&s.root, &s.health, &s.pins, &s.rates, auth, payload, &s.cache).await
}

/// POST /v1/messages：Anthropic 请求 → 转 chat → 路由 → 响应转回 Anthropic（学 cc-switch 接管 Claude）
pub(crate) async fn gateway_messages(
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
pub(crate) async fn gateway_responses(
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

/// GET /v1/logs：查询最近请求日志（observability）
pub(crate) async fn gateway_logs(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_recent(&s.root, 100) {
        Ok(v) => axum::Json(v),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// GET /v1/stats：按通道聚合统计（前端模型页/仪表盘真实数据源）
pub(crate) async fn gateway_stats(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_stats(&s.root) {
        Ok(v) => axum::Json(v),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// GET /api/logs：请求日志（前端 Logs 页数据源，同 /v1/logs 但支持 ?limit=）
pub(crate) async fn api_logs(State(s): State<S>, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::Json<Value> {
    let limit = q.get("limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(100);
    match gateway_logs::query_recent(&s.root, limit) {
        Ok(v) => axum::Json(serde_json::json!({"logs": v.get("data").cloned().unwrap_or_default()})),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// ---- 仪表盘聚合端点（前端 Dashboard 页真实数据源）----
pub(crate) async fn stats_summary(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_summary(&s.root) {
        Ok(v) => axum::Json(v),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

pub(crate) async fn stats_providers(State(s): State<S>) -> axum::Json<Value> {
    match gateway_logs::query_provider_stats(&s.root) {
        Ok(v) => axum::Json(v.get("data").cloned().unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

pub(crate) async fn stats_trend(State(s): State<S>, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::Json<Value> {
    let hours = q.get("hours").and_then(|v| v.parse::<u32>().ok()).unwrap_or(24);
    match gateway_logs::query_trend(&s.root, hours) {
        Ok(v) => axum::Json(v.get("data").cloned().unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

pub(crate) async fn stats_daily(State(s): State<S>, Query(q): Query<std::collections::HashMap<String, String>>) -> axum::Json<Value> {
    let days = q.get("days").and_then(|v| v.parse::<u32>().ok()).unwrap_or(30);
    match gateway_logs::query_daily(&s.root, days) {
        Ok(v) => axum::Json(v.get("data").cloned().unwrap_or_default()),
        Err(e) => axum::Json(serde_json::json!({"error": {"message": e, "type": "api_error"}})),
    }
}

/// GET /health：健康检查（探活，容器/k8s 探针/负载均衡用）
pub(crate) async fn health() -> axum::Json<Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

/// GET /v1/health/upstream：主动探测上游通道可达性（学 LiteLLM /health ping deployment）
pub(crate) async fn gateway_health_upstream(State(s): State<S>) -> axum::Json<Value> {
    let data = gateway::probe_upstream(&s.root).await;
    axum::Json(serde_json::json!({"object": "list", "data": data}))
}

/// POST /api/providers/fetch-models：拉上游模型列表（学 new-api FetchUpstreamModels）
/// 入参 {base_url, api_key, headers?} → GET 上游 /models → 返回 {models: [id...]}
pub(crate) async fn fetch_upstream_models_handler(
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
pub(crate) async fn detect_upstream_models_handler(
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
pub(crate) async fn apply_upstream_models_handler(
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
pub(crate) async fn test_channel_handler(
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