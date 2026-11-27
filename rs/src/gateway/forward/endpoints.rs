//! endpoints（精确拆分）
use super::super::*;

/// 转发到单个通道（Auto 和普通路由共用）：失败返回 ForwardError（区分可重试），成功返回响应
/// 通道测试（学 new-api TestChannel）：真发一个 chat 请求验证通道"真能出 token"
/// 用配置的默认模型（空=通道第一个模型）+ 测试对话。返回 {ok, status, latency_ms, content/error}
pub async fn test_channel(
    root: &std::path::Path,
    provider_name: &str,
    channel_id: &str,
) -> serde_json::Value {
    let config = load_config(root);
    let health: HealthMap = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let start = std::time::Instant::now();

    // 定位 provider + channel
    let Some(provider) = config.providers.iter().find(|p| p.name == provider_name) else {
        return serde_json::json!({"ok": false, "error": format!("供应商 {} 不存在", provider_name)});
    };
    let Some(channel) = provider.channels.iter().find(|c| c.id == channel_id) else {
        return serde_json::json!({"ok": false, "error": format!("通道 {} 不存在", channel_id)});
    };
    // 测试模型：配置的默认模型，空则用通道第一个模型的上游名
    let model = if !config.test.default_model.is_empty() {
        config.test.default_model.clone()
    } else {
        channel.models.first().map(|m| m.name.clone()).unwrap_or_default()
    };
    if model.is_empty() {
        return serde_json::json!({"ok": false, "error": "无可用测试模型（通道无模型且未配 default_model）"});
    }
    let api_key = resolve_key(provider, channel).unwrap_or_default();
    let sel = SelectedChannel {
        provider: provider.clone(),
        channel: channel.clone(),
        upstream_model: model.clone(),
        api_key,
        protocol: channel.models.iter().find(|mm| mm.name == model).and_then(|mm| mm.protocol.clone()),
    };
    // 强制非流式，便于取回整包内容
    let payload = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": config.test.test_message}],
        "max_tokens": config.test.max_tokens,
        "stream": false,
    });
    let policy = resolve_retry_policy(&config.retry, channel.retry.as_ref());
    match forward_one(root, &config, &health, &sel, &model, &payload, "/chat/completions", None, &policy).await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let bytes = http_body_util::BodyExt::collect(resp.into_body()).await
                .map(|b| b.to_bytes().to_vec()).unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
            let content = v.get("choices").and_then(|c| c.get(0))
                .and_then(|c| c.get("message")).and_then(|m| m.get("content"))
                .and_then(|x| x.as_str()).map(|s| s.to_string());
            let ok = (200..300).contains(&status) && content.is_some();
            serde_json::json!({
                "ok": ok, "status": status, "latency_ms": latency,
                "model": model,
                "content": content.unwrap_or_default(),
                "error": if ok { serde_json::Value::Null } else { serde_json::Value::String(format!("HTTP {}", status)) },
            })
        }
        Err(e) => {
            serde_json::json!({
                "ok": false, "status": e.status().unwrap_or(0),
                "latency_ms": start.elapsed().as_millis() as u64,
                "model": model, "content": "", "error": e.message(),
            })
        }
    }
}

/// chat 端点包装（forward_generic + /chat/completions）
pub async fn forward_chat(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    cache: &ResponseCacheStore,
) -> axum::response::Response {
    forward_generic(root, health, pins, rates, auth_header, payload, "/chat/completions", cache).await
}

/// embeddings 端点（复用路由+failover，走 /embeddings，无流式）
pub async fn forward_embeddings(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    cache: &ResponseCacheStore,
) -> axum::response::Response {
    // embeddings 不流式，强制去掉 stream 标记
    let mut p = payload;
    if let Some(obj) = p.as_object_mut() {
        obj.remove("stream");
    }
    forward_generic(root, health, pins, rates, auth_header, p, "/embeddings", cache).await
}