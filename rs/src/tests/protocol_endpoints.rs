//! protocol_endpoints（按主题拆分）
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
async fn messages_thinking_required_no_capability_409() {
    // anthropic 协议（/v1/messages）带 thinking → 模型无 thinking 能力 → 409（同 chat 硬约束）
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
    let router = build_router(S::new(dir.path().to_path_buf()));
    // /v1/messages 带 thinking enabled → 转 chat reasoning_effort → 模型无 thinking → 409
    let body = json!({"model": "my-model", "max_tokens": 100,
        "thinking": {"type": "enabled", "budget_tokens": 16000},
        "messages": [{"role": "user", "content": "hi"}]});
    let resp = router.oneshot(Request::post("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT, "anthropic 带 thinking + 模型无 thinking → 409");
}

#[tokio::test]
async fn messages_endpoint_anthropic_roundtrip() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "你好"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5},
        }))).expect(1).mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));

    let body = json!({"model": "up-model", "max_tokens": 100,
        "system": "你是助手", "messages": [{"role": "user", "content": "你好"}]});
    let resp = router.oneshot(Request::post("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(j["role"], "assistant");
    assert_eq!(j["content"][0]["type"], "text");
    assert_eq!(j["content"][0]["text"], "你好");
    assert_eq!(j["stop_reason"], "end_turn");
    assert_eq!(j["usage"]["input_tokens"], 10);
    assert_eq!(j["usage"]["output_tokens"], 5);
}

#[tokio::test]
async fn messages_endpoint_multi_turn_history() {
    // anthropic /v1/messages 多轮（user/assistant/user）→ 网关转 chat → 上游收到完整 3 条
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "回复"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));
    let body = json!({"model": "auto", "max_tokens": 100, "messages": [
        {"role": "user", "content": "第一轮"},
        {"role": "assistant", "content": "第一轮回复"},
        {"role": "user", "content": "第二轮"},
    ]});
    let resp = router.oneshot(Request::post("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = upstream.received_requests().await.unwrap();
    let mbody: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(mbody["messages"].as_array().unwrap().len(), 3, "messages 多轮历史完整");
    assert_eq!(mbody["messages"][1]["content"], "第一轮回复", "assistant 轮保留");
}

#[tokio::test]
async fn responses_endpoint_multi_turn_history() {
    // responses /v1/responses 多轮（message user/assistant/user）→ 转 chat → 上游完整 3 条
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));
    let body = json!({"model": "auto", "instructions": "you are helper",
        "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "第一轮"}]},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "第一轮回复"}]},
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "第二轮"}]},
        ]});
    let resp = router.oneshot(Request::post("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = upstream.received_requests().await.unwrap();
    let mbody: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(mbody["messages"].as_array().unwrap().len(), 4, "responses 多轮历史完整（system + 3 message）");
    assert_eq!(mbody["messages"][1]["content"], "第一轮", "第一轮 user 保留");
    assert_eq!(mbody["messages"][2]["content"], "第一轮回复", "assistant 轮保留");
    assert_eq!(mbody["messages"][3]["content"], "第二轮");
}

#[tokio::test]
async fn messages_endpoint_anthropic_stream() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(concat!("data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"index\":0}],\"model\":\"up-model\"}\n\n", "data: {\"choices\":[{\"delta\":{},\"index\":0}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n", "data: [DONE]\n\n")))
        .expect(1).mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));

    let body = json!({"model": "up-model", "max_tokens": 100, "stream": true,
        "messages": [{"role": "user", "content": "你好"}]});
    let resp = router.oneshot(Request::post("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(s.contains("message_start"), "应发 message_start，实际: {s}");
    assert!(s.contains("text_delta"), "应发 text_delta，实际: {s}");
    assert!(s.contains("你好"), "文本应转换，实际: {s}");
    assert!(s.contains("message_delta"), "应发 message_delta，实际: {s}");
    assert!(s.contains("message_stop"), "应发 message_stop，实际: {s}");
}

#[tokio::test]
async fn responses_endpoint_roundtrip() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-1", "model": "up-model", "created": 123,
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2},
        }))).expect(1).mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));

    let body = json!({"model": "up-model", "instructions": "you are helper",
        "input": [{"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]}]});
    let resp = router.oneshot(Request::post("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(j["object"], "response");
    assert_eq!(j["status"], "completed");
    assert_eq!(j["output"][0]["type"], "message");
    assert_eq!(j["output"][0]["content"][0]["text"], "hi");
    assert_eq!(j["usage"]["input_tokens"], 3);
}

#[tokio::test]
async fn responses_endpoint_stream() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(concat!("data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"index\":0}],\"model\":\"up-model\"}\n\n", "data: {\"choices\":[{\"delta\":{},\"index\":0}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n", "data: [DONE]\n\n")))
        .expect(1).mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));

    let body = json!({"model": "up-model", "stream": true,
        "input": [{"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]}]});
    let resp = router.oneshot(Request::post("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(s.contains("response.created"), "应发 response.created，实际: {s}");
    assert!(s.contains("response.output_text.delta"), "应发 delta，实际: {s}");
    assert!(s.contains("response.completed"), "应发 completed，实际: {s}");
    assert!(s.contains("\"status\":\"completed\""));
}

#[tokio::test]
async fn anthropic_upstream_nonstream_roundtrip() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .and(wiremock::matchers::header("x-api-key", "sk-upstream-test"))
        .and(wiremock::matchers::body_partial_json(json!({
            "messages": [{"role": "user", "content": [{"type": "text", "text": "你好"}]}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1", "model": "claude-up",
            "content": [{"type": "text", "text": "你好"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5},
        }))).expect(1).mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    let ch = json!([{"id": "ch1", "name": "ch1", "key_name": "主",
        "models": [{"name": "claude-up", "protocol": "anthropic"}]}]);
    write_gateway_yaml(dir.path(), &upstream.uri(), ch);
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "claude-up", "max_tokens": 100,
        "messages": [{"role": "user", "content": "你好"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    // 上游 anthropic → 转回 chat
    assert_eq!(j["choices"][0]["message"]["content"], "你好");
    assert_eq!(j["choices"][0]["finish_reason"], "stop");
    assert_eq!(j["usage"]["prompt_tokens"], 10);
    assert_eq!(j["usage"]["completion_tokens"], 5);
}

#[tokio::test]
async fn anthropic_upstream_stream_to_chat_sse() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(concat!(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-up\"}}\n\n",
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
                "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"))).expect(1).mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    let ch = json!([{"id": "ch1", "name": "ch1", "key_name": "主",
        "models": [{"name": "claude-up", "protocol": "anthropic"}]}]);
    write_gateway_yaml(dir.path(), &upstream.uri(), ch);
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "claude-up", "max_tokens": 100, "stream": true,
        "messages": [{"role": "user", "content": "你好"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let s = String::from_utf8(bytes.to_vec()).unwrap();
    // anthropic SSE → chat SSE（delta.content + [DONE] + usage chunk）
    assert!(s.contains("\"content\":\"你好\""), "应含 chat delta content，实际: {s}");
    assert!(s.contains("[DONE]"), "应含 [DONE]，实际: {s}");
    assert!(s.contains("prompt_tokens"), "应含 usage chunk，实际: {s}");

    // 流式 usage 抓取：日志应记 input=10 output=5（poll 等入库，避免固定 sleep flaky）
    assert!(wait_summary(&dir.path(), Some(10), Some(5)).await, "流式 usage 应入库");
    let logs = gateway_logs::query_recent(&dir.path(), 5).unwrap();
    let data = logs.get("data").and_then(|v| v.as_array()).unwrap();
    assert!(!data.is_empty(), "流式请求应记日志");
    assert_eq!(data[0].get("input_tokens").and_then(|v| v.as_i64()), Some(10), "流式 anthropic usage input_tokens");
    assert_eq!(data[0].get("output_tokens").and_then(|v| v.as_i64()), Some(5), "流式 anthropic usage output_tokens");
}