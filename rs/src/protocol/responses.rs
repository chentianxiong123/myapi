//! responses（精确拆分）
use super::*;

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
    // 思考模式：responses `reasoning: {effort}` → chat `reasoning_effort`（学 anthropic thinking 映射）
    if let Some(effort) = body.get("reasoning").and_then(|v| v.get("effort")).and_then(|v| v.as_str()) {
        result["reasoning_effort"] = json!(effort);
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
pub(crate) fn append_responses_input(input: &Value, messages: &mut Vec<Value>) -> Result<(), String> {
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
pub(crate) fn append_responses_item(item: &Value, messages: &mut Vec<Value>) -> Result<(), String> {
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
pub(crate) fn responses_content_to_chat(content: &Value) -> Value {
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
pub(crate) fn chat_content_to_responses_content(content: &Value) -> Value {
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

pub(crate) fn instruction_text(ins: &Value) -> String {
    match ins {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr.iter().filter_map(|i| i.get("text").and_then(|v| v.as_str())).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}