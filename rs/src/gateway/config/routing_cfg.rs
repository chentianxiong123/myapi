//! routing_cfg（精确拆分）
use super::super::*;

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

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            streaming_first_byte: def_stream_first_byte(),
            non_streaming: def_non_streaming(),
            streaming_idle: def_streaming_idle(),
        }
    }
}