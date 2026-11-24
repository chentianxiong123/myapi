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
}

fn default_weight() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelModel {
    /// 上游真实模型名
    pub name: String,
    /// 对外别名（空=用 name）
    #[serde(default)]
    pub alias: Option<String>,
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
}

fn def_listen() -> u16 { 8380 }

// ==================== 健康度（被动统计，学 LiteLLM cooldown，先留接口）====================

/// 通道健康度（滑动窗口统计）
/// state/Open/HalfOpen 是三态熔断预留字段，管道通了才启用，暂标 allow(dead_code)
#[allow(dead_code)]
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
}

/// 熔断三态（closed/open/half_open，学 cc-switch），暂未启用故 allow(dead_code)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CircuitState {
    #[default]
    Closed,
    Open,
    HalfOpen,
}

/// 健康度表：channel_id → health（全局共享，Arc<RwLock>）
pub type HealthMap = Arc<RwLock<HashMap<String, ChannelHealth>>>;

/// 健康度快照（转发后更新，当前先记录，熔断判定后加）
pub async fn record_outcome(
    health: &HealthMap,
    channel_id: &str,
    ok: bool,
    ttft_ms: u64,
) {
    let mut map = health.write().await;
    let h = map.entry(channel_id.to_string()).or_default();
    h.total_requests += 1;
    h.last_ttft_ms = ttft_ms;
    if ok {
        h.consecutive_successes += 1;
        h.consecutive_failures = 0;
    } else {
        h.failed_requests += 1;
        h.consecutive_failures += 1;
        h.consecutive_successes = 0;
    }
    // TODO: 熔断判定（失败率 > 阈值 → Open，超时 → HalfOpen，连续成功 → Closed）
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
    // 同步快照健康度（避免持锁跨 await；这里 select 是同步的，try_read 足够）
    let health_snapshot = health.try_read().ok();
    for provider in &config.providers {
        if provider.disabled { continue; }
        for channel in &provider.channels {
            for m in &channel.models {
                let external = m.alias.as_deref().unwrap_or(&m.name);
                if external == requested_model || m.name == requested_model {
                    // 解析密钥：key_name 引用，空=第一个
                    let api_key = resolve_key(provider, channel)
                        .unwrap_or_default();
                    let _ = &health_snapshot;
                    return Some(SelectedChannel {
                        provider: provider.clone(),
                        channel: channel.clone(),
                        upstream_model: m.name.clone(),
                        api_key,
                    });
                }
            }
        }
    }
    None
}

/// 解析通道实际用的 key：key_name 指定 → 池里找同名；否则用第一个（主）
fn resolve_key(provider: &Provider, channel: &Channel) -> Option<String> {
    if let Some(kn) = &channel.key_name {
        provider.keys.iter()
            .find(|k| &k.name == kn)
            .map(|k| k.api_key.clone())
    } else {
        provider.keys.first().map(|k| k.api_key.clone())
    }
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
pub async fn forward_chat(
    root: &std::path::Path,
    health: &HealthMap,
    payload: serde_json::Value,
) -> axum::response::Response {
    let config = load_config(root);
    let requested_model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // 1. 选通道
    let selected = match select_channel(&config, &requested_model, health) {
        Some(s) => s,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({
                    "error": {"message": format!("模型 {} 无可用通道", requested_model), "type": "invalid_request_error"}
                })),
            ).into_response();
        }
    };

    // 2. 把请求里的模型名换成上游真实名
    let mut upstream_payload = payload.clone();
    if let Some(obj) = upstream_payload.as_object_mut() {
        obj.insert("model".into(), serde_json::Value::String(selected.upstream_model.clone()));
    }
    let is_stream = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    // 3. 构造上游 URL（base_url + /chat/completions）
    let base = selected.provider.base_url.trim_end_matches('/');
    let url = format!("{}/chat/completions", base);

    // 4. 构造 reqwest 客户端（TODO: 通道代理 path / provider.proxy）
    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => return error_response(500, &format!("client 构造失败: {}", e)),
    };

    let mut req = client.post(&url)
        .header("Authorization", format!("Bearer {}", selected.api_key))
        .header("Content-Type", "application/json");
    // 供应商自定义 headers
    for (k, v) in &selected.provider.headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let start = std::time::Instant::now();
    let resp = match req.json(&upstream_payload).send().await {
        Ok(r) => r,
        Err(e) => {
            record_outcome(health, &selected.channel.id, false, 0).await;
            return error_response(502, &format!("上游请求失败: {}", e));
        }
    };
    let ttft = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let ok = status.is_success();
    record_outcome(health, &selected.channel.id, ok, ttft).await;

    // 5. 回传（流式透传字节流 / 非流式回 JSON）
    let upstream_headers = resp.headers().clone();
    let mut out = axum::response::Response::builder().status(status);
    // 透传 content-type（SSE 必须保留 text/event-stream）
    if let Some(ct) = upstream_headers.get("content-type") {
        out = out.header("content-type", ct);
    }
    let body_bytes = resp.bytes().await.unwrap_or_default();
    let _ = is_stream; // 流式透传已在 body 层面处理（上游回什么我们回什么）
    out.body(axum::body::Body::from(body_bytes)).unwrap_or_else(|_| error_response(500, "回传失败"))
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
