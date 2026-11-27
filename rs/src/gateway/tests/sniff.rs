//! sniff（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use std::sync::Arc;
use serde_json::json;

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