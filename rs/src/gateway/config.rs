//! 配置结构（从 gateway.rs 拆出：config 独立模块）
use super::*;
use serde;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// 配置相关默认值/结构（Retry/Sticky/Timeout/Routing/Gateway/Auth/Cache/Circuit）

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

fn def_listen() -> u16 { 8380 }

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

pub(crate) fn downstream_key(auth: &AuthConfig, auth_header: Option<&str>) -> Option<String> {
    let token = auth_header.and_then(|h| h.strip_prefix("Bearer "))?;
    for k in &auth.api_keys {
        if k.matches(token) {
            return Some(k.device_name().to_string());
        }
    }
    Some(token.to_string())
}
