//! 协议转换层（内部统一 chat 协议）

//! 协议转换层（学 cc-switch proxy/providers/transform.rs）。
//! myapi 内部统一 OpenAI Chat Completions 协议；客户端 Anthropic/Responses 转进来，
//! 上游 Anthropic/Responses 再转出去。纯函数 JSON→JSON，无 IO。

use serde_json::{json, Value};


fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

mod anthropic;
pub(crate) use anthropic::*;
mod chat_to_anthropic;
pub(crate) use chat_to_anthropic::*;
mod responses;
pub(crate) use responses::*;
mod reasoning;
pub(crate) use reasoning::*;

#[cfg(test)]
mod tests;
