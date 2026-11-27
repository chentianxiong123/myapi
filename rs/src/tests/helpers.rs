//! helpers（按主题拆分）
#![allow(unused_imports)]
use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use tempfile::TempDir;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};
use crate::gateway::{hit_disable_keyword, record_outcome, HealthMap, CircuitState};

/// 构造隔离测试 Router：tempdir 做 root（gateway.yaml / SQLite 都写临时目录，不污染真实配置）
pub(super) async fn test_router() -> (Router, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    // db_path = root/config/gateway-logs.db，SQLite 不自动建父目录 → 先建 config 子目录
    std::fs::create_dir_all(dir.path().join("config")).expect("mkdir config");
    let state = S::new(dir.path().to_path_buf());
    // 建日志表（幂等，测 /v1/logs /stats 才有表）
    gateway_logs::init_table(dir.path()).expect("init_table");
    (build_router(state), dir)
}

pub(super) async fn http_get(router: Router, uri: &str) -> (StatusCode, Value) {
    let resp = router.oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

pub(super) async fn http_post(router: Router, uri: &str, body: Value) -> (StatusCode, Value) {
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

/// 写 gateway.yaml 到 tempdir（config 子目录），指向给定 upstream base_url
pub(super) fn write_gateway_yaml(root: &std::path::Path, base_url: &str, channels: Value) {
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

pub(super) fn one_channel(id: &str, model: &str) -> Value {
    json!([{
        "id": id, "name": id, "key_name": "主",
        "models": [{"name": model}],
    }])
}

/// 档位通道：两个模型（pro 质量型 / flash 速度型）
pub(super) fn tier_channels() -> Value {
    json!([{
        "id": "ch1", "name": "ch1", "key_name": "主",
        "models": [
            {"name": "pro-model", "class": "pro"},
            {"name": "flash-model", "class": "flash"},
        ],
    }])
}

/// 写 gateway.yaml：两通道（ch1/ch2）都服务同一对外别名，上游名不同（好区分打了几次）
pub(super) fn write_two_channel_yaml(root: &std::path::Path, base_url: &str, ch1_model: &str, ch2_model: &str, sticky: bool) {
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

pub(super) async fn stream_mock_state(dir: &std::path::Path, upstream: &MockServer) -> S {
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

pub(super) async fn wait_summary(root: &std::path::Path, expect_in: Option<i64>, expect_out: Option<i64>) -> bool {
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

pub(super) fn sse(chunks: &[String]) -> String {
    let mut s = String::new();
    for c in chunks { s.push_str("data: "); s.push_str(c); s.push_str("\n\n"); }
    s.push_str("data: [DONE]\n\n");
    s
}

#[test]
pub(super) fn hit_disable_keyword_matches_case_insensitive() {
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