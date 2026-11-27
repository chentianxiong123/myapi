//! config（从 gateway.rs 精确拆分）
use super::*;

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

pub(crate) fn def_retry_max() -> usize { 3 }

pub(crate) fn def_retry_on_status() -> Vec<u16> { vec![408, 409, 429, 500, 502, 503, 504] }

pub(crate) fn def_fatal_status() -> Vec<u16> { vec![400, 401, 403, 404] }

pub(crate) fn def_backoff_initial() -> u64 { 500 }

pub(crate) fn def_backoff_max() -> u64 { 8000 }

pub(crate) fn def_jitter() -> f64 { 0.75 }

pub(crate) fn def_respect_retry_after() -> bool { true }

/// 关键词熔断默认词表（学 new-api，覆盖常见欠费/封号/配额耗尽表述）
pub(crate) fn def_disable_keywords() -> Vec<String> {
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
pub(crate) fn def_context_keywords() -> Vec<String> {
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

pub(crate) fn default_weight() -> u32 { 1 }

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

pub(crate) fn def_test_message() -> String { "ping".into() }

pub(crate) fn def_test_max_tokens() -> i64 { 16 }

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

pub(crate) fn def_cache_ttl() -> u64 { 300 }

pub(crate) fn def_cache_max_entries() -> usize { 1000 }

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

pub(crate) fn def_listen() -> u16 { 8380 }

/// TTFT 滑动窗口大小（学 LiteLLM lowest_latency max_latency_list_size=10）
/// 注：算法常量，未做成产品配置（record_outcome 有 26+ 调用点，参数化收益低风险高）。
/// 前端 latency_window_seconds 为「秒」语义，与本处「条数」不一致，故该字段当前仅存档。
pub(crate) const LATENCY_WINDOW_SIZE: usize = 10;

/// 延迟 buffer 比例（学 lowest_latency_buffer，避免延迟接近时反复横跳）
pub(crate) const LATENCY_BUFFER_RATIO: f64 = 0.2;

/// 无延迟记录通道的中性分数（低于已知的差通道，但不抢最快的）
/// 注：算法常量（非产品配置），60s 是经验值。
pub(crate) const NEUTRAL_LATENCY_SCORE: f64 = 60_000.0;

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

pub(crate) fn def_failure_threshold() -> u32 { 5 }

pub(crate) fn def_error_rate() -> u64 { 50 }

pub(crate) fn def_min_requests() -> u64 { 5 }

pub(crate) fn def_recovery_timeout() -> u64 { 30 }

pub(crate) fn def_recovery_success() -> u32 { 3 }

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

pub(crate) fn def_sticky_enabled() -> bool { true }

pub(crate) fn def_sticky_ttl() -> u64 { 300 }

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

pub(crate) fn def_max_retries() -> usize { 3 }

pub(crate) fn def_auto_enabled() -> bool { true }

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

pub(crate) fn def_stream_first_byte() -> u64 { 30 }

pub(crate) fn def_non_streaming() -> u64 { 120 }

pub(crate) fn def_streaming_idle() -> u64 { 60 }

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
    pub(crate) fn key(&self) -> &str {
        match self { AuthKey::Plain(k) => k, AuthKey::Named { key, .. } => key }
    }
    pub(crate) fn device_name(&self) -> &str {
        match self { AuthKey::Plain(k) => k, AuthKey::Named { name, .. } => name }
    }
    pub(crate) fn matches(&self, token: &str) -> bool { self.key() == token }
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
pub(crate) fn downstream_key(auth: &AuthConfig, auth_header: Option<&str>) -> Option<String> {
    let token = auth_header.and_then(|h| h.strip_prefix("Bearer "))?;
    for k in &auth.api_keys {
        if k.matches(token) {
            return Some(k.device_name().to_string());
        }
    }
    Some(token.to_string())
}

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