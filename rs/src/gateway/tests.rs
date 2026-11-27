// gateway.rs 的单元测试（拆出子模块文件）
    use super::*;
    use std::sync::Arc;
    use serde_json::json;

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

    // ---- 可配置策略测试 ----

    fn two_chan_cfg(strategy: RoutingStrategy) -> GatewayConfig {
        let mut cfg = GatewayConfig::default();
        cfg.routing = RoutingConfig { strategy, auto_mode_enabled: true, max_retries: 3, ..Default::default() };
        cfg.sticky = StickyConfig { enabled: false, ttl_seconds: 300, ..Default::default() };
        cfg.providers.push(Provider {
            name: "p1".into(), base_url: "http://x/v1".into(), disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey { name: "主".into(), api_key: "k".into() }],
            channels: vec![
                Channel { id: "ch1".into(), name: "c1".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
                Channel { id: "ch2".into(), name: "c2".into(), path: None, key_name: None,
                    models: vec![ChannelModel { name: "m".into(), alias: Some("my-model".into()), capabilities: vec![] ,
                        ..Default::default()}], weight: 1, rpm: None, retry: None, max_input_tokens: None },
            ],
            proxy: String::new(),
        
            ..Default::default()});
        cfg
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

    // ---- Auto 模式测试（capability 匹配 + fallback 链）----

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

    // ---- Vision 能力硬约束：当前轮 vs 历史轮（用户确认的方案）----

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

    // ---- failover 候选链测试 ----

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

    // ---- 下游鉴权测试 ----

    #[test]
    fn auth_no_keys_allows_all() {
        // api_keys 空 → 不鉴权（内网可用），任意 header 都放行，无设备名
        let auth = AuthConfig { api_keys: vec![] , daily_token_budget: None };
        assert_eq!(validate_auth(&auth, None).unwrap(), None);
        assert_eq!(validate_auth(&auth, Some("Bearer anything")).unwrap(), None);
    }

    #[test]
    fn auth_configured_requires_valid_key() {
        let auth = AuthConfig { api_keys: vec![AuthKey::Plain("sk-secret".into())] , daily_token_budget: None };
        // 无 header → 401
        assert!(validate_auth(&auth, None).is_err());
        // 错 key → 401
        assert!(validate_auth(&auth, Some("Bearer wrong")).is_err());
        // 对 key → 放行（Plain 设备名=key 本身）
        assert_eq!(validate_auth(&auth, Some("Bearer sk-secret")).unwrap(), Some("sk-secret".into()));
    }

    #[test]
    fn auth_device_name_distinguishes_multi_device() {
        // 多设备身份区分：Named {name,key} → 鉴权返回设备名（日志/统计按设备聚合）
        let auth = AuthConfig {
            api_keys: vec![
                AuthKey::Named { name: "手机".into(), key: "sk-phone".into() },
                AuthKey::Named { name: "笔记本".into(), key: "sk-laptop".into() },
            ],
            daily_token_budget: None,
        };
        // 每设备独立 key → 识别出对应设备名
        assert_eq!(validate_auth(&auth, Some("Bearer sk-phone")).unwrap(), Some("手机".into()));
        assert_eq!(validate_auth(&auth, Some("Bearer sk-laptop")).unwrap(), Some("笔记本".into()));
        // 未知 key → 401
        assert!(validate_auth(&auth, Some("Bearer sk-evil")).is_err());
        // downstream_key 同样返回设备名
        assert_eq!(downstream_key(&auth, Some("Bearer sk-phone")), Some("手机".into()));
        assert_eq!(downstream_key(&auth, Some("Bearer sk-laptop")), Some("笔记本".into()));
    }

    // ---- 限流测试（防打爆上游）----

    #[test]
    fn rate_limit_allows_under_rpm() {
        let store = new_rate_store();
        // 限 10 rpm，发 5 次 → 都放行
        for _ in 0..5 {
            assert!(!is_rate_limited(&store, "ch1", 10), "未超限应放行");
        }
    }

    #[test]
    fn rate_limit_blocks_over_rpm() {
        let store = new_rate_store();
        // 限 3 rpm，发 3 次（都放行），第 4 次应拦
        for _ in 0..3 {
            assert!(!is_rate_limited(&store, "ch2", 3));
        }
        assert!(is_rate_limited(&store, "ch2", 3), "超 rpm 应限流");
    }

    #[test]
    fn rate_limit_per_channel_independent() {
        let store = new_rate_store();
        // ch1 打满，ch2 不受影响（按通道独立计数）
        for _ in 0..3 {
            is_rate_limited(&store, "ch1", 3);
        }
        assert!(is_rate_limited(&store, "ch1", 3), "ch1 应限流");
        assert!(!is_rate_limited(&store, "ch2", 3), "ch2 应独立放行");
    }

    // ==================== 各协议 usage 统计对照测试（非流式 + 流式 SSE）====================
    // 每种协议的 usage 字段名/位置不同，这里逐协议对照验证 parse + SSE scan。
    // 对照基准：new-api relay-{openai,claude,gemini}.go 的 usage 解析。

    #[test]
    fn proto_openai_usage_nonstream() {
        // OpenAI 非流式：usage.prompt_tokens / completion_tokens
        let v = json!({"usage": {"prompt_tokens": 10, "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 3}}});
        let (i, o, cr, cc) = parse_usage_json(&v);
        assert_eq!((i, o, cr, cc), (Some(10), Some(5), Some(3), None), "OpenAI 非流式");
    }

    #[test]
    fn proto_anthropic_usage_nonstream() {
        // Anthropic 非流式：usage.input_tokens / output_tokens + cache 两字段
        let v = json!({"usage": {"input_tokens": 8, "output_tokens": 4,
            "cache_read_input_tokens": 6, "cache_creation_input_tokens": 2}});
        let (i, o, cr, cc) = parse_usage_json(&v);
        assert_eq!((i, o, cr, cc), (Some(8), Some(4), Some(6), Some(2)), "Anthropic 非流式");
    }

    #[test]
    fn proto_gemini_usage_nonstream() {
        // Gemini 非流式：顶层 usageMetadata.promptTokenCount / candidatesTokenCount
        let v = json!({"usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 7,
            "cachedContentTokenCount": 4}});
        let (i, o, cr, _cc) = parse_usage_json(&v);
        assert_eq!((i, o, cr), (Some(12), Some(7), Some(4)), "Gemini 非流式");
    }

    #[test]
    fn proto_openai_usage_stream() {
        // OpenAI 流式：最后 chunk 带 usage（同一 chunk 含 input+output）
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3}}\n\n\
                    data: [DONE]\n\n";
        let u = sse_scan_usage(body.as_bytes()).unwrap();
        assert_eq!(u, (Some(9), Some(3), None, None), "OpenAI 流式");
    }

    #[test]
    fn proto_anthropic_usage_stream_split() {
        // Anthropic 流式：usage 分片——message_start 带 input，message_delta 带 output。
        // 必须"跨 chunk 累积合并"，不能取最后一个（否则丢 input）。
        let body = "data: {\"type\":\"message_start\",\"usage\":{\"input_tokens\":11}}\n\n\
                    data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}\n\n\
                    data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\n";
        let u = sse_scan_usage(body.as_bytes()).unwrap();
        assert_eq!(u, (Some(11), Some(5), None, None), "Anthropic 分片合并 input+output");
    }

    #[test]
    fn proto_gemini_usage_stream() {
        // Gemini 流式：chunk 顶层 usageMetadata（promptTokenCount/candidatesTokenCount）
        let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}\n\n\
                    data: {\"usageMetadata\":{\"promptTokenCount\":15,\"candidatesTokenCount\":6}}\n\n";
        let u = sse_scan_usage(body.as_bytes()).unwrap();
        assert_eq!(u, (Some(15), Some(6), None, None), "Gemini 流式");
    }

    #[test]
    fn proto_stream_no_usage_returns_none() {
        // 无 usage 的流（纯内容）→ None，不误报
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
        assert_eq!(sse_scan_usage(body.as_bytes()), None);
    }


    // ---- context 不足错误反馈（用户拍板：网关全试，全废才反馈）----

    #[test]
    fn context_keyword_detection() {
        let cfg = RetryConfig::default();
        // 标准 OpenAI context_length_exceeded 响应体
        let body = br#"{"error":{"message":"This model's maximum context length is 200000 tokens. You requested 250000 tokens."}}"#;
        assert!(hit_disable_keyword(body, &cfg.context_keywords).is_some(),
            "context 词表应命中 context_length_exceeded");
        // 欠费词表不命中 context 错误（两组词表独立）
        assert!(hit_disable_keyword(body, &cfg.disable_keywords).is_none(),
            "欠费词表不应命中 context 错误");
        // 欠费错误也不命中 context 词表
        let quota = br#"{"error":{"message":"insufficient quota"}}"#;
        assert!(hit_disable_keyword(quota, &cfg.disable_keywords).is_some());
        assert!(hit_disable_keyword(quota, &cfg.context_keywords).is_none());
    }

    #[test]
    fn forward_error_context_is_context() {
        let e = ForwardError::Context { status: Some(400), msg: "ctx".into() };
        assert!(e.is_context(), "Context 变体应 is_context");
        assert!(!e.is_retryable(), "Context 不是普通可重试");
        assert_eq!(e.status(), Some(400));
        let r = ForwardError::Retryable { status: Some(429), retry_after: None, msg: "r".into() };
        assert!(!r.is_context(), "Retryable 不是 context");
    }

    // ---- 上游模型差异检测（纯手动 detect/apply 学 new-api）----
    #[test]
    fn diff_upstream_add_remove_and_regex_ignored() {
        let upstream = vec!["gpt-4o".into(), "gpt-4o-mini".into(), "gpt-5-preview".into(), "new-model".into()];
        let local = vec!["gpt-4o".into(), "gone-model".into()];
        let (add, remove, ignored) = diff_upstream_models(&upstream, &local, &["regex:.*-preview".into()]);
        assert_eq!(add, vec!["gpt-4o-mini".to_string(), "new-model".to_string()], "新增=上游有本地无且未被忽略");
        assert_eq!(remove, vec!["gone-model".to_string()], "消失=本地有上游无");
        assert_eq!(ignored, vec!["gpt-5-preview".to_string()], "regex: 规则命中应进 ignored 不进 add");
    }

    #[test]
    fn diff_upstream_no_change_and_empty_rules() {
        let models = vec!["m1".into(), "m2".into()];
        let (add, remove, ignored) = diff_upstream_models(&models, &models, &[]);
        assert!(add.is_empty() && remove.is_empty() && ignored.is_empty(), "完全一致应无差异");
    }

    #[test]
    fn diff_upstream_exact_rule_not_regex() {
        let upstream = vec!["m1".into(), "m2".into(), "m3".into()];
        let (add, _, ignored) = diff_upstream_models(&upstream, &["m1".into()], &["m3".into()]);
        assert_eq!(add, vec!["m2".to_string()], "无 regex: 前缀应精确匹配");
        assert_eq!(ignored, vec!["m3".to_string()]);
    }

    // ---- 按协议拼嗅探 URL + 前缀剥离（学 new-api fetchChannelUpstreamModelIDs）----
    #[test]
    fn upstream_models_url_by_protocol() {
        assert_eq!(upstream_models_url("http://x:1/", "openai_chat"), "http://x:1/models");
        assert_eq!(upstream_models_url("http://x:1", "openai_responses"), "http://x:1/models");
        assert_eq!(upstream_models_url("http://x:1", "anthropic"), "http://x:1/models");
        assert_eq!(upstream_models_url("http://x:1", "gemini"), "http://x:1/v1beta/models");
        assert_eq!(upstream_models_url("http://x:1", "ollama"), "http://x:1/api/tags");
    }

    #[test]
    fn strip_model_prefix_by_protocol() {
        assert_eq!(strip_model_prefix("models/gemini-2.0-flash", "gemini"), "gemini-2.0-flash");
        assert_eq!(strip_model_prefix("gemini-2.0-flash", "gemini"), "gemini-2.0-flash");
        assert_eq!(strip_model_prefix("llama3:latest", "ollama"), "llama3:latest");
        assert_eq!(strip_model_prefix("gpt-4o", "openai_chat"), "gpt-4o");
    }