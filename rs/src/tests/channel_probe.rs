//! channel_probe（按主题拆分）
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