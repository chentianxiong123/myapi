//! circuit（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

#[tokio::test]
async fn closed_on_success() {
    let h = health_map();
    let c = cb();
    record_outcome(&h, "ch1", true, 100, &c, false, false).await;
    record_outcome(&h, "ch1", true, 100, &c, false, false).await;
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
        record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
    record_outcome(&h, "ch1", true, 0, &c, false, false).await;
    for _ in 0..4 {
        record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
        record_outcome(&h, "ch1", false, 0, &c, true, false).await;
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
        record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
        record_outcome(&h, "ch1", true, 0, &c, false, false).await;
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
        record_outcome(&h, "ch1", false, 0, &c, false, false).await;
    }
    // 时间流逝
    {
        let mut m = h.write().await;
        m.get_mut("ch1").unwrap().last_failure_at = now_secs() - 31;
    }
    // 试探成功 2 次（不到 3）
    record_outcome(&h, "ch1", true, 0, &c, false, false).await;
    record_outcome(&h, "ch1", true, 0, &c, false, false).await;
    // 第 3 次失败 → 重新 open
    record_outcome(&h, "ch1", false, 0, &c, false, false).await;
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
            models: vec![ChannelModel { name: "real".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                ..Default::default()}],
            weight: 1, rpm: None, retry: None, max_input_tokens: None }],
        proxy: String::new(),
    
        ..Default::default()});
    let h = health_map();
    // ch1 打开 + 未超恢复超时 → select_channel 应回退到它（KeepAll，全熔断兼底）
    for _ in 0..5 {
        record_outcome(&h, "ch1", false, 0, &cb(), false, false).await;
    }
    let sel = select_channel(&cfg, "my-model", &h);
    assert!(sel.is_some(), "全熔断时 KeepAll 仍应返回通道");
}

#[tokio::test]
async fn circuit_breaker_excludes_open_channel() {
    // ch1 连续失败（阈值 5）→ Open → 熔断排除，只选 ch2
    let cfg = two_chan_cfg(RoutingStrategy::SimpleShuffle);
    let h = health_map();
    for _ in 0..20 { record_outcome(&h, "ch1", false, 0, &cfg.circuit_breaker, false, false).await; }
    record_outcome(&h, "ch2", true, 0, &cfg.circuit_breaker, false, false).await;
    for _ in 0..50 {
        let sel = select_channel(&cfg, "my-model", &h);
        assert_eq!(sel.unwrap().channel.id, "ch2", "ch1 open 熔断应排除");
    }
}

#[tokio::test]
async fn circuit_breaker_recovers_after_timeout() {
    // ch1 熔断 Open → 超过恢复超时 → half_open 可试探（不再被排除）
    let cfg = two_chan_cfg(RoutingStrategy::SimpleShuffle);
    let h = health_map();
    for _ in 0..10 { record_outcome(&h, "ch1", false, 0, &cfg.circuit_breaker, false, false).await; }
    {
        let mut hm = h.write().await;
        if let Some(e) = hm.get_mut("ch1") {
            e.last_failure_at = now_secs() - cfg.circuit_breaker.recovery_timeout_seconds - 1;
        }
    }
    for _ in 0..50 {
        let sel = select_channel(&cfg, "my-model", &h);
        assert!(sel.is_some(), "恢复后应仍有候选");
    }
}