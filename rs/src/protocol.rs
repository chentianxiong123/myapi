//! 协议转换层（学 cc-switch proxy/providers/transform.rs）。
//! myapi 内部统一 OpenAI Chat Completions 协议；客户端 Anthropic/Responses 转进来，
//! 上游 Anthropic/Responses 再转出去。纯函数 JSON→JSON，无 IO。

use serde_json::{json, Value};

/// Anthropic 请求 → OpenAI Chat 请求（学 cc-switch `anthropic_to_openai`）。
/// 模型映射由路由层处理，这里只做结构转换。
pub fn anthropic_to_openai(body: &Value) -> Result<Value, String> {
    let mut result = json!({});
    if let Some(m) = body.get("model").and_then(|v| v.as_str()) {
        result["model"] = json!(m);
    }

    let mut messages: Vec<Value> = Vec::new();
    // system → 第一条 system 消息（string 或数组合并，跨轮字节稳定利于前缀缓存）
    if let Some(sys) = body.get("system") {
        if let Some(text) = sys.as_str() {
            if !text.is_empty() {
                messages.push(json!({"role": "system", "content": text}));
            }
        } else if let Some(arr) = sys.as_array() {
            let mut parts: Vec<String> = Vec::new();
            for m in arr {
                if let Some(t) = m.get("text").and_then(|v| v.as_str()) {
                    if !t.is_empty() { parts.push(t.to_string()); }
                }
            }
            if !parts.is_empty() {
                messages.push(json!({"role": "system", "content": parts.join("\n")}));
            }
        }
    }

    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            if let Some(content) = msg.get("content") {
                messages.extend(convert_anthropic_message(role, content)?);
            }
        }
    }
    result["messages"] = json!(messages);

    for k in ["max_tokens", "temperature", "top_p"] {
        if let Some(v) = body.get(k) { result[k] = v.clone(); }
    }
    if let Some(v) = body.get("stop_sequences") { result["stop"] = v.clone(); }
    if let Some(v) = body.get("stream") { result["stream"] = v.clone(); }

    // Anthropic thinking → OpenAI reasoning_effort（学 cc-switch resolve_reasoning_effort）
    if let Some(effort) = resolve_reasoning_effort(body) {
        result["reasoning_effort"] = json!(effort);
    }

    // tools（过滤 BatchTool + clean_schema）
    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        let openai_tools: Vec<Value> = tools.iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) != Some("BatchTool"))
            .map(|t| {
                let mut f = json!({
                    "name": t.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                });
                if let Some(d) = t.get("description").filter(|d| !d.is_null()) {
                    f["description"] = d.clone();
                }
                f["parameters"] = clean_schema(t.get("input_schema").cloned().unwrap_or(json!({})));
                json!({"type": "function", "function": f})
            }).collect();
        if !openai_tools.is_empty() { result["tools"] = json!(openai_tools); }
    }

    if let Some(v) = body.get("tool_choice") {
        result["tool_choice"] = map_tool_choice_to_chat(v);
    }
    Ok(result)
}

/// OpenAI Chat 响应 → Anthropic 响应（学 cc-switch `openai_to_anthropic`）。
pub fn openai_to_anthropic(body: &Value) -> Result<Value, String> {
    let choice = body.get("choices").and_then(|v| v.as_array()).and_then(|a| a.first())
        .ok_or_else(|| "chat 响应缺 choices".to_string())?;
    let message = choice.get("message").ok_or_else(|| "chat 响应缺 message".to_string())?;

    let mut content: Vec<Value> = Vec::new();
    // DeepSeek 系 reasoning_content → thinking 块
    if let Some(rc) = message.get("reasoning_content").and_then(|v| v.as_str()) {
        if !rc.is_empty() {
            content.push(json!({"type": "thinking", "thinking": rc}));
        }
    }
    // 文本/拒绝（正文开头内联 </think> 拆 thinking，学 cc-switch split_leading_think_block）
    let mut leading_text = true;
    let mut push_text = |content: &mut Vec<Value>, text: &str| {
        let split = if leading_text { leading_text = false; split_leading_think_block(text) } else { None };
        let Some((thinking, answer)) = split else {
            content.push(json!({"type": "text", "text": text}));
            return;
        };
        if !thinking.is_empty() {
            content.push(json!({"type": "thinking", "thinking": thinking}));
        }
        if !answer.is_empty() {
            content.push(json!({"type": "text", "text": answer}));
        }
    };
    if let Some(msg_content) = message.get("content") {
        if let Some(text) = msg_content.as_str() {
            if !text.is_empty() { push_text(&mut content, text); }
        } else if let Some(parts) = msg_content.as_array() {
            for part in parts {
                match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "text" | "output_text" => {
                        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                            if !t.is_empty() { push_text(&mut content, t); }
                        }
                    }
                    "refusal" => {
                        if let Some(r) = part.get("refusal").and_then(|v| v.as_str()) {
                            if !r.is_empty() { content.push(json!({"type": "text", "text": r})); }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(refusal) = message.get("refusal").and_then(|v| v.as_str()) {
        if !refusal.is_empty() {
            content.push(json!({"type": "text", "text": refusal}));
        }
    }
    // 工具调用 → tool_use 块
    if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("");
            let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}");
            let input: Value = serde_json::from_str(args).unwrap_or(json!({}));
            content.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
        }
    }

    let stop_reason = match choice.get("finish_reason").and_then(|v| v.as_str()) {
        Some("tool_calls") => "tool_use",
        Some("stop") => "end_turn",
        Some("length") => "max_tokens",
        Some(other) => other,
        None => "end_turn",
    };

    let mut result = json!({
        "content": content,
        "role": "assistant",
        "stop_reason": stop_reason,
    });
    if let Some(id) = body.get("id") { result["id"] = id.clone(); }
    if let Some(model) = body.get("model") { result["model"] = model.clone(); }
    if let Some(usage) = body.get("usage") { result["usage"] = chat_usage_to_anthropic(usage); }
    Ok(result)
}

/// chat usage → Anthropic 三桶 usage（学 cc-switch build_anthropic_usage_json）。
fn chat_usage_to_anthropic(u: &Value) -> Value {
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

// ---- 消息转换 ----

fn convert_anthropic_message(role: &str, content: &Value) -> Result<Vec<Value>, String> {
    let mut result: Vec<Value> = Vec::new();
    if let Some(text) = content.as_str() {
        result.push(json!({"role": role, "content": text}));
        return Ok(result);
    }
    if let Some(blocks) = content.as_array() {
        let mut content_parts: Vec<Value> = Vec::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        for block in blocks {
            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        content_parts.push(json!({"type": "text", "text": t}));
                    }
                }
                "image" => {
                    // source: {"type":"base64","media_type":"image/png","data":"..."}
                    if let Some(src) = block.get("source") {
                        if let (Some(media), Some(data)) = (
                            src.get("media_type").and_then(|v| v.as_str()),
                            src.get("data").and_then(|v| v.as_str()),
                        ) {
                            content_parts.push(json!({"type": "image_url", "image_url": {"url": format!("data:{media};base64,{data}")}}));
                        }
                    }
                }
                "tool_use" => {
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "id": id, "type": "function",
                        "function": {"name": name, "arguments": canonical_json_string(&input)},
                    }));
                }
                "tool_result" => {
                    let tool_use_id = block.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
                    let content_str = match block.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(v) => canonical_json_string(v),
                        None => String::new(),
                    };
                    result.push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": content_str}));
                }
                _ => {}
            }
        }
        if !content_parts.is_empty() || !tool_calls.is_empty() {
            let mut msg = json!({"role": role});
            if content_parts.is_empty() {
                msg["content"] = Value::Null;
            } else if content_parts.len() == 1 {
                if let Some(text) = content_parts[0].get("text") {
                    msg["content"] = text.clone();
                } else {
                    msg["content"] = json!(content_parts);
                }
            } else {
                msg["content"] = json!(content_parts);
            }
            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls);
            }
            result.push(msg);
        }
        return Ok(result);
    }
    result.push(json!({"role": role, "content": content.clone()}));
    Ok(result)
}

fn map_tool_choice_to_chat(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(s) => match s.as_str() {
            "any" => json!("required"),
            _ => json!(s),
        },
        Value::Object(obj) => match obj.get("type").and_then(|v| v.as_str()) {
            Some("any") => json!("required"),
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            Some("tool") => {
                let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                json!({"type": "function", "function": {"name": name}})
            }
            _ => tool_choice.clone(),
        },
        _ => tool_choice.clone(),
    }
}

// ---- schema / thinking 辅助 ----

fn clean_schema(mut schema: Value) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        if !obj.contains_key("type") {
            obj.insert("type".to_string(), json!("object"));
            if !obj.contains_key("properties") {
                obj.insert("properties".to_string(), json!({}));
            }
        }
        if obj.get("format").and_then(|v| v.as_str()) == Some("uri") {
            obj.remove("format");
        }
        if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
            for (_, value) in props.iter_mut() {
                *value = clean_schema(value.clone());
            }
        }
        if let Some(items) = obj.get_mut("items") {
            *items = clean_schema(items.clone());
        }
    }
    schema
}

fn canonical_json_string(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn resolve_reasoning_effort(body: &Value) -> Option<&'static str> {
    // Anthropic thinking 块 → reasoning_effort。学 cc-switch：budget_tokens 映射 high/medium/low。
    if let Some(arr) = body.get("thinking").and_then(|v| v.as_array()) {
        for t in arr {
            if t.get("type").and_then(|v| v.as_str()) == Some("enabled") {
                if let Some(budget) = t.get("budget_tokens").and_then(|v| v.as_u64()) {
                    return Some(match budget {
                        b if b > 32000 => "high",
                        b if b > 16000 => "medium",
                        _ => "low",
                    });
                }
                return Some("medium");
            }
        }
    }
    None
}

/// 拆分正文开头的 </think> 内联块（DeepSeek 系）。返回 (thinking, answer)。
fn split_leading_think_block(text: &str) -> Option<(String, String)> {
    let t = text.trim_start();
    if !t.starts_with("thinking") { return None; }
    let after_open = &t["thinking".len()..];
    let close = after_open.find("thinking")?;
    let thinking = after_open[..close].to_string();
    let answer = after_open[close + "thinking".len()..].to_string();
    Some((thinking, answer))
}

// ---- OpenAI Responses ↔ Chat（学 cc-switch transform_codex_chat.rs）----

/// OpenAI Responses 请求 → OpenAI Chat 请求（学 cc-switch `responses_to_chat_completions`）。
pub fn responses_to_chat(body: &Value) -> Result<Value, String> {
    let mut result = json!({});
    if let Some(m) = body.get("model") { result["model"] = m.clone(); }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(ins) = body.get("instructions") {
        let text = instruction_text(ins);
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }
    if let Some(input) = body.get("input") {
        append_responses_input(input, &mut messages)?;
    }
    result["messages"] = json!(messages);

    for (src, dst) in [("max_output_tokens", "max_tokens"), ("temperature", "temperature"), ("top_p", "top_p"), ("stream", "stream")] {
        if let Some(v) = body.get(src) { result[dst] = v.clone(); }
    }
    Ok(result)
}

/// OpenAI Chat 响应 → OpenAI Responses 响应（学 cc-switch `chat_completion_to_response`）。
pub fn chat_to_responses(body: &Value) -> Result<Value, String> {
    let choice = body.get("choices").and_then(|v| v.as_array()).and_then(|a| a.first())
        .ok_or_else(|| "chat 响应缺 choices".to_string())?;
    let message = choice.get("message").ok_or_else(|| "chat 响应缺 message".to_string())?;
    let response_id = body.get("id").and_then(|v| v.as_str())
        .map(|id| if id.starts_with("chatcmpl") { format!("resp_{}", id) } else { id.to_string() })
        .unwrap_or_else(|| format!("resp_{}", now_secs()));
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let created_at = body.get("created").and_then(|v| v.as_u64()).unwrap_or(0);
    let finish_reason = choice.get("finish_reason").and_then(|v| v.as_str());

    let mut output: Vec<Value> = Vec::new();
    // reasoning → reasoning 项
    if let Some(rc) = message.get("reasoning_content").and_then(|v| v.as_str()) {
        if !rc.is_empty() {
            output.push(json!({"type": "reasoning", "id": format!("rs_{}", now_secs()),
                "summary": [{"type": "summary_text", "text": rc}]}));
        }
    }
    // content → message 项
    let msg_content = message.get("content").cloned().unwrap_or(Value::Null);
    if !msg_content.is_null() {
        let content = chat_content_to_responses_content(&msg_content);
        output.push(json!({"type": "message", "id": format!("msg_{}", now_secs()),
            "status": "completed", "role": "assistant", "content": content}));
    }
    // tool_calls → function_call 项
    if let Some(tcs) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tcs {
            let call_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("");
            let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("{}");
            output.push(json!({"type": "function_call", "id": format!("fc_{}", now_secs()),
                "call_id": call_id, "name": name, "arguments": args}));
        }
    }

    let status = match finish_reason {
        Some("tool_calls") => "completed",
        Some("length") => "incomplete",
        _ => "completed",
    };
    let mut result = json!({
        "id": response_id, "object": "response", "created_at": created_at,
        "status": status, "model": model, "output": output,
    });
    if let Some(usage) = body.get("usage") {
        result["usage"] = json!({
            "input_tokens": usage.get("prompt_tokens").cloned().unwrap_or(Value::Null),
            "output_tokens": usage.get("completion_tokens").cloned().unwrap_or(Value::Null),
            "total_tokens": usage.get("total_tokens").cloned().unwrap_or(Value::Null),
        });
    }
    Ok(result)
}

/// Responses input（string / array / object）→ chat messages。
fn append_responses_input(input: &Value, messages: &mut Vec<Value>) -> Result<(), String> {
    match input {
        Value::String(text) => {
            messages.push(json!({"role": "user", "content": text}));
        }
        Value::Array(items) => {
            for item in items {
                append_responses_item(item, messages)?;
            }
        }
        Value::Object(_) => { append_responses_item(input, messages)?; }
        _ => {}
    }
    Ok(())
}

/// 单个 Responses item → chat 消息。
fn append_responses_item(item: &Value, messages: &mut Vec<Value>) -> Result<(), String> {
    match item.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "message" => {
            let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = item.get("content").cloned().unwrap_or(Value::Null);
            messages.push(json!({"role": role, "content": responses_content_to_chat(&content)}));
        }
        "function_call" => {
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = item.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            // 追加到上一条 assistant 消息的 tool_calls（或新建）
            if let Some(last) = messages.last_mut() {
                if last.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                    last["tool_calls"].as_array_mut().unwrap_or(&mut Vec::new()).push(json!({
                        "id": call_id, "type": "function",
                        "function": {"name": name, "arguments": args},
                    }));
                    return Ok(());
                }
            }
            messages.push(json!({"role": "assistant", "content": Value::Null,
                "tool_calls": [{"id": call_id, "type": "function",
                    "function": {"name": name, "arguments": args}}]}));
        }
        "function_call_output" => {
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            let output = item.get("output").cloned().unwrap_or(json!(""));
            let output_str = match output {
                Value::String(s) => s,
                v => v.to_string(),
            };
            messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": output_str}));
        }
        _ => {}
    }
    Ok(())
}

/// Responses content 块 → chat content。
fn responses_content_to_chat(content: &Value) -> Value {
    if content.is_null() || content.is_string() { return content.clone(); }
    let Some(parts) = content.as_array() else { return content.clone(); };
    let mut chat_parts: Vec<Value> = Vec::new();
    for part in parts {
        match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "input_text" | "output_text" | "text" => {
                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                    if !t.is_empty() { chat_parts.push(json!({"type": "text", "text": t})); }
                }
            }
            "input_image" => {
                if let Some(img) = part.get("image_url") {
                    let image_url = if img.is_object() { img.clone() } else { json!({"url": img.as_str().unwrap_or_default()}) };
                    chat_parts.push(json!({"type": "image_url", "image_url": image_url}));
                }
            }
            "refusal" => {
                if let Some(r) = part.get("refusal").and_then(|v| v.as_str()) {
                    if !r.is_empty() { chat_parts.push(json!({"type": "text", "text": r})); }
                }
            }
            _ => {}
        }
    }
    if chat_parts.is_empty() { return Value::Null; }
    if chat_parts.len() == 1 && chat_parts[0].get("text").is_some() {
        return chat_parts[0]["text"].clone();
    }
    json!(chat_parts)
}

/// chat content → Responses content 块。
fn chat_content_to_responses_content(content: &Value) -> Value {
    if content.is_null() { return json!([]); }
    if let Some(text) = content.as_str() {
        return json!([{"type": "output_text", "text": text}]);
    }
    let mut parts: Vec<Value> = Vec::new();
    if let Some(arr) = content.as_array() {
        for part in arr {
            match part.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text" => {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        parts.push(json!({"type": "output_text", "text": t}));
                    }
                }
                "image_url" => {
                    if let Some(url) = part.get("image_url").and_then(|v| v.get("url")).and_then(|v| v.as_str()) {
                        parts.push(json!({"type": "input_image", "image_url": url}));
                    }
                }
                _ => {}
            }
        }
    }
    json!(parts)
}

fn instruction_text(ins: &Value) -> String {
    match ins {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr.iter().filter_map(|i| i.get("text").and_then(|v| v.as_str())).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_basic_to_chat() {
        let body = json!({
            "model": "claude-3-5-sonnet",
            "system": "你是助手",
            "messages": [{"role": "user", "content": "你好"}],
            "max_tokens": 1000,
            "stream": true,
        });
        let out = anthropic_to_openai(&body).unwrap();
        assert_eq!(out["model"], "claude-3-5-sonnet");
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][0]["content"], "你是助手");
        assert_eq!(out["messages"][1]["role"], "user");
        assert_eq!(out["messages"][1]["content"], "你好");
        assert_eq!(out["max_tokens"], 1000);
        assert_eq!(out["stream"], true);
    }

    #[test]
    fn anthropic_tool_use_to_chat() {
        let body = json!({
            "model": "m",
            "messages": [{"role": "assistant", "content": [
                {"type": "tool_use", "id": "call_1", "name": "search", "input": {"q": "rust"}}
            ]}],
            "max_tokens": 100,
        });
        let out = anthropic_to_openai(&body).unwrap();
        let msg = &out["messages"][0];
        assert_eq!(msg["tool_calls"][0]["id"], "call_1");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "search");
        assert!(msg["tool_calls"][0]["function"]["arguments"].as_str().unwrap().contains("rust"));
    }

    #[test]
    fn anthropic_tool_result_to_chat() {
        let body = json!({
            "model": "m",
            "messages": [{"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": "结果"}
            ]}],
            "max_tokens": 100,
        });
        let out = anthropic_to_openai(&body).unwrap();
        assert_eq!(out["messages"][0]["role"], "tool");
        assert_eq!(out["messages"][0]["tool_call_id"], "call_1");
        assert_eq!(out["messages"][0]["content"], "结果");
    }

    #[test]
    fn chat_response_to_anthropic() {
        let body = json!({
            "id": "chatcmpl-1",
            "model": "m",
            "choices": [{"message": {"role": "assistant", "content": "你好"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5},
        });
        let out = openai_to_anthropic(&body).unwrap();
        assert_eq!(out["role"], "assistant");
        assert_eq!(out["content"][0]["type"], "text");
        assert_eq!(out["content"][0]["text"], "你好");
        assert_eq!(out["stop_reason"], "end_turn");
        assert_eq!(out["usage"]["input_tokens"], 10);
        assert_eq!(out["usage"]["output_tokens"], 5);
    }

    #[test]
    fn chat_tool_calls_to_anthropic() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": null,
                "tool_calls": [{"id": "call_1", "type": "function",
                    "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"}}]},
                "finish_reason": "tool_calls"}],
        });
        let out = openai_to_anthropic(&body).unwrap();
        assert_eq!(out["stop_reason"], "tool_use");
        assert_eq!(out["content"][0]["type"], "tool_use");
        assert_eq!(out["content"][0]["name"], "search");
        assert_eq!(out["content"][0]["input"]["q"], "rust");
    }

    #[test]
    fn inject_usage_on_stream() {
        let mut body = json!({"model": "m", "stream": true, "messages": []});
        inject_stream_include_usage(&mut body);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn responses_message_to_chat() {
        let body = json!({
            "model": "gpt-5",
            "instructions": "你是助手",
            "input": [{"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": "你好"}]}],
            "max_output_tokens": 1000,
        });
        let out = responses_to_chat(&body).unwrap();
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][0]["content"], "你是助手");
        assert_eq!(out["messages"][1]["role"], "user");
        assert_eq!(out["messages"][1]["content"], "你好");
        assert_eq!(out["max_tokens"], 1000);
    }

    #[test]
    fn responses_function_call_to_chat() {
        let body = json!({
            "model": "gpt-5",
            "input": [
                {"type": "function_call", "call_id": "call_1", "name": "search",
                 "arguments": "{\"q\":\"rust\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "结果"},
            ],
        });
        let out = responses_to_chat(&body).unwrap();
        assert_eq!(out["messages"][0]["role"], "assistant");
        assert_eq!(out["messages"][0]["tool_calls"][0]["function"]["name"], "search");
        assert_eq!(out["messages"][1]["role"], "tool");
        assert_eq!(out["messages"][1]["tool_call_id"], "call_1");
        assert_eq!(out["messages"][1]["content"], "结果");
    }

    #[test]
    fn chat_response_to_responses() {
        let body = json!({
            "id": "chatcmpl-1", "model": "m", "created": 123,
            "choices": [{"message": {"role": "assistant", "content": "你好"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5},
        });
        let out = chat_to_responses(&body).unwrap();
        assert_eq!(out["object"], "response");
        assert_eq!(out["status"], "completed");
        assert_eq!(out["output"][0]["type"], "message");
        assert_eq!(out["output"][0]["content"][0]["text"], "你好");
        assert_eq!(out["usage"]["input_tokens"], 10);
    }

    #[test]
    fn chat_tool_calls_to_responses() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": null,
                "tool_calls": [{"id": "call_1", "type": "function",
                    "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"}}]},
                "finish_reason": "tool_calls"}],
        });
        let out = chat_to_responses(&body).unwrap();
        assert_eq!(out["output"][0]["type"], "function_call");
        assert_eq!(out["output"][0]["name"], "search");
        assert_eq!(out["output"][0]["call_id"], "call_1");
    }
}
