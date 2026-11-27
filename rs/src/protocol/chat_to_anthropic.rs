//! chat_to_anthropic（精确拆分）
use super::*;

/// chat usage → Anthropic 三桶 usage（学 cc-switch build_anthropic_usage_json）。
pub(crate) fn chat_usage_to_anthropic(u: &Value) -> Value {
    let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let completion = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_read = u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_creation = u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).map(|_| 0).unwrap_or(0);
    let mut out = json!({
        "input_tokens": prompt.saturating_sub(cache_read).saturating_sub(cache_creation),
        "output_tokens": completion,
    });
    if cache_read > 0 { out["cache_read_input_tokens"] = json!(cache_read); }
    if cache_creation > 0 { out["cache_creation_input_tokens"] = json!(cache_creation); }
    out
}

/// 流式请求注入 `stream_options.include_usage`（学 cc-switch `inject_openai_stream_include_usage`）。
/// 上游流式默认不回 usage，必须显式声明才有 token 统计。
pub fn inject_stream_include_usage(body: &mut Value) {
    if body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
        match body.get_mut("stream_options") {
            Some(Value::Object(opts)) => { opts.insert("include_usage".to_string(), json!(true)); }
            _ => { body["stream_options"] = json!({"include_usage": true}); }
        }
    }
}

/// chat 请求 → anthropic 请求（上游 Claude /v1/messages）。cc-switch 无此函数
/// （它假设客户端已是 anthropic）；myapi 内部统一 chat，需自实现。
pub fn chat_to_anthropic_request(chat: &Value) -> Result<Value, String> {
    let mut result = json!({});
    if let Some(m) = chat.get("model") { result["model"] = m.clone(); }

    let mut messages: Vec<Value> = Vec::new();
    let mut system_parts: Vec<String> = Vec::new();
    if let Some(msgs) = chat.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            if role == "system" {
                if let Some(s) = chat_text_of(msg) {
                    if !s.is_empty() { system_parts.push(s); }
                }
                continue;
            }
            messages.push(chat_message_to_anthropic(msg, role));
        }
    }
    if !system_parts.is_empty() { result["system"] = json!(system_parts.join("\n")); }
    result["messages"] = json!(messages);

    for k in ["max_tokens", "max_completion_tokens"] {
        if let Some(v) = chat.get(k) { result["max_tokens"] = v.clone(); break; }
    }
    for k in ["temperature", "top_p"] {
        if let Some(v) = chat.get(k) { result[k] = v.clone(); }
    }
    if let Some(v) = chat.get("stop") { result["stop_sequences"] = v.clone(); }
    if let Some(v) = chat.get("stream") { result["stream"] = v.clone(); }

    // reasoning_effort → thinking 块（学 cc-switch effort_to_thinking_budget：2048/8192/16384/24576）
    if let Some(effort) = chat.get("reasoning_effort").and_then(|v| v.as_str()) {
        let budget = match effort.trim().to_ascii_lowercase().as_str() {
            "minimal" | "low" => 2048,
            "medium" => 8192,
            "high" => 16384,
            "xhigh" | "max" | "ultra" => 24576,
            _ => 8192, // 未知默认 medium 档
        };
        result["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    }

    // tools（function）→ anthropic tools
    if let Some(tools) = chat.get("tools").and_then(|v| v.as_array()) {
        let anth_tools: Vec<Value> = tools.iter().filter_map(|t| {
            if t.get("type").and_then(|v| v.as_str()) != Some("function") { return None; }
            let f = t.get("function")?;
            let mut at = json!({
                "name": f.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                "input_schema": f.get("parameters").cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}})),
            });
            if let Some(d) = f.get("description").filter(|d| !d.is_null()) {
                at["description"] = d.clone();
            }
            Some(json!({"type": "tool", "name": at["name"].clone(),
                "description": at.get("description").cloned().unwrap_or(Value::Null),
                "input_schema": at["input_schema"].clone()}))
        }).collect();
        if !anth_tools.is_empty() { result["tools"] = json!(anth_tools); }
    }

    if let Some(v) = chat.get("tool_choice") {
        result["tool_choice"] = map_tool_choice_to_anthropic(v);
    }
    Ok(result)
}

/// chat 消息 → anthropic 消息（content 块数组）
pub(crate) fn chat_message_to_anthropic(msg: &Value, role: &str) -> Value {
    let mut content: Vec<Value> = Vec::new();
    // reasoning_content → thinking 块
    if role == "assistant" {
        if let Some(rc) = msg.get("reasoning_content").and_then(|v| v.as_str()) {
            if !rc.is_empty() { content.push(json!({"type": "thinking", "thinking": rc})); }
        }
    }
    // content 块
    let c = msg.get("content");
    match c {
        Some(Value::String(s)) => {
            if !s.is_empty() { content.push(json!({"type": "text", "text": s})); }
        }
        Some(Value::Array(parts)) => {
            for part in parts {
                match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "text" | "output_text" => {
                        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                            if !t.is_empty() { content.push(json!({"type": "text", "text": t})); }
                        }
                    }
                    "image_url" => {
                        if let Some(url) = part.get("image_url").and_then(|v| v.get("url")).and_then(|v| v.as_str()) {
                            if let Some(block) = image_url_to_anthropic(url) { content.push(block); }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    // assistant tool_calls → tool_use 块
    if role == "assistant" {
        if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}");
                let input: Value = serde_json::from_str(args).unwrap_or(json!({}));
                content.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
            }
        }
    }
    // tool 消息 → tool_result 块
    if role == "tool" {
        let tool_use_id = msg.get("tool_call_id").and_then(|v| v.as_str()).unwrap_or("");
        let content_str = match c {
            Some(Value::String(s)) => s.clone(),
            Some(v) => canonical_json_string(v),
            None => String::new(),
        };
        return json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": content_str}]});
    }
    let mut out = json!({"role": role});
    if content.is_empty() {
        out["content"] = json!([{"type": "text", "text": ""}]);
    } else if content.len() == 1 && content[0].get("text").is_some() {
        out["content"] = json!([{"type": "text", "text": content[0]["text"].clone()}]);
    } else {
        out["content"] = json!(content);
    }
    out
}

/// chat image_url data URL → anthropic image 块（source base64）
pub(crate) fn image_url_to_anthropic(url: &str) -> Option<Value> {
    let rest = url.strip_prefix("data:")?;
    let (media, data) = rest.split_once(',')?;
    let media_type = media.split(';').next().unwrap_or("image/png");
    Some(json!({"type": "image", "source": {
        "type": "base64", "media_type": media_type, "data": data}}))
}

pub(crate) fn map_tool_choice_to_anthropic(tc: &Value) -> Value {
    match tc {
        Value::String(s) => match s.as_str() {
            "required" => json!("any"),
            _ => json!(s),
        },
        Value::Object(obj) => {
            match obj.get("type").and_then(|v| v.as_str()) {
                Some("function") => {
                    let name = obj.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                    json!({"type": "tool", "name": name})
                }
                _ => tc.clone(),
            }
        }
        _ => tc.clone(),
    }
}

pub(crate) fn chat_text_of(msg: &Value) -> Option<String> {
    match msg.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) => {
            let texts: Vec<String> = parts.iter().filter_map(|p| p.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())).collect();
            if texts.is_empty() { None } else { Some(texts.join("\n")) }
        }
        _ => None,
    }
}