//! 网关转发核心（照 LiteLLM router-core 建模，最小可跑通管道）
//!
//! 数据流：/v1/chat/completions → 选通道（按模型）→ 转发上游 → 流式回传
//! 智能路由（黏性/熔断/延迟感知）先留接口，管道通了再加。
mod config;
mod usage;
mod circuit;
mod cache;
mod routing;
mod forward;
pub use forward::*;
pub use routing::*;
pub use cache::*;
pub use circuit::*;
pub use usage::*;
pub use config::*;


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

/// 关键词熔断默认词表（学 new-api，覆盖常见欠费/封号/配额耗尽表述）

/// 上下文不足默认词表（context_length_exceeded 等）：模型窗口装不下请求



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


/// 通道测试配置（用户可编辑，放 gateway.yaml，学 new-api TestChannel 的可配置测试内容）



/// 响应缓存配置



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

/// 按路由策略从可用候选里选一个（返回 candidates 的索引）。
/// 先熔断过滤，全不可用则 KeepAll 兑底（index 0）。

/// 轻量随机数（避免引入 rand 依赖：系统时间纳秒 + 计数器混合）

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

/// 熔断器配置（学 cc-switch 三态机 + LiteLLM 失败率判定）

/// 黏性路由配置（学 LiteLLM deployment affinity，为 prompt cache 省token）

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

/// 超时配置（学 cc-switch 流式/非流式分开）

/// 下游鉴权配置（agent 调网关时校验 Bearer key，防裸奍）

/// 下游 key：裸字符串（无设备名）或带设备名对象。untagged 向后兼容 `api_keys: ["sk-1"]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthKey {
    Plain(String),
    Named { name: String, key: String },
}




/// 校验下游 Bearer key（api_keys 空=不鉴权内网可用；配了=严格校验）。
/// 返回设备名（多设备身份区分：Named 的 name；Plain 用 key 本身）；未配返回 None。

/// 取下游设备名（多设备身份区分：Named→name；Plain→key 本身；未配→原始 key 兜底）

// ==================== 健康度（被动统计，学 LiteLLM cooldown，先留接口）====================

/// 通道健康度（被动统计 + 三态熔断，学 cc-switch 状态机 + LiteLLM 失败率）
#[derive(Debug, Clone, Default)]

/// 熔断三态（closed/open/half_open，学 cc-switch）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]

/// 健康度表：channel_id → health（全局共享，Arc<RwLock>）
pub type HealthMap = Arc<RwLock<HashMap<String, ChannelHealth>>>;


/// 三态熔断状态机（学 cc-switch closed→open→half_open→closed）
/// 判定融合 LiteLLM：连续失败达阈值 OR 错误率>阈值（请求数≥min）→ open
/// 唯一可用通道不熔断（LiteLLM single_deployment 保护，避免一熔全瘫）

/// 关键词熔断：错误响应体含配置关键词（欠费/封号等"重试也没用"的错误）→ true。
/// 学 new-api AutomaticDisableKeywords。只在失败时调，body 文本小写化后匹配。
pub(crate) fn hit_disable_keyword(body: &[u8], keywords: &[String]) -> Option<String> {
    if keywords.is_empty() { return None; }
    let text = String::from_utf8_lossy(body).to_lowercase();
    keywords.iter().find(|k| !k.is_empty() && text.contains(&k.to_lowercase())).cloned()
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

/// 解析通道实际用的 key：key_name 指定 → 池里找同名；否则用第一个（主）

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

/// Bearer token → hash（不存明文）；无则 None

/// 带黏性的通道选择：有有效 pin 且通道可用→复用；否则新选并打 pin
/// 按路由策略返回排序后的候选链（普通路由 failover 用）。
/// 可用通道优先按策略排序；全熔断则 KeepAll 兑底（原顺序）。pin 的通道提到最前。


/// 按 channel id + 模型名定位通道（黏性复用用）

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


/// 转发失败分类（学 LiteLLM AttemptError）：区分可重试/不可重试


/// 粗估 input token（不引 tokenizer SDK：4 字符≈1 token，个人网关够用）

/// 响应缓存条目
#[derive(Clone)]

/// 响应缓存 store（全局共享）
pub type ResponseCacheStore = std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, CachedResponse>>>;

/// 缓存 key（学 LiteLLM cache-response：SHA256(model + messages)）

/// 查缓存（TTL 内有效）

/// 存缓存（超 max_entries 淘汰最旧）

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

/// 重试退避延迟（学 LiteLLM Backoff）：0.5s 起，指数 ×2 到 8s 封顶，加 0.75 jitter 防雪崩。
/// 优先尊重上游 retry-after。attempted=已重试次数。
/// 重试退避延迟（学 LiteLLM Backoff）：initial 起 ×2 到 max 封顶，加 jitter 防雪崩。
/// 尊重上游 retry-after（若 policy 允许）。attempted=已重试次数。

/// 轻量伪随机 [0,1)，从纳秒时间戳取，避免引 rand 依赖（jitter 防雪崩够用）

/// 转发到单个通道（Auto 和普通路由共用）：失败返回 ForwardError（区分可重试），成功返回响应
/// 通道测试（学 new-api TestChannel）：真发一个 chat 请求验证通道"真能出 token"
/// 用配置的默认模型（空=通道第一个模型）+ 测试对话。返回 {ok, status, latency_ms, content/error}

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

    // ---- 上游模型差异检测（纯手动 detect/apply 学 new-api）----
    #[test]
    fn diff_upstream_add_remove_and_regex_ignored() {
        let upstream = vec!["gpt-4o".into(), "gpt-4o-mini".into(), "gpt-5-preview".into(), "new-model".into()];
        let local = vec!["gpt-4o".into(), "gone-model".into()];
        let (add, remove, ignored) = diff_upstream_models(&upstream, &local, &["regex:.*-preview".into()]);
        assert_eq!(add, vec!["gpt-4o-mini".to_string(), "new-model".to_string()], "新增=上游有本地无且未被忽略");
        assert_eq!(remove, vec!["gone-model".to_string()], "消失=本地有上游无");
        assert_eq!(ignored, vec!["gpt-5-preview".to_string()], "regex: 规则命中应进 ignored 不进 add");
    }

    #[test]
    fn diff_upstream_no_change_and_empty_rules() {
        let models = vec!["m1".into(), "m2".into()];
        let (add, remove, ignored) = diff_upstream_models(&models, &models, &[]);
        assert!(add.is_empty() && remove.is_empty() && ignored.is_empty(), "完全一致应无差异");
    }

    #[test]
    fn diff_upstream_exact_rule_not_regex() {
        let upstream = vec!["m1".into(), "m2".into(), "m3".into()];
        let (add, _, ignored) = diff_upstream_models(&upstream, &["m1".into()], &["m3".into()]);
        assert_eq!(add, vec!["m2".to_string()], "无 regex: 前缀应精确匹配");
        assert_eq!(ignored, vec!["m3".to_string()]);
    }

    // ---- 按协议拼嗅探 URL + 前缀剥离（学 new-api fetchChannelUpstreamModelIDs）----
    #[test]
    fn upstream_models_url_by_protocol() {
        assert_eq!(upstream_models_url("http://x:1/", "openai_chat"), "http://x:1/models");
        assert_eq!(upstream_models_url("http://x:1", "openai_responses"), "http://x:1/models");
        assert_eq!(upstream_models_url("http://x:1", "anthropic"), "http://x:1/models");
        assert_eq!(upstream_models_url("http://x:1", "gemini"), "http://x:1/v1beta/models");
        assert_eq!(upstream_models_url("http://x:1", "ollama"), "http://x:1/api/tags");
    }

    #[test]
    fn strip_model_prefix_by_protocol() {
        assert_eq!(strip_model_prefix("models/gemini-2.0-flash", "gemini"), "gemini-2.0-flash");
        assert_eq!(strip_model_prefix("gemini-2.0-flash", "gemini"), "gemini-2.0-flash");
        assert_eq!(strip_model_prefix("llama3:latest", "ollama"), "llama3:latest");
        assert_eq!(strip_model_prefix("gpt-4o", "openai_chat"), "gpt-4o");
    }
}
