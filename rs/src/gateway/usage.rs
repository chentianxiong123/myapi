//! usage（从 gateway.rs 精确拆分）
use super::*;

/// 从非流式响应解析 usage（OpenAI 格式 usage.input_tokens/output_tokens）
pub(crate) fn parse_usage(body: &[u8]) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None),
    };
    parse_usage_json(&v)
}

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

/// 合并两个 usage 元组：逐字段取非 None（后者优先）。用于流式跨 chunk 累积——
/// Anthropic 把 usage 拆两处（message_start 带 input，message_delta 带 output），
/// 必须合并而非"取最后一个"，否则丢 input。
pub(crate) fn merge_usage(
    a: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
    b: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    (
        b.0.or(a.0),
        b.1.or(a.1),
        b.2.or(a.2),
        b.3.or(a.3),
    )
}

/// 扫描 SSE 缓冲里所有 `data:` 行，累积合并各 chunk 的 usage token。
/// 覆盖三种协议的流式格式：
///   OpenAI：最后 chunk 的 usage.prompt_tokens/completion_tokens
///   Anthropic：message_start.usage.input_tokens + message_delta.usage.output_tokens（分片）
///   Gemini：chunk 顶层 usageMetadata.promptTokenCount/candidatesTokenCount
pub(crate) fn sse_scan_usage(buf: &[u8]) -> Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> {
    let s = std::str::from_utf8(buf).ok()?;
    let mut usage: Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = None;
    for line in s.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("data:") {
            let d = rest.trim();
            if d.is_empty() || d == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(d) {
                // usage 位置因协议而异：OpenAI/Anthropic 在 usage 字段；Gemini 在顶层 usageMetadata
                let has_usage = v.get("usage").map(|u| !u.is_null()).unwrap_or(false)
                    || v.get("usageMetadata").map(|u| !u.is_null()).unwrap_or(false);
                if has_usage {
                    let (in_tok, out_tok, cr, cc) = parse_usage_json(&v);
                    if in_tok.is_some() || out_tok.is_some() || cr.is_some() || cc.is_some() {
                        let cur = usage.unwrap_or((None, None, None, None));
                        usage = Some(merge_usage(cur, (in_tok, out_tok, cr, cc)));
                    }
                }
            }
        }
    }
    usage
}