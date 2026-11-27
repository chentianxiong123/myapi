//! 多机 HA Nginx 配置生成（从 forward.rs 拆出）
use super::*;

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
