//! circuit（从 gateway.rs 精确拆分）
use super::*;

/// 平均 TTFT * buffer 系数（延迟感知打分，越低越好）
pub(crate) fn avg_ttft_with_buffer(h: &ChannelHealth) -> f64 {
    if h.ttft_window.is_empty() {
        return NEUTRAL_LATENCY_SCORE;
    }
    let sum: f64 = h.ttft_window.iter().map(|&x| x as f64).sum();
    let avg = sum / h.ttft_window.len() as f64;
    avg * (1.0 + LATENCY_BUFFER_RATIO)
}

/// 按路由策略从可用候选里选一个（返回 candidates 的索引）。
/// 先熔断过滤，全不可用则 KeepAll 兑底（index 0）。
pub(crate) fn apply_strategy(
    candidates: &[(&Provider, &Channel, &ChannelModel)],
    hs: Option<&HashMap<String, ChannelHealth>>,
    cb: &CircuitBreakerConfig,
    strategy: &RoutingStrategy,
    when_all: &str,
) -> Option<usize> {
    // 收集可用候选索引 + 健康度
    let available: Vec<(usize, Option<&ChannelHealth>)> = candidates.iter().enumerate()
        .map(|(i, (_, ch, _))| {
            let h = hs.and_then(|m| m.get(&ch.id));
            (i, h)
        })
        .filter(|(_, h)| h.map(|x| is_available(x, cb)).unwrap_or(true))
        .collect();
    if available.is_empty() {
        // 全熔断兜底（可配置 keep-all/fail）：keep-all 用第一个，fail 直接无候选
        return if when_all == "fail" { None } else { Some(0) };
    }
    match strategy {
        RoutingStrategy::LatencyBased => {
            // 平均 TTFT 最低（buffer 防横跳）
            available.iter()
                .min_by(|a, b| {
                    let sa = a.1.map(avg_ttft_with_buffer).unwrap_or(NEUTRAL_LATENCY_SCORE);
                    let sb = b.1.map(avg_ttft_with_buffer).unwrap_or(NEUTRAL_LATENCY_SCORE);
                    sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| Some(*i))
                .unwrap_or(Some(0))
        }
        RoutingStrategy::Weight => {
            // 按权重加权随机（学 LiteLLM weights.rs）
            let total: u32 = available.iter()
                .map(|(i, _)| candidates[*i].1.weight)
                .sum();
            if total == 0 {
                return Some(available[0].0);
            }
            let mut pick = rand_range(total);
            for (i, _) in &available {
                let w = candidates[*i].1.weight;
                if pick < w { return Some(*i); }
                pick -= w;
            }
            Some(available[available.len() - 1].0)
        }
        RoutingStrategy::SimpleShuffle => {
            // 简单随机
            let n = available.len();
            Some(available[rand_range(n as u32) as usize].0)
        }
        RoutingStrategy::LeastBusy => {
            // 最闲：总请求数最低（并发代理，暂用 total_requests）
            available.iter()
                .min_by(|a, b| {
                    let ra = a.1.map(|h| h.total_requests).unwrap_or(0);
                    let rb = b.1.map(|h| h.total_requests).unwrap_or(0);
                    ra.cmp(&rb)
                })
                .map(|(i, _)| Some(*i))
                .unwrap_or(Some(0))
        }
    }
}

/// 轻量随机数（避免引入 rand 依赖：系统时间纳秒 + 计数器混合）
pub(crate) fn rand_range(max: u32) -> u32 {
    if max <= 1 { return 0; }
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // 简单 xorshift 混合
    let mut x = nanos ^ n.wrapping_mul(0x9E3779B97F4A7C15);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x % max as u64) as u32
}

/// 通道健康度（被动统计 + 三态熔断，学 cc-switch 状态机 + LiteLLM 失败率）
#[derive(Debug, Clone, Default)]
pub struct ChannelHealth {
    pub total_requests: u64,
    pub failed_requests: u64,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    /// 熔断状态：closed 正常 / open 熔断 / half_open 试探（学 cc-switch 三态）
    pub state: CircuitState,
    /// 最近 TTFT（毫秒），延迟感知用
    pub last_ttft_ms: u64,
    /// 最近一次失败的时间戳（秒），open→half_open 恢复超时用
    pub last_failure_at: u64,
    /// TTFT 滑动窗口（学 LiteLLM lowest_latency：存最近 N 次 TTFT，取平均选最低）
    pub ttft_window: Vec<u64>,
}

/// 熔断三态（closed/open/half_open，学 cc-switch）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CircuitState {
    #[default]
    Closed,
    Open,
    HalfOpen,
}

/// 健康度表：channel_id → health（全局共享，Arc<RwLock>）
pub type HealthMap = Arc<RwLock<HashMap<String, ChannelHealth>>>;

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 三态熔断状态机（学 cc-switch closed→open→half_open→closed）
/// 判定融合 LiteLLM：连续失败达阈值 OR 错误率>阈值（请求数≥min）→ open
/// 唯一可用通道不熔断（LiteLLM single_deployment 保护，避免一熔全瘫）
pub async fn record_outcome(
    health: &HealthMap,
    channel_id: &str,
    ok: bool,
    ttft_ms: u64,
    cb: &CircuitBreakerConfig,
    is_single_healthy: bool,
    force_open: bool,
) {
    let mut map = health.write().await;
    let h = map.entry(channel_id.to_string()).or_default();
    h.total_requests += 1;
    h.last_ttft_ms = ttft_ms;

    // 超时探测转移：open 超过恢复超时 → half_open（允许试探），与 is_available 一致
    if h.state == CircuitState::Open {
        let elapsed = now_secs().saturating_sub(h.last_failure_at);
        if elapsed >= cb.recovery_timeout_seconds {
            h.state = CircuitState::HalfOpen;
            h.consecutive_successes = 0;
        }
    }

    if ok {
        h.consecutive_successes += 1;
        h.consecutive_failures = 0;
        // TTFT 滑动窗口采样（学 LiteLLM lowest_latency：存最近 N 次 TTFT 取平均）
        h.ttft_window.push(ttft_ms);
        if h.ttft_window.len() > LATENCY_WINDOW_SIZE {
            h.ttft_window.remove(0);
        }
        match h.state {
            CircuitState::Closed => {}
            CircuitState::HalfOpen => {
                // half_open 下连续成功达阈值 → closed（恢复）
                if h.consecutive_successes >= cb.recovery_success_threshold {
                    h.state = CircuitState::Closed;
                    h.consecutive_successes = 0;
                }
            }
            CircuitState::Open => {}
        }
    } else {
        h.failed_requests += 1;
        h.consecutive_failures += 1;
        h.consecutive_successes = 0;
        h.last_failure_at = now_secs();
        match h.state {
            CircuitState::Closed => {
                // 连续失败达阈值 OR 错误率超阈值（且非唯一通道）→ open
                // force_open（关键词熔断命中）：跳过阈值，一次即 open
                let error_rate = h.failed_requests as f64 / h.total_requests as f64 * 100.0;
                let rate_cools = h.total_requests >= cb.min_requests
                    && error_rate > cb.error_rate_threshold as f64
                    && !is_single_healthy;
                let consecutive_cools = h.consecutive_failures >= cb.failure_threshold;
                if (consecutive_cools || rate_cools || force_open) && !is_single_healthy {
                    h.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                // half_open 下失败 → 重新 open
                h.state = CircuitState::Open;
                h.last_failure_at = now_secs();
            }
            CircuitState::Open => {
                h.last_failure_at = now_secs();
            }
        }
    }
}

/// 关键词熔断：错误响应体含配置关键词（欠费/封号等"重试也没用"的错误）→ true。
/// 学 new-api AutomaticDisableKeywords。只在失败时调，body 文本小写化后匹配。
pub(crate) fn hit_disable_keyword(body: &[u8], keywords: &[String]) -> Option<String> {
    if keywords.is_empty() { return None; }
    let text = String::from_utf8_lossy(body).to_lowercase();
    keywords.iter().find(|k| !k.is_empty() && text.contains(&k.to_lowercase())).cloned()
}

pub fn is_available(h: &ChannelHealth, cb: &CircuitBreakerConfig) -> bool {
    match h.state {
        CircuitState::Closed => true,
        CircuitState::Open | CircuitState::HalfOpen => {
            // open 超过恢复超时 → 允许试探（half_open）
            let elapsed = now_secs().saturating_sub(h.last_failure_at);
            elapsed >= cb.recovery_timeout_seconds
        }
    }
}