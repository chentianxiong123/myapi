//! anthropic（精确拆分）
use super::*;

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

pub(crate) fn convert_anthropic_message(role: &str, content: &Value) -> Result<Vec<Value>, String> {
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

pub(crate) fn map_tool_choice_to_chat(tool_choice: &Value) -> Value {
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

pub(crate) fn clean_schema(mut schema: Value) -> Value {
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

pub(crate) fn canonical_json_string(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

/// 拆分正文开头的 </think> 内联块（DeepSeek 系）。返回 (thinking, answer)。
pub(crate) fn split_leading_think_block(text: &str) -> Option<(String, String)> {
    let t = text.trim_start();
    if !t.starts_with("thinking") { return None; }
    let after_open = &t["thinking".len()..];
    let close = after_open.find("thinking")?;
    let thinking = after_open[..close].to_string();
    let answer = after_open[close + "thinking".len()..].to_string();
    Some((thinking, answer))
}

/// anthropic 响应 → chat 响应（上游 Claude 返回，myapi 内部转回 chat）
pub fn anthropic_to_openai_response(anth: &Value) -> Result<Value, String> {
    let content = anth.get("content").and_then(|v| v.as_array())
        .cloned().unwrap_or_default();
    let mut msg_content: Vec<Value> = Vec::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for block in &content {
        match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    if !t.is_empty() { msg_content.push(json!({"type": "text", "text": t})); }
                }
            }
            "thinking" => {
                if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                    if !t.is_empty() { reasoning.push_str(t); }
                }
            }
            "tool_use" => {
                let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or(json!({}));
                tool_calls.push(json!({"id": id, "type": "function",
                    "function": {"name": name, "arguments": canonical_json_string(&input)}}));
            }
            _ => {}
        }
    }
    let content_value = if msg_content.is_empty() {
        Value::Null
    } else if msg_content.len() == 1 && msg_content[0].get("text").is_some() {
        msg_content[0]["text"].clone()
    } else {
        json!(msg_content)
    };
    let mut message = json!({"role": "assistant", "content": content_value});
    if !reasoning.is_empty() { message["reasoning_content"] = json!(reasoning); }
    if !tool_calls.is_empty() { message["tool_calls"] = json!(tool_calls); }

    let finish_reason = match anth.get("stop_reason").and_then(|v| v.as_str()) {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        Some("end_turn") => "stop",
        Some(other) => other,
        None => "stop",
    };

    let mut result = json!({
        "id": anth.get("id").cloned().unwrap_or_else(|| json!("chatcmpl-anthropic")),
        "object": "chat.completion",
        "model": anth.get("model").cloned().unwrap_or(Value::Null),
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
    });
    if let Some(usage) = anth.get("usage") {
        result["usage"] = anthropic_usage_to_chat(usage);
    }
    Ok(result)
}

/// anthropic usage → chat usage（三桶合并回 prompt）
pub(crate) fn anthropic_usage_to_chat(u: &Value) -> Value {
    let input = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_read = u.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_creation = u.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    json!({
        "prompt_tokens": input + cache_read + cache_creation,
        "completion_tokens": output,
        "total_tokens": input + output + cache_read + cache_creation,
    })
}