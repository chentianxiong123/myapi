//! sticky（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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