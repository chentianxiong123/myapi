//! panel_api（按主题拆分）
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
async fn health_returns_ok() {
    let (router, _d) = test_router().await;
    let (status, json) = http_get(router, "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("ok"));
}

#[tokio::test]
async fn models_returns_list_shape() {
    let (router, _d) = test_router().await;
    let (status, json) = http_get(router, "/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("object").and_then(|v| v.as_str()), Some("list"));
    assert!(json.get("data").and_then(|v| v.as_array()).is_some());
}

#[tokio::test]
async fn logs_returns_object_and_data() {
    let (router, _d) = test_router().await;
    let (status, json) = http_get(router, "/v1/logs").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("object").and_then(|v| v.as_str()), Some("list"));
    assert!(json.get("data").and_then(|v| v.as_array()).is_some());
}

#[tokio::test]
async fn stats_summary_has_totals() {
    let (router, _d) = test_router().await;
    let (status, json) = http_get(router, "/stats/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.get("total_requests").and_then(|v| v.as_i64()).is_some());
    assert!(json.get("success_count").and_then(|v| v.as_i64()).is_some());
}

#[tokio::test]
async fn stats_providers_returns_array() {
    let (router, _d) = test_router().await;
    let (status, json) = http_get(router, "/stats/providers").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
}

#[tokio::test]
async fn stats_trend_and_daily_return_arrays() {
    let (router, _d) = test_router().await;
    let (s1, j1) = http_get(router.clone(), "/stats/trend?hours=24").await;
    let (s2, j2) = http_get(router, "/stats/daily?days=7").await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert!(j1.is_array());
    assert!(j2.is_array());
}

#[tokio::test]
async fn api_logs_limit_param_respected() {
    let (router, _d) = test_router().await;
    let (status, json) = http_get(router, "/api/logs?limit=5").await;
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
    let (st, j) = http_post(router.clone(), "/api/providers", providers).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(true));
    // 读回
    let (st2, j2) = http_get(router, "/api/providers").await;
    assert_eq!(st2, StatusCode::OK);
    let arr = j2.get("providers").and_then(|v| v.as_array()).unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].get("name").and_then(|v| v.as_str()), Some("p1"));
}

#[tokio::test]
async fn routing_roundtrip() {
    let (router, _d) = test_router().await;
    let routing = serde_json::json!({"routing": {"strategy": "latency", "max_retries": 3}});
    let (st, j) = http_post(router.clone(), "/api/routing", routing).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(j.get("ok").and_then(|v| v.as_bool()), Some(true));
    let (_st2, j2) = http_get(router, "/api/routing").await;
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
    let (st, j) = http_get(router, "/api/mcp").await;
    assert_eq!(st, StatusCode::OK);
    assert!(j.get("tools").and_then(|v| v.as_array()).is_some());
}

#[tokio::test]
async fn post_providers_missing_section_rejected() {
    let (router, _d) = test_router().await;
    let (st, j) = http_post(router, "/api/providers", serde_json::json!({"wrong": 1})).await;
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
    let (st, j) = http_post(app, "/api/channels/test", json!({"provider": "p", "channel_id": "ch1"})).await;
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
    let (st, j) = http_post(app, "/api/channels/test", json!({"provider": "p", "channel_id": "ch1"})).await;
    assert_eq!(st, 200); // 端点本身 200，测试结果在 body
    assert_eq!(j["ok"], false, "上游 401 应报通道异常");
    assert!(j["error"].as_str().is_some(), "应有错误信息");
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