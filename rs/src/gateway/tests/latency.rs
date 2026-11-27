//! latency（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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