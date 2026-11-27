//! tier_caps（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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