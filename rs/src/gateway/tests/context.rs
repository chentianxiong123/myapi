//! context（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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