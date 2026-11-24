//! 网关转发核心（照 LiteLLM router-core 建模，最小可跑通管道）
//!
//! 数据流：/v1/chat/completions → 选通道（按模型）→ 转发上游 → 流式回传
//! 智能路由（黏性/熔断/延迟感知）先留接口，管道通了再加。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use axum::response::IntoResponse;

// ==================== 数据结构（对接前端 types.ts 的 Provider/Channel）====================

/// 密钥池里的一条命名密钥
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedKey {
    pub name: String,
    #[serde(rename = "api-key", alias = "api_key")]
    pub api_key: String,
}

/// 通道：供应商内部一条具体接入方式（key × 路径 × 模型映射三件套绑死，学 LiteLLM Deployment）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// 网络路径描述（IPv4/IPv6）；空=直连/继承供应商代理
    #[serde(default)]
    pub path: Option<String>,
    /// 引用密钥池的名字；空=用第一个「主」密钥
    #[serde(default)]
    pub key_name: Option<String>,
    /// 该通道覆盖的模型映射：上游真实名 → 可选对外别名
    #[serde(default)]
    pub models: Vec<ChannelModel>,
    /// 路由权重（默认 1）
    #[serde(default = "default_weight")]
    pub weight: u32,
    /// 每分钟请求上限（RPM，空=不限）；超限当不可用跳过（防打爆上游）
    #[serde(default)]
    pub rpm: Option<u32>,
}

fn default_weight() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelModel {
    /// 上游真实模型名
    pub name: String,
    /// 对外别名（空=用 name）
    #[serde(default)]
    pub alias: Option<String>,
    /// 硬能力（vision/pdf/audio，学 9router HARD_CAPS）；Auto 模式按请求需求匹配
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// 供应商：上游一个厂商（含密钥池 + 多条通道）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// 密钥池（第一个默认「主」）
    #[serde(default)]
    pub keys: Vec<NamedKey>,
    /// 通道列表（供应商的子层）
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// 渠道级代理（空=直连）
    #[serde(default)]
    pub proxy: String,
}

/// 网关配置（config/gateway.yaml）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    #[serde(default = "def_listen")]
    pub listen: u16,
    #[serde(default)]
    pub providers: Vec<Provider>,
    /// 熔断器参数（学 cc-switch 三态 + LiteLLM 失败率）
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
    /// 黏性路由（省token，学 LiteLLM affinity）
    #[serde(default)]
    pub sticky: StickyConfig,
    /// 路由策略（延迟感知/权重/随机/最闲）
    #[serde(default)]
    pub routing: RoutingConfig,
    /// 超时（流式首字/非流式，学 cc-switch）
    #[serde(default)]
    pub timeout: TimeoutConfig,
    /// 下游鉴权（agent 调网关校验 Bearer key）
    #[serde(default)]
    pub auth: AuthConfig,
}

fn def_listen() -> u16 { 8380 }

/// TTFT 滑动窗口大小（学 LiteLLM lowest_latency max_latency_list_size=10）
const LATENCY_WINDOW_SIZE: usize = 10;
/// 延迟 buffer 比例（学 lowest_latency_buffer，避免延迟接近时反复横跳）
const LATENCY_BUFFER_RATIO: f64 = 0.2;
/// 无延迟记录通道的中性分数（低于已知的差通道，但不抢最快的）
const NEUTRAL_LATENCY_SCORE: f64 = 60_000.0;

/// 平均 TTFT * buffer 系数（延迟感知打分，越低越好）
fn avg_ttft_with_buffer(h: &ChannelHealth) -> f64 {
    if h.ttft_window.is_empty() {
        return NEUTRAL_LATENCY_SCORE;
    }
    let sum: f64 = h.ttft_window.iter().map(|&x| x as f64).sum();
    let avg = sum / h.ttft_window.len() as f64;
    avg * (1.0 + LATENCY_BUFFER_RATIO)
}

/// 按路由策略从可用候选里选一个（返回 candidates 的索引）。
/// 先熔断过滤，全不可用则 KeepAll 兑底（index 0）。
fn apply_strategy(
    candidates: &[(&Provider, &Channel, &ChannelModel)],
    hs: Option<&HashMap<String, ChannelHealth>>,
    cb: &CircuitBreakerConfig,
    strategy: &RoutingStrategy,
) -> usize {
    // 收集可用候选索引 + 健康度
    let available: Vec<(usize, Option<&ChannelHealth>)> = candidates.iter().enumerate()
        .map(|(i, (_, ch, _))| {
            let h = hs.and_then(|m| m.get(&ch.id));
            (i, h)
        })
        .filter(|(_, h)| h.map(|x| is_available(x, cb)).unwrap_or(true))
        .collect();
    if available.is_empty() {
        return 0; // 全熔断 → KeepAll 兑底第一个
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
                .map(|(i, _)| *i)
                .unwrap_or(0)
        }
        RoutingStrategy::Weight => {
            // 按权重加权随机（学 LiteLLM weights.rs）
            let total: u32 = available.iter()
                .map(|(i, _)| candidates[*i].1.weight)
                .sum();
            if total == 0 {
                return available[0].0;
            }
            let mut pick = rand_range(total);
            for (i, _) in &available {
                let w = candidates[*i].1.weight;
                if pick < w { return *i; }
                pick -= w;
            }
            available[available.len() - 1].0
        }
        RoutingStrategy::SimpleShuffle => {
            // 简单随机
            let n = available.len();
            available[rand_range(n as u32) as usize].0
        }
        RoutingStrategy::LeastBusy => {
            // 最闲：总请求数最低（并发代理，暂用 total_requests）
            available.iter()
                .min_by(|a, b| {
                    let ra = a.1.map(|h| h.total_requests).unwrap_or(0);
                    let rb = b.1.map(|h| h.total_requests).unwrap_or(0);
                    ra.cmp(&rb)
                })
                .map(|(i, _)| *i)
                .unwrap_or(0)
        }
    }
}

/// 轻量随机数（避免引入 rand 依赖：系统时间纳秒 + 计数器混合）
fn rand_range(max: u32) -> u32 {
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

// ==================== Auto 全自动路由（学 9router combo：capability 匹配 + fallback 链）====================

/// Auto 候选项：某通道的某模型 + 上游真实名 + key
#[derive(Debug, Clone)]
pub struct AutoCandidate {
    pub provider: Provider,
    pub channel: Channel,
    pub upstream_model: String,
    pub api_key: String,
    pub score: f64, // 排序分（延迟，越低越优先）
}

/// 检测请求需要的硬能力（vision：messages 里带 image_url；pdf：file）
fn detect_required_caps(payload: &serde_json::Value) -> Vec<String> {
    let mut caps = Vec::new();
    if let Some(msgs) = payload.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            if let Some(content) = msg.get("content").and_then(|v| v.as_array()) {
                for part in content {
                    match part.get("type").and_then(|v| v.as_str()) {
                        Some("image_url") => caps.push("vision".into()),
                        Some("file") => caps.push("pdf".into()),
                        _ => {}
                    }
                }
            }
        }
    }
    caps
}

/// Auto 模式：聚合所有可用通道的模型，硬能力匹配（缺 vision 不接带图请求），
/// 按延迟升序排成 fallback 链（跳过熔断通道）。
pub fn auto_candidates(
    config: &GatewayConfig,
    payload: &serde_json::Value,
    health: &HealthMap,
) -> Vec<AutoCandidate> {
    let required = detect_required_caps(payload);
    let hs = health.try_read().ok();
    let cb = &config.circuit_breaker;
    let mut out: Vec<AutoCandidate> = Vec::new();

    for provider in &config.providers {
        if provider.disabled { continue; }
        for channel in &provider.channels {
            for m in &channel.models {
                // 硬能力匹配：请求需要的能力模型必须都有
                let has_all = required.iter().all(|c| m.capabilities.iter().any(|mc| mc == c));
                if !has_all { continue; }

                let h_opt = hs.as_ref().and_then(|map| map.get(&channel.id));
                let available = h_opt.map(|h| is_available(h, cb)).unwrap_or(true);
                if !available { continue; }

                let score = h_opt.map(avg_ttft_with_buffer).unwrap_or(NEUTRAL_LATENCY_SCORE);
                out.push(AutoCandidate {
                    provider: provider.clone(),
                    channel: channel.clone(),
                    upstream_model: m.name.clone(),
                    api_key: resolve_key(provider, channel).unwrap_or_default(),
                    score,
                });
            }
        }
    }
    out.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// 熔断器配置（学 cc-switch 三态机 + LiteLLM 失败率判定）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// 连续失败多少次开熔断（建议 3-10）
    #[serde(default = "def_failure_threshold")]
    pub failure_threshold: u32,
    /// 错误率阈值 %（LiteLLM: 失败率>此值且请求数≥min → 熔断）
    #[serde(default = "def_error_rate")]
    pub error_rate_threshold: u64,
    /// 错误率有意义的最小请求数
    #[serde(default = "def_min_requests")]
    pub min_requests: u64,
    /// 恢复等待秒数（open→half_open，建议 30-120）
    #[serde(default = "def_recovery_timeout")]
    pub recovery_timeout_seconds: u64,
    /// 恢复成功阈值（half_open 下连续成功几次→closed）
    #[serde(default = "def_recovery_success")]
    pub recovery_success_threshold: u32,
}
fn def_failure_threshold() -> u32 { 5 }
fn def_error_rate() -> u64 { 50 }
fn def_min_requests() -> u64 { 5 }
fn def_recovery_timeout() -> u64 { 30 }
fn def_recovery_success() -> u32 { 3 }
impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: def_failure_threshold(),
            error_rate_threshold: def_error_rate(),
            min_requests: def_min_requests(),
            recovery_timeout_seconds: def_recovery_timeout(),
            recovery_success_threshold: def_recovery_success(),
        }
    }
}

/// 黏性路由配置（学 LiteLLM deployment affinity，为 prompt cache 省token）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickyConfig {
    #[serde(default = "def_sticky_enabled")]
    pub enabled: bool,
    /// pin 保持秒数（TTL，过期重选）
    #[serde(default = "def_sticky_ttl")]
    pub ttl_seconds: u64,
}
fn def_sticky_enabled() -> bool { true }
fn def_sticky_ttl() -> u64 { 300 }
impl Default for StickyConfig {
    fn default() -> Self {
        Self { enabled: def_sticky_enabled(), ttl_seconds: def_sticky_ttl() }
    }
}

/// 路由策略（可配置，学 LiteLLM routing_strategy）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RoutingStrategy {
    /// 延迟感知（平均 TTFT 最低优先，默认）
    #[default]
    LatencyBased,
    /// 权重（按 Channel.weight 加权随机）
    Weight,
    /// 随机（简单洗牌）
    SimpleShuffle,
    /// 最闲（并发最低，暂用总请求数代理）
    LeastBusy,
}

/// 路由策略配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    #[serde(default)]
    pub strategy: RoutingStrategy,
    /// Auto 模式开关（model=auto 时才生效）
    #[serde(default = "def_auto_enabled")]
    pub auto_mode_enabled: bool,
    /// 最大重试数（failover 尝试多少条通道）
    #[serde(default = "def_max_retries")]
    pub max_retries: usize,
}
fn def_max_retries() -> usize { 3 }
fn def_auto_enabled() -> bool { true }
impl Default for RoutingConfig {
    fn default() -> Self {
        Self { strategy: RoutingStrategy::default(), auto_mode_enabled: def_auto_enabled(), max_retries: def_max_retries() }
    }
}

/// 超时配置（学 cc-switch 流式/非流式分开）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// 流式首字超时秒（TTFT 上限）
    #[serde(default = "def_stream_first_byte")]
    pub streaming_first_byte: u64,
    /// 非流式总超时秒
    #[serde(default = "def_non_streaming")]
    pub non_streaming: u64,
}
fn def_stream_first_byte() -> u64 { 30 }
fn def_non_streaming() -> u64 { 120 }
impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            streaming_first_byte: def_stream_first_byte(),
            non_streaming: def_non_streaming(),
        }
    }
}

/// 下游鉴权配置（agent 调网关时校验 Bearer key，防裸奍）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 允许的下游 key 列表（空=不鉴权，本机内网可用；配了=严格校验 Bearer）
    #[serde(default)]
    pub api_keys: Vec<String>,
}
impl Default for AuthConfig {
    fn default() -> Self {
        Self { api_keys: Vec::new() }
    }
}

/// 校验下游 Bearer key（api_keys 空=不鉴权内网可用；配了=严格校验）
pub fn validate_auth(auth: &AuthConfig, auth_header: Option<&str>) -> Result<(), String> {
    if auth.api_keys.is_empty() {
        return Ok(()); // 未配 key → 不鉴权（本机/内网）
    }
    let token = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    if token.is_empty() {
        return Err("缺少 Authorization Bearer key".into());
    }
    if auth.api_keys.iter().any(|k| k == token) {
        Ok(())
    } else {
        Err("无效的 API key".into())
    }
}

// ==================== 健康度（被动统计，学 LiteLLM cooldown，先留接口）====================

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

fn now_secs() -> u64 {
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
                let error_rate = h.failed_requests as f64 / h.total_requests as f64 * 100.0;
                let rate_cools = h.total_requests >= cb.min_requests
                    && error_rate > cb.error_rate_threshold as f64
                    && !is_single_healthy;
                let consecutive_cools = h.consecutive_failures >= cb.failure_threshold;
                if (consecutive_cools || rate_cools) && !is_single_healthy {
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

/// 通道当前是否可用（open 状态且未到恢复超时 → 不可用）
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

// ==================== 通道选择（按模型名，最小版：第一个能服务该模型的健康通道）====================

/// 选中的通道 + 解析出的上游真实模型名 + 用的 key
pub struct SelectedChannel {
    pub provider: Provider,
    pub channel: Channel,
    /// 上游真实模型名（把请求里的 alias 翻译回 name）
    pub upstream_model: String,
    /// 实际用的 API key（从密钥池解析）
    pub api_key: String,
}

/// 按对外模型名选通道。
/// 匹配规则：遍历所有供应商的所有通道，通道的 models 里 alias==请求模型名 或 name==请求模型名 则命中。
/// 最小版：返回第一个命中的（TODO: 按权重/延迟感知/黏性/熔断过滤择优）。
pub fn select_channel(
    config: &GatewayConfig,
    requested_model: &str,
    health: &HealthMap,
) -> Option<SelectedChannel> {
    let health_snapshot = health.try_read().ok();
    let cb = &config.circuit_breaker;
    // 先收集所有能服务该模型的候选通道（含健康度）
    let mut candidates: Vec<(&Provider, &Channel, &ChannelModel)> = Vec::new();
    for provider in &config.providers {
        if provider.disabled { continue; }
        for channel in &provider.channels {
            for m in &channel.models {
                let external = m.alias.as_deref().unwrap_or(&m.name);
                if external == requested_model || m.name == requested_model {
                    candidates.push((provider, channel, m));
                }
            }
        }
    }
    if candidates.is_empty() { return None; }

    // 熔断过滤 + 按策略选通道（可配置：延迟感知/权重/随机/最闲）
    let hs = health_snapshot.as_deref();
    let idx = apply_strategy(&candidates, hs, cb, &config.routing.strategy);
    let (provider, channel, m) = candidates[idx];
    let api_key = resolve_key(provider, channel).unwrap_or_default();
    Some(SelectedChannel {
        provider: (*provider).clone(),
        channel: (*channel).clone(),
        upstream_model: m.name.clone(),
        api_key,
    })
}

/// 解析通道实际用的 key：key_name 指定 → 池里找同名；否则用第一个（主）
pub fn resolve_key(provider: &Provider, channel: &Channel) -> Option<String> {
    if let Some(kn) = &channel.key_name {
        provider.keys.iter()
            .find(|k| &k.name == kn)
            .map(|k| k.api_key.clone())
    } else {
        provider.keys.first().map(|k| k.api_key.clone())
    }
}

// ==================== 黏性路由（学 LiteLLM deployment affinity，prompt cache 省 token）====================

/// pin 条目：通道 id + 打 pin 的时间戳
#[derive(Debug, Clone)]
pub struct PinEntry {
    pub channel_id: String,
    pub pinned_at: u64,
}

/// pin 表：pin_key → PinEntry（全局共享）
pub type PinStore = Arc<RwLock<HashMap<String, PinEntry>>>;

pub fn new_pin_store() -> PinStore {
    Arc::new(RwLock::new(HashMap::new()))
}

// ==================== 限流（防打爆上游，学 LiteLLM rpm/tpm）====================

/// 限流计数：通道 id → 最近请求时间戳列表（滑动窗口）
pub type RateStore = Arc<RwLock<HashMap<String, Vec<u64>>>>;

pub fn new_rate_store() -> RateStore {
    Arc::new(RwLock::new(HashMap::new()))
}

const RATE_WINDOW_SECS: u64 = 60;

/// 检查通道是否被限流：滑动窗口内请求数 ≥ rpm → 限流（超限跳过该通道）
pub fn is_rate_limited(store: &RateStore, channel_id: &str, rpm: u32) -> bool {
    let now = now_secs();
    let mut map = match store.try_write() {
        Ok(m) => m,
        Err(_) => return false, // 拿不到锁不拦（宁放行不误杀）
    };
    let list = map.entry(channel_id.to_string()).or_default();
    // 清掉窗口外的旧时间戳
    list.retain(|&t| now.saturating_sub(t) < RATE_WINDOW_SECS);
    if list.len() >= rpm as usize {
        true // 超限
    } else {
        list.push(now); // 记本次
        false
    }
}

/// 从请求提取 session 标识（pin key）。
/// 优先级：X-Session-Id header > Authorization Bearer key hash（学 LiteLLM api-key affinity）
pub fn session_key(payload: &serde_json::Value, auth_header: Option<&str>) -> Option<String> {
    // 1. 显式 session id（agent 传 X-Session-Id）
    if let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()) {
        if !sid.is_empty() {
            return Some(format!("sess:{}", sid));
        }
    }
    // 2. 下游 API key（同一 key 的连续请求黏同一通道）
    if let Some(auth) = auth_header {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            // hash 一下不存明文
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            token.hash(&mut h);
            return Some(format!("key:{:016x}", h.finish()));
        }
    }
    None
}

/// 带黏性的通道选择：有有效 pin 且通道可用→复用；否则新选并打 pin
/// 按路由策略返回排序后的候选链（普通路由 failover 用）。
/// 可用通道优先按策略排序；全熔断则 KeepAll 兑底（原顺序）。pin 的通道提到最前。
pub fn ordered_candidates(
    config: &GatewayConfig,
    requested_model: &str,
    health: &HealthMap,
    pin_channel_id: Option<&str>,
) -> Vec<SelectedChannel> {
    let hs = health.try_read().ok();
    let hs = hs.as_deref();
    let cb = &config.circuit_breaker;
    let strategy = &config.routing.strategy;

    // 收集能服务该模型的候选
    let mut candidates: Vec<(&Provider, &Channel, &ChannelModel)> = Vec::new();
    for provider in &config.providers {
        if provider.disabled { continue; }
        for channel in &provider.channels {
            for m in &channel.models {
                let external = m.alias.as_deref().unwrap_or(&m.name);
                if external == requested_model || m.name == requested_model {
                    candidates.push((provider, channel, m));
                }
            }
        }
    }
    if candidates.is_empty() { return Vec::new(); }

    // 按策略选出的首选 index（apply_strategy 处理熔断过滤 + 策略排序）
    let first = apply_strategy(&candidates, hs, cb, strategy);

    // 构造 SelectedChannel 列表：首选在前，其余按原序在后
    let mut ordered: Vec<SelectedChannel> = Vec::new();
    let mut used = vec![false; candidates.len()];
    let push = |ordered: &mut Vec<SelectedChannel>, i: usize, used: &mut Vec<bool>| {
        if used[i] { return; }
        let (p, c, m) = candidates[i];
        ordered.push(SelectedChannel {
            provider: (*p).clone(),
            channel: (*c).clone(),
            upstream_model: m.name.clone(),
            api_key: resolve_key(p, c).unwrap_or_default(),
        });
        used[i] = true;
    };

    // pin 的通道最优先（黏性复用）
    if let Some(pin_id) = pin_channel_id {
        if let Some(i) = candidates.iter().position(|(_, c, _)| c.id == pin_id) {
            push(&mut ordered, i, &mut used);
        }
    }
    push(&mut ordered, first, &mut used);
    for i in 0..candidates.len() {
        push(&mut ordered, i, &mut used);
    }
    ordered
}

pub fn select_channel_sticky(
    config: &GatewayConfig,
    requested_model: &str,
    health: &HealthMap,
    pins: &PinStore,
    pin_key: Option<&str>,
) -> Option<SelectedChannel> {
    let cb = &config.circuit_breaker;
    // 1. 查有效 pin（TTL 内 + 通道仍健康）
    if let Some(key) = pin_key {
        if config.sticky.enabled {
            if let Ok(map) = pins.try_read() {
                if let Some(entry) = map.get(key) {
                    let age = now_secs().saturating_sub(entry.pinned_at);
                    if age < config.sticky.ttl_seconds {
                        // pin 有效，验证通道仍可用
                        let channel_ok = {
                            let hm_guard = health.try_read().ok();
                            match hm_guard.as_deref().and_then(|hm| hm.get(&entry.channel_id)) {
                                Some(h) => is_available(h, cb),
                                None => true,
                            }
                        };
                        if channel_ok {
                            if let Some(sel) = find_channel_by_id(config, &entry.channel_id, requested_model) {
                                return Some(sel);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. 无有效 pin → 按熔断/健康选通道
    let selected = select_channel(config, requested_model, health)?;

    // 3. 打 pin（供下次复用）
    if let Some(key) = pin_key {
        if config.sticky.enabled {
            if let Ok(mut map) = pins.try_write() {
                map.insert(key.to_string(), PinEntry {
                    channel_id: selected.channel.id.clone(),
                    pinned_at: now_secs(),
                });
            }
        }
    }
    Some(selected)
}

/// 按 channel id + 模型名定位通道（黏性复用用）
fn find_channel_by_id(config: &GatewayConfig, channel_id: &str, requested_model: &str) -> Option<SelectedChannel> {
    for provider in &config.providers {
        if provider.disabled { continue; }
        for channel in &provider.channels {
            if channel.id != channel_id { continue; }
            for m in &channel.models {
                let external = m.alias.as_deref().unwrap_or(&m.name);
                if external == requested_model || m.name == requested_model {
                    return Some(SelectedChannel {
                        provider: provider.clone(),
                        channel: channel.clone(),
                        upstream_model: m.name.clone(),
                        api_key: resolve_key(provider, channel).unwrap_or_default(),
                    });
                }
            }
        }
    }
    None
}

// ==================== 配置加载（读 config/gateway.yaml）====================

pub fn load_config(root: &std::path::Path) -> GatewayConfig {
    let path = root.join("config").join("gateway.yaml");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return GatewayConfig::default(),
    };
    serde_yaml::from_str(&raw).unwrap_or_default()
}

// ==================== 转发管道（/v1/chat/completions 核心）====================

/// 选通道 → 构造上游请求 → 转发 → 回传。
/// 最小版：非流式直接回 JSON；流式透传 SSE 字节流。
pub async fn forward_generic(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    upstream_path: &str,
) -> axum::response::Response {
    let config = load_config(root);
    // 下游鉴权：校验 Bearer key（api_keys 配了才校验，空=内网不鉴权）
    if let Err(e) = validate_auth(&config.auth, auth_header) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error": {"message": e, "type": "authentication_error"}})),
        ).into_response();
    }
    let requested_model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let pin_key = session_key(&payload, auth_header);

    // Auto 模式：model=auto 且开启 auto_mode → 聚合所有可用模型按 capability+延迟排序，逐个试，失败 fallback 下一个
    if requested_model == "auto" && config.routing.auto_mode_enabled {
        let candidates = auto_candidates(&config, &payload, health);
        if candidates.is_empty() {
            return error_response(404, "auto: 无可用模型");
        }
        let mut last_err = String::new();
        for cand in &candidates {
            let sel = SelectedChannel {
                provider: cand.provider.clone(),
                channel: cand.channel.clone(),
                upstream_model: cand.upstream_model.clone(),
                api_key: cand.api_key.clone(),
            };
            // 限流：通道配了 rpm 且超限 → 跳过（防打爆上游）
            if let Some(rpm) = cand.channel.rpm {
                if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
            }
            match forward_one(root, &config, health, &sel, &requested_model, &payload, upstream_path).await {
                Ok(resp) => return resp,
                Err(e) => {
                    last_err = e;
                    continue; // fallback 下一个
                }
            }
        }
        return error_response(502, &format!("auto: 所有候选失败, 最后错误: {}", last_err));
    }

    // 非 Auto：候选链 + failover（逐个试，失败切下一个，受 max_retries 限制）
    // 黏性：从 pin 表取上次通道提到链首（复用），失败则 failover 到其他通道
    let pinned_id = match pin_key.as_deref() {
        Some(k) => pins.read().await.get(k).map(|e| e.channel_id.clone()),
        None => None,
    };
    let candidates = ordered_candidates(&config, &requested_model, health, pinned_id.as_deref());
    if candidates.is_empty() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": {"message": format!("模型 {} 无可用通道", requested_model), "type": "invalid_request_error"}
            })),
        ).into_response();
    }
    let max_retries = config.routing.max_retries.min(candidates.len());
    let mut last_err = String::new();
    for cand in candidates.iter().take(max_retries.max(1)) {
        // 限流：通道配了 rpm 且超限 → 跳过（防打爆上游）
        if let Some(rpm) = cand.channel.rpm {
            if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
        }
        match forward_one(root, &config, health, cand, &requested_model, &payload, upstream_path).await {
            Ok(resp) => {
                // 成功：更新黏性 pin 到这条通道（供后续复用）
                if let Some(k) = pin_key.as_deref() {
                    if config.sticky.enabled {
                        pins.write().await.insert(k.to_string(), PinEntry { channel_id: cand.channel.id.clone(), pinned_at: now_secs() });
                    }
                }
                return resp;
            }
            Err(e) => {
                last_err = e;
                continue; // failover 下一个
            }
        }
    }
    error_response(502, &format!("所有通道失败（试了 {} 条）: {}", max_retries.max(1), last_err))
}

/// 转发到单个通道（Auto 和普通路由共用）：失败返回 Err(msg)，成功返回响应
async fn forward_one(
    root: &std::path::Path,
    config: &GatewayConfig,
    health: &HealthMap,
    selected: &SelectedChannel,
    requested_model: &str,
    payload: &serde_json::Value,
    upstream_path: &str,
) -> Result<axum::response::Response, String> {
    // 把请求里的模型名换成上游真实名
    let mut upstream_payload = payload.clone();
    if let Some(obj) = upstream_payload.as_object_mut() {
        obj.insert("model".into(), serde_json::Value::String(selected.upstream_model.clone()));
    }
    let is_stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    // 构造上游 URL（base_url + 可配置路径，如 /chat/completions 或 /embeddings）
    let base = selected.provider.base_url.trim_end_matches('/');
    let url = format!("{}{}", base, upstream_path);

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(config.timeout.streaming_first_byte))
        .timeout(std::time::Duration::from_secs(config.timeout.non_streaming))
        .build()
        .map_err(|e| format!("client 构造失败: {}", e))?;
    let mut req = client.post(&url)
        .header("Authorization", format!("Bearer {}", selected.api_key))
        .header("Content-Type", "application/json");
    for (k, v) in &selected.provider.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let start = std::time::Instant::now();
    let cb = &config.circuit_breaker;
    let resp = match req.json(&upstream_payload).send().await {
        Ok(r) => r,
        Err(e) => {
            record_outcome(health, &selected.channel.id, false, 0, cb, false).await;
            return Err(format!("上游请求失败: {}", e));
        }
    };
    let ttft = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let ok = status.is_success();
    record_outcome(health, &selected.channel.id, ok, ttft, cb, false).await;

    // 回传（真流式：SSE 逐 chunk 透传；非流式回整包）
    let upstream_headers = resp.headers().clone();
    let is_sse = upstream_headers.get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false) || is_stream;

    let mut out = axum::response::Response::builder().status(status);
    if let Some(ct) = upstream_headers.get("content-type") {
        out = out.header("content-type", ct);
    }
    // 透传上游关键 header
    for hk in ["x-request-id", "openai-organization", "ratelimit-remaining-requests"] {
        if let Some(hv) = upstream_headers.get(hk) {
            out = out.header(hk, hv);
        }
    }

    if is_sse {
        // 真流式：逐 chunk 转发字节流，客户端实时收到每个 SSE event
        use axum::body::Body;
        // 流式日志（token 解析需聚合 SSE，先记 TTFT/状态）
        crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
            model: requested_model.into(),
            provider: selected.provider.name.clone(),
            channel: selected.channel.name.clone(),
            status: status.as_u16(),
            latency_ms: start.elapsed().as_millis() as u64,
            ttft_ms: ttft,
            input_tokens: None,
            output_tokens: None,
            error: if ok { None } else { Some(format!("HTTP {}", status.as_u16())) },
        }).await;
        let stream = resp.bytes_stream();
        let body = Body::from_stream(stream);
        out.body(body).map_err(|e| format!("流式回传失败: {}", e))
    } else {
        // 非流式：整包 + 解析 usage token 入日志
        let body_bytes = resp.bytes().await.unwrap_or_default();
        let (in_tok, out_tok) = parse_usage(&body_bytes);
        crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
            model: requested_model.into(),
            provider: selected.provider.name.clone(),
            channel: selected.channel.name.clone(),
            status: status.as_u16(),
            latency_ms: start.elapsed().as_millis() as u64,
            ttft_ms: ttft,
            input_tokens: in_tok,
            output_tokens: out_tok,
            error: if ok { None } else { Some(format!("HTTP {}", status.as_u16())) },
        }).await;
        out.body(axum::body::Body::from(body_bytes))
            .map_err(|e| format!("回传失败: {}", e))
    }
}

/// 从非流式响应解析 usage（OpenAI 格式 usage.input_tokens/output_tokens）
fn parse_usage(body: &[u8]) -> (Option<i64>, Option<i64>) {
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let u = v.get("usage");
    let get = |k: &str| u.and_then(|x| x.get(k)).and_then(|x| x.as_i64());
    (get("input_tokens").or_else(|| get("prompt_tokens")),
     get("output_tokens").or_else(|| get("completion_tokens")))
}

/// chat 端点包装（forward_generic + /chat/completions）
pub async fn forward_chat(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
) -> axum::response::Response {
    forward_generic(root, health, pins, rates, auth_header, payload, "/chat/completions").await
}

/// embeddings 端点（复用路由+failover，走 /embeddings，无流式）
pub async fn forward_embeddings(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
) -> axum::response::Response {
    // embeddings 不流式，强制去掉 stream 标记
    let mut p = payload;
    if let Some(obj) = p.as_object_mut() {
        obj.remove("stream");
    }
    forward_generic(root, health, pins, rates, auth_header, p, "/embeddings").await
}


fn error_response(code: u16, msg: &str) -> axum::response::Response {
    (
        axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        axum::Json(serde_json::json!({"error": {"message": msg, "type": "api_error"}})),
    ).into_response()
}

/// 列出所有对外可用模型名（/v1/models 用）：聚合所有通道的 alias/name
pub fn list_models(config: &GatewayConfig) -> Vec<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in &config.providers {
        if p.disabled { continue; }
        for c in &p.channels {
            for m in &c.models {
                let external = m.alias.clone().unwrap_or_else(|| m.name.clone());
                if seen.insert(external.clone()) {
                    out.push(serde_json::json!({
                        "id": external,
                        "object": "model",
                        "owned_by": p.name,
                    }));
                }
            }
        }
    }
    out
}

/// 主动健康探测（学 LiteLLM /health 主动 ping deployment）：轻量 GET {base_url}/models
/// 不花 token，只探通道上游可达性。返回每通道 live/dead + 延迟 + HTTP status。
pub async fn probe_upstream(root: &std::path::Path) -> Vec<serde_json::Value> {
    let config = load_config(root);
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for p in &config.providers {
        if p.disabled {
            continue;
        }
        let base = p.base_url.trim_end_matches('/');
        let url = format!("{}/models", base);
        for c in &p.channels {
            let api_key = resolve_key(p, c).unwrap_or_default();
            let start = std::time::Instant::now();
            let mut req = client.get(&url).header("Authorization", format!("Bearer {}", api_key));
            for (k, v) in &p.headers {
                req = req.header(k.as_str(), v.as_str());
            }
            let (live, status_code, err) = match req.send().await {
                Ok(r) => {
                    let sc = r.status().as_u16();
                    // 401/403 说明上游活着（只是 key 问题）；2xx/429 都算可达
                    let ok = sc < 500;
                    (ok, sc, String::new())
                }
                Err(e) => (false, 0, e.to_string()),
            };
            let latency = start.elapsed().as_millis() as u64;
            out.push(serde_json::json!({
                "provider": p.name,
                "channel": c.name,
                "channel_id": c.id,
                "live": live,
                "status": if live { "healthy" } else { "unreachable" },
                "status_code": status_code,
                "latency_ms": latency,
                "error": err,
            }));
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn cb() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 5,
            error_rate_threshold: 50,
            min_requests: 5,
            recovery_timeout_seconds: 30,
            recovery_success_threshold: 3,
        }
    }

    fn health_map() -> HealthMap {
        Arc::new(RwLock::new(HashMap::new()))
    }

    #[tokio::test]
    async fn closed_on_success() {
        let h = health_map();
        let c = cb();
        record_outcome(&h, "ch1", true, 100, &c, false).await;
        record_outcome(&h, "ch1", true, 100, &c, false).await;
        let m = h.read().await;
        let hc = m.get("ch1").unwrap();
        assert_eq!(hc.state, CircuitState::Closed);
        assert_eq!(hc.total_requests, 2);
        assert_eq!(hc.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn opens_on_consecutive_failures() {
        let h = health_map();
        let c = cb();
        // 连续失败 5 次（failure_threshold）→ open
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &c, false).await;
        }
        let m = h.read().await;
        let hc = m.get("ch1").unwrap();
        assert_eq!(hc.state, CircuitState::Open, "连续失败达阈值应 open");
    }

    #[tokio::test]
    async fn opens_on_error_rate() {
        let h = health_map();
        let c = cb();
        // 5 次里 4 失败（80% > 50%，请求数≥5）→ open
        record_outcome(&h, "ch1", true, 0, &c, false).await;
        for _ in 0..4 {
            record_outcome(&h, "ch1", false, 0, &c, false).await;
        }
        let m = h.read().await;
        let hc = m.get("ch1").unwrap();
        assert_eq!(hc.state, CircuitState::Open, "错误率超阈值应 open");
    }

    #[tokio::test]
    async fn single_deployment_never_opens() {
        let h = health_map();
        let c = cb();
        // 唯一通道连续失败 10 次，也不 open（单部署保护）
        for _ in 0..10 {
            record_outcome(&h, "ch1", false, 0, &c, true).await;
        }
        let m = h.read().await;
        let hc = m.get("ch1").unwrap();
        assert_eq!(hc.state, CircuitState::Closed, "唯一通道不熔断（避免全瘫）");
    }

    #[tokio::test]
    async fn halfopen_then_recover() {
        let h = health_map();
        let c = cb();
        // 先打(open: 连续失败5)
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &c, false).await;
        }
        // 模拟时间流逝（把 last_failure_at 拨回 31 秒前）
        {
            let mut m = h.write().await;
            m.get_mut("ch1").unwrap().last_failure_at = now_secs() - 31;
        }
        // is_available 应为 true（open 超时→可试探）
        {
            let m = h.read().await;
            assert!(is_available(m.get("ch1").unwrap(), &c), "恢复超时后应可试探");
        }
        // half_open 下连续成功 3 次 → closed
        for _ in 0..3 {
            record_outcome(&h, "ch1", true, 0, &c, false).await;
        }
        let m = h.read().await;
        assert_eq!(m.get("ch1").unwrap().state, CircuitState::Closed, "half_open 恢复后 closed");
    }

    #[tokio::test]
    async fn halfopen_fail_reopens() {
        let h = health_map();
        let c = cb();
        // 打开
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &c, false).await;
        }
        // 时间流逝
        {
            let mut m = h.write().await;
            m.get_mut("ch1").unwrap().last_failure_at = now_secs() - 31;
        }
        // 试探成功 2 次（不到 3）
        record_outcome(&h, "ch1", true, 0, &c, false).await;
        record_outcome(&h, "ch1", true, 0, &c, false).await;
        // 第 3 次失败 → 重新 open
        record_outcome(&h, "ch1", false, 0, &c, false).await;
        let m = h.read().await;
        assert_eq!(m.get("ch1").unwrap().state, CircuitState::Open, "half_open 失败应重新 open");
    }

    #[tokio::test]
    async fn select_channel_skips_open() {
        let mut cfg = GatewayConfig::default();
        cfg.circuit_breaker = cb();
        cfg.providers.push(Provider {
            name: "p1".into(),
            base_url: "http://x/v1".into(),
            disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel {
                id: "ch1".into(),
                name: "c1".into(),
                path: None,
                key_name: None,
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] }],
                weight: 1, rpm: None,
            }],
            proxy: String::new(),
        });
        let h = health_map();
        // ch1 打开 + 未超恢复超时 → select_channel 应回退到它（KeepAll，全熔断兼底）
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &cb(), false).await;
        }
        let sel = select_channel(&cfg, "my-model", &h);
        assert!(sel.is_some(), "全熔断时 KeepAll 仍应返回通道");
    }

    #[tokio::test]
    async fn sticky_reuses_same_channel() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300 };
        // 两条通道都能服务 my-model
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
            ],
            proxy: String::new(),
        });
        let h = health_map();
        let pins = new_pin_store();
        // 第一次选通道（应 pin 到 ch1，第一个候选）
        let s1 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:abc"));
        assert!(s1.is_some());
        let ch1_id = s1.unwrap().channel.id.clone();
        // 第二次同 session → 应复用同通道（黏性）
        let s2 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:abc"));
        assert_eq!(s2.unwrap().channel.id, ch1_id, "同 session 应黏同通道");
    }

    #[tokio::test]
    async fn sticky_repins_when_channel_unhealthy() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300 };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
            ],
            proxy: String::new(),
        });
        let h = health_map();
        let pins = new_pin_store();
        // 先 pin 到 ch1
        let s1 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:xyz"));
        let ch1 = s1.unwrap().channel.id.clone();
        assert_eq!(ch1, "ch1");
        // ch1 连续失败 5 次 → open（但 ch2 健康，非单部署）
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &cfg.circuit_breaker, false).await;
        }
        // 同 session 再选 → ch1 熔断，应重新 pin 到 ch2
        let s2 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:xyz"));
        assert_eq!(s2.unwrap().channel.id, "ch2", "pin 的通道熔断应重新 pin");
    }

    #[tokio::test]
    async fn sticky_disabled_always_fresh() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300 };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None }],
            proxy: String::new(),
        });
        let h = health_map();
        let pins = new_pin_store();
        // 关闭黏性 → 不打 pin
        let _ = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:1"));
        let map = pins.try_read().unwrap();
        assert!(map.is_empty(), "黏性关闭不应打 pin");
    }

    #[tokio::test]
    async fn sticky_ttl_expired_repins() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300 };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None }],
            proxy: String::new(),
        });
        let h = health_map();
        let pins = new_pin_store();
        // pin 到 ch1
        let _ = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:t"));
        // 把 pinned_at 拨回 301 秒前（TTL 300 已过期）
        {
            let mut m = pins.write().await;
            m.get_mut("sess:t").unwrap().pinned_at = now_secs() - 301;
        }
        // TTL 过期 → pin 失效 → 重新选 + 重新 pin（pinned_at 更新）
        let sel = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:t"));
        assert!(sel.is_some());
        let m = pins.read().await;
        let age = now_secs().saturating_sub(m.get("sess:t").unwrap().pinned_at);
        assert!(age < 300, "TTL 过期后应重新 pin（pinned_at 刷新）");
    }

    #[tokio::test]
    async fn latency_picks_lowest_ttft() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300 }; // 关黏性，纯测延迟
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "slow".into(), name: "slow".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
                Channel { id: "fast".into(), name: "fast".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
            ],
            proxy: String::new(),
        });
        let h = health_map();
        // slow 通道平均 TTFT 800ms，fast 通道 200ms
        for _ in 0..5 {
            record_outcome(&h, "slow", true, 800, &cfg.circuit_breaker, false).await;
            record_outcome(&h, "fast", true, 200, &cfg.circuit_breaker, false).await;
        }
        // 延迟感知应选 fast 通道
        let sel = select_channel(&cfg, "my-model", &h);
        assert_eq!(sel.unwrap().channel.id, "fast", "应选 TTFT 最低的通道");
    }

    #[tokio::test]
    async fn latency_window_capped_at_10() {
        let h = health_map();
        let c = cb();
        // 记录 20 次成功，TTFT 递增
        for i in 0..20 {
            record_outcome(&h, "ch1", true, (i as u64) * 10, &c, false).await;
        }
        let m = h.read().await;
        let hc = m.get("ch1").unwrap();
        assert_eq!(hc.ttft_window.len(), LATENCY_WINDOW_SIZE, "窗口应封顶 10");
        // 最近 10 个是 i=10..19 → ttft 100..190
        assert_eq!(hc.ttft_window.first(), Some(&100), "窗口应是最 recent 的 10 个");
    }

    // ---- 可配置策略测试 ----

    fn two_chan_cfg(strategy: RoutingStrategy) -> GatewayConfig {
        let mut cfg = GatewayConfig::default();
        cfg.routing = RoutingConfig { strategy, auto_mode_enabled: true, max_retries: 3 };
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300 };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
            ],
            proxy: String::new(),
        });
        cfg
    }

    #[tokio::test]
    async fn strategy_least_busy_picks_fewer_requests() {
        let cfg = two_chan_cfg(RoutingStrategy::LeastBusy);
        let h = health_map();
        // ch1 已经接了 100 单，ch2 只有 1 单
        for _ in 0..100 { record_outcome(&h, "ch1", true, 0, &cfg.circuit_breaker, false).await; }
        record_outcome(&h, "ch2", true, 0, &cfg.circuit_breaker, false).await;
        // 最闲策略应选 ch2
        let sel = select_channel(&cfg, "my-model", &h);
        assert_eq!(sel.unwrap().channel.id, "ch2", "最闲应选请求数最低的");
    }

    #[tokio::test]
    async fn strategy_weight_skews_to_heavy() {
        let mut cfg = two_chan_cfg(RoutingStrategy::Weight);
        // ch1 权重 9，ch2 权重 1
        cfg.providers[0].channels[0].weight = 9;
        cfg.providers[0].channels[1].weight = 1;
        let h = health_map();
        // 跑 200 次，权重高的应明显多（统计断言，非精确）
        let mut ch1_count = 0;
        for _ in 0..200 {
            if select_channel(&cfg, "my-model", &h).unwrap().channel.id == "ch1" {
                ch1_count += 1;
            }
        }
        assert!(ch1_count > 120, "权重 9:1，ch1 应显著多（实际 {ch1_count}/200）");
    }

    #[tokio::test]
    async fn strategy_shuffle_always_valid() {
        let cfg = two_chan_cfg(RoutingStrategy::SimpleShuffle);
        let h = health_map();
        // 随机策略每次都应返回合法通道（不 panic、是两个之一）
        for _ in 0..50 {
            let id = select_channel(&cfg, "my-model", &h).unwrap().channel.id;
            assert!(id == "ch1" || id == "ch2");
        }
    }

    // ---- Auto 模式测试（capability 匹配 + fallback 链）----

    #[tokio::test]
    async fn auto_filters_by_capability() {
        let mut cfg = GatewayConfig::default();
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel {
                id: "ch-text".into(), name: "text".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "text-only".into(), alias: None, capabilities: vec![] }],
                weight: 1, rpm: None,
            }],
            proxy: String::new(),
        });
        let h = health_map();
        // 带图请求需要 vision，text-only 没该能力 → 应过滤掉
        let payload = serde_json::json!({"messages":[{"role":"user","content":[
            {"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"x"}}]}]});
        let cands = auto_candidates(&cfg, &payload, &h);
        assert!(cands.is_empty(), "无 vision 能力应被过滤");
    }

    #[tokio::test]
    async fn auto_matches_vision_and_sorts_by_latency() {
        let mut cfg = GatewayConfig::default();
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch-slow".into(), name: "slow".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m-slow".into(), alias: None, capabilities: vec!["vision".into()] }],
                    weight: 1, rpm: None },
                Channel { id: "ch-fast".into(), name: "fast".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m-fast".into(), alias: None, capabilities: vec!["vision".into()] }],
                    weight: 1, rpm: None },
            ],
            proxy: String::new(),
        });
        let h = health_map();
        // fast 200ms, slow 800ms
        for _ in 0..5 {
            record_outcome(&h, "ch-slow", true, 800, &cfg.circuit_breaker, false).await;
            record_outcome(&h, "ch-fast", true, 200, &cfg.circuit_breaker, false).await;
        }
        // 带图请求 → 两个都有 vision，按延迟排 fast 优先
        let payload = serde_json::json!({"messages":[{"role":"user","content":[
            {"type":"image_url","image_url":{"url":"x"}}]}]});
        let cands = auto_candidates(&cfg, &payload, &h);
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].channel.id, "ch-fast", "延迟最低应排第一");
    }

    #[tokio::test]
    async fn auto_skips_open_channel() {
        let mut cfg = GatewayConfig::default();
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel {
                id: "ch-open".into(), name: "open".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "m".into(), alias: None, capabilities: vec![] }],
                weight: 1, rpm: None,
            }],
            proxy: String::new(),
        });
        let h = health_map();
        // ch-open 连续失败 5 次 → open（未超恢复时间）
        for _ in 0..5 {
            record_outcome(&h, "ch-open", false, 0, &cfg.circuit_breaker, false).await;
        }
        // auto 应跳过熔断通道
        let payload = serde_json::json!({"messages":[{"role":"user","content":"hi"}]});
        let cands = auto_candidates(&cfg, &payload, &h);
        assert!(cands.is_empty(), "熔断通道应被跳过");
    }

    // ---- failover 候选链测试 ----

    #[tokio::test]
    async fn ordered_candidates_pin_first_and_chain() {
        let mut cfg = GatewayConfig::default();
        cfg.routing.max_retries = 3;
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "chA".into(), name: "a".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
                Channel { id: "chB".into(), name: "b".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None },
            ],
            proxy: String::new(),
        });
        let h = health_map();
        // 无 pin：应返回两条候选（failover 链完整）
        let cands = ordered_candidates(&cfg, "my-model", &h, None);
        assert_eq!(cands.len(), 2, "应返回完整 failover 链");
        // pin chB：应提到链首
        let cands2 = ordered_candidates(&cfg, "my-model", &h, Some("chB"));
        assert_eq!(cands2[0].channel.id, "chB", "pin 通道应优先");
    }

    #[tokio::test]
    async fn ordered_candidates_skips_disabled_provider() {
        let mut cfg = GatewayConfig::default();
        cfg.providers.push(Provider {
            name: "off".into(), base_url: "http://x/v1".into(), disabled: true,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "chOff".into(), name: "off".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None }],
            proxy: String::new(),
        });
        cfg.providers.push(Provider {
            name: "on".into(), base_url: "http://y/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "chOn".into(), name: "on".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] }], weight: 1, rpm: None }],
            proxy: String::new(),
        });
        let h = health_map();
        let cands = ordered_candidates(&cfg, "my-model", &h, None);
        assert_eq!(cands.len(), 1, "disabled provider 应被跳过");
        assert_eq!(cands[0].channel.id, "chOn");
    }

    // ---- 下游鉴权测试 ----

    #[test]
    fn auth_no_keys_allows_all() {
        // api_keys 空 → 不鉴权（内网可用），任意 header 都放行
        let auth = AuthConfig { api_keys: vec![] };
        assert!(validate_auth(&auth, None).is_ok());
        assert!(validate_auth(&auth, Some("Bearer anything")).is_ok());
    }

    #[test]
    fn auth_configured_requires_valid_key() {
        let auth = AuthConfig { api_keys: vec!["sk-secret".into()] };
        // 无 header → 401
        assert!(validate_auth(&auth, None).is_err());
        // 错 key → 401
        assert!(validate_auth(&auth, Some("Bearer wrong")).is_err());
        // 对 key → 放行
        assert!(validate_auth(&auth, Some("Bearer sk-secret")).is_ok());
    }

    // ---- 限流测试（防打爆上游）----

    #[test]
    fn rate_limit_allows_under_rpm() {
        let store = new_rate_store();
        // 限 10 rpm，发 5 次 → 都放行
        for _ in 0..5 {
            assert!(!is_rate_limited(&store, "ch1", 10), "未超限应放行");
        }
    }

    #[test]
    fn rate_limit_blocks_over_rpm() {
        let store = new_rate_store();
        // 限 3 rpm，发 3 次（都放行），第 4 次应拦
        for _ in 0..3 {
            assert!(!is_rate_limited(&store, "ch2", 3));
        }
        assert!(is_rate_limited(&store, "ch2", 3), "超 rpm 应限流");
    }

    #[test]
    fn rate_limit_per_channel_independent() {
        let store = new_rate_store();
        // ch1 打满，ch2 不受影响（按通道独立计数）
        for _ in 0..3 {
            is_rate_limited(&store, "ch1", 3);
        }
        assert!(is_rate_limited(&store, "ch1", 3), "ch1 应限流");
        assert!(!is_rate_limited(&store, "ch2", 3), "ch2 应独立放行");
    }
}
