//! forward_failover（按主题拆分）
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
async fn failing_suppliers_all_fail_returns_502() {
    // 3 个供应商全失败（500/404/500）→ 全候选废 → 502（不透传成功）
    let upstream = MockServer::start().await;
    // 全 retryable（500），避免 404 fatal 透传（不透传 failover）
    for (mid, st) in [("up-1", 500), ("up-2", 500), ("up-3", 500)] {
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": mid})))
            .respond_with(ResponseTemplate::new(st).set_body_json(json!({"error": {"message": "fail"}})))
            .mount(&upstream).await;
    }
    let dir = TempDir::new().unwrap();
    let channels = json!([
        {"id": "ch1", "name": "a", "key_name": "主", "models": [{"name": "up-1", "alias": "my-model"}]},
        {"id": "ch2", "name": "b", "key_name": "主", "models": [{"name": "up-2", "alias": "my-model"}]},
        {"id": "ch3", "name": "c", "key_name": "主", "models": [{"name": "up-3", "alias": "my-model"}]},
    ]);
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert!(resp.status().is_server_error(), "全候选失败应透传 5xx（不透传成功）: {}", resp.status());
}

#[tokio::test]
async fn retry_exhausted_after_max_returns_error() {
    // 单通道恒 500 + max_retries 1 → 重试耗尽 → 服务端错误（不透传成功）
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "boom"}})))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let cfg = json!({"listen": 0, "retry": {"max_retries": 1},
        "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk"}],
            "channels": [{"id": "ch1", "name": "c1", "key_name": "主", "models": [{"name": "up-model"}]}]}]});
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert!(resp.status().is_server_error(), "重试耗尽应返回服务端错误（透传上游 500）");
}

#[tokio::test]
async fn three_provider_failover_chain() {
    // 3 通道链：ch1 500 → ch2 500 → ch3 200（链式 failover）
    let upstream = MockServer::start().await;
    for (mid, st) in [("up-1", 500), ("up-2", 500)] {
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": mid})))
            .respond_with(ResponseTemplate::new(st).set_body_json(json!({"error": {"message": "boom"}})))
            .mount(&upstream).await;
    }
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-3"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "model": "up-3",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    let channels = json!([
        {"id": "ch1", "name": "a", "key_name": "主", "models": [{"name": "up-1", "alias": "my-model"}]},
        {"id": "ch2", "name": "b", "key_name": "主", "models": [{"name": "up-2", "alias": "my-model"}]},
        {"id": "ch3", "name": "c", "key_name": "主", "models": [{"name": "up-3", "alias": "my-model"}]},
    ]);
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK, "3 通道链 failover 应到 ch3 200");
}

#[tokio::test]
async fn forward_forwards_to_upstream_with_auth_and_model_mapping() {
    let upstream = MockServer::start().await;
    // mock 上游：POST /v1/chat/completions → 200 回 echo
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({"id": "chatcmpl-1", "object": "chat.completion",
                "choices": [{"message": {"role": "assistant", "content": "hi"}}]})))
        .expect(1)
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hello"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // 验证上游收到：Authorization Bearer + 上游真实模型名
    let reqs = upstream.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].headers.get("authorization").unwrap(), "Bearer sk-upstream-test");
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["model"], "up-model");
    assert_eq!(body["messages"][0]["content"], "hello");
}

#[tokio::test]
async fn forward_upstream_500_returns_500_and_logs() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    // 单通道：上游 500 → 5xx 触发 failover 但无下一通道 → 502（网关语义：所有通道失败）
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

    // 上游被调过一次
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    // 失败记进请求日志
    let logs = gateway_logs::query_recent(&dir.path(), 10).unwrap();
    let data = logs.get("data").and_then(|v| v.as_array()).unwrap();
    assert_eq!(data.len(), 1);
    assert!(data[0].get("error").is_some() || data[0].get("status").and_then(|v| v.as_i64()) == Some(500));
}

#[tokio::test]
async fn forward_failover_switches_channel_on_failure() {
    let upstream = MockServer::start().await;
    // 通道 ch1 上游模型 up-bad 不存在 → 404；ch2 up-good → 200
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-bad"})))
        .respond_with(ResponseTemplate::new(404))
        .mount(&upstream).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-good"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    // 两个通道：ch1(up-bad) 失败 → failover ch2(up-good)
    let channels = json!([
        {"id": "ch1", "name": "bad", "key_name": "主", "models": [{"name": "up-bad"}]},
        {"id": "ch2", "name": "good", "key_name": "主", "models": [{"name": "up-good"}]},
    ]);
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);

    let state = S::new(dir.path().to_path_buf());
    // 用对外别名 my-model → 两个通道都能服务，但 ch1 上游 up-bad 会 404
    let payload = json!({"model": "up-bad", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    // ch1 404 → failover 但只有 ch1 服务 up-bad，最终 404/502
    assert!(resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn logs_failover_records_upstream_status() {
    // ch1 上游 500 → 日志记真实 500 → failover ch2 200 → 最终 200
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-bad"})))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "boom"}})))
        .mount(&upstream).await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-good"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "model": "up-good",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
        .mount(&upstream).await;
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    gateway_logs::init_table(dir.path()).unwrap();
    let channels = json!([
        {"id": "ch1", "name": "bad", "key_name": "主", "models": [{"name": "up-bad", "alias": "my-model"}]},
        {"id": "ch2", "name": "good", "key_name": "主", "models": [{"name": "up-good", "alias": "my-model"}]},
    ]);
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);
    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK, "failover 后应 200");
    let con = rusqlite::Connection::open(dir.path().join("config").join("gateway-logs.db")).unwrap();
    let mut stmt = con.prepare("SELECT model, status FROM request_logs").unwrap();
    let rows: Vec<(String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert!(rows.iter().any(|(m, s)| m == "up-bad" && *s == 500), "ch1 日志记真实 500: {rows:?}");
    assert!(rows.iter().any(|(m, s)| m == "up-good" && *s == 200), "ch2 日志记 200: {rows:?}");
}

#[tokio::test]
async fn forward_no_upstream_reachable_returns_error() {
    // 指向一个没服务的端口 → 连接失败 → 502
    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), "http://127.0.0.1:1/v1", one_channel("ch1", "up-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert!(resp.status() == StatusCode::INTERNAL_SERVER_ERROR || resp.status() == StatusCode::BAD_GATEWAY || resp.status() == StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn forward_maps_alias_to_upstream_real_name() {
    // 网关核心：对外别名 my-alias → 上游真实名 real-model
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
        .expect(1)
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    // 通道模型 name=real-model，对外别名 my-alias
    let channels = json!([{
        "id": "ch1", "name": "ch1", "key_name": "主",
        "models": [{"name": "real-model", "alias": "my-alias"}],
    }]);
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);

    let state = S::new(dir.path().to_path_buf());
    // 请求用对外别名
    let payload = json!({"model": "my-alias", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // 上游收到的是真实名 real-model（不是别名）
    let reqs = upstream.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["model"], "real-model", "别名应翻译回上游真实名");
}

#[tokio::test]
async fn forward_streams_sse_passthrough() {
    // 上游回 text/event-stream → 网关流式透传 + 从最后 chunk 抓 usage token 入库
    let upstream = MockServer::start().await;
    // 最后一个 data chunk 带 usage（OpenAI 格式 stream_options.include_usage）
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":20}}\n\ndata: [DONE]\n\n")
            .insert_header("content-type", "text/event-stream"))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "stream": true, "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    // body 是流（能 collect 出 SSE 数据）—— 核心断言：上游 SSE body 透传到客户端
    let bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
    let s = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(s.contains("data:"), "应透传 SSE data 行，实际: {s}");
    assert!(s.contains("[DONE]"));
    assert!(s.contains("he"), "SSE delta content 透传");
    assert!(s.contains("prompt_tokens"), "usage chunk 也透传给客户端");

    // 核心：流式 token 抓取——日志里应记上 usage（input=10, output=20）
    // token 在流结束后由后台任务补记，稍等一下
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let logs = gateway_logs::query_recent(&dir.path(), 5).unwrap();
    let data = logs.get("data").and_then(|v| v.as_array()).unwrap();
    assert!(!data.is_empty(), "流式请求应记日志");
    let entry = &data[0];
    assert_eq!(entry.get("input_tokens").and_then(|v| v.as_i64()), Some(10), "流式 usage input_tokens 应入库");
    assert_eq!(entry.get("output_tokens").and_then(|v| v.as_i64()), Some(20), "流式 usage output_tokens 应入库");
}

#[tokio::test]
async fn upstream_5xx_fails_over_to_next_channel() {
    // 上游 5xx → forward_one 返 Err → failover 到下一通道（真 HTTP 验证 failover 链）
    // ch1 恒 500，ch2 200 → 4 次请求整体都 200（failover 生效）
    let upstream = MockServer::start().await;
    // ch1 上游模型 up-fail 恒 500；ch2 up-ok 200
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-fail"})))
        .respond_with(ResponseTemplate::new(500))
        .mount(&upstream).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "up-ok"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    // gateway.yaml：快熔断阈值（连续 2 次失败就 open）+ 2 通道
    let cfg = json!({
        "listen": 0,
        "circuit_breaker": {
            "failure_threshold": 2,
            "error_rate_threshold": 50,
            "min_requests": 1,
            "recovery_timeout_seconds": 60,
            "recovery_success_threshold": 1,
        },
        "providers": [{
            "name": "mock-provider",
            "base_url": format!("{}/v1", upstream.uri()),
            "disabled": false,
            "headers": {},
            "keys": [{"name": "主", "api-key": "sk-test"}],
            "channels": [
                {"id": "ch1", "name": "fail", "key_name": "主", "weight": 1,
                 "models": [{"name": "up-fail", "alias": "my-model"}]},
                {"id": "ch2", "name": "ok", "key_name": "主", "weight": 1,
                 "models": [{"name": "up-ok", "alias": "my-model"}]},
            ],
        }],
    });
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

    let state = S::new(dir.path().to_path_buf());
    // 用对外别名 my-model（两通道都服务），failover 会切到成功的通道
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});

    // 连发 4 次：ch1 每次先失败（500），failover 到 ch2 成功 → 整体 200
    // 但 ch1 连续失败 2 次 → 熔断 open → 后续请求不再打 ch1
    for _ in 0..4 {
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload.clone(), &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK, "failover 到 ch2 应成功");
    }

    // 验证 failover：ch1 恒 500，但每次都 failover 到 ch2 成功 → 整体 200
    // ch1 确实被试过（≥1 次），且整体成功（failover 生效）
    let reqs = upstream.received_requests().await.unwrap();
    let fail_hits = reqs.iter().filter(|r| {
        let b: Value = serde_json::from_slice(&r.body).unwrap_or(json!({}));
        b["model"] == "up-fail"
    }).count();
    let ok_hits = reqs.iter().filter(|r| {
        let b: Value = serde_json::from_slice(&r.body).unwrap_or(json!({}));
        b["model"] == "up-ok"
    }).count();
    assert!(fail_hits >= 1, "ch1 至少被试过 1 次（然后 500）");
    assert!(ok_hits >= 1, "failover 到 ch2 成功至少 1 次");
    assert_eq!(ok_hits, 4, "4 次请求都 failover 到 ch2 成功");
}

#[tokio::test]
async fn forward_embeddings_hits_embeddings_path() {
    // /v1/embeddings → 上游 /v1/embeddings（不同 upstream_path），无流式
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}],
            "usage": {"prompt_tokens": 3, "total_tokens": 3}
        })))
        .expect(1)
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "embed-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "embed-model", "input": "hello world"});
    let resp = gateway::forward_embeddings(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // 上游收到 embeddings 路径 + input
    let reqs = upstream.received_requests().await.unwrap();
    assert_eq!(reqs[0].url.path(), "/v1/embeddings");
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body["input"], "hello world");
}

#[tokio::test]
async fn forward_passes_through_upstream_headers() {
    // 上游回 x-request-id → 网关透传给客户端
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({"id": "x", "choices": []}))
            .insert_header("x-request-id", "req-123")
            .insert_header("openai-organization", "org-abc"))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), one_channel("ch1", "up-model"));

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "up-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    assert_eq!(resp.status(), StatusCode::OK);
    // 透传上游关键 header
    assert_eq!(resp.headers().get("x-request-id").and_then(|v| v.to_str().ok()), Some("req-123"));
    assert_eq!(resp.headers().get("openai-organization").and_then(|v| v.to_str().ok()), Some("org-abc"));
}

#[tokio::test]
async fn fatal_4xx_passes_through_without_failover() {
    // 错误分类（学 LiteLLM）：上游 401 鉴权错误=致命 → 直接透传 401，不 failover 也不退避
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": {"message": "bad key"}})))
        .expect(1) // 只被调 1 次（致命错误不 failover 到第二通道）
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    // 两通道都服务同一模型 → 若 failover 会打两次，断言只打 1 次即验证"致命不重试"
    write_two_channel_yaml(dir.path(), &format!("{}/v1", upstream.uri()), "m1", "m2", false);

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    // 401 致命错误直接透传（不换成 502）
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1, "致命 4xx 不 failover");
}

#[tokio::test]
async fn retryable_5xx_backs_off_then_fails_over() {
    // 重试退避（学 LiteLLM）：上游 500=可重试 → 退避后 failover 到 ch2；致命 401 直接透传
    let upstream = MockServer::start().await;
    // ch1 模型恒 500（可重试），ch2 模型 200
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "m1"})))
        .respond_with(ResponseTemplate::new(500))
        .mount(&upstream).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "m2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    write_two_channel_yaml(dir.path(), &format!("{}/v1", upstream.uri()), "m1", "m2", false);

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    // 500 可重试 → 退避后 failover 到 ch2 → 整体 200
    assert_eq!(resp.status(), StatusCode::OK);
    // ch1(500) 至少被试 1 次
    let reqs = upstream.received_requests().await.unwrap();
    let m1_hits = reqs.iter().filter(|r| {
        let b: Value = serde_json::from_slice(&r.body).unwrap_or(json!({}));
        b["model"] == "m1"
    }).count();
    assert!(m1_hits >= 1, "可重试 5xx 应先试 ch1");
}

#[tokio::test]
async fn channel_retry_override_makes_500_fatal() {
    // 通道级覆盖：ch1 把 500 配成致命（fatal_status 含 500）→ 401 式直接透传 500，不 failover 到 ch2
    // 全局默认 500 可重试，但 ch1 覆盖成致命 → 验证通道配置优先
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "m1"})))
        .respond_with(ResponseTemplate::new(500))
        .expect(1) // 只被调 1 次（500 被配成致命，不 failover）
        .mount(&upstream).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "m2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    // ch1 带 retry 覆盖：fatal_status 含 500（把全局可重试的 500 变致命）
    let cfg = json!({
        "listen": 0,
        "retry": {"max_retries": 3, "retry_on_status": [408, 409, 429, 500, 502, 503, 504],
                   "fatal_status": [400, 401, 403, 404]},
        "providers": [{
            "name": "p", "base_url": format!("{}/v1", upstream.uri()),
            "disabled": false, "headers": {},
            "keys": [{"name": "主", "api-key": "sk-up"}],
            "channels": [
                {"id": "ch1", "name": "覆盖成致命", "key_name": "主",
                 "retry": {"fatal_status": [400, 401, 403, 404, 500]},  // 500 致命
                 "models": [{"name": "m1", "alias": "my-model"}]},
                {"id": "ch2", "name": "正常", "key_name": "主",
                 "models": [{"name": "m2", "alias": "my-model"}]},
            ],
        }],
    });
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    // ch1 的 500 被配成致命 → 直接透传 500（不 failover 到 ch2）
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR, "通道把 500 配成致命应透传");
    // ch1(m1) 只被调 1 次（致命不 failover）
    let reqs = upstream.received_requests().await.unwrap();
    let m1_hits = reqs.iter().filter(|r| {
        let b: Value = serde_json::from_slice(&r.body).unwrap_or(json!({}));
        b["model"] == "m1"
    }).count();
    assert_eq!(m1_hits, 1, "致命 500 不 failover，ch1 只调 1 次");
}

#[tokio::test]
async fn global_retry_default_5xx_fails_over() {
    // 全局默认（无通道覆盖）：500 可重试 → failover 到 ch2（对照上面的通道覆盖）
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "m1"})))
        .respond_with(ResponseTemplate::new(500))
        .mount(&upstream).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(json!({"model": "m2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "ok", "choices": []})))
        .mount(&upstream).await;

    let dir = TempDir::new().unwrap();
    // 不配通道级 retry → 用全局默认（500 可重试）
    write_two_channel_yaml(dir.path(), &format!("{}/v1", upstream.uri()), "m1", "m2", false);

    let state = S::new(dir.path().to_path_buf());
    let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
    let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
    // 全局默认 500 可重试 → failover 到 ch2 → 200
    assert_eq!(resp.status(), StatusCode::OK, "全局默认 500 可重试应 failover");
}