//! retry（精确拆分）
use super::super::*;

/// 粗估 input token（不引 tokenizer SDK：4 字符≈1 token，个人网关够用）
pub fn estimate_input_tokens(payload: &serde_json::Value) -> i64 {
    let mut total: i64 = 0;
    if let Some(msgs) = payload.get("messages").and_then(|v| v.as_array()) {
        for m in msgs {
            if let Some(c) = m.get("content").and_then(|v| v.as_str()) {
                total += (c.len() as i64) / 4 + 1;
            }
        }
    }
    total
}

/// 合并全局重试策略 + 通道级覆盖（通道只配想改的字段，其余继承全局）
pub(crate) fn resolve_retry_policy<'a>(global: &'a RetryConfig, channel_override: Option<&'a RetryOverride>) -> RetryConfig {
    let mut policy = global.clone();
    if let Some(o) = channel_override {
        if let Some(v) = o.max_retries { policy.max_retries = v; }
        if let Some(ref v) = o.retry_on_status { policy.retry_on_status = v.clone(); }
        if let Some(ref v) = o.fatal_status { policy.fatal_status = v.clone(); }
        if let Some(v) = o.backoff_initial_ms { policy.backoff_initial_ms = v; }
        if let Some(v) = o.backoff_max_ms { policy.backoff_max_ms = v; }
        if let Some(v) = o.jitter { policy.jitter = v; }
        if let Some(v) = o.respect_retry_after { policy.respect_retry_after = v; }
    }
    policy
}

/// 从上游响应头读 retry-after（秒），仅尊重 ≤ 60s 的（学 LiteLLM MAX_HONOURED_RETRY_AFTER）
pub(crate) fn parse_retry_after(resp: &reqwest::Response) -> Option<std::time::Duration> {
    let v = resp.headers().get("retry-after")?.to_str().ok()?;
    let secs: u64 = v.trim().parse().ok()?;
    let d = std::time::Duration::from_secs(secs);
    if d <= std::time::Duration::from_secs(60) { Some(d) } else { None }
}

/// 重试退避延迟（学 LiteLLM Backoff）：0.5s 起，指数 ×2 到 8s 封顶，加 0.75 jitter 防雪崩。
/// 优先尊重上游 retry-after。attempted=已重试次数。
/// 重试退避延迟（学 LiteLLM Backoff）：initial 起 ×2 到 max 封顶，加 jitter 防雪崩。
/// 尊重上游 retry-after（若 policy 允许）。attempted=已重试次数。
pub(crate) fn retry_delay(policy: &RetryConfig, attempted: u32, retry_after: Option<std::time::Duration>) -> std::time::Duration {
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

/// 轻量伪随机 [0,1)，从纳秒时间戳取，避免引 rand 依赖（jitter 防雪崩够用）
pub(crate) fn pseudo_rand() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    (nanos % 1000) as f64 / 1000.0
}

pub(crate) fn error_response(code: u16, msg: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(serde_json::json!({"error": {"message": msg, "type": "api_error"}})),
    ).into_response()
}