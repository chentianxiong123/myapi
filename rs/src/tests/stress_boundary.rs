//! stress_boundary（按主题拆分）
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
async fn chat_completions_with_no_upstream_returns_error() {
    let (router, _d) = test_router().await;
    // 无真实上游（tempdir 无 providers → 无通道）→ 404 invalid_request
    let payload = serde_json::json!({"model": "gpt-x", "messages": [{"role": "user", "content": "hi"}]});
    let (st, j) = http_post(router, "/v1/chat/completions", payload).await;
    assert!(st == StatusCode::NOT_FOUND || st == StatusCode::BAD_REQUEST || st == StatusCode::INTERNAL_SERVER_ERROR || st == StatusCode::SERVICE_UNAVAILABLE);
    assert!(j.get("error").is_some());
}

#[tokio::test]
async fn concurrent_100_requests_all_ok() {
    // 100 并发 → 全 200 + 日志 100（更高强度）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}})))
        .expect(100).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let mut handles = Vec::new();
    for _ in 0..100 {
        let (root, health, pins, rates, cache) = (state.root.clone(), state.health.clone(), state.pins.clone(), state.rates.clone(), state.cache.clone());
        handles.push(tokio::spawn(async move {
            let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
            gateway::forward_chat(&root, &health, &pins, &rates, None, payload, &cache).await.status()
        }));
    }
    for h in handles { assert_eq!(h.await.unwrap(), StatusCode::OK); }
    let con = rusqlite::Connection::open(dir.path().join("config").join("gateway-logs.db")).unwrap();
    let n: i64 = con.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 100, "100 并发应记 100 条日志");
}

#[tokio::test]
async fn concurrent_stream_50_all_ok() {
    // 50 流式并发 → 全 200（SSE 流式并发稳定）
    let upstream = MockServer::start().await;
    let body = sse(&[
        json!({"choices":[{"delta":{"content":"he"}}]}).to_string(),
        json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":8}}).to_string(),
    ]);
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body)
            .insert_header("content-type", "text/event-stream"))
        .expect(50).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let mut handles = Vec::new();
    for _ in 0..50 {
        let (root, health, pins, rates, cache) = (state.root.clone(), state.health.clone(), state.pins.clone(), state.rates.clone(), state.cache.clone());
        handles.push(tokio::spawn(async move {
            let payload = json!({"model": "up-model", "stream": true, "messages": [{"role": "user", "content": "hi"}]});
            let resp = gateway::forward_chat(&root, &health, &pins, &rates, None, payload, &cache).await;
            let st = resp.status();
            let _ = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap();
            st
        }));
    }
    for h in handles { assert_eq!(h.await.unwrap(), StatusCode::OK, "流式并发应全 200"); }
}

#[tokio::test]
async fn sequential_100_stress() {
    // 100 连续请求（串行）→ 全 200 + 日志 100（压力稳定）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}})))
        .expect(100).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    for _ in 0..100 {
        let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let con = rusqlite::Connection::open(dir.path().join("config").join("gateway-logs.db")).unwrap();
    let n: i64 = con.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 100, "100 连续应记 100 条日志");
}

#[tokio::test]
async fn long_conversation_1000_messages() {
    // 长对话：1000 条消息（500 轮 user/assistant）→ 上游收到完整 1000 条
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let mut msgs = Vec::new();
    for i in 0..1000 {
        msgs.push(json!({"role": if i % 2 == 0 {"user"} else {"assistant"}, "content": format!("第{}轮消息", i)}));
    }
    let payload = json!({"model": "up-model", "messages": msgs});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = upstream.received_requests().await.unwrap();
    let v: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(v["messages"].as_array().unwrap().len(), 1000, "长对话 1000 条完整转发");
}

#[tokio::test]
async fn concurrent_50_requests_all_ok() {
    // 50 并发 → 全 200 + 日志 50 条（稳定性/无丢）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}})))
        .expect(50).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let mut handles = Vec::new();
    for _ in 0..50 {
        let (root, health, pins, rates, cache) = (state.root.clone(), state.health.clone(), state.pins.clone(), state.rates.clone(), state.cache.clone());
        handles.push(tokio::spawn(async move {
            let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
            gateway::forward_chat(&root, &health, &pins, &rates, None, payload, &cache).await.status()
        }));
    }
    for h in handles { assert_eq!(h.await.unwrap(), StatusCode::OK); }
    let con = rusqlite::Connection::open(dir.path().join("config").join("gateway-logs.db")).unwrap();
    let n: i64 = con.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 50, "50 并发应记 50 条日志（无丢）");
}

#[tokio::test]
async fn concurrent_failover_stable() {
    // 并发 + ch1 500 → ch2 200（failover 稳定无 panic）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-bad"})))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "boom"}})))
        .mount(&upstream).await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-good"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "model": "up-good",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let channels = json!([
        {"id": "ch1", "name": "bad", "key_name": "主", "models": [{"name": "up-bad", "alias": "my-model"}]},
        {"id": "ch2", "name": "good", "key_name": "主", "models": [{"name": "up-good", "alias": "my-model"}]},
    ]);
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);
    let state = S::new(dir.path().to_path_buf());
    let mut handles = Vec::new();
    for _ in 0..30 {
        let (root, health, pins, rates, cache) = (state.root.clone(), state.health.clone(), state.pins.clone(), state.rates.clone(), state.cache.clone());
        handles.push(tokio::spawn(async move {
            let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
            gateway::forward_chat(&root, &health, &pins, &rates, None, payload, &cache).await.status()
        }));
    }
    for h in handles { assert_eq!(h.await.unwrap(), StatusCode::OK, "并发 failover 应全 200"); }
}

#[tokio::test]
async fn boundary_empty_and_large_payload() {
    // 边界：空 content 消息 + 大 payload（大量消息）→ 正常处理
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}]})))
        .expect(2).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    // 空 content
    let payload1 = json!({"model": "up-model", "messages": [{"role": "user", "content": ""}]});
    let r1 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload1, &state.cache).await;
    assert_eq!(r1.status(), StatusCode::OK);
    // 大 payload（500 条消息）
    let mut msgs = Vec::new();
    for i in 0..500 {
        msgs.push(json!({"role": if i % 2 == 0 {"user"} else {"assistant"}, "content": format!("消息 {}", i)}));
    }
    let payload2 = json!({"model": "up-model", "messages": msgs});
    let r2 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload2, &state.cache).await;
    assert_eq!(r2.status(), StatusCode::OK);
}