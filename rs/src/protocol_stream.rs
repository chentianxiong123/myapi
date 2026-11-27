//! 协议流式 SSE 转换层

//! 流式 SSE 转换（学 cc-switch proxy/providers/streaming.rs + streaming_responses.rs）。
//! 上游返回 OpenAI Chat SSE，网关转成 Anthropic SSE（Claude 客户端）或
//! Responses SSE（Codex 客户端）。有状态状态机：逐 data 行 push，返回
//! 目标协议 SSE 行（"event: X\ndata: {...}" 格式，以 "\n\n" 结束）。
//!
//! 第一版覆盖主流路径：文本 + thinking（reasoning_content）+ 工具调用 + usage 三桶。

use serde_json::{json, Value};




mod sse_helpers;
pub(crate) use sse_helpers::*;
mod anthropic_sse;
pub(crate) use anthropic_sse::*;
mod responses_sse;
pub(crate) use responses_sse::*;
mod upstream_sse;
pub(crate) use upstream_sse::*;

#[cfg(test)]
mod tests;