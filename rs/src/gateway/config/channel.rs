//! channel（精确拆分）
use super::super::*;

/// 密钥池里的一条命名密钥
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedKey {
    pub name: String,
    #[serde(rename = "api-key", alias = "api_key")]
    pub api_key: String,
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

pub(crate) fn def_listen() -> u16 { 8380 }

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