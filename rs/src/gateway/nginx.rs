//! nginx（从 gateway.rs 精确拆分）
use super::*;

pub async fn probe_upstream(root: &std::path::Path) -> Vec<serde_json::Value> {
    let config = load_config(root);
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for p in &config.providers {
        if p.disabled {
            continue;
        }
        let base = p.base_url.trim_end_matches('/');
        let url = format!("{}/models", base);
        for c in &p.channels {
            let api_key = resolve_key(p, c).unwrap_or_default();
            let start = std::time::Instant::now();
            let mut req = client.get(&url).header("Authorization", format!("Bearer {}", api_key));
            for (k, v) in &p.headers {
                req = req.header(k.as_str(), v.as_str());
            }
            let (live, status_code, err) = match req.send().await {
                Ok(r) => {
                    let sc = r.status().as_u16();
                    // 401/403 说明上游活着（只是 key 问题）；2xx/429 都算可达
                    let ok = sc < 500;
                    (ok, sc, String::new())
                }
                Err(e) => (false, 0, e.to_string()),
            };
            let latency = start.elapsed().as_millis() as u64;
            out.push(serde_json::json!({
                "provider": p.name,
                "channel": c.name,
                "channel_id": c.id,
                "live": live,
                "status": if live { "healthy" } else { "unreachable" },
                "status_code": status_code,
                "latency_ms": latency,
                "error": err,
            }));
        }
    }
    out
}

/// 生成 Nginx 高可用配置（学 new-api 多机部署）：从 deploy 配置生成完整 nginx.conf
/// 客户端 → Nginx 固定入口 → upstream 主/备 → 网关节点。故障切换靠 proxy_next_upstream。
/// SSE 流式：proxy_buffering off + 长 read timeout。
pub fn generate_nginx_conf(deploy: &serde_json::Value) -> String {
    let entry = deploy.get("entry").cloned().unwrap_or_else(|| serde_json::json!({}));
    let listen = entry.get("listen").and_then(|v| v.as_u64()).unwrap_or(8380);
    let read_timeout = entry.get("readTimeout").and_then(|v| v.as_u64()).unwrap_or(120);
    let next_upstream = entry.get("nextUpstream").and_then(|v| v.as_str()).unwrap_or("error timeout http_502 http_503 http_504");
    let instances = deploy.get("instances").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    // upstream 块：master 在前，backup 标记 backup（只有主挂了才用备）
    let mut servers = String::new();
    for inst in &instances {
        let addr = inst.get("address").and_then(|v| v.as_str()).unwrap_or("");
        if addr.is_empty() { continue; }
        let role = inst.get("role").and_then(|v| v.as_str()).unwrap_or("backup");
        let max_fails = inst.get("heartbeatMs").and_then(|v| v.as_u64()).unwrap_or(5000) / 1000;
        if role == "master" {
            servers.push_str(&format!("        server {} max_fails={} fail_timeout={}s;\n", addr, max_fails.max(2), max_fails.max(5)));
        } else {
            servers.push_str(&format!("        server {} backup max_fails={} fail_timeout={}s;\n", addr, max_fails.max(2), max_fails.max(5)));
        }
    }
    if servers.is_empty() {
        servers.push_str("        # 无实例，请在面板添加网关节点\n");
    }

    format!(
        r#"# myapi 高可用 Nginx 配置（由面板生成）
# 部署：放到 Nginx 常驻机（不要装在 meilan 手机，保持手机省电）
# 客户端认固定入口 :{listen}，主节点挂了 proxy_next_upstream 自动切备用

worker_processes auto;
events {{ worker_connections 1024; }}

http {{
    # SSE 流式透传（网关的 /v1/chat/completions 是 SSE，必须关缓冲）
    proxy_buffering off;
    proxy_cache off;
    proxy_http_version 1.1;
    proxy_set_header Connection "";

    upstream myapi {{
{servers}    }}

    server {{
        listen {listen};
        # 客户端只认这个入口，无感知主备切换

        location / {{
            proxy_pass http://myapi;
            proxy_next_upstream {next_upstream};
            proxy_next_upstream_tries 3;
            proxy_next_upstream_timeout 30s;

            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
            proxy_read_timeout {read_timeout}s;
            proxy_send_timeout {read_timeout}s;
            proxy_connect_timeout 5s;
        }}

        # 健康检查（Nginx 主动探活，摘除死节点）
        location = /health {{
            proxy_pass http://myapi;
            proxy_next_upstream error timeout;
        }}
    }}
}}
"#,
        listen = listen,
        servers = servers,
        next_upstream = next_upstream,
        read_timeout = read_timeout,
    )
}