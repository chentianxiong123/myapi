//! constants（精确拆分）
use super::super::*;

/// TTFT 滑动窗口大小（学 LiteLLM lowest_latency max_latency_list_size=10）
/// 注：算法常量，未做成产品配置（record_outcome 有 26+ 调用点，参数化收益低风险高）。
/// 前端 latency_window_seconds 为「秒」语义，与本处「条数」不一致，故该字段当前仅存档。
pub(crate) const LATENCY_WINDOW_SIZE: usize = 10;

/// 延迟 buffer 比例（学 lowest_latency_buffer，避免延迟接近时反复横跳）
pub(crate) const LATENCY_BUFFER_RATIO: f64 = 0.2;

/// 无延迟记录通道的中性分数（低于已知的差通道，但不抢最快的）
/// 注：算法常量（非产品配置），60s 是经验值。
pub(crate) const NEUTRAL_LATENCY_SCORE: f64 = 60_000.0;