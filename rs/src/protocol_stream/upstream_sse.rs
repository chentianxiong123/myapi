//! upstream_sse（精确拆分）
use super::*;

/// anthropic SSE data 行 → chat SSE data 行（上游 Claude 流式，myapi 内部转回 chat）。
pub struct AnthropicUpstreamSseState {
    pub(crate) cur_tool: Option<Value>,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_creation: u64,
    pub(crate) stop_reason: Option<String>,
}

impl AnthropicUpstreamSseState {
    pub fn new() -> Self {
        Self { cur_tool: None, input_tokens: 0, output_tokens: 0, cache_read: 0, cache_creation: 0, stop_reason: None }
    }
    pub fn usage(&self) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
        (Some(self.input_tokens as i64), Some(self.output_tokens as i64),
         Some(self.cache_read as i64), Some(self.cache_creation as i64))
    }
    pub(crate) fn chat_delta(delta: &Value) -> String {
        format!("data: {}\n\n", serde_json::to_string(&json!({
            "choices": [{"delta": delta.clone(), "index": 0}]
        })).unwrap_or_default())
    }
}

impl SseTransformer for AnthropicUpstreamSseState {
    fn push(&mut self, data: &str) -> Vec<String> {
        let mut out = Vec::new();
        let trimmed = data.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" { return out; }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else { return out; };
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "content_block_start" => {
                if v.get("content_block").and_then(|b| b.get("type")).and_then(|t| t.as_str()) == Some("tool_use") {
                    let cb = v.get("content_block").cloned().unwrap_or(json!({}));
                    self.cur_tool = Some(json!({
                        "index": v.get("index").and_then(|i| i.as_u64()).unwrap_or(0),
                        "id": cb.get("id").and_then(|i| i.as_str()).unwrap_or(""),
                        "name": cb.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                        "arguments": "",
                    }));
                }
            }
            "content_block_delta" => {
                if let Some(delta) = v.get("delta") {
                    match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "text_delta" => {
                            if let Some(t) = delta.get("text").and_then(|x| x.as_str()) {
                                out.push(Self::chat_delta(&json!({"content": t})));
                            }
                        }
                        "thinking_delta" => {
                            if let Some(t) = delta.get("thinking").and_then(|x| x.as_str()) {
                                out.push(Self::chat_delta(&json!({"reasoning_content": t})));
                            }
                        }
                        "input_json_delta" => {
                            if let Some(partial) = delta.get("partial_json").and_then(|x| x.as_str()) {
                                if let Some(tool) = self.cur_tool.as_mut() {
                                    let cur = tool.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                                    tool["arguments"] = json!(format!("{cur}{partial}"));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                if let Some(tool) = self.cur_tool.take() {
                    let idx = tool.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                    let id = tool.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let args = tool.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
                    out.push(Self::chat_delta(&json!({
                        "tool_calls": [{"index": idx, "id": id, "type": "function",
                            "function": {"name": name, "arguments": args}}]
                    })));
                }
            }
            "message_delta" => {
                if let Some(u) = v.get("usage") {
                    self.input_tokens = u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                    self.output_tokens = u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                    self.cache_read = u.get("cache_read_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                    self.cache_creation = u.get("cache_creation_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
                }
                if let Some(delta) = v.get("delta") {
                    if let Some(sr) = delta.get("stop_reason").and_then(|x| x.as_str()) {
                        self.stop_reason = Some(sr.to_string());
                    }
                }
                // 流式 usage 单独 chunk（chat 格式），下游 SseUsageTap 能抓到
                out.push(Self::chat_delta(&json!({})));
                let usage_json = json!({
                    "prompt_tokens": self.input_tokens + self.cache_read + self.cache_creation,
                    "completion_tokens": self.output_tokens,
                    "total_tokens": self.input_tokens + self.output_tokens + self.cache_read + self.cache_creation,
                });
                out.push(format!("data: {}\n\n", serde_json::to_string(&json!({
                    "choices": [{"delta": {}, "index": 0}], "usage": usage_json
                })).unwrap_or_default()));
            }
            "error" => {
                if let Some(e) = v.get("error") {
                    out.push(format!("data: {}\n\n", serde_json::to_string(&json!({
                        "error": e.clone()
                    })).unwrap_or_default()));
                }
            }
            _ => {}
        }
        out
    }
    fn finish(&mut self) -> Vec<String> {
        // 残余工具块（无 content_block_stop）
        let mut out = Vec::new();
        if let Some(tool) = self.cur_tool.take() {
            let idx = tool.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
            let id = tool.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = tool.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
            out.push(Self::chat_delta(&json!({
                "tool_calls": [{"index": idx, "id": id, "type": "function",
                    "function": {"name": name, "arguments": args}}]
            })));
        }
        out.push("data: [DONE]\n\n".to_string());
        out
    }
}

/// 上游 anthropic SSE → chat SSE 转换流（gateway forward_one 流式路径使用）。
/// 逐 data 行转 chat SSE 输出，同时抓 anthropic usage（message_delta），流结束发给后台补记日志。
pub struct AnthropicUpstreamTap {
    pub(crate) inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    pub(crate) state: AnthropicUpstreamSseState,
    pub(crate) buf: String,
    pub(crate) usage_tx: Option<tokio::sync::oneshot::Sender<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>>,
    pub(crate) finished: bool,
}

impl AnthropicUpstreamTap {
    pub fn new(
        stream: impl futures_core::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
        usage_tx: tokio::sync::oneshot::Sender<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>,
    ) -> Self {
        Self { inner: Box::pin(stream), state: AnthropicUpstreamSseState::new(), buf: String::new(), usage_tx: Some(usage_tx), finished: false }
    }
}

impl futures_core::Stream for AnthropicUpstreamTap {
    type Item = Result<bytes::Bytes, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = &mut *self;
        loop {
            if let Some(pos) = this.buf.find('\n') {
                let line = this.buf[..pos].trim().to_string();
                this.buf = this.buf[pos + 1..].to_string();
                if let Some(data) = line.strip_prefix("data:") {
                    let out = this.state.push(data.trim());
                    if !out.is_empty() {
                        return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                    }
                }
                continue;
            }
            match futures_core::Stream::poll_next(this.inner.as_mut(), cx) {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    this.buf.push_str(&String::from_utf8_lossy(&chunk));
                }
                std::task::Poll::Ready(Some(Err(e))) => {
                    return std::task::Poll::Ready(Some(Err(std::io::Error::other(e.to_string()))));
                }
                std::task::Poll::Ready(None) => {
                    if let Some(tx) = this.usage_tx.take() {
                        let _ = tx.send(this.state.usage());
                    }
                    // 流结束：输出收尾（残余工具块 + [DONE]），只一次
                    if !this.finished {
                        this.finished = true;
                        let out = this.state.finish();
                        if !out.is_empty() {
                            return std::task::Poll::Ready(Some(Ok(bytes::Bytes::from(out.join("")))));
                        }
                    }
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}