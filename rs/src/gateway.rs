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

/// 重试策略（全局默认，每通道可用 retry 字段覆盖部分项）。学 LiteLLM retry/allowed_fails_policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// 最大重试次数（failover 尝试多少条通道）
    #[serde(default = "def_retry_max")]
    pub max_retries: usize,
    /// 哪些状态码可重试（failover 到其他通道）
    #[serde(default = "def_retry_on_status")]
    pub retry_on_status: Vec<u16>,
    /// 哪些状态码致命（直接透传，不 failover）——鉴权/参数错误换通道也没用
    #[serde(default = "def_fatal_status")]
    pub fatal_status: Vec<u16>,
    /// 退避起始毫秒
    #[serde(default = "def_backoff_initial")]
    pub backoff_initial_ms: u64,
    /// 退避上限毫秒
    #[serde(default = "def_backoff_max")]
    pub backoff_max_ms: u64,
    /// jitter 比例 [0,1)，防雪崩
    #[serde(default = "def_jitter")]
    pub jitter: f64,
    /// 是否尊重上游 retry-after 响应头
    #[serde(default = "def_respect_retry_after")]
    pub respect_retry_after: bool,
    /// 关键词熔断（学 new-api AutomaticDisableKeywords）：上游错误响应体含这些词 →
    /// 一次命中即强制熔断该通道 + failover（不等连续失败次数）。针对欠费/封号等"重试也没用"的错误。
    #[serde(default)]
    pub disable_keywords: Vec<String>,
    /// 上下文不足关键词（context_length_exceeded 等）：命中 → 视为"模型窗口不够"，
    /// 可换更大窗口通道重试（豁免 max_retries），全候选都窗口不够才反馈客户端。
    #[serde(default)]
    pub context_keywords: Vec<String>,
}

fn def_retry_max() -> usize { 3 }
fn def_retry_on_status() -> Vec<u16> { vec![408, 409, 429, 500, 502, 503, 504] }
fn def_fatal_status() -> Vec<u16> { vec![400, 401, 403, 404] }
fn def_backoff_initial() -> u64 { 500 }
fn def_backoff_max() -> u64 { 8000 }
fn def_jitter() -> f64 { 0.75 }
fn def_respect_retry_after() -> bool { true }
/// 关键词熔断默认词表（学 new-api，覆盖常见欠费/封号/配额耗尽表述）
fn def_disable_keywords() -> Vec<String> {
    vec![
        "insufficient quota".into(),
        "insufficient_quota".into(),
        "credit balance is too low".into(),
        "exceeded your current quota".into(),
        "organization has been disabled".into(),
        "account has been disabled".into(),
        "billing".into(),
        "余额不足".into(),
        "配额已用尽".into(),
    ]
}

/// 上下文不足默认词表（context_length_exceeded 等）：模型窗口装不下请求
fn def_context_keywords() -> Vec<String> {
    vec![
        "context_length_exceeded".into(),
        "maximum context length".into(),
        "exceeds the maximum".into(),
        "too many tokens".into(),
        "context length".into(),
    ]
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: def_retry_max(),
            retry_on_status: def_retry_on_status(),
            fatal_status: def_fatal_status(),
            backoff_initial_ms: def_backoff_initial(),
            backoff_max_ms: def_backoff_max(),
            jitter: def_jitter(),
            respect_retry_after: def_respect_retry_after(),
            disable_keywords: def_disable_keywords(),
            context_keywords: def_context_keywords(),
        }
    }
}

impl RetryConfig {
    /// 状态码是否可重试（failover）：在 retry_on_status 或 5xx 默认可重试
    pub fn is_retryable(&self, status: u16) -> bool {
        self.retry_on_status.contains(&status) || (status >= 500 && !self.fatal_status.contains(&status))
    }
    /// 状态码是否致命（直接透传）
    pub fn is_fatal(&self, status: u16) -> bool {
        self.fatal_status.contains(&status)
    }
}

/// 通道级重试覆盖：只配想改的字段，None=继承全局 RetryConfig
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetryOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_on_status: Option<Vec<u16>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fatal_status: Option<Vec<u16>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_initial_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_max_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respect_retry_after: Option<bool>,
}

/// 通道：供应商内部一条具体接入方式（key × 路径 × 模型映射三件套绑死，学 LiteLLM Deployment）
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// 通道级重试覆盖（空=继承全局 retry）；只配想改的字段
    #[serde(default)]
    pub retry: Option<RetryOverride>,
    /// 该通道模型的最大输入 token（上下文窗口过滤：超长请求跳过装不下的通道）
    #[serde(default)]
    pub max_input_tokens: Option<i64>,
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
    /// 协议（openai_chat/openai_responses/anthropic/gemini/ollama，前端配置）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// 模型类型（chat/webSearch/webFetch/embedding/vision，前端配置）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 模型级上下文窗口（token）：上下文错误反馈时跳过装不下的（配置时填，运行时零成本）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    /// 档位标注（对标 Gemini pro/flash）：pro=质量型、flash=速度型；空=auto 专属
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
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
    /// 代理池引用（学 9router proxyPools；空=不用池）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_pool_id: String,
    /// 代理池选择策略（round-robin/random/none）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_strategy: String,
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
    /// 重试策略（全局默认，学 LiteLLM retry；通道可覆盖）
    #[serde(default)]
    pub retry: RetryConfig,
    /// 超时（流式首字/非流式，学 cc-switch）
    #[serde(default)]
    pub timeout: TimeoutConfig,
    /// 下游鉴权（agent 调网关校验 Bearer key）
    #[serde(default)]
    pub auth: AuthConfig,
    /// 响应缓存（相同请求返回缓存，省 token，学 LiteLLM caching）
    #[serde(default)]
    pub response_cache: ResponseCacheConfig,
    /// 模型级降级链（gpt-4 挂→降级 gpt-4o，区别于通道级 failover，学 LiteLLM fallbacks）
    #[serde(default)]
    pub model_fallback: ModelFallbackConfig,
    /// 通道测试配置（默认模型 + 测试对话，用户可编辑）
    #[serde(default)]
    pub test: TestConfig,
}

impl GatewayConfig {
    /// 熔断器配置：routing.circuit_breaker 优先（前端保存路径），回退顶层 circuit_breaker
    pub fn cb(&self) -> &CircuitBreakerConfig {
        self.routing.circuit_breaker.as_ref().unwrap_or(&self.circuit_breaker)
    }
    /// 黏性配置：routing.sticky 优先，回退顶层 sticky
    pub fn sticky_cfg(&self) -> &StickyConfig {
        self.routing.sticky.as_ref().unwrap_or(&self.sticky)
    }
    /// 超时配置：routing.timeout 优先，回退顶层 timeout
    pub fn timeout_cfg(&self) -> &TimeoutConfig {
        self.routing.timeout.as_ref().unwrap_or(&self.timeout)
    }
}

/// 通道测试配置（用户可编辑，放 gateway.yaml，学 new-api TestChannel 的可配置测试内容）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    /// 测试用的默认模型（空=用通道第一个模型）
    #[serde(default)]
    pub default_model: String,
    /// 测试对话内容
    #[serde(default = "def_test_message")]
    pub test_message: String,
    /// 测试最大输出 token
    #[serde(default = "def_test_max_tokens")]
    pub max_tokens: i64,
}

fn def_test_message() -> String { "ping".into() }
fn def_test_max_tokens() -> i64 { 16 }

impl Default for TestConfig {
    fn default() -> Self {
        Self { default_model: String::new(), test_message: def_test_message(), max_tokens: def_test_max_tokens() }
    }
}

/// 响应缓存配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseCacheConfig {
    /// 是否启用
    #[serde(default)]
    pub enabled: bool,
    /// 缓存 TTL 秒
    #[serde(default = "def_cache_ttl")]
    pub ttl_seconds: u64,
    /// 最大缓存条目（防内存涨，个人网关够用）
    #[serde(default = "def_cache_max_entries")]
    pub max_entries: usize,
}

fn def_cache_ttl() -> u64 { 300 }
fn def_cache_max_entries() -> usize { 1000 }

impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self { enabled: false, ttl_seconds: def_cache_ttl(), max_entries: def_cache_max_entries() }
    }
}

/// 模型级降级链配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelFallbackConfig {
    /// 是否启用
    #[serde(default)]
    pub enabled: bool,
    /// 降级链：主模型 → [备选模型...]（按序尝试）。键=对外请求的模型名
    #[serde(default)]
    pub chains: std::collections::HashMap<String, Vec<String>>,
}

fn def_listen() -> u16 { 8380 }

/// TTFT 滑动窗口大小（学 LiteLLM lowest_latency max_latency_list_size=10）
/// 注：算法常量，未做成产品配置（record_outcome 有 26+ 调用点，参数化收益低风险高）。
/// 前端 latency_window_seconds 为「秒」语义，与本处「条数」不一致，故该字段当前仅存档。
const LATENCY_WINDOW_SIZE: usize = 10;
/// 延迟 buffer 比例（学 lowest_latency_buffer，避免延迟接近时反复横跳）
const LATENCY_BUFFER_RATIO: f64 = 0.2;
/// 无延迟记录通道的中性分数（低于已知的差通道，但不抢最快的）
/// 注：算法常量（非产品配置），60s 是经验值。
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
    pub protocol: Option<String>,
}

/// 当前 user turn = 最后一条 assistant/model 之后到末尾的消息（学 9router `trailingUserItems`）。
/// 这是「本轮硬需求」的边界：只有这里的媒体代表客户端此刻真的要模型看。
/// 最后一个 assistant 之后的所有消息都是当前 user turn（可跨多条，如 text + image 分块）。
fn trailing_user_turn(msgs: &[serde_json::Value]) -> &[serde_json::Value] {
    let mut i = msgs.len();
    while i > 0 {
        let role = msgs[i - 1].get("role").and_then(|v| v.as_str());
        if role == Some("assistant") || role == Some("model") { break; }
        i -= 1;
    }
    &msgs[i..]
}

/// 检测请求需要的硬能力（vision：带 image_url；pdf：file）。
/// **只扫当前 user turn**（历史轮的媒体属软需求，走 strip，不 pin 住能力选择）。
fn detect_required_caps(payload: &serde_json::Value) -> Vec<String> {
    let mut caps = Vec::new();
    // 思考模式（同 vision 硬约束待遇）：请求显式开启思考（顶层参数）→ 需要 thinking 能力。
    // 覆盖三种协议：anthropic `thinking`、responses `reasoning`、chat `reasoning_effort`。
    // `thinking: {type: "disabled"}` 不算（客户端明确关）。
    let thinking_requires = payload.get("thinking").is_some_and(|t| {
        t.get("type").and_then(|v| v.as_str()) != Some("disabled")
    });
    if thinking_requires || payload.get("reasoning").is_some() || payload.get("reasoning_effort").is_some() {
        caps.push("thinking".into());
    }
    if let Some(msgs) = payload.get("messages").and_then(|v| v.as_array()) {
        for msg in trailing_user_turn(msgs) {
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

/// 历史轮的媒体块 → 占位符文本（软降级，不报错也不丢语义）。
/// 当前 user turn 的媒体**不动**（硬需求，由能力过滤/409 处理）。
/// 返回实际替换的块数（便于测试/日志）。
fn strip_history_media(messages: &mut Vec<serde_json::Value>, cap: &str) -> usize {
    const IMAGE_OMITTED: &str = "[image omitted: model does not support vision]";
    const FILE_OMITTED: &str = "[file omitted: model does not support pdf]";
    let boundary = if messages.is_empty() { 0 } else { (messages.len() - trailing_user_turn(messages).len()) };
    let mut replaced = 0usize;
    for msg in messages.iter_mut().take(boundary) {
        let is_array = msg.get("content").map(|v| v.is_array()).unwrap_or(false);
        if !is_array { continue; }
        let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) else { continue; };
        let mut out = Vec::with_capacity(content.len());
        for part in content.drain(..) {
            let is_media = matches!(part.get("type").and_then(|v| v.as_str()),
                Some("image_url") | Some("image") | Some("file"));
            if is_media {
                let placeholder = if cap == "pdf" { FILE_OMITTED } else { IMAGE_OMITTED };
                out.push(serde_json::json!({ "type": "text", "text": placeholder }));
                replaced += 1;
            } else {
                out.push(part);
            }
        }
        if out.is_empty() { msg["content"] = serde_json::Value::String(String::new()); } else { *content = out; }
    }
    replaced
}

/// 该模型名是否存在于任一未禁用通道的 models 里（不滤能力）。
/// 用于区分「模型不存在（404）」和「模型存在但无一个满足当前轮硬能力（409）」。
fn model_exists(config: &GatewayConfig, requested_model: &str) -> bool {
    config.providers.iter().any(|p| !p.disabled && p.channels.iter().any(|c| {
        c.models.iter().any(|m| {
            let external = m.alias.as_deref().unwrap_or(&m.name);
            external == requested_model || m.name == requested_model
        })
    }))
}
/// 选中通道模型的能力清单（上游模型名 → 找模型定义取 capabilities）。
/// 找不到时返回空（视为不支持任何硬能力 → 历史轮媒体一律降级）。
fn upstream_caps_of(config: &GatewayConfig, selected: &SelectedChannel) -> Vec<String> {
    for provider in &config.providers {
        for channel in &provider.channels {
            if channel.id != selected.channel.id { continue; }
            for m in &channel.models {
                if m.name == selected.upstream_model {
                    return m.capabilities.clone();
                }
            }
        }
    }
    Vec::new()
}



/// Auto 模式：聚合所有可用通道的模型，硬能力匹配（缺 vision 不接带图请求），
/// 按延迟升序排成 fallback 链（跳过熔断通道）。
pub fn auto_candidates(
    config: &GatewayConfig,
    payload: &serde_json::Value,
    health: &HealthMap,
    class_filter: Option<&str>,
) -> Vec<AutoCandidate> {
    let required = detect_required_caps(payload);
    let hs = health.try_read().ok();
    let cb = config.cb();
    let mut out: Vec<AutoCandidate> = Vec::new();

    for provider in &config.providers {
        if provider.disabled { continue; }
        for channel in &provider.channels {
            for m in &channel.models {
                // 档位过滤：pro/flash 请求只收对应 class 的模型（空=不过滤，全部）
                if let Some(cf) = class_filter {
                    if m.class.as_deref() != Some(cf) { continue; }
                }
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
                    protocol: m.protocol.clone(),
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
    /// 黏性粒度（session/api-key/continuity，前端可配）
    #[serde(default)]
    pub granularity: Option<String>,
}
fn def_sticky_enabled() -> bool { true }
fn def_sticky_ttl() -> u64 { 300 }
impl Default for StickyConfig {
    fn default() -> Self {
        Self { enabled: def_sticky_enabled(), ttl_seconds: def_sticky_ttl(), granularity: None }
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
    /// 前端保存路径：熔断器在 routing 段下（嵌套）；读时优先，回退顶层 circuit_breaker（向后兼容）
    #[serde(default)]
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    /// 前端保存路径：黏性在 routing 段下；优先，回退顶层 sticky
    #[serde(default)]
    pub sticky: Option<StickyConfig>,
    /// 前端保存路径：超时在 routing 段下；优先，回退顶层 timeout
    #[serde(default)]
    pub timeout: Option<TimeoutConfig>,
    /// 延迟滑动窗口秒（TTFT 统计窗口，前端可配）
    #[serde(default)]
    pub latency_window_seconds: Option<u64>,
    /// 全熔断兜底策略（keep-all/fail，前端可配）
    #[serde(default)]
    pub when_all_unhealthy: Option<String>,
}
fn def_max_retries() -> usize { 3 }
fn def_auto_enabled() -> bool { true }
impl Default for RoutingConfig {
    fn default() -> Self {
        Self { strategy: RoutingStrategy::default(), auto_mode_enabled: def_auto_enabled(), max_retries: def_max_retries(),
            circuit_breaker: None, sticky: None, timeout: None, latency_window_seconds: None, when_all_unhealthy: None }
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
    /// 流式空闲超时秒（两个 chunk 间上限；前端可配，对齐前后端字段）
    #[serde(default = "def_streaming_idle")]
    pub streaming_idle: u64,
}
fn def_stream_first_byte() -> u64 { 30 }
fn def_non_streaming() -> u64 { 120 }
fn def_streaming_idle() -> u64 { 60 }
impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            streaming_first_byte: def_stream_first_byte(),
            non_streaming: def_non_streaming(),
            streaming_idle: def_streaming_idle(),
        }
    }
}

/// 下游鉴权配置（agent 调网关时校验 Bearer key，防裸奍）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 允许的下游 key 列表（空=不鉴权，本机内网可用；配了=严格校验 Bearer）。
    /// 支持两种形态：字符串 `sk-xxx`（无设备名）或对象 `{name: "手机", key: "sk-xxx"}`（多设备身份区分）。
    #[serde(default)]
    pub api_keys: Vec<AuthKey>,
    /// 每个 key 每日 token 上限（input+output+cache；None/0=不限）。超限返 429。学 LiteLLM max_budget
    #[serde(default)]
    pub daily_token_budget: Option<i64>,
}

/// 下游 key：裸字符串（无设备名）或带设备名对象。untagged 向后兼容 `api_keys: ["sk-1"]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthKey {
    Plain(String),
    Named { name: String, key: String },
}

impl AuthKey {
    fn key(&self) -> &str {
        match self { AuthKey::Plain(k) => k, AuthKey::Named { key, .. } => key }
    }
    fn device_name(&self) -> &str {
        match self { AuthKey::Plain(k) => k, AuthKey::Named { name, .. } => name }
    }
    fn matches(&self, token: &str) -> bool { self.key() == token }
}


impl Default for AuthConfig {
    fn default() -> Self {
        Self { api_keys: Vec::new(), daily_token_budget: None }
    }
}

/// 校验下游 Bearer key（api_keys 空=不鉴权内网可用；配了=严格校验）。
/// 返回设备名（多设备身份区分：Named 的 name；Plain 用 key 本身）；未配返回 None。
pub fn validate_auth(auth: &AuthConfig, auth_header: Option<&str>) -> Result<Option<String>, String> {
    if auth.api_keys.is_empty() {
        return Ok(None); // 未配 key → 不鉴权（本机/内网）
    }
    let token = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    if token.is_empty() {
        return Err("缺少 Authorization Bearer key".into());
    }
    for k in &auth.api_keys {
        if k.matches(token) {
            return Ok(Some(k.device_name().to_string()));
        }
    }
    Err("无效的 API key".into())
}

/// 取下游设备名（多设备身份区分：Named→name；Plain→key 本身；未配→原始 key 兜底）
fn downstream_key(auth: &AuthConfig, auth_header: Option<&str>) -> Option<String> {
    let token = auth_header.and_then(|h| h.strip_prefix("Bearer "))?;
    for k in &auth.api_keys {
        if k.matches(token) {
            return Some(k.device_name().to_string());
        }
    }
    Some(token.to_string())
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

// ==================== 通道选择（按模型名，最小版：第一个能服务该模型的健康通道）====================

/// 选中的通道 + 解析出的上游真实模型名 + 用的 key
pub struct SelectedChannel {
    pub provider: Provider,
    pub channel: Channel,
    /// 上游真实模型名（把请求里的 alias 翻译回 name）
    pub upstream_model: String,
    /// 实际用的 API key（从密钥池解析）
    pub api_key: String,
    /// 上游协议（ChannelModel.protocol）：None/"chat"=OpenAI chat；"anthropic"=Claude 上游
    pub protocol: Option<String>,
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
    let cb = config.cb();
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
    let when_all = config.routing.when_all_unhealthy.as_deref().unwrap_or("keep-all");
    let Some(idx) = apply_strategy(&candidates, hs, cb, &config.routing.strategy, when_all) else { return None; };
    let (provider, channel, m) = candidates[idx];
    let api_key = resolve_key(provider, channel).unwrap_or_default();
    Some(SelectedChannel {
        provider: (*provider).clone(),
        channel: (*channel).clone(),
        upstream_model: m.name.clone(),
        api_key,
        protocol: m.protocol.clone(),
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
pub fn session_key(payload: &serde_json::Value, auth_header: Option<&str>, granularity: &str) -> Option<String> {
    // api-key 粒度：只用下游 key hash（不读 session_id）
    if granularity == "api-key" {
        return hash_bearer(auth_header);
    }
    // continuity 粒度：只用 previous_response_id（Responses API 多轮连续）
    if granularity == "continuity" {
        if let Some(prev) = payload.get("previous_response_id").and_then(|v| v.as_str()) {
            if !prev.is_empty() { return Some(format!("cont:{}", prev)); }
        }
        return None;
    }
    // 1. 显式 session id（agent 传 X-Session-Id）
    if let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()) {
        if !sid.is_empty() {
            return Some(format!("sess:{}", sid));
        }
    }
    // 2. 下游 API key（同一 key 的连续请求黏同一通道）
    hash_bearer(auth_header)
}

/// Bearer token → hash（不存明文）；无则 None
fn hash_bearer(auth_header: Option<&str>) -> Option<String> {
    if let Some(auth) = auth_header {
        if let Some(token) = auth.strip_prefix("Bearer ") {
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
    input_tokens: i64,
    required: &[String],
) -> Vec<SelectedChannel> {
    let hs = health.try_read().ok();
    let hs = hs.as_deref();
    let cb = config.cb();
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

    // 上下文窗口过滤（学 LiteLLM max_input_tokens）：超长请求跳过装不下的通道
    if input_tokens > 0 {
        candidates.retain(|(_, c, _)| c.max_input_tokens.map_or(true, |max| input_tokens <= max));
        if candidates.is_empty() { return Vec::new(); }
    }

    // 硬能力过滤（学 9router HARD_CAPS）：当前轮需要的能力模型必须都有，缺则不接
    if !required.is_empty() {
        candidates.retain(|(_, _, m)| {
            required.iter().all(|c| m.capabilities.iter().any(|mc| mc == c))
        });
        if candidates.is_empty() { return Vec::new(); }
    }

    let when_all = config.routing.when_all_unhealthy.as_deref().unwrap_or("keep-all");
    let Some(first) = apply_strategy(&candidates, hs, cb, strategy, when_all) else { return Vec::new(); };
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
            protocol: m.protocol.clone() });
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
    let cb = config.cb();
    // 1. 查有效 pin（TTL 内 + 通道仍健康）
    if let Some(key) = pin_key {
        if config.sticky_cfg().enabled {
            if let Ok(map) = pins.try_read() {
                if let Some(entry) = map.get(key) {
                    let age = now_secs().saturating_sub(entry.pinned_at);
                    if age < config.sticky_cfg().ttl_seconds {
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
        if config.sticky_cfg().enabled {
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
                        protocol: m.protocol.clone() });
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

/// 学 cc-switch strip_one_m_suffix_for_route_lookup：剥离 Claude Code 的 [1M]/[1m] 1M 上下文标记。
/// 客户端发 model=auto[1M]，网关剥离后识别档位 auto（黑盒调度），真实模型名由网关选。
fn strip_1m_suffix(model: &str) -> String {
    if model.ends_with("[1M]") || model.ends_with("[1m]") {
        model[..model.len() - 4].to_string()
    } else {
        model.to_string()
    }
}

pub async fn forward_generic(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    upstream_path: &str,
    cache: &ResponseCacheStore,
) -> axum::response::Response {
    let config = load_config(root);
    // 下游鉴权：校验 Bearer key（api_keys 配了才校验，空=内网不鉴权）。
    // 成功时返回设备名（多设备身份区分：Named→name，日志/统计按设备聚合）。
    let device_name = match validate_auth(&config.auth, auth_header) {
        Ok(n) => n,
        Err(e) => {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": {"message": e, "type": "authentication_error"}})),
            ).into_response();
        }
    };
    // 用量限额（学 LiteLLM max_budget）：配了 daily_token_budget → 查今日该 key 已用 token，超限返 429
    if let Some(budget) = config.auth.daily_token_budget {
        if budget > 0 {
            if let Some(dkey) = downstream_key(&config.auth, auth_header) {
                let used = crate::gateway_logs::today_tokens_for_key(root, &dkey).unwrap_or(0);
                if used >= budget {
                    return (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        axum::Json(serde_json::json!({"error": {
                            "message": format!("每日 token 预算已用尽（{used}/{budget}），明日重置", ),
                            "type": "budget_exceeded_error"
                        }})),
                    ).into_response();
                }
            }
        }
    }
    let requested_model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // 学 cc-switch：剥离 [1M]/[1m] 后缀（Claude Code 内置的 1M 上下文标记，大小写不敏感）
    // 客户端发 model=auto[1M]，网关剥离后识别档位 auto（黑盒调度），真实模型名由网关选
    let requested_model = strip_1m_suffix(&requested_model);
    let granularity = config.sticky_cfg().granularity.as_deref().unwrap_or("session");
    let pin_key = session_key(&payload, auth_header, granularity);
    let dkey = device_name.or_else(|| downstream_key(&config.auth, auth_header)); // 多设备身份：日志/记账按设备名

    // 档位（auto/pro/flash 是客户端配置的 model 名，保留字）：网关按档位过滤候选
    //   auto → 全部（网关随意，不偏）；pro → 只 class:pro；flash → 只 class:flash
    let is_tier = requested_model == "auto" || requested_model == "pro" || requested_model == "flash";
    if is_tier && (requested_model != "auto" || config.routing.auto_mode_enabled) {
        let class_filter = match requested_model.as_str() {
            "pro" => Some("pro"),
            "flash" => Some("flash"),
            _ => None, // auto：全部
        };
        let candidates = auto_candidates(&config, &payload, health, class_filter);
        if candidates.is_empty() {
            // 带硬需求（当前轮带图）但无满足能力的模型 → 409 显式失败，不静默降级
            let req = detect_required_caps(&payload);
            if !req.is_empty() {
                return (
                    axum::http::StatusCode::CONFLICT,
                    axum::Json(serde_json::json!({"error": {"type": "capability_unavailable",
                        "message": format!("无满足能力的模型可用（需要 {}）", req.join("/")),
                        "required": req}})),
                ).into_response();
            }
            return error_response(404, &format!("{}: 无可用模型", requested_model));
        }
        let mut last_err = String::new();
        let mut attempted: u32 = 0; // 已重试次数（退避指数用）
        let mut last_context_err: Option<String> = None;
        for cand in &candidates {
            let sel = SelectedChannel {
                provider: cand.provider.clone(),
                channel: cand.channel.clone(),
                upstream_model: cand.upstream_model.clone(),
                api_key: cand.api_key.clone(),
                protocol: cand.protocol.clone() };
            // 限流：通道配了 rpm 且超限 → 跳过（防打爆上游）
            if let Some(rpm) = cand.channel.rpm {
                if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
            }
            let policy = resolve_retry_policy(&config.retry, cand.channel.retry.as_ref());
            match forward_one(root, &config, health, &sel, &requested_model, &payload, upstream_path, dkey.as_deref(), &policy).await {
                Ok(resp) => return resp,
                Err(e) => {
                    // context 不足：豁免 max_retries，继续试下一个（换更大窗口通道，全废才反馈）
                    if e.is_context() {
                        last_context_err = Some(e.message());
                        continue;
                    }
                    // 错误分类（学 LiteLLM）：致命 4xx（鉴权/参数）重试也没用 → 直接透传
                    if !e.is_retryable() {
                        return error_response(e.status().unwrap_or(502), &e.message());
                    }
                    last_err = e.message();
                    // 可重试：退避（0.5×2→8s + jitter，尊重 retry-after）再 failover
                    if attempted as usize + 1 < candidates.len() {
                        tokio::time::sleep(retry_delay(&policy, attempted, e.retry_after())).await;
                    }
                    attempted += 1;
                    continue; // fallback 下一个
                }
            }
        }
        if let Some(ctx) = last_context_err {
            return error_response(502, &format!("auto: 所有候选都装不下上下文（最后: {}）", ctx));
        }
        return error_response(502, &format!("auto: 所有候选失败, 最后错误: {}", last_err));
    }

    // 响应缓存查询（学 LiteLLM caching）：非流式 + 启用缓存 → 命中直接返回
    if config.response_cache.enabled && !payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
        let ck = cache_key(&requested_model, &payload);
        if let Some(c) = cache_get(cache, &ck, config.response_cache.ttl_seconds).await {
            let mut r = axum::response::Response::builder().status(c.status);
            if let Some(ct) = c.content_type { r = r.header("content-type", ct); }
            r = r.header("x-cache", "HIT");
            return r.body(axum::body::Body::from(c.body)).unwrap_or_else(|_| error_response(500, "缓存回放失败"));
        }
    }

    // 非 Auto：候选链 + failover（逐个试，失败切下一个，受 max_retries 限制）
    // 黏性：从 pin 表取上次通道提到链首（复用），失败则 failover 到其他通道
    let pinned_id = match pin_key.as_deref() {
        Some(k) => pins.read().await.get(k).map(|e| e.channel_id.clone()),
        None => None,
    };
    let input_tokens = estimate_input_tokens(&payload);
    let required = detect_required_caps(&payload); // 当前轮硬需求（vision/pdf）
    let candidates = ordered_candidates(&config, &requested_model, health, pinned_id.as_deref(), input_tokens, &required);
    if candidates.is_empty() {
        // 模型存在但无一个满足当前轮硬能力 → 显式 409，不静默降级（静默=客户端失守）
        if !required.is_empty() && model_exists(&config, &requested_model) {
            return (
                axum::http::StatusCode::CONFLICT,
                axum::Json(serde_json::json!({"error": {"type": "capability_unavailable",
                    "message": format!("模型 {} 无满足能力的通道（需要 {}）", requested_model, required.join("/")),
                    "required": required}})),
            ).into_response();
        }
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": {"message": format!("模型 {} 无可用通道", requested_model), "type": "invalid_request_error"}
            })),
        ).into_response();
    }
    let max_retries = config.retry.max_retries.min(candidates.len());
    let mut last_err = String::new();
    let mut attempted: u32 = 0; // 已重试次数（退避指数用）
    let mut last_context_err: Option<String> = None;
    for cand in candidates.iter() {
        if (attempted as usize) >= max_retries.max(1) { break; } // 普通失败达上限；context 不足不占次数
        // 限流：通道配了 rpm 且超限 → 跳过（防打爆上游）
        if let Some(rpm) = cand.channel.rpm {
            if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
        }
        let policy = resolve_retry_policy(&config.retry, cand.channel.retry.as_ref());
        match forward_one(root, &config, health, cand, &requested_model, &payload, upstream_path, dkey.as_deref(), &policy).await {
            Ok(resp) => {
                // 成功：更新黏性 pin 到这条通道（供后续复用）
                if let Some(k) = pin_key.as_deref() {
                    if config.sticky_cfg().enabled {
                        pins.write().await.insert(k.to_string(), PinEntry { channel_id: cand.channel.id.clone(), pinned_at: now_secs() });
                    }
                }
                // 响应缓存存储（学 LiteLLM caching）：非流式 + 启用缓存 → 存 body
                if config.response_cache.enabled && !payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let (parts, body) = resp.into_parts();
                    let ct = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                    let bytes = http_body_util::BodyExt::collect(body).await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
                    let ck = cache_key(&requested_model, &payload);
                    cache_put(cache, ck, CachedResponse { status: parts.status.as_u16(), content_type: ct, body: bytes.clone(), cached_at: std::time::Instant::now() }, config.response_cache.max_entries).await;
                    let mut r = axum::response::Response::from_parts(parts, axum::body::Body::empty());
                    *r.body_mut() = axum::body::Body::from(bytes);
                    return r;
                }
                return resp;
            }
            Err(e) => {
                // context 不足：豁免 max_retries，继续试下一个（换更大窗口通道，全废才反馈）
                if e.is_context() {
                    last_context_err = Some(e.message());
                    continue;
                }
                // 错误分类（学 LiteLLM）：致命 4xx（鉴权/参数）重试也没用 → 直接透传
                if !e.is_retryable() {
                    return error_response(e.status().unwrap_or(502), &e.message());
                }
                last_err = e.message();
                // 可重试：退避（0.5×2→8s + jitter，尊重 retry-after）再 failover
                if attempted as usize + 1 < max_retries.max(1) {
                    tokio::time::sleep(retry_delay(&policy, attempted, e.retry_after())).await;
                }
                attempted += 1;
                continue; // failover 下一个
            }
        }
    }
    // 模型级降级链（学 LiteLLM fallbacks）：主模型所有通道失败 → 按降级链试备选模型
    if config.model_fallback.enabled {
        if let Some(chain) = config.model_fallback.chains.get(&requested_model) {
            for fb_model in chain {
                let fb_candidates = ordered_candidates(&config, fb_model, health, None, input_tokens, &required);
                if fb_candidates.is_empty() { continue; }
                let fb_max = config.retry.max_retries.min(fb_candidates.len());
                for cand in fb_candidates.iter().take(fb_max.max(1)) {
                    if let Some(rpm) = cand.channel.rpm {
                        if is_rate_limited(rates, &cand.channel.id, rpm) { continue; }
                    }
                    let policy = resolve_retry_policy(&config.retry, cand.channel.retry.as_ref());
                    let mut fb_payload = payload.clone();
                    fb_payload["model"] = serde_json::Value::String(fb_model.clone());
                    match forward_one(root, &config, health, cand, fb_model, &fb_payload, upstream_path, dkey.as_deref(), &policy).await {
                        Ok(resp) => return resp,
                        Err(e) => { if !e.is_retryable() { return error_response(e.status().unwrap_or(502), &e.message()); } continue; }
                    }
                }
            }
        }
    }
    // context 不足优先反馈（客户端无感，除非全候选窗口都不够）
    if let Some(ctx) = last_context_err {
        return error_response(502, &format!("所有候选都装不下上下文（最后: {}）", ctx));
    }
    error_response(502, &format!("所有通道失败（试了 {} 条）: {}", max_retries.max(1), last_err))
}

/// 转发失败分类（学 LiteLLM AttemptError）：区分可重试/不可重试
#[derive(Debug, Clone)]
enum ForwardError {
    /// 可重试：连接失败 / 408 409 429 / 5xx。重试退避后可试其他通道。
    Retryable { status: Option<u16>, retry_after: Option<std::time::Duration>, msg: String },
    /// 不可重试：其他 4xx（鉴权/参数/找不到）。重试也没用，直接透传。
    Fatal { status: Option<u16>, msg: String },
    /// 上下文不足（context_length_exceeded）：模型窗口装不下请求。可换更大窗口通道重试，豁免 max_retries。
    Context { status: Option<u16>, msg: String },
}

impl ForwardError {
    fn is_retryable(&self) -> bool {
        matches!(self, ForwardError::Retryable { .. })
    }
    fn is_context(&self) -> bool {
        matches!(self, ForwardError::Context { .. })
    }
    fn status(&self) -> Option<u16> {
        match self {
            ForwardError::Retryable { status, .. } | ForwardError::Fatal { status, .. } | ForwardError::Context { status, .. } => *status,
        }
    }
    fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            ForwardError::Retryable { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
    fn message(&self) -> String {
        match self {
            ForwardError::Retryable { msg, .. } | ForwardError::Fatal { msg, .. } | ForwardError::Context { msg, .. } => msg.clone(),
        }
    }
}

/// 粗估 input token（不引 tokenizer SDK：4 字符≈1 token，个人网关够用）
pub fn estimate_input_tokens(payload: &serde_json::Value) -> i64 {
    let mut total: i64 = 0;
    if let Some(msgs) = payload.get("messages").and_then(|v| v.as_array()) {
        for m in msgs {
            if let Some(c) = m.get("content").and_then(|v| v.as_str()) {
                total += (c.len() as i64) / 4 + 1;
            }
        }
    }
    total
}

/// 响应缓存条目
#[derive(Clone)]
pub struct CachedResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub cached_at: std::time::Instant,
}

/// 响应缓存 store（全局共享）
pub type ResponseCacheStore = std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, CachedResponse>>>;
pub fn new_response_cache() -> ResponseCacheStore {
    std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))
}

/// 缓存 key（学 LiteLLM cache-response：SHA256(model + messages)）
pub fn cache_key(model: &str, payload: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    if let Some(msgs) = payload.get("messages") {
        hasher.update(msgs.to_string().as_bytes());
    }
    let h = hasher.finalize();
    h.iter().map(|b| format!("{:02x}", b)).collect::<String>()
}

/// 查缓存（TTL 内有效）
pub async fn cache_get(store: &ResponseCacheStore, key: &str, ttl_seconds: u64) -> Option<CachedResponse> {
    let map = store.read().await;
    map.get(key).cloned().filter(|c| c.cached_at.elapsed().as_secs() < ttl_seconds)
}

/// 存缓存（超 max_entries 淘汰最旧）
pub async fn cache_put(store: &ResponseCacheStore, key: String, resp: CachedResponse, max_entries: usize) {
    let mut map = store.write().await;
    if map.len() >= max_entries {
        // 淘汰最旧（按 cached_at）
        if let Some(oldest) = map.iter().min_by_key(|(_, v)| v.cached_at).map(|(k, _)| k.clone()) {
            map.remove(&oldest);
        }
    }
    map.insert(key, resp);
}

/// 合并全局重试策略 + 通道级覆盖（通道只配想改的字段，其余继承全局）
fn resolve_retry_policy<'a>(global: &'a RetryConfig, channel_override: Option<&'a RetryOverride>) -> RetryConfig {
    let mut policy = global.clone();
    if let Some(o) = channel_override {
        if let Some(v) = o.max_retries { policy.max_retries = v; }
        if let Some(ref v) = o.retry_on_status { policy.retry_on_status = v.clone(); }
        if let Some(ref v) = o.fatal_status { policy.fatal_status = v.clone(); }
        if let Some(v) = o.backoff_initial_ms { policy.backoff_initial_ms = v; }
        if let Some(v) = o.backoff_max_ms { policy.backoff_max_ms = v; }
        if let Some(v) = o.jitter { policy.jitter = v; }
        if let Some(v) = o.respect_retry_after { policy.respect_retry_after = v; }
    }
    policy
}

/// 从上游响应头读 retry-after（秒），仅尊重 ≤ 60s 的（学 LiteLLM MAX_HONOURED_RETRY_AFTER）
fn parse_retry_after(resp: &reqwest::Response) -> Option<std::time::Duration> {
    let v = resp.headers().get("retry-after")?.to_str().ok()?;
    let secs: u64 = v.trim().parse().ok()?;
    let d = std::time::Duration::from_secs(secs);
    if d <= std::time::Duration::from_secs(60) { Some(d) } else { None }
}

/// 重试退避延迟（学 LiteLLM Backoff）：0.5s 起，指数 ×2 到 8s 封顶，加 0.75 jitter 防雪崩。
/// 优先尊重上游 retry-after。attempted=已重试次数。
/// 重试退避延迟（学 LiteLLM Backoff）：initial 起 ×2 到 max 封顶，加 jitter 防雪崩。
/// 尊重上游 retry-after（若 policy 允许）。attempted=已重试次数。
fn retry_delay(policy: &RetryConfig, attempted: u32, retry_after: Option<std::time::Duration>) -> std::time::Duration {
    let initial = policy.backoff_initial_ms as f64 / 1000.0;
    let max = policy.backoff_max_ms as f64 / 1000.0;
    let jitter = policy.jitter;
    // 尊重上游明确要求的 retry-after
    if policy.respect_retry_after {
        if let Some(ra) = retry_after {
            return ra + std::time::Duration::from_secs_f64(jitter * pseudo_rand());
        }
    }
    let doubled = initial * 2f64.powi(attempted as i32);
    let secs = doubled.min(max);
    std::time::Duration::from_secs_f64(secs + jitter * pseudo_rand())
}

/// 轻量伪随机 [0,1)，从纳秒时间戳取，避免引 rand 依赖（jitter 防雪崩够用）
fn pseudo_rand() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    (nanos % 1000) as f64 / 1000.0
}

/// 转发到单个通道（Auto 和普通路由共用）：失败返回 ForwardError（区分可重试），成功返回响应
/// 通道测试（学 new-api TestChannel）：真发一个 chat 请求验证通道"真能出 token"
/// 用配置的默认模型（空=通道第一个模型）+ 测试对话。返回 {ok, status, latency_ms, content/error}
pub async fn test_channel(
    root: &std::path::Path,
    provider_name: &str,
    channel_id: &str,
) -> serde_json::Value {
    let config = load_config(root);
    let health: HealthMap = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let start = std::time::Instant::now();

    // 定位 provider + channel
    let Some(provider) = config.providers.iter().find(|p| p.name == provider_name) else {
        return serde_json::json!({"ok": false, "error": format!("供应商 {} 不存在", provider_name)});
    };
    let Some(channel) = provider.channels.iter().find(|c| c.id == channel_id) else {
        return serde_json::json!({"ok": false, "error": format!("通道 {} 不存在", channel_id)});
    };
    // 测试模型：配置的默认模型，空则用通道第一个模型的上游名
    let model = if !config.test.default_model.is_empty() {
        config.test.default_model.clone()
    } else {
        channel.models.first().map(|m| m.name.clone()).unwrap_or_default()
    };
    if model.is_empty() {
        return serde_json::json!({"ok": false, "error": "无可用测试模型（通道无模型且未配 default_model）"});
    }
    let api_key = resolve_key(provider, channel).unwrap_or_default();
    let sel = SelectedChannel {
        provider: provider.clone(),
        channel: channel.clone(),
        upstream_model: model.clone(),
        api_key,
        protocol: channel.models.iter().find(|mm| mm.name == model).and_then(|mm| mm.protocol.clone()),
    };
    // 强制非流式，便于取回整包内容
    let payload = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": config.test.test_message}],
        "max_tokens": config.test.max_tokens,
        "stream": false,
    });
    let policy = resolve_retry_policy(&config.retry, channel.retry.as_ref());
    match forward_one(root, &config, &health, &sel, &model, &payload, "/chat/completions", None, &policy).await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let bytes = http_body_util::BodyExt::collect(resp.into_body()).await
                .map(|b| b.to_bytes().to_vec()).unwrap_or_default();
            let latency = start.elapsed().as_millis() as u64;
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
            let content = v.get("choices").and_then(|c| c.get(0))
                .and_then(|c| c.get("message")).and_then(|m| m.get("content"))
                .and_then(|x| x.as_str()).map(|s| s.to_string());
            let ok = (200..300).contains(&status) && content.is_some();
            serde_json::json!({
                "ok": ok, "status": status, "latency_ms": latency,
                "model": model,
                "content": content.unwrap_or_default(),
                "error": if ok { serde_json::Value::Null } else { serde_json::Value::String(format!("HTTP {}", status)) },
            })
        }
        Err(e) => {
            serde_json::json!({
                "ok": false, "status": e.status().unwrap_or(0),
                "latency_ms": start.elapsed().as_millis() as u64,
                "model": model, "content": "", "error": e.message(),
            })
        }
    }
}

async fn forward_one(
    root: &std::path::Path,
    config: &GatewayConfig,
    health: &HealthMap,
    selected: &SelectedChannel,
    requested_model: &str,
    payload: &serde_json::Value,
    upstream_path: &str,
    dkey: Option<&str>,
    policy: &RetryConfig,
) -> Result<axum::response::Response, ForwardError> {
    // 把请求里的模型名换成上游真实名
    let mut upstream_payload = payload.clone();
    if let Some(obj) = upstream_payload.as_object_mut() {
        obj.insert("model".into(), serde_json::Value::String(selected.upstream_model.clone()));
        // 历史轮媒体降级：上游模型不支持 vision → 历史轮的图换占位符（当前轮不动）
        if let Some(msgs) = obj.get_mut("messages").and_then(|v| v.as_array_mut()) {
            let supports_vision = upstream_caps_of(config, selected).iter().any(|c| c == "vision");
            if !supports_vision {
                strip_history_media(msgs, "vision");
            }
        }
    }
    let is_stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_anthropic = selected.protocol.as_deref() == Some("anthropic");

    // 构造上游 URL + 请求体（按协议：anthropic → /v1/messages + chat→anthropic 请求）
    let base = selected.provider.base_url.trim_end_matches('/');
    let (url, send_payload) = if is_anthropic {
        let anth = crate::protocol::chat_to_anthropic_request(&upstream_payload)
            .map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("chat→anthropic 转换失败: {}", e) })?;
        (format!("{}/v1/messages", base), anth)
    } else {
        (format!("{}{}", base, upstream_path), upstream_payload)
    };

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(config.timeout_cfg().streaming_first_byte))
        .timeout(std::time::Duration::from_secs(config.timeout_cfg().non_streaming))
        .build()
        .map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("client 构造失败: {}", e) })?;
    let mut req = if is_anthropic {
        // Claude 上游用 x-api-key + anthropic-version（非 Bearer）
        client.post(&url)
            .header("x-api-key", &selected.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
    } else {
        client.post(&url)
            .header("Authorization", format!("Bearer {}", selected.api_key))
            .header("Content-Type", "application/json")
    };
    for (k, v) in &selected.provider.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let start = std::time::Instant::now();
    let cb = config.cb();
    let resp = match req.json(&send_payload).send().await {
        Ok(r) => r,
        Err(e) => {
            record_outcome(health, &selected.channel.id, false, 0, cb, false, false).await;
            return Err(ForwardError::Retryable { status: None, retry_after: None, msg: format!("上游请求失败: {}", e) });
        }
    };
    let ttft = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let ok = status.is_success();
    record_outcome(health, &selected.channel.id, ok, ttft, cb, false, false).await;

    // 错误分类（学 LiteLLM，可配置）：致命 4xx（鉴权/参数）直接透传；可重试状态码返 Err 触发 failover
    let code = status.as_u16();
    if !status.is_success() {
        // 失败路径：读 body 分类（move resp 后不再需要回传）
        let retry_after = parse_retry_after(&resp);
        let body_bytes = resp.bytes().await.unwrap_or_default().to_vec();
        // context 不足检测（任何 fatal 判断前）：模型窗口装不下 → 可换更大窗口通道，豁免 max_retries
        if !policy.context_keywords.is_empty() {
            if let Some(kw) = hit_disable_keyword(&body_bytes, &policy.context_keywords) {
                record_outcome(health, &selected.channel.id, false, ttft, cb, false, false).await;
                crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                    model: selected.upstream_model.clone(),
                    provider: selected.provider.name.clone(),
                    channel: selected.channel.name.clone(),
                    status: status.as_u16(),
                    latency_ms: start.elapsed().as_millis() as u64,
                    ttft_ms: ttft,
                    input_tokens: None, output_tokens: None,
                    cache_read_tokens: None, cache_creation_tokens: None,
                    downstream_key: dkey.map(|s| s.to_string()),
                    error: Some(format!("上下文不足 '{}'：{}", kw, String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(200)]))),
                }).await;
                return Err(ForwardError::Context { status: Some(code), msg: format!("上下文不足：{}", kw) });
            }
        }
        if status.is_server_error() && policy.is_fatal(code) {
            return Err(ForwardError::Fatal { status: Some(code), msg: format!("上游错误（配置为致命）{}", code) });
        }
        if status.is_client_error() && policy.is_fatal(code) {
            return Err(ForwardError::Fatal { status: Some(code), msg: format!("上游客户端错误 {}", code) });
        }
        if status.is_server_error() || (status.is_client_error() && policy.is_retryable(code)) {
            // 关键词熔断（学 new-api）：含欠费/封号等关键词 → 强制 open + Retryable 切通道
            if let Some(kw) = hit_disable_keyword(&body_bytes, &policy.disable_keywords) {
                record_outcome(health, &selected.channel.id, false, ttft, cb, false, true).await;
                crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                    model: selected.upstream_model.clone(),
                    provider: selected.provider.name.clone(),
                    channel: selected.channel.name.clone(),
                    status: status.as_u16(),
                    latency_ms: start.elapsed().as_millis() as u64,
                    ttft_ms: ttft,
                    input_tokens: None, output_tokens: None,
                    cache_read_tokens: None, cache_creation_tokens: None,
                    downstream_key: dkey.map(|s| s.to_string()),
                    error: Some(format!("关键词熔断命中 '{}'：{}", kw, String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(200)]))),
                }).await;
                return Err(ForwardError::Retryable { status: Some(status.as_u16()), retry_after: None,
                    msg: format!("关键词熔断：响应体命中 '{}'，切通道", kw) });
            }
            // 普通错误（无关键词）：记日志 + 早返回触发 failover
            crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
                model: selected.upstream_model.clone(),
                provider: selected.provider.name.clone(),
                channel: selected.channel.name.clone(),
                status: status.as_u16(),
                latency_ms: start.elapsed().as_millis() as u64,
                ttft_ms: ttft,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                downstream_key: dkey.map(|s| s.to_string()),
                error: Some(format!("HTTP {}", status.as_u16())),
            }).await;
            return Err(ForwardError::Retryable { status: Some(status.as_u16()), retry_after, msg: format!("上游服务端错误 {}", status.as_u16()) });
        }
        return Err(ForwardError::Fatal { status: Some(code), msg: format!("上游错误 {}", code) });
    }


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
        // 同时用 SseUsageTap 扫描 data: 行抓 usage token，流结束后补记完整日志
        use axum::body::Body;
        // 日志延到流结束：建 oneshot channel，tap 流完时把 usage 发给后台任务补记
        let (usage_tx, usage_rx) = tokio::sync::oneshot::channel::<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>();
        let stream = resp.bytes_stream();
        // 按上游协议选 tap：anthropic → 转换流（anthropic SSE→chat SSE）；否则透传
        let tap: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> = if is_anthropic {
            Box::pin(crate::protocol_stream::AnthropicUpstreamTap::new(stream, usage_tx))
        } else {
            Box::pin(SseUsageTap {
                inner: Box::pin(stream),
                buf: Vec::new(),
                usage: (None, None, None, None),
                usage_tx: Some(usage_tx),
            })
        };
        // 流式 model 回显档位名（对齐非流式）：档位请求时把 SSE data chunk 的 model 字段改成客户端配的档位名
        let tap: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>> =
            if requested_model == "pro" || requested_model == "flash" || requested_model == "auto" {
                Box::pin(ModelRewriteStream { inner: tap, model: requested_model.to_string() })
            } else {
                tap
            };
        // 后台任务：等流结束后拿 usage token，补一条带 token 的完整日志
        let log_root = root.to_path_buf();
        // 日志记真实上游模型名（用户要求：档位请求也记实际模型，不记档位别名）
        let log_model = selected.upstream_model.clone();
        let log_provider = selected.provider.name.clone();
        let log_channel = selected.channel.name.clone();
        let log_status = status.as_u16();
        let log_latency = start.elapsed().as_millis() as u64;
        let log_error = if ok { None } else { Some(format!("HTTP {}", status.as_u16())) };
        let log_dkey = dkey.map(|s| s.to_string()); // spawn 要 'static，先把借用的 dkey 转 owned
        tokio::spawn(async move {
            // 等流结束（tap drop / 发送端关闭时 rx 得 None）
            let (in_tok, out_tok, cache_read, cache_creation) = usage_rx.await.unwrap_or((None, None, None, None));
            crate::gateway_logs::log_request(log_root, crate::gateway_logs::LogEntry {
                model: log_model,
                provider: log_provider,
                channel: log_channel,
                status: log_status,
                latency_ms: log_latency,
                ttft_ms: ttft,
                input_tokens: in_tok,
                output_tokens: out_tok,
                cache_read_tokens: cache_read,
                cache_creation_tokens: cache_creation,
                downstream_key: log_dkey,
                error: log_error,
            }).await;
        });
        let body = Body::from_stream(tap);
        out.body(body).map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("流式回传失败: {}", e) })
    } else {
        // 非流式：整包 + 解析 usage token 入日志
        // （关键词熔断已在前面 5xx/可重试早返回块统一处理，这里只处理成功/非重试响应）
        let raw_bytes = resp.bytes().await.unwrap_or_default().to_vec();
        // 上游 anthropic → 转回 chat（myapi 内部统一协议）
        let body_bytes: Vec<u8> = if is_anthropic {
            let converted = serde_json::from_slice::<serde_json::Value>(&raw_bytes).ok()
                .and_then(|v| crate::protocol::anthropic_to_openai_response(&v).ok())
                .and_then(|chat| serde_json::to_vec(&chat).ok());
            match converted {
                Some(bytes) => bytes,
                None => raw_bytes,
            }
        } else { raw_bytes };
        // 档位请求：响应 model 字段回显档位名（客户端配的 model 名，对称）；
        // 客户端如 Claude Code 用它配的 model 名做内部判断（上下文窗口），真实模型名已记日志
        let body_bytes = if requested_model == "pro" || requested_model == "flash" || requested_model == "auto" {
            let mut v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null);
            if v.is_object() { v["model"] = serde_json::json!(requested_model); }
            serde_json::to_vec(&v).unwrap_or(body_bytes)
        } else { body_bytes };
        let (in_tok, out_tok, cache_read, cache_creation) = parse_usage(&body_bytes);
        crate::gateway_logs::log_request(root.to_path_buf(), crate::gateway_logs::LogEntry {
            // 记真实上游模型名（非档位别名）
            model: selected.upstream_model.clone(),
            provider: selected.provider.name.clone(),
            channel: selected.channel.name.clone(),
            status: status.as_u16(),
            latency_ms: start.elapsed().as_millis() as u64,
            ttft_ms: ttft,
            input_tokens: in_tok,
            output_tokens: out_tok,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            downstream_key: dkey.map(|s| s.to_string()),
            error: if ok { None } else { Some(format!("HTTP {}", status.as_u16())) },
        }).await;
        out.body(axum::body::Body::from(body_bytes))
            .map_err(|e| ForwardError::Retryable { status: None, retry_after: None, msg: format!("回传失败: {}", e) })
    }
}

/// 从非流式响应解析 usage（OpenAI 格式 usage.input_tokens/output_tokens）
fn parse_usage(body: &[u8]) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None),
    };
    parse_usage_json(&v)
}

/// 从 usage JSON 提取 (input, output, cache_read, cache_creation)
/// 抄 LiteLLM/llms-types 字段名：OpenAI prompt_tokens_details.cached_tokens /
/// cache_creation_tokens；Anthropic cache_read_input_tokens / cache_creation_input_tokens
pub(crate) fn parse_usage_json(v: &serde_json::Value) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    // usage 位置因协议而异：
    //   OpenAI/Anthropic: usage 字段
    //   Gemini: 顶层 usageMetadata（promptTokenCount/candidatesTokenCount）
    let u = if let Some(u) = v.get("usage") {
        if !u.is_null() { u } else { return (None, None, None, None) }
    } else if let Some(g) = v.get("usageMetadata") {
        if !g.is_null() { g } else { return (None, None, None, None) }
    } else {
        return (None, None, None, None);
    };
    let get = |k: &str| u.get(k).and_then(|x| x.as_i64());
    // input: OpenAI prompt_tokens / Anthropic input_tokens / Gemini promptTokenCount
    let input = get("input_tokens").or_else(|| get("prompt_tokens")).or_else(|| get("promptTokenCount"));
    // output: OpenAI completion_tokens / Anthropic output_tokens / Gemini candidatesTokenCount
    let output = get("output_tokens").or_else(|| get("completion_tokens")).or_else(|| get("candidatesTokenCount"));
    // cache read: OpenAI prompt_tokens_details.cached_tokens；Anthropic cache_read_input_tokens；Gemini cachedContentTokenCount
    let cache_read = get("cache_read_input_tokens")
        .or_else(|| get("cachedContentTokenCount"))
        .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(|x| x.as_i64()));
    // cache creation: Anthropic cache_creation_input_tokens；OpenAI cache_creation_tokens
    let cache_creation = get("cache_creation_input_tokens")
        .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cache_creation_tokens")).and_then(|x| x.as_i64()));
    (input, output, cache_read, cache_creation)
}

/// SSE 流式 usage 捕获：包装字节流，逐 chunk 透传客户端的同时累积缓冲，
/// 扫描 `data:` 行里的 usage token（OpenAI 最后 chunk / Anthropic message_delta）。
/// 流结束后把 (input_tokens, output_tokens) 通过 oneshot channel 发给后台任务补记日志。
/// 流式 SSE 的 model 字段改写为档位名（客户端配的 model 名，对齐非流式回显）。
struct ModelRewriteStream {
    inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send>>,
    model: String,
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
fn rewrite_chunk_model(chunk: &[u8], model: &str) -> Vec<u8> {
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

struct SseUsageTap {
    inner: std::pin::Pin<Box<dyn futures_core::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    buf: Vec<u8>,
    usage: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
    usage_tx: Option<tokio::sync::oneshot::Sender<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)>>,
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

/// 合并两个 usage 元组：逐字段取非 None（后者优先）。用于流式跨 chunk 累积——
/// Anthropic 把 usage 拆两处（message_start 带 input，message_delta 带 output），
/// 必须合并而非"取最后一个"，否则丢 input。
fn merge_usage(
    a: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
    b: (Option<i64>, Option<i64>, Option<i64>, Option<i64>),
) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    (
        b.0.or(a.0),
        b.1.or(a.1),
        b.2.or(a.2),
        b.3.or(a.3),
    )
}

/// 扫描 SSE 缓冲里所有 `data:` 行，累积合并各 chunk 的 usage token。
/// 覆盖三种协议的流式格式：
///   OpenAI：最后 chunk 的 usage.prompt_tokens/completion_tokens
///   Anthropic：message_start.usage.input_tokens + message_delta.usage.output_tokens（分片）
///   Gemini：chunk 顶层 usageMetadata.promptTokenCount/candidatesTokenCount
fn sse_scan_usage(buf: &[u8]) -> Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> {
    let s = std::str::from_utf8(buf).ok()?;
    let mut usage: Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = None;
    for line in s.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("data:") {
            let d = rest.trim();
            if d.is_empty() || d == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(d) {
                // usage 位置因协议而异：OpenAI/Anthropic 在 usage 字段；Gemini 在顶层 usageMetadata
                let has_usage = v.get("usage").map(|u| !u.is_null()).unwrap_or(false)
                    || v.get("usageMetadata").map(|u| !u.is_null()).unwrap_or(false);
                if has_usage {
                    let (in_tok, out_tok, cr, cc) = parse_usage_json(&v);
                    if in_tok.is_some() || out_tok.is_some() || cr.is_some() || cc.is_some() {
                        let cur = usage.unwrap_or((None, None, None, None));
                        usage = Some(merge_usage(cur, (in_tok, out_tok, cr, cc)));
                    }
                }
            }
        }
    }
    usage
}

/// chat 端点包装（forward_generic + /chat/completions）
pub async fn forward_chat(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    cache: &ResponseCacheStore,
) -> axum::response::Response {
    forward_generic(root, health, pins, rates, auth_header, payload, "/chat/completions", cache).await
}

/// embeddings 端点（复用路由+failover，走 /embeddings，无流式）
pub async fn forward_embeddings(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    rates: &RateStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
    cache: &ResponseCacheStore,
) -> axum::response::Response {
    // embeddings 不流式，强制去掉 stream 标记
    let mut p = payload;
    if let Some(obj) = p.as_object_mut() {
        obj.remove("stream");
    }
    forward_generic(root, health, pins, rates, auth_header, p, "/embeddings", cache).await
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
/// 拉取上游模型列表（学 new-api FetchUpstreamModels）：GET base_url/models，解析 OpenAI 格式
/// 返回模型 id 列表（供面板自动填充，避免手填写错）
pub async fn fetch_upstream_models(base_url: &str, api_key: &str, headers: &[(String, String)]) -> Result<Vec<String>, String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{}/models", base);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client 构造失败: {}", e))?;
    let mut req = client.get(&url).header("Authorization", format!("Bearer {}", api_key));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("请求上游失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("上游返回 {}", status.as_u16()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    // OpenAI 格式 {data: [{id: "gpt-4", ...}, ...]}；兼容 {models: [...]} / 裸数组
    let ids: Vec<String> = if let Some(data) = body.get("data").and_then(|v| v.as_array()) {
        data.iter().filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())).collect()
    } else if let Some(models) = body.get("models").and_then(|v| v.as_array()) {
        models.iter().filter_map(|m| {
            m.get("id").or_else(|| m.get("name")).and_then(|x| x.as_str()).map(|s| s.to_string())
        }).collect()
    } else if let Some(arr) = body.as_array() {
        arr.iter().filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())).collect()
    } else {
        vec![]
    };
    Ok(ids)
}

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
/// 生成 Nginx 高可用配置（学 new-api 多机部署）：从 deploy 配置生成完整 nginx.conf
/// 客户端 → Nginx 固定入口 → upstream 主/备 → 网关节点。故障切换靠 proxy_next_upstream。
/// SSE 流式：proxy_buffering off + 长 read timeout。
pub fn generate_nginx_conf(deploy: &serde_json::Value) -> String {
    let entry = deploy.get("entry").cloned().unwrap_or_else(|| serde_json::json!({}));
    let listen = entry.get("listen").and_then(|v| v.as_u64()).unwrap_or(8380);
    let read_timeout = entry.get("readTimeout").and_then(|v| v.as_u64()).unwrap_or(120);
    let next_upstream = entry.get("nextUpstream").and_then(|v| v.as_str()).unwrap_or("error timeout http_502 http_503 http_504");
    let instances = deploy.get("instances").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    // upstream 块：master 在前，backup 标记 backup（只有主挂了才用备）
    let mut servers = String::new();
    for inst in &instances {
        let addr = inst.get("address").and_then(|v| v.as_str()).unwrap_or("");
        if addr.is_empty() { continue; }
        let role = inst.get("role").and_then(|v| v.as_str()).unwrap_or("backup");
        let max_fails = inst.get("heartbeatMs").and_then(|v| v.as_u64()).unwrap_or(5000) / 1000;
        if role == "master" {
            servers.push_str(&format!("        server {} max_fails={} fail_timeout={}s;\n", addr, max_fails.max(2), max_fails.max(5)));
        } else {
            servers.push_str(&format!("        server {} backup max_fails={} fail_timeout={}s;\n", addr, max_fails.max(2), max_fails.max(5)));
        }
    }
    if servers.is_empty() {
        servers.push_str("        # 无实例，请在面板添加网关节点\n");
    }

    format!(
        r#"# myapi 高可用 Nginx 配置（由面板生成）
# 部署：放到 Nginx 常驻机（不要装在 meilan 手机，保持手机省电）
# 客户端认固定入口 :{listen}，主节点挂了 proxy_next_upstream 自动切备用

worker_processes auto;
events {{ worker_connections 1024; }}

http {{
    # SSE 流式透传（网关的 /v1/chat/completions 是 SSE，必须关缓冲）
    proxy_buffering off;
    proxy_cache off;
    proxy_http_version 1.1;
    proxy_set_header Connection "";

    upstream myapi {{
{servers}    }}

    server {{
        listen {listen};
        # 客户端只认这个入口，无感知主备切换

        location / {{
            proxy_pass http://myapi;
            proxy_next_upstream {next_upstream};
            proxy_next_upstream_tries 3;
            proxy_next_upstream_timeout 30s;

            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_read_timeout {read_timeout}s;
            proxy_send_timeout {read_timeout}s;
            proxy_connect_timeout 5s;
        }}

        # 健康检查（Nginx 主动探活，摘除死节点）
        location = /health {{
            proxy_pass http://myapi;
            proxy_next_upstream error timeout;
        }}
    }}
}}
"#,
        listen = listen,
        servers = servers,
        next_upstream = next_upstream,
        read_timeout = read_timeout,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use serde_json::json;

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
        record_outcome(&h, "ch1", true, 100, &c, false, false).await;
        record_outcome(&h, "ch1", true, 100, &c, false, false).await;
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
            record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
        record_outcome(&h, "ch1", true, 0, &c, false, false).await;
        for _ in 0..4 {
            record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
            record_outcome(&h, "ch1", false, 0, &c, true, false).await;
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
            record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
            record_outcome(&h, "ch1", true, 0, &c, false, false).await;
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
            record_outcome(&h, "ch1", false, 0, &c, false, false).await;
        }
        // 时间流逝
        {
            let mut m = h.write().await;
            m.get_mut("ch1").unwrap().last_failure_at = now_secs() - 31;
        }
        // 试探成功 2 次（不到 3）
        record_outcome(&h, "ch1", true, 0, &c, false, false).await;
        record_outcome(&h, "ch1", true, 0, &c, false, false).await;
        // 第 3 次失败 → 重新 open
        record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                    ..Default::default()}],
                weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // ch1 打开 + 未超恢复超时 → select_channel 应回退到它（KeepAll，全熔断兼底）
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &cb(), false, false).await;
        }
        let sel = select_channel(&cfg, "my-model", &h);
        assert!(sel.is_some(), "全熔断时 KeepAll 仍应返回通道");
    }


    #[tokio::test]
    async fn sticky_multi_client_independent() {
        // 两个不同 session → 各自独立 pin（互不干扰）
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] , ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()), capabilities: vec![] , ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(), ..Default::default()});
        let h = health_map();
        let pins = new_pin_store();
        // session A 第一次 → pin 到某通道
        let a1 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:a")).unwrap().channel.id;
        // session B 第一次 → 独立选择
        let b1 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:b")).unwrap().channel.id;
        // A 再选 → 仍黏 A 的 pin（B 的选择不影响 A）
        let a2 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:a")).unwrap().channel.id;
        assert_eq!(a2, a1, "session A 应持续黏 A 的 pin");
        // B 再选 → 仍黏 B 的 pin（A 的重复选择不影响 B）
        let b2 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:b")).unwrap().channel.id;
        assert_eq!(b2, b1, "session B 应持续黏 B 的 pin");
    }

    #[tokio::test]
    async fn sticky_ttl_expires_repins_fresh() {
        // TTL 过期 → pin 失效 → 重新选（不再强制黏旧通道）
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 60, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] , ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()), capabilities: vec![] , ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(), ..Default::default()});
        let h = health_map();
        let pins = new_pin_store();
        let s1 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:a")).unwrap().channel.id;
        // 手动把 pinned_at 拨回很久以前（TTL 过期）
        {
            let mut p = pins.write().await;
            if let Some(e) = p.get_mut("sess:a") { e.pinned_at = 1; }
        }
        // TTL 过期后：不再强制黏旧（重新选，可能换通道）
        let s2 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:a")).unwrap().channel.id;
        assert!(["ch1", "ch2"].contains(&s2.as_str()), "TTL 过期后应可重新选: {s2}");
    }

    #[test]
    fn stream_chunk_model_rewritten_to_tier() {
        // 流式 SSE chunk 的 model 字段改写为档位名（对齐非流式回显）
        let chunk = br#"data: {"id":"x","model":"up-real","choices":[{"delta":{"content":"hi"}}]}"#;
        let out = rewrite_chunk_model(chunk, "pro");
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("\"model\":\"pro\""), "chunk model 应改为档位名: {s}");
        assert!(s.contains("up-real") == false, "不应残留真实模型名: {s}");
    }

    #[test]
    fn stream_chunk_model_with_space_rewritten() {
        // 上游透传带空格格式也改写
        let chunk = br#"data: {"model": "up-real", "choices": []}"#;
        let out = rewrite_chunk_model(chunk, "flash");
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("\"model\": \"flash\""), "带空格 model 应改写: {s}");
    }

    #[test]
    fn stream_chunk_without_model_untouched() {
        // 无 model 字段的 chunk 原样保留
        let chunk = br#"data: {"choices":[{"delta":{"content":"hi"}}]}"#;
        let out = rewrite_chunk_model(chunk, "pro");
        assert_eq!(out, chunk.to_vec());
    }

    #[tokio::test]
    async fn sticky_reuses_same_channel() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300, ..Default::default() };
        // 两条通道都能服务 my-model
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
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
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        let pins = new_pin_store();
        // 先 pin 到 ch1
        let s1 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:xyz"));
        let ch1 = s1.unwrap().channel.id.clone();
        assert_eq!(ch1, "ch1");
        // ch1 连续失败 5 次 → open（但 ch2 健康，非单部署）
        for _ in 0..5 {
            record_outcome(&h, "ch1", false, 0, &cfg.circuit_breaker, false, false).await;
        }
        // 同 session 再选 → ch1 熔断，应重新 pin 到 ch2
        let s2 = select_channel_sticky(&cfg, "my-model", &h, &pins, Some("sess:xyz"));
        assert_eq!(s2.unwrap().channel.id, "ch2", "pin 的通道熔断应重新 pin");
    }

    #[tokio::test]
    async fn sticky_disabled_always_fresh() {
        let mut cfg = GatewayConfig::default();
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                    ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
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
        cfg.sticky = StickyConfig { enabled: true, ttl_seconds: 300, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                    ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
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
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300, ..Default::default() }; // 关黏性，纯测延迟
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "slow".into(), name: "slow".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "fast".into(), name: "fast".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // slow 通道平均 TTFT 800ms，fast 通道 200ms
        for _ in 0..5 {
            record_outcome(&h, "slow", true, 800, &cfg.circuit_breaker, false, false).await;
            record_outcome(&h, "fast", true, 200, &cfg.circuit_breaker, false, false).await;
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
            record_outcome(&h, "ch1", true, (i as u64) * 10, &c, false, false).await;
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
        cfg.routing = RoutingConfig { strategy, auto_mode_enabled: true, max_retries: 3, ..Default::default() };
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
        cfg
    }


    #[tokio::test]
    async fn strategy_latency_picks_lowest_ttft() {
        // smart（LatencyBased）：平均 TTFT 最低优先
        let cfg = two_chan_cfg(RoutingStrategy::LatencyBased);
        let h = health_map();
        for _ in 0..5 { record_outcome(&h, "ch1", true, 200, &cfg.circuit_breaker, false, false).await; }
        for _ in 0..5 { record_outcome(&h, "ch2", true, 50, &cfg.circuit_breaker, false, false).await; }
        let sel = select_channel(&cfg, "my-model", &h);
        assert_eq!(sel.unwrap().channel.id, "ch2", "smart 应选 TTFT 最低的");
    }

    #[tokio::test]
    async fn circuit_breaker_excludes_open_channel() {
        // ch1 连续失败（阈值 5）→ Open → 熔断排除，只选 ch2
        let cfg = two_chan_cfg(RoutingStrategy::SimpleShuffle);
        let h = health_map();
        for _ in 0..20 { record_outcome(&h, "ch1", false, 0, &cfg.circuit_breaker, false, false).await; }
        record_outcome(&h, "ch2", true, 0, &cfg.circuit_breaker, false, false).await;
        for _ in 0..50 {
            let sel = select_channel(&cfg, "my-model", &h);
            assert_eq!(sel.unwrap().channel.id, "ch2", "ch1 open 熔断应排除");
        }
    }

    #[test]
    fn multi_provider_candidates_merged() {
        // 2 个 provider 各 1 通道 → auto_candidates 合并候选
        let mut cfg = GatewayConfig::default();
        cfg.routing = RoutingConfig { strategy: RoutingStrategy::LatencyBased, auto_mode_enabled: true, max_retries: 3, ..Default::default() };
        for (name, mid) in [("p1", "a-model"), ("p2", "b-model")] {
            cfg.providers.push(Provider {
                name: name.into(), base_url: format!("http://{name}/v1"), disabled: false,
                headers: HashMap::new(),
                keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
                channels: vec![Channel {
                    id: format!("ch-{name}"), name: format!("ch-{name}"), path: None, key_name: None,
                    models: vec![ChannelModel { name: mid.into(), alias: None, capabilities: vec![], ..Default::default() }],
                    weight: 1, rpm: None, retry: None, max_input_tokens: None }],
                proxy: String::new(),
                ..Default::default()
            });
        }
        let h = health_map();
        let payload = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        let cands = auto_candidates(&cfg, &payload, &h, None);
        assert_eq!(cands.len(), 2, "两个 provider 候选应合并");
    }


    #[test]
    fn disabled_provider_excluded_from_candidates() {
        let mut cfg = GatewayConfig::default();
        cfg.routing = RoutingConfig { strategy: RoutingStrategy::LatencyBased, auto_mode_enabled: true, max_retries: 3, ..Default::default() };
        for (name, mid, dis) in [("p-on", "on-model", false), ("p-off", "off-model", true)] {
            cfg.providers.push(Provider {
                name: name.into(), base_url: format!("http://{name}/v1"), disabled: dis,
                headers: HashMap::new(), keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
                channels: vec![Channel { id: format!("ch-{name}"), name: format!("ch-{name}"), path: None, key_name: None,
                    models: vec![ChannelModel { name: mid.into(), alias: None, capabilities: vec![], ..Default::default() }],
                    weight: 1, rpm: None, retry: None, max_input_tokens: None }],
                proxy: String::new(), ..Default::default()
            });
        }
        let h = health_map();
        let payload = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        let cands = auto_candidates(&cfg, &payload, &h, None);
        assert_eq!(cands.len(), 1, "disabled provider 应排除");
        assert_eq!(cands[0].upstream_model, "on-model");
    }

    #[tokio::test]
    async fn circuit_breaker_recovers_after_timeout() {
        // ch1 熔断 Open → 超过恢复超时 → half_open 可试探（不再被排除）
        let cfg = two_chan_cfg(RoutingStrategy::SimpleShuffle);
        let h = health_map();
        for _ in 0..10 { record_outcome(&h, "ch1", false, 0, &cfg.circuit_breaker, false, false).await; }
        {
            let mut hm = h.write().await;
            if let Some(e) = hm.get_mut("ch1") {
                e.last_failure_at = now_secs() - cfg.circuit_breaker.recovery_timeout_seconds - 1;
            }
        }
        for _ in 0..50 {
            let sel = select_channel(&cfg, "my-model", &h);
            assert!(sel.is_some(), "恢复后应仍有候选");
        }
    }

    #[test]
    fn tier_pro_filters_to_pro_with_vision() {
        // pro 档 + 当前轮带图（硬需求 vision）→ 只 class:pro 且带 vision 能力
        let mut cfg = GatewayConfig::default();
        cfg.routing = RoutingConfig { strategy: RoutingStrategy::LatencyBased, auto_mode_enabled: true, max_retries: 3, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel {
                id: "ch1".into(), name: "ch1".into(), path: None, key_name: None,
                models: vec![
                    ChannelModel { name: "pro-vision".into(), alias: None, capabilities: vec!["vision".into()], class: Some("pro".into()), ..Default::default() },
                    ChannelModel { name: "pro-plain".into(), alias: None, capabilities: vec![], class: Some("pro".into()), ..Default::default() },
                    ChannelModel { name: "flash-vision".into(), alias: None, capabilities: vec!["vision".into()], class: Some("flash".into()), ..Default::default() },
                ],
                weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
            ..Default::default()
        });
        let h = health_map();
        // 当前轮带图 → 硬需求 vision
        let payload = serde_json::json!({"messages": [{"role": "user", "content": [
            {"type": "text", "text": "hi"}, {"type": "image_url", "image_url": {"url": "x"}}]}]});
        let cands = auto_candidates(&cfg, &payload, &h, Some("pro"));
        assert_eq!(cands.len(), 1, "pro 档 + 带图 → 只 pro-vision（pro-plain 无 vision、flash 非 pro 档）");
        assert_eq!(cands[0].upstream_model, "pro-vision");
    }

    #[tokio::test]
    async fn strategy_least_busy_picks_fewer_requests() {
        let cfg = two_chan_cfg(RoutingStrategy::LeastBusy);
        let h = health_map();
        // ch1 已经接了 100 单，ch2 只有 1 单
        for _ in 0..100 { record_outcome(&h, "ch1", true, 0, &cfg.circuit_breaker, false, false).await; }
        record_outcome(&h, "ch2", true, 0, &cfg.circuit_breaker, false, false).await;
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



    #[test]
    fn strip_1m_suffix_strips_marker() {
        assert_eq!(strip_1m_suffix("auto[1M]"), "auto");
        assert_eq!(strip_1m_suffix("auto[1m]"), "auto");
        assert_eq!(strip_1m_suffix("flash[1M]"), "flash");
        assert_eq!(strip_1m_suffix("auto"), "auto");
        assert_eq!(strip_1m_suffix("gpt-5[1M]"), "gpt-5");
        assert_eq!(strip_1m_suffix(""), "");
    }

    #[test]
    fn auto_candidates_filters_by_tier_class() {
        let mut cfg = GatewayConfig::default();
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel {
                id: "ch1".into(), name: "ch1".into(), path: None, key_name: None,
                models: vec![
                    ChannelModel { name: "pro-model".into(), alias: None, capabilities: vec![],
                        class: Some("pro".into()), ..Default::default() },
                    ChannelModel { name: "flash-model".into(), alias: None, capabilities: vec![],
                        class: Some("flash".into()), ..Default::default() },
                    ChannelModel { name: "plain-model".into(), alias: None, capabilities: vec![],
                        class: None, ..Default::default() },
                ],
                weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
            ..Default::default()
        });
        let h = health_map();
        let payload = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        // auto → 全部（含无标注）
        let auto = auto_candidates(&cfg, &payload, &h, None);
        assert_eq!(auto.len(), 3, "auto 应含全部 3 个模型");
        // pro → 只 class:pro
        let pro = auto_candidates(&cfg, &payload, &h, Some("pro"));
        assert_eq!(pro.len(), 1, "pro 只应含 pro-model");
        assert_eq!(pro[0].upstream_model, "pro-model");
        // flash → 只 class:flash
        let flash = auto_candidates(&cfg, &payload, &h, Some("flash"));
        assert_eq!(flash.len(), 1, "flash 只应含 flash-model");
        assert_eq!(flash[0].upstream_model, "flash-model");
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
                models: vec![ChannelModel { name: "text-only".into(), alias: None, capabilities: vec![] ,
                    ..Default::default()}],
                weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // 带图请求需要 vision，text-only 没该能力 → 应过滤掉
        let payload = serde_json::json!({"messages":[{"role":"user","content":[
            {"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"x"}}]}]});
        let cands = auto_candidates(&cfg, &payload, &h, None);
        assert!(cands.is_empty(), "无 vision 能力应被过滤");
    }

    // ---- Vision 能力硬约束：当前轮 vs 历史轮（用户确认的方案）----

    #[test]
    fn trailing_user_turn_stops_at_last_assistant() {
        // 历史轮带图，当前轮纯文本
        let msgs = vec![
            serde_json::json!({"role":"user","content":[
                {"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"x"}}]}),
            serde_json::json!({"role":"assistant","content":"ok"}),
            serde_json::json!({"role":"user","content":"hi"}),
        ];
        let cur = trailing_user_turn(&msgs);
        assert_eq!(cur.len(), 1, "当前轮=最后 assistant 之后的全部 user 消息");
        // 历史轮带图、当前轮不带图 → 无硬需求（历史媒体不 pin 住能力选择）
        assert!(detect_required_caps(&serde_json::json!({"messages": msgs.clone()})).is_empty(),
            "历史轮带图、当前轮不带图 → 无硬需求");
        // 当前轮带图 → 有 vision 硬需求
        let msgs2 = vec![msgs[0].clone(), msgs[1].clone(), serde_json::json!({"role":"user","content":[
            {"type":"text","text":"看这图"},{"type":"image_url","image_url":{"url":"y"}}]})];
        assert_eq!(detect_required_caps(&serde_json::json!({"messages": msgs2})), vec!["vision"]);
    }

    #[test]
    fn strip_history_media_only_touches_history() {
        let mut msgs = vec![
            serde_json::json!({"role":"user","content":[
                {"type":"text","text":"旧图"},{"type":"image_url","image_url":{"url":"x"}}]}),
            serde_json::json!({"role":"assistant","content":"已看"}),
            serde_json::json!({"role":"user","content":[
                {"type":"text","text":"当前"},{"type":"image_url","image_url":{"url":"y"}}]}),
        ];
        let n = strip_history_media(&mut msgs, "vision");
        assert_eq!(n, 1, "只替换历史轮那张图");
        let hist = msgs[0].get("content").and_then(|v| v.as_array()).unwrap();
        assert_eq!(hist[0].get("text").and_then(|v| v.as_str()), Some("旧图"));
        assert_eq!(hist[1].get("text").and_then(|v| v.as_str()),
            Some("[image omitted: model does not support vision]"));
        // 当前轮原样不动
        let cur = msgs[2].get("content").and_then(|v| v.as_array()).unwrap();
        assert_eq!(cur.len(), 2);
        assert_eq!(cur[1].get("type").and_then(|v| v.as_str()), Some("image_url"));
    }

    #[test]
    fn ordered_candidates_filters_by_capability() {
        let mut cfg = GatewayConfig::default();
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "k".into(), api_key: "k".into() }],
            channels: vec![Channel {
                id: "ch-text".into(), name: "t".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "my-model".into(), alias: None, capabilities: vec![] ,
                    ..Default::default()}],
                weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // 模型存在但缺 vision 能力 → 能力过滤后空（调用点据此返回 409，不静默降级）
        assert!(ordered_candidates(&cfg, "my-model", &h, None, 0, &["vision".into()]).is_empty());
        // 无硬需求 → 正常命中
        assert!(!ordered_candidates(&cfg, "my-model", &h, None, 0, &[]).is_empty());
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
                    models: vec![ChannelModel { name: "m-slow".into(), alias: None, capabilities: vec!["vision".into()] ,
                        ..Default::default()}],
                    weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch-fast".into(), name: "fast".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m-fast".into(), alias: None, capabilities: vec!["vision".into()] ,
                        ..Default::default()}],
                    weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // fast 200ms, slow 800ms
        for _ in 0..5 {
            record_outcome(&h, "ch-slow", true, 800, &cfg.circuit_breaker, false, false).await;
            record_outcome(&h, "ch-fast", true, 200, &cfg.circuit_breaker, false, false).await;
        }
        // 带图请求 → 两个都有 vision，按延迟排 fast 优先
        let payload = serde_json::json!({"messages":[{"role":"user","content":[
            {"type":"image_url","image_url":{"url":"x"}}]}]});
        let cands = auto_candidates(&cfg, &payload, &h, None);
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
                models: vec![ChannelModel { name: "m".into(), alias: None, capabilities: vec![] ,
                    ..Default::default()}],
                weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // ch-open 连续失败 5 次 → open（未超恢复时间）
        for _ in 0..5 {
            record_outcome(&h, "ch-open", false, 0, &cfg.circuit_breaker, false, false).await;
        }
        // auto 应跳过熔断通道
        let payload = serde_json::json!({"messages":[{"role":"user","content":"hi"}]});
        let cands = auto_candidates(&cfg, &payload, &h, None);
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
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "chB".into(), name: "b".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        // 无 pin：应返回两条候选（failover 链完整）
        let cands = ordered_candidates(&cfg, "my-model", &h, None, 0, &[]);
        assert_eq!(cands.len(), 2, "应返回完整 failover 链");
        // pin chB：应提到链首
        let cands2 = ordered_candidates(&cfg, "my-model", &h, Some("chB"), 0, &[]);
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
                models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                    ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
        cfg.providers.push(Provider {
            name: "on".into(), base_url: "http://y/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![Channel { id: "chOn".into(), name: "on".into(), path: None, key_name: None,
                models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                    ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None }],
            proxy: String::new(),
        
            ..Default::default()});
        let h = health_map();
        let cands = ordered_candidates(&cfg, "my-model", &h, None, 0, &[]);
        assert_eq!(cands.len(), 1, "disabled provider 应被跳过");
        assert_eq!(cands[0].channel.id, "chOn");
    }

    // ---- 下游鉴权测试 ----

    #[test]
    fn auth_no_keys_allows_all() {
        // api_keys 空 → 不鉴权（内网可用），任意 header 都放行，无设备名
        let auth = AuthConfig { api_keys: vec![] , daily_token_budget: None };
        assert_eq!(validate_auth(&auth, None).unwrap(), None);
        assert_eq!(validate_auth(&auth, Some("Bearer anything")).unwrap(), None);
    }

    #[test]
    fn auth_configured_requires_valid_key() {
        let auth = AuthConfig { api_keys: vec![AuthKey::Plain("sk-secret".into())] , daily_token_budget: None };
        // 无 header → 401
        assert!(validate_auth(&auth, None).is_err());
        // 错 key → 401
        assert!(validate_auth(&auth, Some("Bearer wrong")).is_err());
        // 对 key → 放行（Plain 设备名=key 本身）
        assert_eq!(validate_auth(&auth, Some("Bearer sk-secret")).unwrap(), Some("sk-secret".into()));
    }

    #[test]
    fn auth_device_name_distinguishes_multi_device() {
        // 多设备身份区分：Named {name,key} → 鉴权返回设备名（日志/统计按设备聚合）
        let auth = AuthConfig {
            api_keys: vec![
                AuthKey::Named { name: "手机".into(), key: "sk-phone".into() },
                AuthKey::Named { name: "笔记本".into(), key: "sk-laptop".into() },
            ],
            daily_token_budget: None,
        };
        // 每设备独立 key → 识别出对应设备名
        assert_eq!(validate_auth(&auth, Some("Bearer sk-phone")).unwrap(), Some("手机".into()));
        assert_eq!(validate_auth(&auth, Some("Bearer sk-laptop")).unwrap(), Some("笔记本".into()));
        // 未知 key → 401
        assert!(validate_auth(&auth, Some("Bearer sk-evil")).is_err());
        // downstream_key 同样返回设备名
        assert_eq!(downstream_key(&auth, Some("Bearer sk-phone")), Some("手机".into()));
        assert_eq!(downstream_key(&auth, Some("Bearer sk-laptop")), Some("笔记本".into()));
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

    // ==================== 各协议 usage 统计对照测试（非流式 + 流式 SSE）====================
    // 每种协议的 usage 字段名/位置不同，这里逐协议对照验证 parse + SSE scan。
    // 对照基准：new-api relay-{openai,claude,gemini}.go 的 usage 解析。

    #[test]
    fn proto_openai_usage_nonstream() {
        // OpenAI 非流式：usage.prompt_tokens / completion_tokens
        let v = json!({"usage": {"prompt_tokens": 10, "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 3}}});
        let (i, o, cr, cc) = parse_usage_json(&v);
        assert_eq!((i, o, cr, cc), (Some(10), Some(5), Some(3), None), "OpenAI 非流式");
    }

    #[test]
    fn proto_anthropic_usage_nonstream() {
        // Anthropic 非流式：usage.input_tokens / output_tokens + cache 两字段
        let v = json!({"usage": {"input_tokens": 8, "output_tokens": 4,
            "cache_read_input_tokens": 6, "cache_creation_input_tokens": 2}});
        let (i, o, cr, cc) = parse_usage_json(&v);
        assert_eq!((i, o, cr, cc), (Some(8), Some(4), Some(6), Some(2)), "Anthropic 非流式");
    }

    #[test]
    fn proto_gemini_usage_nonstream() {
        // Gemini 非流式：顶层 usageMetadata.promptTokenCount / candidatesTokenCount
        let v = json!({"usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 7,
            "cachedContentTokenCount": 4}});
        let (i, o, cr, _cc) = parse_usage_json(&v);
        assert_eq!((i, o, cr), (Some(12), Some(7), Some(4)), "Gemini 非流式");
    }

    #[test]
    fn proto_openai_usage_stream() {
        // OpenAI 流式：最后 chunk 带 usage（同一 chunk 含 input+output）
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3}}\n\n\
                    data: [DONE]\n\n";
        let u = sse_scan_usage(body.as_bytes()).unwrap();
        assert_eq!(u, (Some(9), Some(3), None, None), "OpenAI 流式");
    }

    #[test]
    fn proto_anthropic_usage_stream_split() {
        // Anthropic 流式：usage 分片——message_start 带 input，message_delta 带 output。
        // 必须"跨 chunk 累积合并"，不能取最后一个（否则丢 input）。
        let body = "data: {\"type\":\"message_start\",\"usage\":{\"input_tokens\":11}}\n\n\
                    data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n\
                    data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\n";
        let u = sse_scan_usage(body.as_bytes()).unwrap();
        assert_eq!(u, (Some(11), Some(5), None, None), "Anthropic 分片合并 input+output");
    }

    #[test]
    fn proto_gemini_usage_stream() {
        // Gemini 流式：chunk 顶层 usageMetadata（promptTokenCount/candidatesTokenCount）
        let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}\n\n\
                    data: {\"usageMetadata\":{\"promptTokenCount\":15,\"candidatesTokenCount\":6}}\n\n";
        let u = sse_scan_usage(body.as_bytes()).unwrap();
        assert_eq!(u, (Some(15), Some(6), None, None), "Gemini 流式");
    }

    #[test]
    fn proto_stream_no_usage_returns_none() {
        // 无 usage 的流（纯内容）→ None，不误报
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
        assert_eq!(sse_scan_usage(body.as_bytes()), None);
    }


    // ---- context 不足错误反馈（用户拍板：网关全试，全废才反馈）----

    #[test]
    fn context_keyword_detection() {
        let cfg = RetryConfig::default();
        // 标准 OpenAI context_length_exceeded 响应体
        let body = br#"{"error":{"message":"This model's maximum context length is 200000 tokens. You requested 250000 tokens."}}"#;
        assert!(hit_disable_keyword(body, &cfg.context_keywords).is_some(),
            "context 词表应命中 context_length_exceeded");
        // 欠费词表不命中 context 错误（两组词表独立）
        assert!(hit_disable_keyword(body, &cfg.disable_keywords).is_none(),
            "欠费词表不应命中 context 错误");
        // 欠费错误也不命中 context 词表
        let quota = br#"{"error":{"message":"insufficient quota"}}"#;
        assert!(hit_disable_keyword(quota, &cfg.disable_keywords).is_some());
        assert!(hit_disable_keyword(quota, &cfg.context_keywords).is_none());
    }

    #[test]
    fn forward_error_context_is_context() {
        let e = ForwardError::Context { status: Some(400), msg: "ctx".into() };
        assert!(e.is_context(), "Context 变体应 is_context");
        assert!(!e.is_retryable(), "Context 不是普通可重试");
        assert_eq!(e.status(), Some(400));
        let r = ForwardError::Retryable { status: Some(429), retry_after: None, msg: "r".into() };
        assert!(!r.is_context(), "Retryable 不是 context");
    }
}
