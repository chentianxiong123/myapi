//! usage token 解析/合并（纯函数，从 gateway.rs 拆出）
/// usage 四元组（input/output/cache_read/cache_creation）
pub type Usage = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

fn parse_usage(body: &[u8]) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None),
    };
    parse_usage_json(&v)
}

fn merge_usage(
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

fn sse_scan_usage(buf: &[u8]) -> Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> {
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
