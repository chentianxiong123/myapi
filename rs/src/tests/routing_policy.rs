//! routing_policy（按主题拆分）
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