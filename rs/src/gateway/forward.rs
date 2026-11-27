//! 转发核心（从 gateway.rs 拆出）
use super::*;

pub async fn forward_generic(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    upstream_path: &str,
    cache: &ResponseCacheStore,
) -> axum::response::Response {
    let config = load_config(root);
    // 下游鉴权：校验 Bearer key（api_keys 配了才校验，空=内网不鉴权）。
    // 成功时返回设备名（多设备身份区分：Named→name，日志/统计按设备聚合）。
    let device_name = match validate_auth(&config.auth, auth_header) {
        Ok(n) => n,
        Err(e) => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": {"message": e, "type": "authentication_error"}})),
            ).into_response();
        }
    };
    // 用量限额（学 LiteLLM max_budget）：配了 daily_token_budget → 查今日该 key 已用 token，超限返 429
    if let Some(budget) = config.auth.daily_token_budget {
        if budget > 0 {
            if let Some(dkey) = downstream_key(&config.auth, auth_header) {
                let used = crate::gateway_logs::today_tokens_for_key(root, &dkey).unwrap_or(0);
                if used >= budget {
                    return (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        axum::Json(serde_json::json!({"error": {
                            "message": format!("每日 token 预算已用尽（{used}/{budget}），明日重置", ),
                            "type": "budget_exceeded_error"
                        }})),
                    ).into_response();
                }
            }
        }
    }
    let requested_model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // 学 cc-switch：剥离 [1M]/[1m] 后缀（Claude Code 内置的 1M 上下文标记，大小写不敏感）
    // 客户端发 model=auto[1M]，网关剥离后识别档位 auto（黑盒调度），真实模型名由网关选
    let requested_model = strip_1m_suffix(&requested_model);
    let granularity = config.sticky_cfg().granularity.as_deref().unwrap_or("session");
    let pin_key = session_key(&payload, auth_header, granularity);
    let dkey = device_name.or_else(|| downstream_key(&config.auth, auth_header)); // 多设备身份：日志/记账按设备名

    // 档位（auto/pro/flash 是客户端配置的 model 名，保留字）：网关按档位过滤候选
    //   auto → 全部（网关随意，不偏）；pro → 只 class:pro；flash → 只 class:flash
    let is_tier = requested_model == "auto" || requested_model == "pro" || requested_model == "flash";
    if is_tier && (requested_model != "auto" || config.routing.auto_mode_enabled) {
        let class_filter = match requested_model.as_str() {
            "pro" => Some("pro"),
            "flash" => Some("flash"),
            _ => None, // auto：全部
        };
        let candidates = auto_candidates(&config, &payload, health, class_filter);
        if candidates.is_empty() {
            // 带硬需求（当前轮带图）但无满足能力的模型 → 409 显式失败，不静默降级
            let req = detect_required_caps(&payload);
            if !req.is_empty() {
                return (
                    axum::http::StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({"error": {"type": "capability_unavailable",
                        "message": format!("无满足能力的模型可用（需要 {}）", req.join("/")),
                        "required": req}})),
                ).into_response();
            }
            return error_response(404, &format!("{}: 无可用模型", requested_model));
        }
        let mut last_err = String::new();
        let mut attempted: u32 = 0; // 已重试次数（退避指数用）
        let mut last_context_err: Option<String> = None;
        for cand in &candidates {
            let sel = SelectedChannel {
                provider: cand.provider.clone(),
                channel: cand.channel.clone(),
                upstream_model: cand.upstream_model.clone(),
                api_key: cand.api_key.clone(),
                protocol: cand.protocol.clone() };
            // 限流：通道配了 rpm 且超限 → 跳过（防打爆上游）
            if let Some(rpm) = cand.channel.rpm {
                if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
            }
            let policy = resolve_retry_policy(&config.retry, cand.channel.retry.as_ref());
            match forward_one(root, &config, health, &sel, &requested_model, &payload, upstream_path, dkey.as_deref(), &policy).await {
                Ok(resp) => return resp,
                Err(e) => {
                    // context 不足：豁免 max_retries，继续试下一个（换更大窗口通道，全废才反馈）
                    if e.is_context() {
                        last_context_err = Some(e.message());
                        continue;
                    }
                    // 错误分类（学 LiteLLM）：致命 4xx（鉴权/参数）重试也没用 → 直接透传
                    if !e.is_retryable() {
                        return error_response(e.status().unwrap_or(502), &e.message());
                    }
                    last_err = e.message();
                    // 可重试：退避（0.5×2→8s + jitter，尊重 retry-after）再 failover
                    if attempted as usize + 1 < candidates.len() {
                        tokio::time::sleep(retry_delay(&policy, attempted, e.retry_after())).await;
                    }
                    attempted += 1;
                    continue; // fallback 下一个
                }
            }
        }
        if let Some(ctx) = last_context_err {
            return error_response(502, &format!("auto: 所有候选都装不下上下文（最后: {}）", ctx));
        }
        return error_response(502, &format!("auto: 所有候选失败, 最后错误: {}", last_err));
    }

    // 响应缓存查询（学 LiteLLM caching）：非流式 + 启用缓存 → 命中直接返回
    if config.response_cache.enabled && !payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
        let ck = cache_key(&requested_model, &payload);
        if let Some(c) = cache_get(cache, &ck, config.response_cache.ttl_seconds).await {
            let mut r = axum::response::Response::builder().status(c.status);
            if let Some(ct) = c.content_type { r = r.header("content-type", ct); }
            r = r.header("x-cache", "HIT");
            return r.body(axum::body::Body::from(c.body)).unwrap_or_else(|_| error_response(500, "缓存回放失败"));
        }
    }

    // 非 Auto：候选链 + failover（逐个试，失败切下一个，受 max_retries 限制）
    // 黏性：从 pin 表取上次通道提到链首（复用），失败则 failover 到其他通道
    let pinned_id = match pin_key.as_deref() {
        Some(k) => pins.read().await.get(k).map(|e| e.channel_id.clone()),
        None => None,
    };
    let input_tokens = estimate_input_tokens(&payload);
    let required = detect_required_caps(&payload); // 当前轮硬需求（vision/pdf）
    let candidates = ordered_candidates(&config, &requested_model, health, pinned_id.as_deref(), input_tokens, &required);
    if candidates.is_empty() {
        // 模型存在但无一个满足当前轮硬能力 → 显式 409，不静默降级（静默=客户端失守）
        if !required.is_empty() && model_exists(&config, &requested_model) {
            return (
                axum::http::StatusCode::CONFLICT,
                axum::Json(serde_json::json!({"error": {"type": "capability_unavailable",
                    "message": format!("模型 {} 无满足能力的通道（需要 {}）", requested_model, required.join("/")),
                    "required": required}})),
            ).into_response();
        }
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": {"message": format!("模型 {} 无可用通道", requested_model), "type": "invalid_request_error"}
            })),
        ).into_response();
    }
    let max_retries = config.retry.max_retries.min(candidates.len());
    let mut last_err = String::new();
    let mut attempted: u32 = 0; // 已重试次数（退避指数用）
    let mut last_context_err: Option<String> = None;
    for cand in candidates.iter() {
        if (attempted as usize) >= max_retries.max(1) { break; } // 普通失败达上限；context 不足不占次数
        // 限流：通道配了 rpm 且超限 → 跳过（防打爆上游）
        if let Some(rpm) = cand.channel.rpm {
            if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
        }
        let policy = resolve_retry_policy(&config.retry, cand.channel.retry.as_ref());
        match forward_one(root, &config, health, cand, &requested_model, &payload, upstream_path, dkey.as_deref(), &policy).await {
            Ok(resp) => {
                // 成功：更新黏性 pin 到这条通道（供后续复用）
                if let Some(k) = pin_key.as_deref() {
                    if config.sticky_cfg().enabled {
                        pins.write().await.insert(k.to_string(), PinEntry { channel_id: cand.channel.id.clone(), pinned_at: now_secs() });
                    }
                }
                // 响应缓存存储（学 LiteLLM caching）：非流式 + 启用缓存 → 存 body
                if config.response_cache.enabled && !payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let (parts, body) = resp.into_parts();
                    let ct = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                    let bytes = http_body_util::BodyExt::collect(body).await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
                    let ck = cache_key(&requested_model, &payload);
                    cache_put(cache, ck, CachedResponse { status: parts.status.as_u16(), content_type: ct, body: bytes.clone(), cached_at: std::time::Instant::now() }, config.response_cache.max_entries).await;
                    let mut r = axum::response::Response::from_parts(parts, axum::body::Body::empty());
                    *r.body_mut() = axum::body::Body::from(bytes);
                    return r;
                }
                return resp;
            }
            Err(e) => {
                // context 不足：豁免 max_retries，继续试下一个（换更大窗口通道，全废才反馈）
                if e.is_context() {
                    last_context_err = Some(e.message());
                    continue;
                }
                // 错误分类（学 LiteLLM）：致命 4xx（鉴权/参数）重试也没用 → 直接透传
                if !e.is_retryable() {
                    return error_response(e.status().unwrap_or(502), &e.message());
                }
                last_err = e.message();
                // 可重试：退避（0.5×2→8s + jitter，尊重 retry-after）再 failover
                if attempted as usize + 1 < max_retries.max(1) {
                    tokio::time::sleep(retry_delay(&policy, attempted, e.retry_after())).await;
                }
                attempted += 1;
                continue; // failover 下一个
            }
        }
    }
    // 模型级降级链（学 LiteLLM fallbacks）：主模型所有通道失败 → 按降级链试备选模型
    if config.model_fallback.enabled {
        if let Some(chain) = config.model_fallback.chains.get(&requested_model) {
            for fb_model in chain {
                let fb_candidates = ordered_candidates(&config, fb_model, health, None, input_tokens, &required);
                if fb_candidates.is_empty() { continue; }
                let fb_max = config.retry.max_retries.min(fb_candidates.len());
                for cand in fb_candidates.iter().take(fb_max.max(1)) {
                    if let Some(rpm) = cand.channel.rpm {
                        if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
                    }
                    let policy = resolve_retry_policy(&config.retry, cand.channel.retry.as_ref());
                    let mut fb_payload = payload.clone();
                    fb_payload["model"] = serde_json::Value::String(fb_model.clone());
                    match forward_one(root, &config, health, cand, fb_model, &fb_payload, upstream_path, dkey.as_deref(), &policy).await {
                        Ok(resp) => return resp,
                        Err(e) => { if !e.is_retryable() { return error_response(e.status().unwrap_or(502), &e.message()); } continue; }
                    }
                }
            }
        }
    }
    // context 不足优先反馈（客户端无感，除非全候选窗口都不够）
    if let Some(ctx) = last_context_err {
        return error_response(502, &format!("所有候选都装不下上下文（最后: {}）", ctx));
    }
    error_response(502, &format!("所有通道失败（试了 {} 条）: {}", max_retries.max(1), last_err))
}

enum ForwardError {
    /// 可重试：连接失败 / 408 409 429 / 5xx。重试退避后可试其他通道。
    Retryable { status: Option<u16>, retry_after: Option<std::time::Duration>, msg: String },
    /// 不可重试：其他 4xx（鉴权/参数/找不到）。重试也没用，直接透传。
    Fatal { status: Option<u16>, msg: String },
    /// 上下文不足（context_length_exceeded）：模型窗口装不下请求。可换更大窗口通道重试，豁免 max_retries。
    Context { status: Option<u16>, msg: String },
}

impl ForwardError {
    fn is_retryable(&self) -> bool {
        matches!(self, ForwardError::Retryable { .. })
    }
    fn is_context(&self) -> bool {
        matches!(self, ForwardError::Context { .. })
    }
    fn status(&self) -> Option<u16> {
        match self {
            ForwardError::Retryable { status, .. } | ForwardError::Fatal { status, .. } | ForwardError::Context { status, .. } => *status,
        }
    }
    fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            ForwardError::Retryable { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
    fn message(&self) -> String {
        match self {
            ForwardError::Retryable { msg, .. } | ForwardError::Fatal { msg, .. } | ForwardError::Context { msg, .. } => msg.clone(),
        }
    }
}

fn parse_retry_after(resp: &reqwest::Response) -> Option<std::time::Duration> {
    let v = resp.headers().get("retry-after")?.to_str().ok()?;
    let secs: u64 = v.trim().parse().ok()?;
    let d = std::time::Duration::from_secs(secs);
    if d <= std::time::Duration::from_secs(60) { Some(d) } else { None }
}

fn retry_delay(policy: &RetryConfig, attempted: u32, retry_after: Option<std::time::Duration>) -> std::time::Duration {
    let initial = policy.backoff_initial_ms as f64 / 1000.0;
    let max = policy.backoff_max_ms as f64 / 1000.0;
    let jitter = policy.jitter;
    // 尊重上游明确要求的 retry-after
    if policy.respect_retry_after {
        if let Some(ra) = retry_after {
            return ra + std::time::Duration::from_secs_f64(jitter * pseudo_rand());
        }
    }
    let doubled = initial * 2f64.powi(attempted as i32);
    let secs = doubled.min(max);
    std::time::Duration::from_secs_f64(secs + jitter * pseudo_rand())
}

fn pseudo_rand() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    (nanos % 1000) as f64 / 1000.0
}

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

async fn forward_one(
    root: &std::path::Path,
    config: &GatewayConfig,
    health: &HealthMap,
    selected: &SelectedChannel,
    requested_model: &str,
    payload: &serde_json::Value,
    upstream_path: &str,
    dkey: Option<&str>,
    policy: &RetryConfig,
) -> Result<axum::response::Response, ForwardError> {
    // 把请求里的模型名换成上游真实名
    let mut upstream_payload = payload.clone();
    if let Some(obj) = upstream_payload.as_object_mut() {
        obj.insert("model".into(), serde_json::Value::String(selected.upstream_model.clone()));
        // 历史轮媒体降级：上游模型不支持 vision → 历史轮的图换占位符（当前轮不动）
        if let Some(msgs) = obj.get_mut("messages").and_then(|v| v.as_array_mut()) {
            let supports_vision = upstream_caps_of(config, selected).iter().any(|c| c == "vision");
            if !supports_vision {
                strip_history_media(msgs, "vision");
            }
        }
    }
    let is_stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_anthropic = selected.protocol.as_deref() == Some("anthropic");

    // 构造上游 URL + 请求体（按协议：anthropic → /v1/messages + chat→anthropic 请求）
    let base = selected.provider.base_url.trim_end_matches('/');
    let (url, send_payload) = if is_anthropic {
        let anth = crate::protocol::chat_to_anthropic_request(&upstream_payload)
            .map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("chat→anthropic 转换失败: {}", e) })?;
        (format!("{}/v1/messages", base), anth)
    } else {
        (format!("{}{}", base, upstream_path), upstream_payload)
    };

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(config.timeout_cfg().streaming_first_byte))
        .timeout(std::time::Duration::from_secs(config.timeout_cfg().non_streaming))
        .build()
        .map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("client 构造失败: {}", e) })?;
    let mut req = if is_anthropic {
        // Claude 上游用 x-api-key + anthropic-version（非 Bearer）
        client.post(&url)
            .header("x-api-key", &selected.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
    } else {
        client.post(&url)
            .header("Authorization", format!("Bearer {}", selected.api_key))
            .header("Content-Type", "application/json")
    };
    for (k, v) in &selected.provider.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let start = std::time::Instant::now();
    let cb = config.cb();
    let resp = match req.json(&send_payload).send().await {
        Ok(r) => r,
        Err(e) => {
            record_outcome(health, &selected.channel.id, false, 0, cb, false, false).await;
            return Err(ForwardError::Retryable { status: None, retry_after: None, msg: format!("上游请求失败: {}", e) });
        }
    };
    let ttft = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let ok = status.is_success();
    record_outcome(health, &selected.channel.id, ok, ttft, cb, false, false).await;

    // 错误分类（学 LiteLLM，可配置）：致命 4xx（鉴权/参数）直接透传；可重试状态码返 Err 触发 failover
    let code = status.as_u16();
    if !status.is_success() {
        // 失败路径：读 body 分类（move resp 后不再需要回传）
        let retry_after = parse_retry_after(&resp);
        let body_bytes = resp.bytes().await.unwrap_or_default().to_vec();
        // context 不足检测（任何 fatal 判断前）：模型窗口装不下 → 可换更大窗口通道，豁免 max_retries
        if !policy.context_keywords.is_empty() {
            if let Some(kw) = hit_disable_keyword(&body_bytes, &policy.context_keywords) {
                record_outcome(health, &selected.channel.id, false, ttft, cb, false, false).await;
                crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                    model: selected.upstream_model.clone(),
                    provider: selected.provider.name.clone(),
                    channel: selected.channel.name.clone(),
                    status: status.as_u16(),
                    latency_ms: start.elapsed().as_millis() as u64,
                    ttft_ms: ttft,
                    input_tokens: None, output_tokens: None,
                    cache_read_tokens: None, cache_creation_tokens: None,
                    downstream_key: dkey.map(|s| s.to_string()),
                    error: Some(format!("上下文不足 '{}'：{}", kw, String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(200)]))),
                }).await;
                return Err(ForwardError::Context { status: Some(code), msg: format!("上下文不足：{}", kw) });
            }
        }
        if status.is_server_error() && policy.is_fatal(code) {
            return Err(ForwardError::Fatal { status: Some(code), msg: format!("上游错误（配置为致命）{}", code) });
        }
        if status.is_client_error() && policy.is_fatal(code) {
            return Err(ForwardError::Fatal { status: Some(code), msg: format!("上游客户端错误 {}", code) });
        }
        if status.is_server_error() || (status.is_client_error() && policy.is_retryable(code)) {
            // 关键词熔断（学 new-api）：含欠费/封号等关键词 → 强制 open + Retryable 切通道
            if let Some(kw) = hit_disable_keyword(&body_bytes, &policy.disable_keywords) {
                record_outcome(health, &selected.channel.id, false, ttft, cb, false, true).await;
                crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                    model: selected.upstream_model.clone(),
                    provider: selected.provider.name.clone(),
                    channel: selected.channel.name.clone(),
                    status: status.as_u16(),
                    latency_ms: start.elapsed().as_millis() as u64,
                    ttft_ms: ttft,
                    input_tokens: None, output_tokens: None,
                    cache_read_tokens: None, cache_creation_tokens: None,
                    downstream_key: dkey.map(|s| s.to_string()),
                    error: Some(format!("关键词熔断命中 '{}'：{}", kw, String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(200)]))),
                }).await;
                return Err(ForwardError::Retryable { status: Some(status.as_u16()), retry_after: None,
                    msg: format!("关键词熔断：响应体命中 '{}'，切通道", kw) });
            }
            // 普通错误（无关键词）：记日志 + 早返回触发 failover
            crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                model: selected.upstream_model.clone(),
                provider: selected.provider.name.clone(),
                channel: selected.channel.name.clone(),
                status: status.as_u16(),
                latency_ms: start.elapsed().as_millis() as u64,
                ttft_ms: ttft,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                downstream_key: dkey.map(|s| s.to_string()),
                error: Some(format!("HTTP {}", status.as_u16())),
            }).await;
            return Err(ForwardError::Retryable { status: Some(status.as_u16()), retry_after, msg: format!("上游服务端错误 {}", status.as_u16()) });
        }
        return Err(ForwardError::Fatal { status: Some(code), msg: format!("上游错误 {}", code) });
    }


    // 回传（真流式：SSE 逐 chunk 透传；非流式回整包）
    let upstream_headers = resp.headers().clone();
    let is_sse = upstream_headers.get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false) || is_stream;

    let mut out = axum::response::Response::builder().status(status);
    if let Some(ct) = upstream_headers.get("content-type") {
        out = out.header("content-type", ct);
    }
    // 透传上游关键 header
    for hk in ["x-request-id", "openai-organization", "ratelimit-remaining-requests"] {
        if let Some(hv) = upstream_headers.get(hk) {
            out = out.header(hk, hv);
        }
    }

    if is_sse {
        // 真流式：逐 chunk 转发字节流，客户端实时收到每个 SSE event
        // 同时用 SseUsageTap 扫描 data: 行抓 usage token，流结束后补记完整日志
        use axum::body::Body;
        // 日志延到流结束：建 oneshot channel，tap 流完时把 usage 发给后台任务补记
        let (usage_tx, usage_rx) = tokio::sync::oneshot::channel::<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>();
        let stream = resp.bytes_stream();
        // 按上游协议选 tap：anthropic → 转换流（anthropic SSE→chat SSE）；否则透传
        let tap: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> = if is_anthropic {
            Box::pin(crate::protocol_stream::AnthropicUpstreamTap::new(stream, usage_tx))
        } else {
            Box::pin(SseUsageTap {
                inner: Box::pin(stream),
                buf: Vec::new(),
                usage: (None, None, None, None),
                usage_tx: Some(usage_tx),
            })
        };
        // 流式 model 回显档位名（对齐非流式）：档位请求时把 SSE data chunk 的 model 字段改成客户端配的档位名
        let tap: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> =
            if requested_model == "pro" || requested_model == "flash" || requested_model == "auto" {
                Box::pin(ModelRewriteStream { inner: tap, model: requested_model.to_string() })
            } else {
                tap
            };
        // 后台任务：等流结束后拿 usage token，补一条带 token 的完整日志
        let log_root = root.to_path_buf();
        // 日志记真实上游模型名（用户要求：档位请求也记实际模型，不记档位别名）
        let log_model = selected.upstream_model.clone();
        let log_provider = selected.provider.name.clone();
        let log_channel = selected.channel.name.clone();
        let log_status = status.as_u16();
        let log_latency = start.elapsed().as_millis() as u64;
        let log_error = if ok { None } else { Some(format!("HTTP {}", status.as_u16())) };
        let log_dkey = dkey.map(|s| s.to_string()); // spawn 要 'static，先把借用的 dkey 转 owned
        tokio::spawn(async move {
            // 等流结束（tap drop / 发送端关闭时 rx 得 None）
            let (in_tok, out_tok, cache_read, cache_creation) = usage_rx.await.unwrap_or((None, None, None, None));
            crate::gateway_logs::log_request(log_root, crate::gateway_logs::LogEntry {
                model: log_model,
                provider: log_provider,
                channel: log_channel,
                status: log_status,
                latency_ms: log_latency,
                ttft_ms: ttft,
                input_tokens: in_tok,
                output_tokens: out_tok,
                cache_read_tokens: cache_read,
                cache_creation_tokens: cache_creation,
                downstream_key: log_dkey,
                error: log_error,
            }).await;
        });
        let body = Body::from_stream(tap);
        out.body(body).map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("流式回传失败: {}", e) })
    } else {
        // 非流式：整包 + 解析 usage token 入日志
        // （关键词熔断已在前面 5xx/可重试早返回块统一处理，这里只处理成功/非重试响应）
        let raw_bytes = resp.bytes().await.unwrap_or_default().to_vec();
        // 上游 anthropic → 转回 chat（myapi 内部统一协议）
        let body_bytes: Vec<u8> = if is_anthropic {
            let converted = serde_json::from_slice::<serde_json::Value>(&raw_bytes).ok()
                .and_then(|v| crate::protocol::anthropic_to_openai_response(&v).ok())
                .and_then(|chat| serde_json::to_vec(&chat).ok());
            match converted {
                Some(bytes) => bytes,
                None => raw_bytes,
            }
        } else { raw_bytes };
        // 坏数据容错：非流式 2xx 响应 body 应按 API 契约是 JSON（以 {/[ 开头）。
        // 解析失败 → 不透传坏数据给客户端，记失败并触发 failover（全废 502）。
        let looks_json = body_bytes.iter().copied()
            .skip_while(|b| b.is_ascii_whitespace())
            .next().map(|b| b == b'{' || b == b'[').unwrap_or(false);
        if looks_json && serde_json::from_slice::<serde_json::Value>(&body_bytes).is_err() {
            crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                model: selected.upstream_model.clone(),
                provider: selected.provider.name.clone(),
                channel: selected.channel.name.clone(),
                status: status.as_u16(),
                latency_ms: start.elapsed().as_millis() as u64,
                ttft_ms: ttft,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                downstream_key: dkey.map(|s| s.to_string()),
                error: Some("上游返回非法 JSON".to_string()),
            }).await;
            return Err(ForwardError::Retryable { status: Some(502), retry_after: None, msg: "上游返回非法 JSON".to_string() });
        }
        // 档位请求：响应 model 字段回显档位名（客户端配的 model 名，对称）；
        // 客户端如 Claude Code 用它配的 model 名做内部判断（上下文窗口），真实模型名已记日志
        let body_bytes = if requested_model == "pro" || requested_model == "flash" || requested_model == "auto" {
            match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                Ok(mut v) => { if v.is_object() { v["model"] = serde_json::json!(requested_model); } serde_json::to_vec(&v).unwrap_or(body_bytes) }
                Err(_) => body_bytes, // 已在上方容错 502，理论不可达
            }
        } else { body_bytes };
        let (in_tok, out_tok, cache_read, cache_creation) = parse_usage(&body_bytes);
        crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
            // 记真实上游模型名（非档位别名）
            model: selected.upstream_model.clone(),
            provider: selected.provider.name.clone(),
            channel: selected.channel.name.clone(),
            status: status.as_u16(),
            latency_ms: start.elapsed().as_millis() as u64,
            ttft_ms: ttft,
            input_tokens: in_tok,
            output_tokens: out_tok,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            downstream_key: dkey.map(|s| s.to_string()),
            error: if ok { None } else { Some(format!("HTTP {}", status.as_u16())) },
        }).await;
        out.body(axum::body::Body::from(body_bytes))
            .map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("回传失败: {}", e) })
    }
}

/// 从非流式响应解析 usage（OpenAI 格式 usage.input_tokens/output_tokens）

/// 从 usage JSON 提取 (input, output, cache_read, cache_creation)
/// 抄 LiteLLM/llms-types 字段名：OpenAI prompt_tokens_details.cached_tokens /
/// cache_creation_tokens；Anthropic cache_read_input_tokens / cache_creation_input_tokens
pub(crate) fn parse_usage_json(v: &serde_json::Value) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    // usage 位置因协议而异：
    //   OpenAI/Anthropic: usage 字段
    //   Gemini: 顶层 usageMetadata（promptTokenCount/candidatesTokenCount）
    let u = if let Some(u) = v.get("usage") {
        if !u.is_null() { u } else { return (None, None, None, None) }
    } else if let Some(g) = v.get("usageMetadata") {
        if !g.is_null() { g } else { return (None, None, None, None) }
    } else {
        return (None, None, None, None);
    };
    let get = |k: &str| u.get(k).and_then(|x| x.as_i64());
    // input: OpenAI prompt_tokens / Anthropic input_tokens / Gemini promptTokenCount
    let input = get("input_tokens").or_else(|| get("prompt_tokens")).or_else(|| get("promptTokenCount"));
    // output: OpenAI completion_tokens / Anthropic output_tokens / Gemini candidatesTokenCount
    let output = get("output_tokens").or_else(|| get("completion_tokens")).or_else(|| get("candidatesTokenCount"));
    // cache read: OpenAI prompt_tokens_details.cached_tokens；Anthropic cache_read_input_tokens；Gemini cachedContentTokenCount
    let cache_read = get("cache_read_input_tokens")
        .or_else(|| get("cachedContentTokenCount"))
        .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(|x| x.as_i64()));
    // cache creation: Anthropic cache_creation_input_tokens；OpenAI cache_creation_tokens
    let cache_creation = get("cache_creation_input_tokens")
        .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cache_creation_tokens")).and_then(|x| x.as_i64()));
    (input, output, cache_read, cache_creation)
}

/// SSE 流式 usage 捕获：包装字节流，逐 chunk 透传客户端的同时累积缓冲，
/// 扫描 `data:` 行里的 usage token（OpenAI 最后 chunk / Anthropic message_delta）。
/// 流结束后把 (input_tokens, output_tokens) 通过 oneshot channel 发给后台任务补记日志。
/// 流式 SSE 的 model 字段改写为档位名（客户端配的 model 名，对齐非流式回显）。
struct ModelRewriteStream {
    inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>>,
    model: String,
}

impl futures_core::Stream for ModelRewriteStream {
    type Item = Result<bytes::Bytes, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        match this.inner.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                let replaced = rewrite_chunk_model(&chunk, &this.model);
                std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(replaced))))
            }
            other => other,
        }
    }
}

/// 把 chunk 内所有 `"model":"..."` / `"model": "..."` 的值改成档位名。
fn rewrite_chunk_model(chunk: &[u8], model: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(chunk);
    if !s.contains("\"model\":") { return chunk.to_vec(); }
    let mut out = s.to_string();
    // 支持 serde_json（无空格）与上游透传（有空格）两种格式
    for needle in ["\"model\":\"", "\"model\": \""] {
        let mut idx = 0;
        while let Some(pos) = out[idx..].find(needle) {
            let start = idx + pos + needle.len();
            if let Some(end_rel) = out[start..].find('"') {
                let end = start + end_rel;
                out.replace_range(start..end, model);
                idx = end + model.len();
            } else { break; }
        }
    }
    out.into_bytes()
}

struct SseUsageTap {
    inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    buf: Vec<u8>,
    usage: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
    usage_tx: Option<tokio::sync::oneshot::Sender<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>>,
}

impl futures_core::Stream for SseUsageTap {
    type Item = Result<bytes::Bytes, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        match this.inner.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                // 透传给客户端，同时累积缓冲扫 usage
                this.buf.extend_from_slice(&chunk);
                if let Some(u) = sse_scan_usage(&this.buf) {
                    this.usage = u;
                }
                // 只保留未完成的最后一行（防 buf 无限涨）
                if let Some(pos) = this.buf.iter().rposition(|&b| b == b'\n') {
                    this.buf.drain(..=pos);
                }
                std::task::Poll::Ready(Some(Ok(chunk)))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                std::task::Poll::Ready(Some(Err(std::io::Error::other(e.to_string()))))
            }
            std::task::Poll::Ready(None) => {
                // 流结束：把抓到的 usage 通过 channel 发给后台任务补记
                if let Some(tx) = this.usage_tx.take() {
                    let _ = tx.send(this.usage);
                }
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// 合并两个 usage 元组：逐字段取非 None（后者优先）。用于流式跨 chunk 累积——
/// Anthropic 把 usage 拆两处（message_start 带 input，message_delta 带 output），
/// 必须合并而非"取最后一个"，否则丢 input。

/// 扫描 SSE 缓冲里所有 `data:` 行，累积合并各 chunk 的 usage token。
/// 覆盖三种协议的流式格式：
///   OpenAI：最后 chunk 的 usage.prompt_tokens/completion_tokens
///   Anthropic：message_start.usage.input_tokens + message_delta.usage.output_tokens（分片）
///   Gemini：chunk 顶层 usageMetadata.promptTokenCount/candidatesTokenCount

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


fn error_response(code: u16, msg: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(serde_json::json!({"error": {"message": msg, "type": "api_error"}})),
    ).into_response()
}

/// 列出所有对外可用模型名（/v1/models 用）：聚合所有通道的 alias/name
pub fn list_models(config: &GatewayConfig) -> Vec<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in &config.providers {
        if p.disabled { continue; }
        for c in &p.channels {
            for m in &c.models {
                let external = m.alias.clone().unwrap_or_else(|| m.name.clone());
                if seen.insert(external.clone()) {
                    out.push(serde_json::json!({
                        "id": external,
                        "object": "model",
                        "owned_by": p.name,
                    }));
                }
            }
        }
    }
    out
}

/// 主动健康探测（学 LiteLLM /health 主动 ping deployment）：轻量 GET {base_url}/models
/// 不花 token，只探通道上游可达性。返回每通道 live/dead + 延迟 + HTTP status。
/// 拉取上游模型列表（学 new-api FetchUpstreamModels）：GET base_url/models，解析 OpenAI 格式
/// 返回模型 id 列表（供面板自动填充，避免手填写错）
/// 按协议拼上游模型列表 URL（学 new-api fetchChannelUpstreamModelIDs）。
/// 只支持最通用的几个：openai_chat/openai_responses/anthropic→{base}/models（OpenAI 形），
/// gemini→{base}/v1beta/models，ollama→{base}/api/tags。其余（阿里/智谱等专门路径）不做。
pub fn upstream_models_url(base: &str, protocol: &str) -> String {
    let base = base.trim_end_matches('/');
    match protocol {
        "gemini" => format!("{}/v1beta/models", base),
        "ollama" => format!("{}/api/tags", base),
        _ => format!("{}/models", base),
    }
}

/// 按协议剥离模型名前缀（学 new-api TrimPrefix）：gemini 的 name 带 `models/` 前缀，剥掉。
pub fn strip_model_prefix(id: &str, protocol: &str) -> String {
    if protocol == "gemini" {
        id.strip_prefix("models/").unwrap_or(id).to_string()
    } else {
        id.to_string()
    }
}

pub async fn fetch_upstream_models(base_url: &str, api_key: &str, headers: &[(String, String)], protocol: &str) -> Result<Vec<String>, String> {
    let url = upstream_models_url(base_url, protocol);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client 构造失败: {}", e))?;
    let mut req = client.get(&url).header("Authorization", format!("Bearer {}", api_key));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("请求上游失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("上游返回 {}", status.as_u16()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    // OpenAI 格式 {data: [{id: "gpt-4", ...}, ...]}；兼容 {models: [...]}（gemini/ollama）/ 裸数组
    let ids: Vec<String> = if let Some(data) = body.get("data").and_then(|v| v.as_array()) {
        data.iter().filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())).collect()
    } else if let Some(models) = body.get("models").and_then(|v| v.as_array()) {
        models.iter().filter_map(|m| {
            m.get("id").or_else(|| m.get("name")).and_then(|x| x.as_str()).map(|s| s.to_string())
        }).collect()
    } else if let Some(arr) = body.as_array() {
        arr.iter().filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())).collect()
    } else {
        vec![]
    };
    Ok(ids.into_iter().map(|id| strip_model_prefix(&id, protocol)).collect())
}

/// 上游模型差异检测（学 new-api collectPendingUpstreamModelChangesFromModels）。
/// 纯手动：只算差异不写库，由调用方决定加/删。
/// - add    = upstream − local − ignored（ignored 支持 `regex:` 前缀正则）
/// - remove = local − upstream（本地有、上游已消失）
/// - ignored= 被忽略规则命中的上游新模型（提示但不加入）
pub fn diff_upstream_models(upstream: &[String], local: &[String], ignored: &[String])
    -> (Vec<String>, Vec<String>, Vec<String>) {
    use regex::Regex;
    use std::collections::HashSet;

    let local_set: HashSet<&str> = local.iter().map(|s| s.as_str()).collect();
    // 预编译忽略规则（`regex:` 前缀走正则，否则精确匹配）
    let rules: Vec<(String, Option<Regex>)> = ignored
        .iter()
        .filter_map(|r| {
            let r = r.trim().to_string();
            if r.is_empty() { return None; }
            if let Some(pat) = r.strip_prefix("regex:") {
                let re = Regex::new(pat).ok();
                Some((r, re))
            } else {
                Some((r, None))
            }
        })
        .collect();

    let mut ignored_hit = Vec::new();
    let mut add = Vec::new();
    for m in upstream {
        if local_set.contains(m.as_str()) { continue; }
        if rules.iter().any(|(r, re)| match re {
            Some(rx) => rx.is_match(m),
            None => r == m,
        }) {
            ignored_hit.push(m.clone());
            continue;
        }
        add.push(m.clone());
    }

    let up_set: HashSet<&str> = upstream.iter().map(|s| s.as_str()).collect();
    let remove: Vec<String> = local.iter()
        .filter(|m| !up_set.contains(m.as_str()))
        .cloned()
        .collect();

    (add, remove, ignored_hit)
}

/// 应用上游模型变更到 gateway.yaml（纯手动：只写调用方勾选的 add/remove）。
/// add 是完整 ChannelModel JSON（[{name, alias?, protocol?}, ...]），remove 是模型名。
/// 返回更新后的 providers 段。
pub fn apply_upstream_models(
    root: &std::path::Path,
    provider: &str,
    channel_id: &str,
    add: &[serde_json::Value],
    remove: &[String],
) -> Result<serde_json::Value, String> {
    use std::collections::HashSet;
    let path = root.join("config").join("gateway.yaml");
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读 gateway.yaml 失败: {}", e))?;
    let mut doc: serde_yaml::Value = serde_yaml::from_str(&raw)
        .map_err(|e| format!("解析 gateway.yaml 失败: {}", e))?;

    let provs = doc.get_mut("providers").and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| "gateway.yaml 缺 providers 段".to_string())?;
    let p = provs.iter_mut()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(provider))
        .ok_or_else(|| format!("供应商不存在: {}", provider))?;
    let chs = p.get_mut("channels").and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| format!("供应商 {} 缺 channels 段", provider))?;
    let c = chs.iter_mut()
        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(channel_id))
        .ok_or_else(|| format!("通道不存在: {}", channel_id))?;
    let models = c.get_mut("models").and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| format!("通道 {} 缺 models 段", channel_id))?;

    let to_remove: HashSet<&str> = remove.iter().map(|s| s.as_str()).collect();
    let before = models.len();
    models.retain(|m| !to_remove.contains(
        m.get("name").and_then(|v| v.as_str()).unwrap_or(""),
    ));
    let removed = before - models.len();
    for m in add {
        let yv = serde_yaml::to_value(m).map_err(|e| format!("模型项转 YAML 失败: {}", e))?;
        models.push(yv);
    }

    std::fs::write(&path, serde_yaml::to_string(&doc).map_err(|e| format!("序列化失败: {}", e))?)
        .map_err(|e| format!("写 gateway.yaml 失败: {}", e))?;

    let provs_val = doc.get("providers").cloned().unwrap_or(serde_yaml::Value::Null);
    serde_json::to_value(provs_val).map_err(|e| format!("转 JSON 失败: {}", e))
}

pub async fn probe_upstream(root: &std::path::Path) -> Vec<serde_json::Value> {
    let config = load_config(root);
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for p in &config.providers {
        if p.disabled {
            continue;
        }
        let base = p.base_url.trim_end_matches('/');
        let url = format!("{}/models", base);
        for c in &p.channels {
            let api_key = resolve_key(p, c).unwrap_or_default();
            let start = std::time::Instant::now();
            let mut req = client.get(&url).header("Authorization", format!("Bearer {}", api_key));
            for (k, v) in &p.headers {
                req = req.header(k.as_str(), v.as_str());
            }
            let (live, status_code, err) = match req.send().await {
                Ok(r) => {
                    let sc = r.status().as_u16();
                    // 401/403 说明上游活着（只是 key 问题）；2xx/429 都算可达
                    let ok = sc < 500;
                    (ok, sc, String::new())
                }
                Err(e) => (false, 0, e.to_string()),
            };
            let latency = start.elapsed().as_millis() as u64;
            out.push(serde_json::json!({
                "provider": p.name,
                "channel": c.name,
                "channel_id": c.id,
                "live": live,
                "status": if live { "healthy" } else { "unreachable" },
                "status_code": status_code,
                "latency_ms": latency,
                "error": err,
            }));
        }
    }
    out
}
/// 生成 Nginx 高可用配置（学 new-api 多机部署）：从 deploy 配置生成完整 nginx.conf
/// 客户端 → Nginx 固定入口 → upstream 主/备 → 网关节点。故障切换靠 proxy_next_upstream。
/// SSE 流式：proxy_buffering off + 长 read timeout。
pub fn generate_nginx_conf(deploy: &serde_json::Value) -> String {
    let entry = deploy.get("entry").cloned().unwrap_or_else(|| serde_json::json!({}));
    let listen = entry.get("listen").and_then(|v| v.as_u64()).unwrap_or(8380);
    let read_timeout = entry.get("readTimeout").and_then(|v| v.as_u64()).unwrap_or(120);
    let next_upstream = entry.get("nextUpstream").and_then(|v| v.as_str()).unwrap_or("error timeout http_502 http_503 http_504");
    let instances = deploy.get("instances").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    // upstream 块：master 在前，backup 标记 backup（只有主挂了才用备）
    let mut servers = String::new();
    for inst in &instances {
        let addr = inst.get("address").and_then(|v| v.as_str()).unwrap_or("");
        if addr.is_empty() { continue; }
        let role = inst.get("role").and_then(|v| v.as_str()).unwrap_or("backup");
        let max_fails = inst.get("heartbeatMs").and_then(|v| v.as_u64()).unwrap_or(5000) / 1000;
        if role == "master" {
            servers.push_str(&format!("        server {} max_fails={} fail_timeout={}s;\n", addr, max_fails.max(2), max_fails.max(5)));
        } else {
            servers.push_str(&format!("        server {} backup max_fails={} fail_timeout={}s;\n", addr, max_fails.max(2), max_fails.max(5)));
        }
    }
    if servers.is_empty() {
        servers.push_str("        # 无实例，请在面板添加网关节点\n");
    }

    format!(
        r#"# myapi 高可用 Nginx 配置（由面板生成）
# 部署：放到 Nginx 常驻机（不要装在 meilan 手机，保持手机省电）
# 客户端认固定入口 :{listen}，主节点挂了 proxy_next_upstream 自动切备用

worker_processes auto;
events {{ worker_connections 1024; }}

http {{
    # SSE 流式透传（网关的 /v1/chat/completions 是 SSE，必须关缓冲）
    proxy_buffering off;
    proxy_cache off;
    proxy_http_version 1.1;
    proxy_set_header Connection "";

    upstream myapi {{
{servers}    }}

    server {{
        listen {listen};
        # 客户端只认这个入口，无感知主备切换

        location / {{
            proxy_pass http://myapi;
            proxy_next_upstream {next_upstream};
            proxy_next_upstream_tries 3;
            proxy_next_upstream_timeout 30s;

            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_read_timeout {read_timeout}s;
            proxy_send_timeout {read_timeout}s;
            proxy_connect_timeout 5s;
        }}

        # 健康检查（Nginx 主动探活，摘除死节点）
        location = /health {{
            proxy_pass http://myapi;
            proxy_next_upstream error timeout;
        }}
    }}
}}
"#,
        listen = listen,
        servers = servers,
        next_upstream = next_upstream,
        read_timeout = read_timeout,
    )
}

