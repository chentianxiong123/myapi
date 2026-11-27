//! thinking_caps（按主题拆分）
#![allow(unused_imports)]
use super::*;
use super::helpers::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use tempfile::TempDir;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};
use crate::gateway::{hit_disable_keyword, record_outcome, HealthMap, CircuitState};

#[tokio::test]
async fn thinking_required_no_capability_409_end_to_end() {
    // 思考模式同 vision 待遇：请求带 thinking（enabled）+ 模型无 thinking 能力 → 409 显式失败
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let cfg = json!({"listen": 0,
        "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk"}],
            "channels": [{"id": "ch1", "name": "c1", "key_name": "主",
                          "models": [{"name": "up-model", "alias": "my-model", "capabilities": ["vision"]}]}]}]});
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
    let state = S::new(dir.path().to_path_buf());
    // chat 请求带 reasoning_effort（思考开启）→ 需要 thinking，但模型只有 vision → 409
    let payload = json!({"model": "my-model", "reasoning_effort": "high",
        "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT, "带 thinking + 模型无 thinking → 409");
}

#[tokio::test]
async fn thinking_capability_satisfied_200_end_to_end() {
    // 模型带 thinking 能力 + 请求带 thinking → 200（能力匹配）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x",
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let cfg = json!({"listen": 0,
        "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk"}],
            "channels": [{"id": "ch1", "name": "c1", "key_name": "主",
                          "models": [{"name": "up-model", "alias": "my-model", "capabilities": ["thinking"]}]}]}]});
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "reasoning_effort": "high",
        "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK, "模型带 thinking + 请求带 thinking → 200");
}

#[tokio::test]
async fn thinking_disabled_no_requirement_200_end_to_end() {
    // thinking: {type: "disabled"} 不算需求 → 模型无 thinking 也 200（客户端明确关）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x",
            "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let cfg = json!({"listen": 0,
        "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk"}],
            "channels": [{"id": "ch1", "name": "c1", "key_name": "主",
                          "models": [{"name": "up-model", "alias": "my-model"}]}]}]});
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model",
        "thinking": {"type": "disabled"},
        "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK, "thinking disabled 不算需求 → 200");
}