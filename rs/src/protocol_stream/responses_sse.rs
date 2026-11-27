//! responses_sse（精确拆分）
use super::*;

pub struct ResponsesSseState {
    pub(crate) response_id: String,
    pub(crate) model: String,
    pub(crate) message_id: String,
    pub(crate) output_index: u32,
    pub(crate) content_index: u32,
    pub(crate) text: String,
    pub(crate) started: bool,
    pub(crate) finished: bool,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    // 思考模式（reasoning_content → reasoning 项）
    pub(crate) reasoning_started: bool,
    pub(crate) reasoning_id: String,
    pub(crate) reasoning_text: String,
}