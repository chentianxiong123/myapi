//! 流式 SSE 转换（学 cc-switch proxy/providers/streaming.rs + streaming_responses.rs）。
//! 上游返回 OpenAI Chat SSE，网关转成 Anthropic SSE（Claude 客户端）或
//! Responses SSE（Codex 客户端）。有状态状态机：逐 data 行 push，返回
//! 目标协议 SSE 行（"event: X\ndata: {...}" 格式，以 "\n\n" 结束）。
//!
//! 第一版覆盖主流路径：文本 + thinking（reasoning_content）+ 工具调用 + usage 三桶。

use serde_json::{json, Value};

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn sse(event: &str, data: &Value) -> String {
    format!("event: {event}\ndata: {}\n\n", serde_json::to_string(data).unwrap_or_default())
}

/// chat usage → Anthropic 三桶 usage（学 cc-switch build_anthropic_usage_json：
/// input_tokens = prompt - cache_read - cache_creation）
fn anthropic_usage(input: u64, output: u64, cache_read: u64, cache_creation: u64) -> Value {
    let mut u = json!({
        "input_tokens": input.saturating_sub(cache_read).saturating_sub(cache_creation),
        "output_tokens": output,
    });
    if cache_read > 0 { u["cache_read_input_tokens"] = json!(cache_read); }
    if cache_creation > 0 { u["cache_creation_input_tokens"] = json!(cache_creation); }
    u
}

fn anthropic_stop_reason(finish: Option<&str>) -> String {
    match finish {
        Some("tool_calls") => "tool_use".to_string(),
        Some("length") => "max_tokens".to_string(),
        Some("stop") => "end_turn".to_string(),
        Some("content_filter") => "end_turn".to_string(),
        Some(other) => other.to_string(),
        None => "end_turn".to_string(),
    }
}

// ---- Chat SSE → Anthropic SSE ----

pub struct AnthropicSseState {
    message_id: String,
    model: String,
    next_index: u32,
    started: bool,
    cur_text_open: bool,
    cur_thinking_open: bool,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_creation: u64,
    stop_reason: Option<String>,
    tool_calls: Vec<Value>, // 累积的 chat tool_calls
}

impl AnthropicSseState {
    pub fn new() -> Self {
        Self {
            message_id: format!("msg_{}", now_secs()),
            model: String::new(),
            next_index: 0,
            started: false,
            cur_text_open: false,
            cur_thinking_open: false,
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_creation: 0,
            stop_reason: None,
            tool_calls: Vec::new(),
        }
    }

    /// 处理一条 chat SSE data（不含 "data: " 前缀）。返回 anthropic SSE 行。
    pub fn push(&mut self, data: &str) -> Vec<String> {
        let mut out = Vec::new();
        let trimmed = data.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" {
            return Vec::new(); // finish 时单独处理
        }
        let Ok(chunk) = serde_json::from_str::<Value>(trimmed) else { return out; };

        if self.model.is_empty() {
            if let Some(m) = chunk.get("model").and_then(|v| v.as_str()) { self.model = m.to_string(); }
        }
        // usage 累积
        if let Some(u) = chunk.get("usage") {
            self.input_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.output_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.cache_read = u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            self.cache_creation = 0; // OpenAI chat usage 无独立 cache_creation 字段
        }
        let Some(choice) = chunk.get("choices").and_then(|v| v.as_array()).and_then(|a| a.first()) else { return out; };
        if self.stop_reason.is_none() {
            if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                self.stop_reason = Some(fr.to_string());
            }
        }
        let Some(delta) = choice.get("delta") else { return out; };

        // thinking（reasoning_content）→ thinking 块
        if let Some(rc) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !rc.is_empty() {
                if !self.started { self.emit_start(&mut out); }
                if !self.cur_thinking_open {
                    let idx = self.next_index; self.next_index += 1;
                    out.push(sse("content_block_start", &json!({
                        "type": "content_block_start", "index": idx,
                        "content_block": {"type": "thinking", "thinking": ""}
                    })));
                    self.cur_thinking_open = true;
                }
                out.push(sse("content_block_delta", &json!({
                    "type": "content_block_delta", "index": self.next_index - 1,
                    "delta": {"type": "thinking_delta", "thinking": rc}
                })));
            }
        }
        // 文本 → text 块
        if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                if !self.started { self.emit_start(&mut out); }
                if !self.cur_text_open {
                    let idx = self.next_index; self.next_index += 1;
                    out.push(sse("content_block_start", &json!({
                        "type": "content_block_start", "index": idx,
                        "content_block": {"type": "text", "text": ""}
                    })));
                    self.cur_text_open = true;
                }
                out.push(sse("content_block_delta", &json!({
                    "type": "content_block_delta", "index": self.next_index - 1,
                    "delta": {"type": "text_delta", "text": text}
                })));
            }
        }
        // 工具调用 → 累积（finish 时一次性输出）
        if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            if !tcs.is_empty() {
                if !self.started { self.emit_start(&mut out); }
                for tc in tcs {
                    let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    while self.tool_calls.len() <= idx { self.tool_calls.push(json!({})); }
                    let slot = &mut self.tool_calls[idx];
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) { slot["id"] = json!(id); }
                    if let Some(name) = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()) {
                        slot["name"] = json!(name);
                    }
                    if let Some(args) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()) {
                        let cur = slot.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                        slot["arguments"] = json!(format!("{cur}{args}"));
                    }
                }
            }
        }
        out
    }

    fn emit_start(&mut self, out: &mut Vec<String>) {
        if self.started { return; }
        self.started = true;
        let model = if self.model.is_empty() { "unknown".to_string() } else { self.model.clone() };
        out.push(sse("message_start", &json!({
            "type": "message_start",
            "message": {"id": self.message_id, "type": "message", "role": "assistant",
                "content": [], "model": model, "stop_reason": Value::Null,
                "usage": {"input_tokens": 0, "output_tokens": 0}}
        })));
    }

    /// 流结束（收到 [DONE] 或上游断流）。返回收尾 SSE 行。
    pub fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.started {
            // 空流（无内容）：仍发 message_start + message_stop
            self.emit_start(&mut out);
        }
        // 关文本块
        if self.cur_text_open {
            out.push(sse("content_block_stop", &json!({
                "type": "content_block_stop", "index": self.next_index - 1
            })));
            self.cur_text_open = false;
        }
        if self.cur_thinking_open {
            out.push(sse("content_block_stop", &json!({
                "type": "content_block_stop", "index": self.next_index - 1
            })));
            self.cur_thinking_open = false;
        }
        // 工具块（累积完一次性输出）
        for tc in &self.tool_calls {
            let idx = self.next_index; self.next_index += 1;
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args_str = tc.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            out.push(sse("content_block_start", &json!({
                "type": "content_block_start", "index": idx,
                "content_block": {"type": "tool_use", "id": id, "name": name, "input": input}
            })));
            out.push(sse("content_block_stop", &json!({
                "type": "content_block_stop", "index": idx
            })));
        }
        // message_delta（usage + stop_reason）
        let stop = anthropic_stop_reason(self.stop_reason.as_deref());
        out.push(sse("message_delta", &json!({
            "type": "message_delta", "delta": {"stop_reason": stop},
            "usage": anthropic_usage(self.input_tokens, self.output_tokens, self.cache_read, self.cache_creation)
        })));
        out.push(sse("message_stop", &json!({"type": "message_stop"})));
        out
    }
}

// ---- Chat SSE → Responses SSE ----

pub struct ResponsesSseState {
    response_id: String,
    model: String,
    message_id: String,
    output_index: u32,
    content_index: u32,
    text: String,
    started: bool,
    finished: bool,
    input_tokens: u64,
    output_tokens: u64,
}

impl ResponsesSseState {
    pub fn new() -> Self {
        Self {
            response_id: format!("resp_{}", now_secs()),
            model: String::new(),
            message_id: format!("msg_{}", now_secs()),
            output_index: 0,
            content_index: 0,
            text: String::new(),
            started: false,
            finished: false,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    pub fn push(&mut self, data: &str) -> Vec<String> {
        let mut out = Vec::new();
        let trimmed = data.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" { return Vec::new(); }
        let Ok(chunk) = serde_json::from_str::<Value>(trimmed) else { return out; };
        if self.model.is_empty() {
            if let Some(m) = chunk.get("model").and_then(|v| v.as_str()) { self.model = m.to_string(); }
        }
        if let Some(u) = chunk.get("usage") {
            self.input_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.output_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        }
        let Some(choice) = chunk.get("choices").and_then(|v| v.as_array()).and_then(|a| a.first()) else { return out; };
        let Some(delta) = choice.get("delta") else { return out; };
        if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                if !self.started { self.emit_start(&mut out); }
                self.text.push_str(text);
                out.push(sse("response.output_text.delta", &json!({
                    "type": "response.output_text.delta", "item_id": self.message_id,
                    "output_index": self.output_index, "content_index": self.content_index, "delta": text
                })));
            }
        }
        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            if fr != "stop" && fr != "tool_calls" {
                self.finished = true;
            }
        }
        out
    }

    fn emit_start(&mut self, out: &mut Vec<String>) {
        if self.started { return; }
        self.started = true;
        let model = if self.model.is_empty() { "unknown".to_string() } else { self.model.clone() };
        out.push(sse("response.created", &json!({
            "type": "response.created",
            "response": {"id": self.response_id, "object": "response", "status": "in_progress",
                "model": model, "output": []}
        })));
        out.push(sse("response.output_item.added", &json!({
            "type": "response.output_item.added", "output_index": self.output_index,
            "item": {"type": "message", "id": self.message_id, "status": "in_progress",
                "role": "assistant", "content": []}
        })));
        out.push(sse("response.content_part.added", &json!({
            "type": "response.content_part.added", "item_id": self.message_id,
            "output_index": self.output_index, "content_index": self.content_index,
            "part": {"type": "output_text", "text": ""}
        })));
    }

    pub fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.started {
            self.emit_start(&mut out);
        }
        let model = if self.model.is_empty() { "unknown".to_string() } else { self.model.clone() };
        // 关消息项
        out.push(sse("response.output_text.done", &json!({
            "type": "response.output_text.done", "item_id": self.message_id,
            "output_index": self.output_index, "content_index": self.content_index, "text": self.text
        })));
        out.push(sse("response.content_part.done", &json!({
            "type": "response.content_part.done", "item_id": self.message_id,
            "output_index": self.output_index, "content_index": self.content_index,
            "part": {"type": "output_text", "text": self.text}
        })));
        out.push(sse("response.output_item.done", &json!({
            "type": "response.output_item.done", "output_index": self.output_index,
            "item": {"type": "message", "id": self.message_id, "status": "completed",
                "role": "assistant", "content": [{"type": "output_text", "text": self.text}]}
        })));
        // completed / incomplete
        let status = if self.finished { "incomplete" } else { "completed" };
        out.push(sse("response.completed", &json!({
            "type": "response.completed",
            "response": {"id": self.response_id, "object": "response", "status": status,
                "model": model,
                "output": [{"type": "message", "id": self.message_id, "status": "completed",
                    "role": "assistant", "content": [{"type": "output_text", "text": self.text}]}],
                "usage": {"input_tokens": self.input_tokens, "output_tokens": self.output_tokens,
                    "total_tokens": self.input_tokens + self.output_tokens}}
        })));
        out
    }
}

/// 统一 SSE 转换接口（main.rs 的 proto_stream_reply 使用）。
pub trait SseTransformer {
    fn push(&mut self, data: &str) -> Vec<String>;
    fn finish(&mut self) -> Vec<String>;
}

impl SseTransformer for AnthropicSseState {
    fn push(&mut self, data: &str) -> Vec<String> { AnthropicSseState::push(self, data) }
    fn finish(&mut self) -> Vec<String> { AnthropicSseState::finish(self) }
}

impl SseTransformer for ResponsesSseState {
    fn push(&mut self, data: &str) -> Vec<String> { ResponsesSseState::push(self, data) }
    fn finish(&mut self) -> Vec<String> { ResponsesSseState::finish(self) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_sse_to_anthropic_text() {
        let mut st = AnthropicSseState::new();
        let mut out = Vec::new();
        out.extend(st.push(r#"{"choices":[{"delta":{"content":"你好"},"index":0}],"model":"m"}"#));
        out.extend(st.push(r#"{"choices":[{"delta":{"content":"，世界"},"index":0}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#));
        out.extend(st.finish());
        let joined = out.join("\n");
        assert!(joined.contains("message_start"));
        assert!(joined.contains("content_block_start"));
        assert!(joined.contains("text_delta"));
        assert!(joined.contains("你好"));
        assert!(joined.contains("message_delta"));
        assert!(joined.contains("message_stop"));
        // 三桶 usage：input=10, output=5（无 cache）
        assert!(joined.contains("\"input_tokens\":10"));
        assert!(joined.contains("\"output_tokens\":5"));
    }

    #[test]
    fn chat_sse_to_anthropic_tool_call() {
        let mut st = AnthropicSseState::new();
        let mut out = Vec::new();
        out.extend(st.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"search","arguments":""}}]},"index":0}]}"#));
        out.extend(st.push(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"q\":\"r\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#));
        out.extend(st.finish());
        let joined = out.join("\n");
        assert!(joined.contains("tool_use"));
        assert!(joined.contains("search"));
        assert!(joined.contains("message_delta"));
        assert!(joined.contains("\"stop_reason\":\"tool_use\""));
    }

    #[test]
    fn chat_sse_to_responses_text() {
        let mut st = ResponsesSseState::new();
        let mut out = Vec::new();
        out.extend(st.push(r#"{"choices":[{"delta":{"content":"hi"},"index":0}],"model":"m"}"#));
        out.extend(st.finish());
        let joined = out.join("\n");
        assert!(joined.contains("response.created"));
        assert!(joined.contains("response.output_text.delta"));
        assert!(joined.contains("\"delta\":\"hi\""));
        assert!(joined.contains("response.output_text.done"));
        assert!(joined.contains("response.completed"));
        assert!(joined.contains("\"status\":\"completed\""));
    }
}
