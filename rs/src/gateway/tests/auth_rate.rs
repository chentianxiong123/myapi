//! auth_rate（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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