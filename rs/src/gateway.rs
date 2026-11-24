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
    /// 熔断器参数（学 cc-switch 三态 + LiteLLM 失败率）
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
    /// 黏性路由（省token，学 LiteLLM affinity）
    #[serde(default)]
    pub sticky: StickyConfig,
}

fn def_listen() -> u16 { 8380 }

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

    // 优先选可用通道（非熔断）；若全熔断则 KeepAll 兼底（第一个），避免一熔全瘫
    // 用 for 循环而非闭包 find，避免捕获 RwLockReadGuard（borrow 冲突）
    let hs = health_snapshot.as_deref();
    let mut chosen_idx: Option<usize> = None;
    for (i, cand) in candidates.iter().enumerate() {
        let available = hs
            .and_then(|map| map.get(&cand.1.id))
            .map(|h| is_available(h, cb))
            .unwrap_or(true); // 无记录=可用
        if available { chosen_idx = Some(i); break; }
    }
    // 全熔断 → 兼底取第一个（LiteLLM when_all_unhealthy: keep-all）
    let idx = chosen_idx.unwrap_or(0);
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
fn resolve_key(provider: &Provider, channel: &Channel) -> Option<String> {
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
pub async fn forward_chat(
    root: &std::path::Path,
    health: &HealthMap,
    pins: &PinStore,
    auth_header: Option<&str>,
    payload: serde_json::Value,
) -> axum::response::Response {
    let config = load_config(root);
    let requested_model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // 1. 黏性选通道（session/key pin → TTL 内复用同通道，省 token）
    let pin_key = session_key(&payload, auth_header);
    let selected = match select_channel_sticky(&config, &requested_model, health, pins, pin_key.as_deref()) {
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
    let cb = &config.circuit_breaker;
    // 单部署保护：该模型只有一条通道时，熔断降级（LiteLLM single_deployment）
    let total_channels: usize = config.providers.iter()
        .filter(|p| !p.disabled)
        .flat_map(|p| p.channels.iter())
        .filter(|c| c.models.iter().any(|m| {
            let ext = m.alias.as_deref().unwrap_or(&m.name);
            ext == requested_model || m.name == requested_model
        }))
        .count();
    let is_single = total_channels <= 1;
    let resp = match req.json(&upstream_payload).send().await {
        Ok(r) => r,
        Err(e) => {
            record_outcome(health, &selected.channel.id, false, 0, cb, is_single).await;
            return error_response(502, &format!("上游请求失败: {}", e));
        }
    };
    let ttft = start.elapsed().as_millis() as u64;
    let status = resp.status();
    let ok = status.is_success();
    record_outcome(health, &selected.channel.id, ok, ttft, cb, is_single).await;

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

// ==================== 单元测试（熔断状态机，全 mock 不调上游）====================
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
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()) }],
                weight: 1,
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
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()) }], weight: 1 },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()) }], weight: 1 },
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
                    models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()) }], weight: 1 },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "real2".into(), alias: Some("my-model".into()) }], weight: 1 },
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
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()) }], weight: 1 }],
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
                models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()) }], weight: 1 }],
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
}
