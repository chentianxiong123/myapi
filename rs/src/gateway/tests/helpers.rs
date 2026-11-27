//! helpers（按主题拆分）
#![allow(unused_imports)]
use super::*;
use std::sync::Arc;
use serde_json::json;

pub(super) fn cb() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        failure_threshold: 5,
        error_rate_threshold: 50,
        min_requests: 5,
        recovery_timeout_seconds: 30,
        recovery_success_threshold: 3,
    }
}

pub(super) fn health_map() -> HealthMap {
    Arc::new(RwLock::new(HashMap::new()))
}

pub(super) fn two_chan_cfg(strategy: RoutingStrategy) -> GatewayConfig {
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