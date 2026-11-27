//! multi_turn_stream（按主题拆分）
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
async fn multi_turn_history_preserved() {
    // 多轮：user/assistant/user → 上游收到完整历史（3 条，assistant 消息保留）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl", "object": "chat.completion",
            "choices": [{"message": {"role": "assistant", "content": "回复"}}]})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [
        {"role": "user", "content": "第一轮"},
        {"role": "assistant", "content": "第一轮回复"},
        {"role": "user", "content": "第二轮"},
    ]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = upstream.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 3, "多轮历史应完整转发");
    assert_eq!(body["messages"][1]["content"], "第一轮回复", "assistant 消息保留");
    assert_eq!(body["messages"][2]["content"], "第二轮");
}

#[tokio::test]
async fn multi_turn_tool_round_preserved() {
    // tool_use（assistant）+ tool_result（user）轮 → 上游收到 chat 格式 tool_calls/tool
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl", "object": "chat.completion",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}]})))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "search", "input": {"q": "x"}}]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "结果"}]},
    ]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = upstream.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    let msgs = body["messages"].as_array().unwrap();
    // assistant 轮转成 chat tool_calls；user tool_result 转成 tool 消息
    assert!(msgs[1]["tool_calls"].is_array() || msgs[1]["content"].is_array(), "tool_use 应转 chat");
    assert!(msgs[2]["tool_calls"].is_null() || msgs[2].get("tool_call_id").is_some() || msgs[2]["content"].is_array(), "tool_result 应转 chat");
}

#[tokio::test]
async fn multi_turn_each_round_logged() {
    // 两次请求（两轮）→ request_logs 记 2 条，真实模型名 up-model
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl", "object": "chat.completion",
            "choices": [{"message": {"role": "assistant", "content": "回复"}}]})))
        .expect(2).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    for _ in 0..2 {
        let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    // 查 request_logs：2 条，模型名 up-model（真实）
    let con = rusqlite::Connection::open(dir.path().join("config").join("gateway-logs.db")).unwrap();
    let mut stmt = con.prepare("SELECT model, status FROM request_logs").unwrap();
    let rows: Vec<(String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(rows.len(), 2, "两轮应记 2 条日志");
    assert!(rows.iter().all(|(m, _)| m == "up-model"), "日志记真实模型名");
    assert!(rows.iter().all(|(_, s)| *s == 200));
}

#[tokio::test]
async fn multi_turn_stream_history() {
    // 流式多轮：user/assistant/user → 上游收到完整 3 条
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"回复\"},\"index\":0}],\"model\":\"up-model\"}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"index\":0}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
                "data: [DONE]\n\n")))
        .expect(1).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "stream": true, "messages": [
        {"role": "user", "content": "第一轮"},
        {"role": "assistant", "content": "第一轮回复"},
        {"role": "user", "content": "第二轮"},
    ]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let reqs = upstream.received_requests().await.unwrap();
    let mbody: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(mbody["messages"].as_array().unwrap().len(), 3, "流式多轮历史完整");
    assert_eq!(mbody["messages"][1]["content"], "第一轮回复", "assistant 轮保留");
}

#[tokio::test]
async fn non_streaming_cache_tokens_logged() {
    // cache token（学 LiteLLM 字段）：非流式响应带 prompt_tokens_details.cached_tokens → 入库
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "x", "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            "usage": {
                "prompt_tokens": 100, "completion_tokens": 50,
                "prompt_tokens_details": {"cached_tokens": 80, "cache_creation_tokens": 10}
            }
        })))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let logs = gateway_logs::query_recent(&dir.path(), 5).unwrap();
    let data = logs.get("data").and_then(|v| v.as_array()).unwrap();
    let entry = &data[0];
    assert_eq!(entry.get("input_tokens").and_then(|v| v.as_i64()), Some(100));
    assert_eq!(entry.get("output_tokens").and_then(|v| v.as_i64()), Some(50));
    assert_eq!(entry.get("cache_read_tokens").and_then(|v| v.as_i64()), Some(80), "cache_read 入库");
    assert_eq!(entry.get("cache_creation_tokens").and_then(|v| v.as_i64()), Some(10), "cache_creation 入库");
}

#[tokio::test]
async fn response_cache_hits_second_request() {
    // 响应缓存（学 LiteLLM caching）：相同请求第 2 次命中缓存，上游只被调 1 次
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cached", "choices": [{"message": {"role": "assistant", "content": "hello"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 1}
        })))
        .expect(1) // 只被调 1 次（第 2 次命中缓存）
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    let cfg = json!({
        "listen": 0,
        "response_cache": {"enabled": true, "ttl_seconds": 300, "max_entries": 100},
        "providers": [{
            "name": "p", "base_url": format!("{}/v1", upstream.uri()),
            "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk-up"}],
            "channels": [{"id": "ch1", "name": "c", "key_name": "主",
                          "models": [{"name": "m", "alias": "my-model"}]}],
        }],
    });
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});

    // 第 1 次：未命中，转发上游
    let r1 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload.clone(), &state.cache).await;
    assert_eq!(r1.status(), StatusCode::OK);
    let b1 = http_body_util::BodyExt::collect(r1.into_body()).await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&b1).contains("hello"));

    // 第 2 次：命中缓存（上游只调 1 次），返回同样内容 + x-cache: HIT
    let r2 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers().get("x-cache").and_then(|v| v.to_str().ok()), Some("HIT"), "第 2 次应命中缓存");
    let b2 = http_body_util::BodyExt::collect(r2.into_body()).await.unwrap().to_bytes();
    assert_eq!(b1.to_vec(), b2.to_vec(), "缓存返回同样 body");
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1, "上游只调 1 次");
}

#[test]
fn parse_usage_gemini_format() {
    // Gemini usage 在顶层 usageMetadata，字段名 promptTokenCount/candidatesTokenCount
    let v = json!({
        "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
        "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 20, "totalTokenCount": 30, "cachedContentTokenCount": 5}
    });
    let (inp, out, cr, _cc) = gateway::parse_usage_json(&v);
    assert_eq!(inp, Some(10), "Gemini promptTokenCount → input");
    assert_eq!(out, Some(20), "Gemini candidatesTokenCount → output");
    assert_eq!(cr, Some(5), "Gemini cachedContentTokenCount → cache_read");
}

#[test]
fn parse_usage_openai_and_anthropic_still_work() {
    // 回归：OpenAI / Anthropic 格式没被 Gemini 改动弄坏
    let oai = json!({"usage": {"prompt_tokens": 7, "completion_tokens": 3,
        "prompt_tokens_details": {"cached_tokens": 2}}});
    let (i, o, cr, _) = gateway::parse_usage_json(&oai);
    assert_eq!((i, o, cr), (Some(7), Some(3), Some(2)), "OpenAI 格式");
    let ant = json!({"usage": {"input_tokens": 8, "output_tokens": 4,
        "cache_read_input_tokens": 1, "cache_creation_input_tokens": 2}});
    let (i, o, cr, cc) = gateway::parse_usage_json(&ant);
    assert_eq!((i, o, cr, cc), (Some(8), Some(4), Some(1), Some(2)), "Anthropic 格式");
}

#[tokio::test]
async fn e2e_gemini_usage_recorded() {
    // 造假 Gemini 上游（返回 usageMetadata），打请求核对 token 记进日志
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{"content": {"parts": [{"text": "ok"}]}}],
            "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 6, "totalTokenCount": 18}
        })))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    let cfg = json!({
        "listen": 0, "sticky": {"enabled": false},
        "retry": {"max_retries": 1, "retry_on_status": []},
        "providers": [{
            "name": "gem-co", "base_url": format!("{}/v1", upstream.uri()),
            "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk"}],
            "channels": [{"id": "ch1", "name": "gem通道", "key_name": "主",
                          "models": [{"name": "gemini-pro", "alias": "gemini-pro"}]}],
        }],
    });
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
    let state = S::new(dir.path().to_path_buf());

    let payload = json!({"model": "gemini-pro", "messages": [{"role": "user", "content": "hi"}]});
    let r = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(wait_summary(&dir.path(), Some(12), Some(6)).await, "Gemini 非流式 usage 应入库");
}

#[tokio::test]
async fn stream_mock_openai_usage_recorded() {
    let upstream = MockServer::start().await;
    let body = sse(&[
        json!({"choices":[{"delta":{"content":"he"}}]}).to_string(),
        json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":8}}).to_string(),
    ]);
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body)
            .insert_header("content-type", "text/event-stream"))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let state = stream_mock_state(dir.path(), &upstream).await;
    let payload = json!({"model":"my-model","stream":true,"messages":[{"role":"user","content":"hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status().as_u16(), 200);
    let _ = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap();
    assert!(wait_summary(&dir.path(), Some(20), Some(8)).await, "OpenAI 流式 usage 应入库");
}

#[tokio::test]
async fn stream_mock_anthropic_split_usage_recorded() {
    // Anthropic 流式 usage 分片：message_start(input) + message_delta(output)，跨 chunk 合并
    let upstream = MockServer::start().await;
    let body = sse(&[
        json!({"type":"message_start","usage":{"input_tokens":30}}).to_string(),
        json!({"type":"content_block_delta","delta":{"text":"he"}}).to_string(),
        json!({"type":"message_delta","usage":{"output_tokens":12}}).to_string(),
    ]);
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body)
            .insert_header("content-type", "text/event-stream"))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let state = stream_mock_state(dir.path(), &upstream).await;
    let payload = json!({"model":"my-model","stream":true,"messages":[{"role":"user","content":"hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status().as_u16(), 200);
    let _ = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap();
    assert!(wait_summary(&dir.path(), Some(30), Some(12)).await, "Anthropic 分片 usage 应入库");
}

#[tokio::test]
async fn stream_mock_gemini_usage_recorded() {
    // Gemini 流式：chunk 顶层 usageMetadata
    let upstream = MockServer::start().await;
    let body = sse(&[
        json!({"candidates":[{"content":{"parts":[{"text":"he"}]}}]}).to_string(),
        json!({"usageMetadata":{"promptTokenCount":25,"candidatesTokenCount":9}}).to_string(),
    ]);
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body)
            .insert_header("content-type", "text/event-stream"))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let state = stream_mock_state(dir.path(), &upstream).await;
    let payload = json!({"model":"my-model","stream":true,"messages":[{"role":"user","content":"hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status().as_u16(), 200);
    let _ = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap();
    assert!(wait_summary(&dir.path(), Some(25), Some(9)).await, "Gemini 流式 usage 应入库");
}

#[tokio::test]
async fn logs_usage_aligned_nonstream_three_protocols() {
    // 非流式三协议（chat/messages/responses）→ 日志 usage 对齐（input/output 记录，无 0 填充假值）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl", "model": "up-model",
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 4}})))
        .expect(3).mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));
    let router = build_router(S::new(dir.path().to_path_buf()));
    // chat
    let _ = router.clone().oneshot(Request::post("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]}).to_string())).unwrap()).await.unwrap();
    // messages
    let _ = router.clone().oneshot(Request::post("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(json!({"model": "up-model", "max_tokens": 50, "messages": [{"role": "user", "content": "hi"}]}).to_string())).unwrap()).await.unwrap();
    // responses
    let _ = router.clone().oneshot(Request::post("/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(json!({"model": "up-model", "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}]}).to_string())).unwrap()).await.unwrap();
    // 日志：3 条，usage 对齐（input=7, output=4）
    let con = rusqlite::Connection::open(dir.path().join("config").join("gateway-logs.db")).unwrap();
    let mut stmt = con.prepare("SELECT input_tokens, output_tokens FROM request_logs").unwrap();
    let rows: Vec<(Option<i64>, Option<i64>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(rows.len(), 3, "三协议各记 1 条日志");
    assert!(rows.iter().all(|(i, o)| *i == Some(7) && *o == Some(4)), "三协议日志 usage 对齐（input=7/output=4）: {rows:?}");
}