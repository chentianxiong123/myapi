//! retry（精确拆分）
use super::super::*;

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