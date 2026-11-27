//! anthropic_sse（精确拆分）
use super::*;

pub struct AnthropicSseState {
    pub(crate) message_id: String,
    pub(crate) model: String,
    pub(crate) next_index: u32,
    pub(crate) started: bool,
    pub(crate) cur_text_open: bool,
    pub(crate) cur_thinking_open: bool,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_read: u64,
    pub(crate) cache_creation: u64,
    pub(crate) stop_reason: Option<String>,
    pub(crate) tool_calls: Vec<Value>, // 累积的 chat tool_calls
}