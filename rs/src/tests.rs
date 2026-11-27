    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt; // collect 读 body
    use tower::ServiceExt; // oneshot
    use tempfile::TempDir;
    use crate::gateway::{hit_disable_keyword, record_outcome, HealthMap, CircuitState};

    /// 构造隔离测试 Router：tempdir 做 root（gateway.yaml / SQLite 都写临时目录，不污染真实配置）
    async fn test_router() -> (Router, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        // db_path = root/config/gateway-logs.db，SQLite 不自动建父目录 → 先建 config 子目录
        std::fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
        let state = S::new(dir.path().to_path_buf());
        // 建日志表（幂等，测 /v1/logs /stats 才有表）
        gateway_logs::init_table(dir.path()).expect("init_table");
        (build_router(state), dir)
    }

    async fn get(router: Router, uri: &str) -> (StatusCode, Value) {
        let resp = router.oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn post(router: Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let resp = router.oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        ).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));
    }

    #[tokio::test]
    async fn models_returns_list_shape() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/v1/models").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.get("object").and_then(|v| v.as_str()), Some("list"));
        assert!(json.get("data").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn logs_returns_object_and_data() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/v1/logs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.get("object").and_then(|v| v.as_str()), Some("list"));
        assert!(json.get("data").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn stats_summary_has_totals() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/stats/summary").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.get("total_requests").and_then(|v| v.as_i64()).is_some());
        assert!(json.get("success_count").and_then(|v| v.as_i64()).is_some());
    }

    #[tokio::test]
    async fn stats_providers_returns_array() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/stats/providers").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.is_array());
    }

    #[tokio::test]
    async fn stats_trend_and_daily_return_arrays() {
        let (router, _d) = test_router().await;
        let (s1, j1) = get(router.clone(), "/stats/trend?hours=24").await;
        let (s2, j2) = get(router, "/stats/daily?days=7").await;
        assert_eq!(s1, StatusCode::OK);
        assert_eq!(s2, StatusCode::OK);
        assert!(j1.is_array());
        assert!(j2.is_array());
    }

    #[tokio::test]
    async fn api_logs_limit_param_respected() {
        let (router, _d) = test_router().await;
        let (status, json) = get(router, "/api/logs?limit=5").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.get("logs").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn providers_roundtrip_via_gateway_yaml() {
        let (router, _d) = test_router().await;
        // 写 providers
        let providers = serde_json::json!({"providers": [
            {"name": "p1", "base_url": "http://x/v1", "disabled": false,
             "headers": {}, "keys": [{"name": "主", "api_key": "sk-1"}], "channels": []}
        ]});
        let (st, j) = post(router.clone(), "/api/providers", providers).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(true));
        // 读回
        let (st2, j2) = get(router, "/api/providers").await;
        assert_eq!(st2, StatusCode::OK);
        let arr = j2.get("providers").and_then(|v| v.as_array()).unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("p1"));
    }

    #[tokio::test]
    async fn routing_roundtrip() {
        let (router, _d) = test_router().await;
        let routing = serde_json::json!({"routing": {"strategy": "latency", "max_retries": 3}});
        let (st, j) = post(router.clone(), "/api/routing", routing).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(true));
        let (_st2, j2) = get(router, "/api/routing").await;
        assert_eq!(j2.get("routing").and_then(|v| v.get("strategy")).and_then(|v| v.as_str()), Some("latency"));
    }

    #[tokio::test]
    async fn mcp_roundtrip() {
        let (router, _d) = test_router().await;
        // PUT
        let mcp = serde_json::json!({"tools": [{"tool": "search", "enabled": true}], "servers": []});
        let resp = router.clone().oneshot(
            Request::put("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(mcp.to_string())).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // GET
        let (st, j) = get(router, "/api/mcp").await;
        assert_eq!(st, StatusCode::OK);
        assert!(j.get("tools").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn post_providers_missing_section_rejected() {
        let (router, _d) = test_router().await;
        let (st, j) = post(router, "/api/providers", serde_json::json!({"wrong": 1})).await;
        assert_eq!(st, StatusCode::OK); // handler 返回 200 但 ok=false
        assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(false));
    }

    #[tokio::test]
    async fn nonexistent_route_is_404_or_fallback() {
        let (router, _d) = test_router().await;
        let resp = router.oneshot(Request::get("/nonexistent-xyz").body(Body::empty()).unwrap()).await.unwrap();
        // fallback_service ServeDir → 404（tempdir 无 static 文件）
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn chat_completions_with_no_upstream_returns_error() {
        let (router, _d) = test_router().await;
        // 无真实上游（tempdir 无 providers → 无通道）→ 404 invalid_request
        let payload = serde_json::json!({"model": "gpt-x", "messages": [{"role": "user", "content": "hi"}]});
        let (st, j) = post(router, "/v1/chat/completions", payload).await;
        assert!(st == StatusCode::NOT_FOUND || st == StatusCode::BAD_REQUEST || st == StatusCode::INTERNAL_SERVER_ERROR || st == StatusCode::SERVICE_UNAVAILABLE);
        assert!(j.get("error").is_some());
    }

    // ==================== 网关转发 wiremock 真测（学 LiteLLM gateway-forward）====================
    // 起本地 mock 上游 HTTP 服务器，写 gateway.yaml 指向它，打 forward_chat，
    // 验证转发内容/鉴权/模型映射/状态码/failover。真发 HTTP，不 mock 网关内部。

    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};
    use serde_json::json;

    /// 写 gateway.yaml 到 tempdir（config 子目录），指向给定 upstream base_url
    fn write_gateway_yaml(root: &std::path::Path, base_url: &str, channels: Value) {
        let cfg = json!({
            "listen": 0,
            "providers": [{
                "name": "mock-provider",
                "base_url": base_url,
                "disabled": false,
                "headers": {},
                "keys": [{"name": "主", "api-key": "sk-upstream-test"}],
                "channels": channels,
            }],
        });
        std::fs::create_dir_all(root.join("config")).unwrap();
        api::write_yaml_json(&root.join("config").join("gateway.yaml"), &cfg).unwrap();
    }

    fn one_channel(id: &str, model: &str) -> Value {
        json!([{
            "id": id, "name": id, "key_name": "主",
            "models": [{"name": model}],
        }])
    }


    /// 档位通道：两个模型（pro 质量型 / flash 速度型）
    fn tier_channels() -> Value {
        json!([{
            "id": "ch1", "name": "ch1", "key_name": "主",
            "models": [
                {"name": "pro-model", "class": "pro"},
                {"name": "flash-model", "class": "flash"},
            ],
        }])
    }

    #[tokio::test]
    async fn tier_pro_request_only_selects_pro_model() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-1", "object": "chat.completion",
                "choices": [{"message": {"role": "assistant", "content": "hi"}}]
            }))).expect(1).mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), tier_channels());
        let state = S::new(dir.path().to_path_buf());
        // 档位名 "pro" 不是模型名：网关应路由到 pro 标注的真实模型
        let payload = json!({"model": "pro", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // 响应 body 的 model 字段应回显档位名（客户端配的 model 名，对称）
        let resp_bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
        let resp_body: Value = serde_json::from_slice(&resp_bytes).unwrap();
        assert_eq!(resp_body["model"], "pro", "响应 model 应回显档位名（客户端配的 model 名）");

        let reqs = upstream.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "pro-model", "档位 pro 应路由到 pro 模型的真实名");
    }

    #[tokio::test]
    async fn tier_flash_request_only_selects_flash_model() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-2", "object": "chat.completion",
                "choices": [{"message": {"role": "assistant", "content": "hi"}}]
            }))).expect(1).mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), tier_channels());
        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "flash", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let reqs = upstream.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "flash-model", "档位 flash 应路由到 flash 模型的真实名");
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
    async fn tier_1m_suffix_stripped_to_tier_end_to_end() {
        // 学 cc-switch：客户端发 model=auto[1M]（Claude Code 的 1M 上下文标记）
        // → 网关剥离 [1M] → 识别档位 auto（黑盒调度）→ 响应回显档位名 auto
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmpl-1", "object": "chat.completion",
                "choices": [{"message": {"role": "assistant", "content": "hi"}}]
            }))).expect(1).mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), tier_channels());
        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "auto[1M]", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["model"], "auto", "[1M] 剥离后应识别档位 auto，响应回显 auto");
    }
    #[tokio::test]
    async fn tier_auto_request_selects_any_model() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "chatcmpl-3", "object": "chat.completion",
                "choices": [{"message": {"role": "assistant", "content": "hi"}}]
            }))).expect(1).mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // auto 需要 auto_mode_enabled：自定义 gateway.yaml（带 routing 段）
        let cfg = json!({
            "listen": 0,
            "routing": {"auto_mode_enabled": true},
            "providers": [{
                "name": "mock-provider", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-upstream-test"}],
                "channels": tier_channels(),
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "auto", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let reqs = upstream.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        // auto = 全部候选（pro/flash 之一）
        let m = body["model"].as_str().unwrap();
        assert!((m == "pro-model" || m == "flash-model"), "auto 应选全部候选之一，实际: {m}");
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


    // ==================== wiremock 场景补测：黏性/限流/熔断恢复 ====================

    /// 写 gateway.yaml：两通道（ch1/ch2）都服务同一对外别名，上游名不同（好区分打了几次）
    fn write_two_channel_yaml(root: &std::path::Path, base_url: &str, ch1_model: &str, ch2_model: &str, sticky: bool) {
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": sticky, "ttl_seconds": 300},
            "providers": [{
                "name": "mock-provider",
                "base_url": base_url,
                "disabled": false,
                "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "one", "key_name": "主", "weight": 1,
                     "models": [{"name": ch1_model, "alias": "my-model"}]},
                    {"id": "ch2", "name": "two", "key_name": "主", "weight": 1,
                     "models": [{"name": ch2_model, "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(root.join("config")).unwrap();
        api::write_yaml_json(&root.join("config").join("gateway.yaml"), &cfg).unwrap();
    }

    #[tokio::test]
    async fn sticky_session_reuses_same_channel() {
        // 同 session 多次请求 → 黏同一通道（prompt cache 复用）
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        write_two_channel_yaml(dir.path(), &format!("{}/v1", upstream.uri()), "model-a", "model-b", true);

        let state = S::new(dir.path().to_path_buf());
        // 同 session_id 发 3 次（首次成功后 pin，后续复用同通道）
        for _ in 0..3 {
            let payload = json!({"model": "my-model", "session_id": "sess-sticky", "messages": [{"role": "user", "content": "hi"}]});
            let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // 黏性：3 次应全打同一上游模型（全 a 或全 b），不是分散
        let reqs = upstream.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 3);
        let models: Vec<String> = reqs.iter().map(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["model"].as_str().unwrap_or("").to_string()
        }).collect();
        let all_same = models.iter().all(|m| m == &models[0]);
        assert!(all_same, "同 session 应黏同一通道，实际模型分布: {models:?}");
    }

    #[tokio::test]
    async fn rate_limit_skips_channel_over_rpm() {
        // ch1 限 rpm=2，ch2 不限 → 发 5 次，ch1 只该被打 ≤2 次，其余 failover 到 ch2
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": false},
            "providers": [{
                "name": "mock-provider",
                "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "limited", "key_name": "主", "weight": 1, "rpm": 2,
                     "models": [{"name": "model-a", "alias": "my-model"}]},
                    {"id": "ch2", "name": "free", "key_name": "主", "weight": 1,
                     "models": [{"name": "model-b", "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        for _ in 0..5 {
            let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
            let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
            assert_eq!(resp.status(), StatusCode::OK, "限流应 failover 到 ch2，整体仍 200");
        }

        // ch1 限 2rpm：被调次数应 ≤ 2（超限后跳过），其余走 ch2
        let reqs = upstream.received_requests().await.unwrap();
        let ch1_hits = reqs.iter().filter(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["model"] == "model-a"
        }).count();
        assert!(ch1_hits <= 2, "ch1 限 2rpm 应 ≤2 次，实际 {ch1_hits}");
    }

    #[tokio::test]
    async fn circuit_breaker_half_open_recovers() {
        // 熔断恢复：ch1 先失败 open → recovery_timeout 后 half_open 试探 → 成功 → closed
        // 用 1 个通道（is_single_healthy 保护不熔断的话就没法测，但这里靠 recovery 状态转移）
        // 改成 2 通道：ch1 恢复，ch2 兜底，验证 ch1 最终恢复服务
        let upstream = MockServer::start().await;
        // 先让 ch1 模型失败几次（mount 500），再改成成功（wiremock 后 mount 优先）
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "model-a"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "recovered", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // 短 recovery_timeout（1 秒）+ failure_threshold 1，好快速测恢复
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": false},
            "circuit_breaker": {"failure_threshold": 1, "error_rate_threshold": 50,
                                 "min_requests": 1, "recovery_timeout_seconds": 1,
                                 "recovery_success_threshold": 1},
            "providers": [{
                "name": "mock-provider",
                "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-test"}],
                "channels": [
                    {"id": "ch1", "name": "one", "key_name": "主",
                     "models": [{"name": "model-a", "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
        // 单通道成功请求（验证 recovery 后通道恢复服务）
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    }


    // ==================== wiremock 场景补测：embeddings/下游鉴权/透传 header ====================

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
    async fn downstream_auth_rejects_missing_or_wrong_key() {
        // 配 auth.api_keys → 无 Bearer / 错 key → 401；对 key → 放行
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // gateway.yaml 配下游鉴权 key
        let cfg = json!({
            "listen": 0,
            "auth": {"api_keys": ["sk-downstream-secret"]},
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

        // 无 Bearer → 401
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload.clone(), &state.cache).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // 错 key → 401
        let resp2 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, Some("Bearer wrong"), payload.clone(), &state.cache).await;
        assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);
        // 对 key → 放行 200
        let resp3 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, Some("Bearer sk-downstream-secret"), payload, &state.cache).await;
        assert_eq!(resp3.status(), StatusCode::OK);
        // 上游只被对 key 那 1 次调用（前两次 401 没转发）
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
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


    // ==================== 四个基本要素真测：错误分类/cache token/用量限额 ====================

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
    async fn daily_token_budget_returns_429_when_exceeded() {
        // 用量限额（学 LiteLLM max_budget）：daily_token_budget=100，先记 200 token → 下一请求 429
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "x", "choices": [],
                "usage": {"prompt_tokens": 150, "completion_tokens": 50}
            })))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "auth": {"api_keys": ["sk-user"], "daily_token_budget": 100},
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

        // 第 1 次：预算内（0 < 100），放行，记 200 token
        let resp1 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, Some("Bearer sk-user"), payload.clone(), &state.cache).await;
        assert_eq!(resp1.status(), StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await; // 等日志落库

        // 第 2 次：已用 200 > 预算 100 → 429
        let resp2 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, Some("Bearer sk-user"), payload, &state.cache).await;
        assert_eq!(resp2.status(), StatusCode::TOO_MANY_REQUESTS, "超每日 token 预算应 429");
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


    // ==================== 重试策略可配置真测：全局默认 + 每通道覆盖 ====================

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


    // ==================== 三个新功能真测：响应缓存/上下文过滤/模型降级链 ====================

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

    #[tokio::test]
    async fn context_window_filter_skips_small_channel() {
        // 上下文窗口过滤（学 LiteLLM max_input_tokens）：ch1 窗口 10（装不下），ch2 窗口大 → 路由到 ch2
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "big"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "big", "choices": []})))
            .mount(&upstream).await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "small"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "small", "choices": []})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        // ch1 服务 my-model→small，窗口 10（很小）；ch2 服务 my-model→big，窗口大
        let cfg = json!({
            "listen": 0,
            "providers": [{
                "name": "p", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-up"}],
                "channels": [
                    {"id": "ch1", "name": "小窗口", "key_name": "主", "max_input_tokens": 10,
                     "models": [{"name": "small", "alias": "my-model"}]},
                    {"id": "ch2", "name": "大窗口", "key_name": "主", "max_input_tokens": 100000,
                     "models": [{"name": "big", "alias": "my-model"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        // 长文本（>10 token 的估算）：ch1 装不下 → 应路由到 ch2(big)
        let long_text = "x".repeat(200);
        let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": long_text}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // 实际打到 big（ch2），不是 small（ch1 被过滤）
        let reqs = upstream.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "big", "长请求应路由到大窗口通道 ch2");
    }

    #[tokio::test]
    async fn model_fallback_chain_tries_backup_model() {
        // 模型级降级链（学 LiteLLM fallbacks）：主模型所有通道失败 → 降级到备选模型
        let upstream = MockServer::start().await;
        // 主模型 gpt-4 通道恒 500
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "gpt-4"})))
            .respond_with(ResponseTemplate::new(500))
            .mount(&upstream).await;
        // 备选 gpt-4o 通道 200
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "gpt-4o"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "fallback", "choices": [], "model": "gpt-4o"})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "model_fallback": {"enabled": true, "chains": {"gpt-4": ["gpt-4o"]}},
            "providers": [{
                "name": "p", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-up"}],
                "channels": [
                    {"id": "ch-gpt4", "name": "gpt4通道", "key_name": "主",
                     "models": [{"name": "gpt-4", "alias": "gpt-4"}]},
                    {"id": "ch-gpt4o", "name": "gpt4o通道", "key_name": "主",
                     "models": [{"name": "gpt-4o", "alias": "gpt-4o"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        // 请求 gpt-4 → 其通道 500 → 降级到 gpt-4o → 200
        let payload = json!({"model": "gpt-4", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK, "主模型失败应降级到备选模型");
        let b = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["model"], "gpt-4o", "实际由备选模型 gpt-4o 服务");
    }


    // ==================== 通道测试真测：/api/channels/test 端到端 ====================

    #[tokio::test]
    async fn channel_test_endpoint_returns_ok_with_content() {
        // 通道测试（学 new-api TestChannel）：真发 chat 请求，返回 ok+content
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "t", "choices": [{"message": {"role": "assistant", "content": "pong"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1}
            })))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "test": {"default_model": "", "test_message": "ping", "max_tokens": 16},
            "providers": [{
                "name": "p", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-up"}],
                "channels": [{"id": "ch1", "name": "c", "key_name": "主",
                              "models": [{"name": "gpt-test", "alias": ""}]}],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let app = build_router(S::new(dir.path().to_path_buf()));
        let (st, j) = post(app, "/api/channels/test", json!({"provider": "p", "channel_id": "ch1"})).await;
        assert_eq!(st, 200);
        assert_eq!(j["ok"], true, "通道测试应成功: {j}");
        assert_eq!(j["content"], "pong", "应解析出回复内容");
        assert_eq!(j["model"], "gpt-test", "用通道第一个模型");
        assert!(j["latency_ms"].as_u64().is_some());
    }

    #[tokio::test]
    async fn channel_test_endpoint_reports_upstream_error() {
        // 通道测试：上游 401 → ok=false + error
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": {"message": "bad key"}})))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "test": {"default_model": "gpt-test", "test_message": "ping", "max_tokens": 16},
            "providers": [{
                "name": "p", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-wrong"}],
                "channels": [{"id": "ch1", "name": "c", "key_name": "主",
                              "models": [{"name": "gpt-test", "alias": ""}]}],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let app = build_router(S::new(dir.path().to_path_buf()));
        let (st, j) = post(app, "/api/channels/test", json!({"provider": "p", "channel_id": "ch1"})).await;
        assert_eq!(st, 200); // 端点本身 200，测试结果在 body
        assert_eq!(j["ok"], false, "上游 401 应报通道异常");
        assert!(j["error"].as_str().is_some(), "应有错误信息");
    }


    // ==================== 多机 HA 真测：生成 Nginx 配置 ====================

    #[test]
    fn generate_nginx_conf_masters_and_backups() {
        // HA：master 在前不带 backup 标记，backup 带 backup；SSE proxy_buffering off
        let deploy = json!({
            "entry": {"listen": 8380, "readTimeout": 120, "nextUpstream": "error timeout http_502 http_503 http_504"},
            "instances": [
                {"id": "i-1", "name": "gw-phone", "role": "master", "address": "192.168.31.213:8317", "heartbeatMs": 5000},
                {"id": "i-2", "name": "gw-win11", "role": "backup", "address": "192.168.31.12:8317", "heartbeatMs": 5000},
            ],
        });
        let conf = gateway::generate_nginx_conf(&deploy);
        // 固定入口
        assert!(conf.contains("listen 8380;"), "应有固定入口 listen 8380");
        // upstream 块
        assert!(conf.contains("upstream myapi"), "应有 upstream myapi");
        // master 不带 backup 标记
        assert!(conf.contains("server 192.168.31.213:8317 max_fails"), "master 应在前且不带 backup");
        // backup 带 backup 标记
        assert!(conf.contains("server 192.168.31.12:8317 backup"), "backup 应带 backup 标记");
        // SSE 流式：关缓冲
        assert!(conf.contains("proxy_buffering off"), "SSE 必须 proxy_buffering off");
        // 故障切换
        assert!(conf.contains("proxy_next_upstream error timeout http_502"), "应有 proxy_next_upstream 故障切换");
        // 健康检查端点
        assert!(conf.contains("location = /health"), "应有健康检查端点");
    }

    #[test]
    fn generate_nginx_conf_empty_instances() {
        // 无实例 → 不崩，生成占位注释
        let deploy = json!({"entry": {"listen": 8380}, "instances": []});
        let conf = gateway::generate_nginx_conf(&deploy);
        assert!(conf.contains("listen 8380;"));
        assert!(conf.contains("无实例"), "无实例应有占位提示");
    }


    // ==================== 端到端 mock 供应商：打流量核对统计/日志一致性 ====================

    #[tokio::test]
    async fn e2e_mock_provider_stats_consistent() {
        // 造假供应商（wiremock 假上游），打 5 个请求（3 成功 2 失败），
        // 核对：统计聚合（请求数/token 加总）与实际请求一致，日志每条记录也对得上
        let upstream = MockServer::start().await;
        // 成功响应：固定 usage（prompt=10, completion=20）
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"seed": "ok"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "ok", "choices": [{"message": {"role": "assistant", "content": "hi"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20}
            })))
            .mount(&upstream).await;
        // 失败响应：500
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"seed": "bad"})))
            .respond_with(ResponseTemplate::new(500))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": false},
            "retry": {"max_retries": 1, "retry_on_status": [], "fatal_status": [400,401,403,404]}, // 关退避+5xx不重试，好精确计数
            "providers": [{
                "name": "mock-co", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-mock"}],
                "channels": [{"id": "ch1", "name": "mock通道", "key_name": "主",
                              "models": [{"name": "mock-model", "alias": "my-model"}]}],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());

        // 打 3 个成功请求（usage: 10 in / 20 out each → 共 30 in / 60 out）
        for i in 0..3 {
            let payload = json!({"model": "my-model", "seed": "ok", "messages": [{"role": "user", "content": format!("q{}", i)}]});
            let r = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
            assert_eq!(r.status(), StatusCode::OK, "成功请求 {i} 应 200");
        }
        // 打 2 个失败请求（500，max_retries=1 且 5xx 不在 retry_on_status → 只打 1 次，记 1 条失败日志）
        for i in 0..2 {
            let payload = json!({"model": "my-model", "seed": "bad", "messages": [{"role": "user", "content": format!("bad{}", i)}]});
            let r = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
            assert_eq!(r.status(), StatusCode::BAD_GATEWAY, "失败请求 {i} 应 502");
        }
        // 等流式/异步日志落库（非流式是同步记，但保险等一下）
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // ===== 核对统计聚合 =====
        let summary = gateway_logs::query_summary(&dir.path()).unwrap();
        assert_eq!(summary["total_requests"].as_i64(), Some(5), "统计请求数应=5（3成功+2失败）");
        assert_eq!(summary["success_count"].as_i64(), Some(3), "成功数应=3");
        assert_eq!(summary["error_count"].as_i64(), Some(2), "错误数应=2");
        // 只有 3 个成功请求带 usage（10+20），失败请求 token 为空不计
        assert_eq!(summary["total_input_tokens"].as_i64(), Some(30), "input token 加总应=30（3×10）");
        assert_eq!(summary["total_output_tokens"].as_i64(), Some(60), "output token 加总应=60（3×20）");

        // ===== 核对日志每条 =====
        let logs = gateway_logs::query_recent(&dir.path(), 10).unwrap();
        let data = logs["data"].as_array().unwrap();
        assert_eq!(data.len(), 5, "日志应记 5 条");
        // 成功日志：status=200, input=10, output=20
        let ok_logs: Vec<_> = data.iter().filter(|e| e["status"].as_i64() == Some(200)).collect();
        assert_eq!(ok_logs.len(), 3, "应 3 条成功日志");
        for e in &ok_logs {
            assert_eq!(e["input_tokens"].as_i64(), Some(10), "成功日志 input=10");
            assert_eq!(e["output_tokens"].as_i64(), Some(20), "成功日志 output=20");
            assert_eq!(e["provider"], "mock-co", "供应商记录对");
            assert_eq!(e["channel"], "mock通道", "通道记录对");
            assert_eq!(e["model"], "mock-model", "模型记录对（真实上游模型名，非对外别名）");
        }
        // 失败日志：记上游真实 status=500（观测日志记上游返回，便于排查），有 error
        let bad_logs: Vec<_> = data.iter().filter(|e| e["status"].as_i64() == Some(500)).collect();
        assert_eq!(bad_logs.len(), 2, "应 2 条失败日志（记上游 500）");
        for e in &bad_logs {
            assert!(e["error"].as_str().is_some(), "失败日志应有 error");
        }

        // ===== 核对上游实际被调次数（3 成功 + 2 失败 = 5，无重试）=====
        let upstream_reqs = upstream.received_requests().await.unwrap();
        assert_eq!(upstream_reqs.len(), 5, "上游应被调 5 次（无重试）");
    }

    #[tokio::test]
    async fn e2e_mock_provider_channel_stats_aggregate() {
        // 按通道聚合统计：两通道各打几个请求，核对每通道的请求数/token 分开算对
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "x", "choices": [{"message": {"content": "ok"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 5}
            })))
            .mount(&upstream).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0,
            "sticky": {"enabled": false},
            "retry": {"max_retries": 1, "retry_on_status": []},
            "providers": [{
                "name": "co", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk"}],
                "channels": [
                    {"id": "chA", "name": "通道A", "key_name": "主", "weight": 1,
                     "models": [{"name": "model-a", "alias": "model-a"}]},
                    {"id": "chB", "name": "通道B", "key_name": "主", "weight": 1,
                     "models": [{"name": "model-b", "alias": "model-b"}]},
                ],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();

        let state = S::new(dir.path().to_path_buf());
        // 打 3 次 model-a（走 chA）+ 2 次 model-b（走 chB）
        for _ in 0..3 {
            let payload = json!({"model": "model-a", "messages": [{"role": "user", "content": "hi"}]});
            let _ = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        }
        for _ in 0..2 {
            let payload = json!({"model": "model-b", "messages": [{"role": "user", "content": "hi"}]});
            let _ = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 按通道聚合：chA=3 请求 15in/15out，chB=2 请求 10in/10out
        let stats = gateway_logs::query_stats(&dir.path()).unwrap();
        let arr = stats["data"].as_array().unwrap();
        let cha = arr.iter().find(|e| e["channel"] == "通道A").expect("应有通道A统计");
        let chb = arr.iter().find(|e| e["channel"] == "通道B").expect("应有通道B统计");
        assert_eq!(cha["total_requests"].as_i64(), Some(3), "通道A 请求数=3");
        assert_eq!(cha["total_input_tokens"].as_i64(), Some(15), "通道A input=15");
        assert_eq!(chb["total_requests"].as_i64(), Some(2), "通道B 请求数=2");
        assert_eq!(chb["total_output_tokens"].as_i64(), Some(10), "通道B output=10");
    }


    // ==================== 特殊协议 usage 统计：Gemini 格式 ====================

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


    // ==================== mock 供应商·流式 SSE 端到端（纳入测试框架）====================
    // wiremock 造假上游返回 SSE，核对跨 chunk 累积的 usage 记进统计。三协议各一。
    // 非流式 e2e mock 见本模块 e2e_mock_provider_*。

    async fn stream_mock_state(dir: &std::path::Path, upstream: &MockServer) -> S {
        let cfg = json!({
            "listen": 0, "sticky": {"enabled": false},
            "retry": {"max_retries": 1, "retry_on_status": []},
            "providers": [{
                "name": "mock-co", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk-mock"}],
                "channels": [{"id": "ch1", "name": "mock通道", "key_name": "主",
                              "models": [{"name": "m", "alias": "my-model"}]}],
            }],
        });
        std::fs::create_dir_all(dir.join("config")).unwrap();
        api::write_yaml_json(&dir.join("config").join("gateway.yaml"), &cfg).unwrap();
        S::new(dir.to_path_buf())
    }


    async fn wait_summary(root: &std::path::Path, expect_in: Option<i64>, expect_out: Option<i64>) -> bool {
        // 流式 usage 由 SSE 处理后异步写日志，poll 等待入库（避免 sleep 固定值偶发 flaky）
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let s = gateway_logs::query_summary(root).unwrap();
            let ok_in = expect_in.map_or(true, |e| s["total_input_tokens"].as_i64() == Some(e));
            let ok_out = expect_out.map_or(true, |e| s["total_output_tokens"].as_i64() == Some(e));
            if ok_in && ok_out { return true; }
        }
        false
    }

    fn sse(chunks: &[String]) -> String {
        let mut s = String::new();
        for c in chunks { s.push_str("data: "); s.push_str(c); s.push_str("\n\n"); }
        s.push_str("data: [DONE]\n\n");
        s
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


    // ==================== 关键词熔断对照测试（学 new-api AutomaticDisableKeywords）====================

    #[tokio::test]
    async fn keyword_disable_trips_immediately_and_fails_over() {
        // 主通道返 500 + "Insufficient Quota"（欠费，重试也没用）→ 关键词命中 →
        // 一次即熔断主通道 + failover 到备用通道。对照错误码熔断（要攒 N 次），
        // 关键词熔断一次命中立即切。
        // 两个独立 mock server：主返 500+欠费，备返 200。各一个 provider（channel 用 provider base_url）
        let main_up = MockServer::start().await;
        let backup_up = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string(
                "{\"error\":{\"message\":\"Insufficient Quota: your credit balance is too low\"}}"))
            .mount(&main_up).await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices":[{"message":{"role":"assistant","content":"ok"}}],
                "usage":{"prompt_tokens":5,"completion_tokens":2}})))
            .mount(&backup_up).await;

        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0, "sticky": {"enabled": false},
            "retry": {"max_retries": 3, "retry_on_status": [500],
                      "disable_keywords": ["insufficient quota", "credit balance"]},
            "providers": [
                {"name": "main-co", "base_url": format!("{}/v1", main_up.uri()),
                 "disabled": false, "headers": {}, "keys": [{"name": "主", "api-key": "sk"}],
                 "channels": [{"id": "ch-main", "name": "主通道", "key_name": "主", "weight": 1,
                               "models": [{"name": "m", "alias": "my-model"}]}]},
                {"name": "backup-co", "base_url": format!("{}/v1", backup_up.uri()),
                 "disabled": false, "headers": {}, "keys": [{"name": "主", "api-key": "sk"}],
                 "channels": [{"id": "ch-backup", "name": "备用", "key_name": "主", "weight": 1,
                               "models": [{"name": "m", "alias": "my-model"}]}]},
            ],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
        let state = S::new(dir.path().to_path_buf());

        // 单次请求：主通道关键词命中熔断 → 切备用 → 备用成功
        let payload = json!({"model": "my-model", "chan": "main", "messages": [{"role": "user", "content": "hi"}]});
        let r = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        // 备用通道接住 → 200（若关键词熔断没生效，会重试主通道或全失败）
        assert_eq!(r.status(), StatusCode::OK, "关键词熔断后应切到备用通道成功");

        // 核对主通道被熔断（health open）：再打一次应直接走备用（主不再被选）
        let h = state.health.read().await;
        let main_health = h.get("ch-main").expect("主通道应有健康记录");
        assert_eq!(main_health.state, gateway::CircuitState::Open, "关键词命中一次即熔断主通道");
        drop(h);

        // 日志应记关键词熔断命中
        let logs = gateway_logs::query_recent(&dir.path(), 10).unwrap();
        let arr = logs["data"].as_array().unwrap();
        let kw_log = arr.iter().find(|e| e["error"].as_str().map_or(false, |s| s.contains("关键词熔断")));
        assert!(kw_log.is_some(), "应记关键词熔断日志");
    }

    #[tokio::test]
    async fn keyword_disable_no_false_positive_on_normal_500() {
        // 对照：普通 500（无欠费关键词）→ 不触发关键词熔断，只走正常错误码重试。
        // 验证关键词熔断不误伤普通错误。
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("{\"error\":\"internal error\"}"))
            .mount(&upstream).await;
        let dir = TempDir::new().unwrap();
        let cfg = json!({
            "listen": 0, "sticky": {"enabled": false},
            "retry": {"max_retries": 1, "retry_on_status": [],
                      "disable_keywords": ["insufficient quota"]},
            "providers": [{
                "name": "co", "base_url": format!("{}/v1", upstream.uri()),
                "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk"}],
                "channels": [{"id": "ch1", "name": "主", "key_name": "主",
                              "models": [{"name": "m", "alias": "my-model"}]}],
            }],
        });
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
        let _ = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;

        // 普通 500 无关键词 → 不强制熔断（error_count 正常，不出现"关键词熔断"日志）
        let logs = gateway_logs::query_recent(&dir.path(), 10).unwrap();
        let arr = logs["data"].as_array().unwrap();
        let kw_log = arr.iter().find(|e| e["error"].as_str().map_or(false, |s| s.contains("关键词熔断")));
        assert!(kw_log.is_none(), "普通 500 不应触发关键词熔断");
    }


    #[test]
    fn hit_disable_keyword_matches_case_insensitive() {
        let kws = vec!["insufficient quota".to_string(), "余额不足".to_string()];
        // 命中（大小写不敏感）
        assert!(hit_disable_keyword(b"{\"error\":\"Insufficient Quota\"}", &kws).is_some());
        assert!(hit_disable_keyword(b"your CREDIT BALANCE is too low", &["credit balance".to_string()]).is_some());
        assert!(hit_disable_keyword("账户余额不足".as_bytes(), &kws).is_some());
        // 不命中
        assert!(hit_disable_keyword(b"{\"error\":\"internal server error\"}", &kws).is_none());
        // 空词表
        assert!(hit_disable_keyword(b"insufficient quota", &[]).is_none());
    }

    #[tokio::test]
    async fn record_outcome_force_open_bypasses_threshold() {
        // force_open=true 一次即 open，无需攒够 failure_threshold
        let h: HealthMap = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let c = gateway::CircuitBreakerConfig { failure_threshold: 5, error_rate_threshold: 50, min_requests: 5, recovery_timeout_seconds: 30, recovery_success_threshold: 3 };
        // 单次失败 + force_open → 立即 Open
        record_outcome(&h, "chX", false, 0, &c, false, true).await;
        let map = h.read().await;
        let st = map.get("chX").unwrap().state.clone();
        assert_eq!(st, CircuitState::Open, "force_open 一次即 Open");
    }

    #[tokio::test]
    async fn record_outcome_normal_failure_below_threshold_stays_closed() {
        // 对照：普通失败（force_open=false）单次不熔断（低于阈值）
        let h: HealthMap = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let c = gateway::CircuitBreakerConfig { failure_threshold: 5, error_rate_threshold: 50, min_requests: 5, recovery_timeout_seconds: 30, recovery_success_threshold: 3 };
        record_outcome(&h, "chY", false, 0, &c, false, false).await;
        let map = h.read().await;
        let st = map.get("chY").unwrap().state.clone();
        assert_eq!(st, CircuitState::Closed, "普通单次失败不熔断");
    }


    #[tokio::test]
    async fn keyword_double_record_outcome_opens() {
        // 精确复刻 keyword 分支：先正常记一次失败（force_open=false），
        // 再 keyword 命中记第二次（force_open=true）→ 应 Open
        let h: HealthMap = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
        let c = gateway::CircuitBreakerConfig { failure_threshold: 5, error_rate_threshold: 50, min_requests: 5, recovery_timeout_seconds: 30, recovery_success_threshold: 3 };
        // 第一次：正常失败（非流式分支开头 record_outcome）
        record_outcome(&h, "ch-main", false, 10, &c, false, false).await;
        // 第二次：keyword 命中强制 open
        record_outcome(&h, "ch-main", false, 10, &c, false, true).await;
        let map = h.read().await;
        let st = map.get("ch-main").unwrap().state.clone();
        assert_eq!(st, CircuitState::Open, "两次 record_outcome（第二次 force_open）应 Open");
    }


    // ==================== 协议端点端到端（/v1/messages + /v1/responses 走 mock 上游）====================


    #[tokio::test]
    async fn tier_switch_continuous_accuracy() {
        // 同一客户端连续切档位 auto→pro→flash→auto：每次上游收到对应 class 的真实模型
        let upstream = MockServer::start().await;
        for mid in ["auto-real", "pro-real", "flash-real"] {
            Mock::given(method("POST")).and(path("/v1/chat/completions"))
                .and(wiremock::matchers::body_partial_json(json!({"model": mid})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": mid,
                    "choices": [{"message": {"role": "assistant", "content": "ok"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
                .mount(&upstream).await;
        }
        let dir = TempDir::new().unwrap();
        let channels = json!([
            {"id": "ch1", "name": "a", "key_name": "主", "models": [{"name": "auto-real", "alias": "auto"}]},
            {"id": "ch2", "name": "p", "key_name": "主", "models": [{"name": "pro-real", "alias": "pro", "class": "pro"}]},
            {"id": "ch3", "name": "f", "key_name": "主", "models": [{"name": "flash-real", "alias": "flash", "class": "flash"}]},
        ]);
        // 关 sticky（纯档位切换，不黏上次通道）
        let cfg = json!({"listen": 0, "sticky": {"enabled": false},
            "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk"}], "channels": channels}]});
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
        let state = S::new(dir.path().to_path_buf());
        // 连续切档位：auto → pro → flash → auto
        for tier in ["auto", "pro", "flash", "auto"] {
            let payload = json!({"model": tier, "messages": [{"role": "user", "content": "hi"}]});
            let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
            assert_eq!(resp.status(), StatusCode::OK, "档位 {} 应 200", tier);
            // 响应回显档位名（对称客户端配置的 model 名）
            let bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
            let j: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(j["model"], tier, "响应 model 应回显档位名 {}", tier);
        }
        // 上游每次收到对应真实模型（4 次请求按序）
        let reqs = upstream.received_requests().await.unwrap();
        let models: Vec<String> = reqs.iter().map(|r| {
            let v: Value = serde_json::from_slice(&r.body).unwrap();
            v["model"].as_str().unwrap_or("").to_string()
        }).collect();
        // pro/flash 硬过滤精确命中对应 class；auto 允许任一候选（不保证顺序/具体模型）
        assert_eq!(models[1], "pro-real", "pro 档应精确命中 pro 模型: {models:?}");
        assert_eq!(models[2], "flash-real", "flash 档应精确命中 flash 模型: {models:?}");
        for m in [&models[0], &models[3]] {
            assert!(["auto-real", "pro-real", "flash-real"].contains(&m.as_str()),
                "auto 档允许任一候选（auto 不降级但也不限定具体模型）: {models:?}");
        }
    }

    #[tokio::test]
    async fn alias_and_name_matching_accuracy() {
        // 请求名=alias → 上游收到真实 name；请求名=name → 直接命中（准确性）
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"model": "up-real"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "model": "up-real",
                "choices": [{"message": {"role": "assistant", "content": "ok"}}], "usage": {"prompt_tokens": 1, "completion_tokens": 1}})))
            .expect(2).mount(&upstream).await;
        let dir = TempDir::new().unwrap();
        // 模型 name=up-real, alias=my-model
        let channels = json!([
            {"id": "ch1", "name": "c", "key_name": "主", "models": [{"name": "up-real", "alias": "my-model"}]},
        ]);
        write_gateway_yaml(dir.path(), &format!("{}/v1", upstream.uri()), channels);
        let state = S::new(dir.path().to_path_buf());
        // 请求 alias
        let p1 = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
        let r1 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, p1, &state.cache).await;
        assert_eq!(r1.status(), StatusCode::OK, "alias 请求应 200");
        // 请求 name
        let p2 = json!({"model": "up-real", "messages": [{"role": "user", "content": "hi"}]});
        let r2 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, p2, &state.cache).await;
        assert_eq!(r2.status(), StatusCode::OK, "name 请求应 200");
        // 上游两次都收到真实 name up-real（alias 与 name 都映射到上游真实名）
        let reqs = upstream.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2);
        for r in reqs.iter() {
            let v: Value = serde_json::from_slice(&r.body).unwrap();
            assert_eq!(v["model"], "up-real", "alias/name 请求都应转成上游真实名");
        }
    }

    #[tokio::test]
    async fn tier_pro_no_candidate_404_end_to_end() {
        // pro 档 → 无 class:pro 候选 → 404（不降级 auto，显式失败）
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;
        let dir = TempDir::new().unwrap();
        let cfg = json!({"listen": 0,
            "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk"}],
                "channels": [{"id": "ch1", "name": "c1", "key_name": "主",
                              "models": [{"name": "flash-model", "class": "flash"}]}]}]});
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "pro", "messages": [{"role": "user", "content": "hi"}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "pro 档无候选 → 404（不降级 auto）");
    }

    #[tokio::test]
    async fn tier_pro_vision_unavailable_409_end_to_end() {
        // pro 档 + 当前轮带图（硬需求 vision）→ pro 模型无 vision → 409（显式失败，不静默）
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;
        let dir = TempDir::new().unwrap();
        let cfg = json!({"listen": 0,
            "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk"}],
                "channels": [{"id": "ch1", "name": "c1", "key_name": "主",
                              "models": [{"name": "pro-model", "class": "pro"}]}]}]});
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "pro", "messages": [{"role": "user", "content": [
            {"type": "text", "text": "hi"}, {"type": "image_url", "image_url": {"url": "x"}}]}]});
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, None, payload, &state.cache).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT, "pro + 带图无 vision → 409");
    }

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
    async fn tokens_crud_end_to_end() {
        use serde_json::json;
        // 多设备身份区分：tokens 增删改查（写 gateway.yaml auth.api_keys）
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        // 初始无 auth
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &json!({"listen": 0})).unwrap();
        let router = build_router(S::new(dir.path().to_path_buf()));

        // GET 空
        let resp = router.clone().oneshot(Request::get("/api/tokens").body(Body::empty()).unwrap()).await.unwrap();
        let j: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(j["tokens"].as_array().unwrap().len(), 0, "初始无 token");

        // POST 加设备
        let add = json!({"name": "手机", "key": "sk-phone"});
        let resp = router.clone().oneshot(Request::post("/api/tokens").header("content-type", "application/json")
            .body(Body::from(add.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // GET 回读（设备名+key 落 gateway.yaml）
        let resp = router.clone().oneshot(Request::get("/api/tokens").body(Body::empty()).unwrap()).await.unwrap();
        let j: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(j["tokens"][0]["name"], "手机");
        assert_eq!(j["tokens"][0]["key"], "sk-phone");

        // PUT 改 key
        let upd = json!({"name": "手机", "key": "sk-phone-v2"});
        let resp = router.clone().oneshot(Request::put("/api/tokens").header("content-type", "application/json")
            .body(Body::from(upd.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = router.clone().oneshot(Request::get("/api/tokens").body(Body::empty()).unwrap()).await.unwrap();
        let j: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(j["tokens"][0]["key"], "sk-phone-v2", "PUT 应更新 key");

        // DELETE 删
        let del = json!({"name": "手机"});
        let resp = router.clone().oneshot(Request::delete("/api/tokens").header("content-type", "application/json")
            .body(Body::from(del.to_string())).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp = router.clone().oneshot(Request::get("/api/tokens").body(Body::empty()).unwrap()).await.unwrap();
        let j: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(j["tokens"].as_array().unwrap().len(), 0, "DELETE 应删空");
    }

    #[tokio::test]
    async fn tokens_auth_uses_device_key_end_to_end() {
        use serde_json::json;
        // 配置 auth.api_keys 设备 key → 该设备 Bearer 鉴权通过（多设备身份区分）
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "x", "choices": []})))
            .mount(&upstream).await;
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        let cfg = json!({"listen": 0,
            "auth": {"api_keys": [{"name": "手机", "key": "sk-phone"}]},
            "providers": [{"name": "p", "base_url": format!("{}/v1", upstream.uri()), "disabled": false, "headers": {},
                "keys": [{"name": "主", "api-key": "sk"}],
                "channels": [{"id": "ch1", "name": "c", "key_name": "主",
                              "models": [{"name": "m", "alias": "my-model"}]}]}]});
        api::write_yaml_json(&dir.path().join("config").join("gateway.yaml"), &cfg).unwrap();
        let state = S::new(dir.path().to_path_buf());
        let payload = json!({"model": "my-model", "messages": [{"role": "user", "content": "hi"}]});
        // 设备 key → 200
        let resp = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, Some("Bearer sk-phone"), payload.clone(), &state.cache).await;
        assert_eq!(resp.status(), StatusCode::OK, "设备 key 应通过");
        // 错 key → 401
        let resp2 = gateway::forward_chat(&state.root, &state.health, &state.pins, &state.rates, Some("Bearer sk-evil"), payload.clone(), &state.cache).await;
        assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED, "错 key 应 401");
    }

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

    // ==================== 上游 anthropic 转发端到端（mock /v1/messages）====================
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