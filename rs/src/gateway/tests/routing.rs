//! routing（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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