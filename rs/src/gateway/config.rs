//! gateway 配置（精确拆分）

//! config（从 gateway.rs 精确拆分）
use super::*;


pub(crate) fn def_fatal_status() -> Vec<u16> { vec![400, 401, 403, 404] }

pub(crate) fn def_backoff_initial() -> u64 { 500 }

pub(crate) fn def_backoff_max() -> u64 { 8000 }

pub(crate) fn def_jitter() -> f64 { 0.75 }

pub(crate) fn def_respect_retry_after() -> bool { true }

pub(crate) fn def_streaming_idle() -> u64 { 60 }

mod retry;
mod channel;
mod routing_cfg;
mod auth;
mod constants;
pub(crate) use retry::*;
pub(crate) use channel::*;
pub(crate) use routing_cfg::*;
pub(crate) use auth::*;
pub(crate) use constants::*;