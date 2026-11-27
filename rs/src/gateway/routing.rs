//! routing（从 gateway.rs 精确拆分）
use super::*;

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
pub(crate) fn trailing_user_turn(msgs: &[serde_json::Value]) -> &[serde_json::Value] {
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
pub(crate) fn detect_required_caps(payload: &serde_json::Value) -> Vec<String> {
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
pub(crate) fn strip_history_media(messages: &mut Vec<serde_json::Value>, cap: &str) -> usize {
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
pub(crate) fn model_exists(config: &GatewayConfig, requested_model: &str) -> bool {
    config.providers.iter().any(|p| !p.disabled && p.channels.iter().any(|c| {
        c.models.iter().any(|m| {
            let external = m.alias.as_deref().unwrap_or(&m.name);
            external == requested_model || m.name == requested_model
        })
    }))
}

/// 选中通道模型的能力清单（上游模型名 → 找模型定义取 capabilities）。
/// 找不到时返回空（视为不支持任何硬能力 → 历史轮媒体一律降级）。
pub(crate) fn upstream_caps_of(config: &GatewayConfig, selected: &SelectedChannel) -> Vec<String> {
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

/// 限流计数：通道 id → 最近请求时间戳列表（滑动窗口）
pub type RateStore = Arc<RwLock<HashMap<String, Vec<u64>>>>;

pub fn new_rate_store() -> RateStore {
    Arc::new(RwLock::new(HashMap::new()))
}

pub(crate) const RATE_WINDOW_SECS: u64 = 60;

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
pub(crate) fn hash_bearer(auth_header: Option<&str>) -> Option<String> {
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
pub(crate) fn find_channel_by_id(config: &GatewayConfig, channel_id: &str, requested_model: &str) -> Option<SelectedChannel> {
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

pub fn load_config(root: &std::path::Path) -> GatewayConfig {
    let path = root.join("config").join("gateway.yaml");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return GatewayConfig::default(),
    };
    serde_yaml::from_str(&raw).unwrap_or_default()
}

/// 选通道 → 构造上游请求 → 转发 → 回传。
/// 最小版：非流式直接回 JSON；流式透传 SSE 字节流。

/// 学 cc-switch strip_one_m_suffix_for_route_lookup：剥离 Claude Code 的 [1M]/[1m] 1M 上下文标记。
/// 客户端发 model=auto[1M]，网关剥离后识别档位 auto（黑盒调度），真实模型名由网关选。
pub(crate) fn strip_1m_suffix(model: &str) -> String {
    if model.ends_with("[1M]") || model.ends_with("[1m]") {
        model[..model.len() - 4].to_string()
    } else {
        model.to_string()
    }
}