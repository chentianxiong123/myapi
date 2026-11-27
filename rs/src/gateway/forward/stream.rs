//! stream（精确拆分）
use super::super::*;

/// 转发失败分类（学 LiteLLM AttemptError）：区分可重试/不可重试
#[derive(Debug, Clone)]
pub(crate) enum ForwardError {
    /// 可重试：连接失败 / 408 409 429 / 5xx。重试退避后可试其他通道。
    Retryable { status: Option<u16>, retry_after: Option<std::time::Duration>, msg: String },
    /// 不可重试：其他 4xx（鉴权/参数/找不到）。重试也没用，直接透传。
    Fatal { status: Option<u16>, msg: String },
    /// 上下文不足（context_length_exceeded）：模型窗口装不下请求。可换更大窗口通道重试，豁免 max_retries。
    Context { status: Option<u16>, msg: String },
}

impl ForwardError {
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, ForwardError::Retryable { .. })
    }
    pub(crate) fn is_context(&self) -> bool {
        matches!(self, ForwardError::Context { .. })
    }
    pub(crate) fn status(&self) -> Option<u16> {
        match self {
            ForwardError::Retryable { status, .. } | ForwardError::Fatal { status, .. } | ForwardError::Context { status, .. } => *status,
        }
    }
    pub(crate) fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            ForwardError::Retryable { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
    pub(crate) fn message(&self) -> String {
        match self {
            ForwardError::Retryable { msg, .. } | ForwardError::Fatal { msg, .. } | ForwardError::Context { msg, .. } => msg.clone(),
        }
    }
}

/// SSE 流式 usage 捕获：包装字节流，逐 chunk 透传客户端的同时累积缓冲，
/// 扫描 `data:` 行里的 usage token（OpenAI 最后 chunk / Anthropic message_delta）。
/// 流结束后把 (input_tokens, output_tokens) 通过 oneshot channel 发给后台任务补记日志。
/// 流式 SSE 的 model 字段改写为档位名（客户端配的 model 名，对齐非流式回显）。
pub(crate) struct ModelRewriteStream {
    pub(crate) inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>>,
    pub(crate) model: String,
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
pub(crate) fn rewrite_chunk_model(chunk: &[u8], model: &str) -> Vec<u8> {
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

pub(crate) struct SseUsageTap {
    pub(crate) inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    pub(crate) buf: Vec<u8>,
    pub(crate) usage: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
    pub(crate) usage_tx: Option<tokio::sync::oneshot::Sender<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>>,
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