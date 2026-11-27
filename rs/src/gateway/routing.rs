//! 档位路由/候选选择（从 gateway.rs 拆出）
use super::*;

fn trailing_user_turn(msgs: &[serde_json::Value]) -> &[serde_json::Value] {
    let mut i = msgs.len();
    while i > 0 {
        let role = msgs[i - 1].get("role").and_then(|v| v.as_str());
        if role == Some("assistant") || role == Some("model") { break; }
        i -= 1;
    }
    &msgs[i..]
}

fn model_exists(config: &GatewayConfig, requested_model: &str) -> bool {
    config.providers.iter().any(|p| !p.disabled && p.channels.iter().any(|c| {
        c.models.iter().any(|m| {
            let external = m.alias.as_deref().unwrap_or(&m.name);
            external == requested_model || m.name == requested_model
        })
    }))
}

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

pub fn resolve_key(provider: &Provider, channel: &Channel) -> Option<String> {
    if let Some(kn) = &channel.key_name {
        provider.keys.iter()
            .find(|k| &k.name == kn)
            .map(|k| k.api_key.clone())
    } else {
        provider.keys.first().map(|k| k.api_key.clone())
    }
}

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

fn hash_bearer(auth_header: Option<&str>) -> Option<String> {
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
                        protocol: m.protocol.clone() });
                }
            }
        }
    }
    None
}

pub fn estimate_input_tokens(payload: &serde_json::Value) -> i64 {
    let mut total: i64 = 0;
    if let Some(msgs) = payload.get("messages").and_then(|v| v.as_array()) {
        for m in msgs {
            if let Some(c) = m.get("content").and_then(|v| v.as_str()) {
                total += (c.len() as i64) / 4 + 1;
            }
        }
    }
    total
}
