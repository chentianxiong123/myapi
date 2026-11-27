//! circuit_keyword（按主题拆分）
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