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
mod tests;