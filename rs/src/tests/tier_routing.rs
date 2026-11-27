//! tier_routing（按主题拆分）
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