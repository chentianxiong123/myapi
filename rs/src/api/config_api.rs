//! config_api（精确拆分）
use super::*;

pub fn get_panel_config() -> Value {
    let (_, bind) = panel_cfg();
    let (auth, _) = panel_cfg();
    let g = gateway_cfg();
    let upstreams = g.upstreams;
    json!({
        "ok": true, "auth_set": !auth.is_empty(), "bind": bind,
        "topology": {
            "listen": g.listen,
            "panel_port": g.panel_port,
            "upstream_count": upstreams.len(),
            "upstreams": upstreams,
            "nginx_needed": upstreams.len() > 1,
        }
    })
}

/// 对应 set_panel_config()：写 gateway.yaml panel 段
pub fn set_panel_config(payload: &Value) -> Value {
    let path = root().join("config").join("gateway.yaml");
    let raw = read_yaml_json(&path);
    let mut m = raw.as_object().cloned().unwrap_or_default();
    let mut gw = m.get("gateway").cloned().unwrap_or(Value::Object(Default::default()));
    let mut pl = gw.get("panel").cloned().unwrap_or(Value::Object(Default::default()));
    if let Some(a) = pl.as_object_mut() {
        if payload.get("auth_key").is_some() {
            a.insert("auth_key".into(), Value::String(payload.get("auth_key").and_then(|v| v.as_str()).unwrap_or("").into()));
        }
        if payload.get("bind").is_some() {
            a.insert("bind".into(), Value::String(payload.get("bind").and_then(|v| v.as_str()).unwrap_or("127.0.0.1").into()));
        }
    }
    if let Some(g) = gw.as_object_mut() { g.insert("panel".into(), pl); }
    m.insert("gateway".into(), gw);
    match write_yaml_json(&path, &Value::Object(m)) {
        Ok(_) => json!({"ok": true}),
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    }
}

/// 探测结果缓存：存 gateway.yaml 的 gateway.probe_cache 段（按 host 记录上次探测）。
pub fn get_probe_cache() -> Value {
    let p = root().join("config").join("gateway.yaml");
    let raw = read_yaml_json(&p);
    let pc = raw.get("gateway").and_then(|g| g.get("probe_cache")).cloned().unwrap_or(Value::Null);
    json!({"ok": true, "probe_cache": pc})
}

/// 写探测结果缓存（前端手动触发探测后回写）。payload: {host, result, probed_at}。
/// result 为 null 时 = 删除该 host 的缓存条目。
pub fn set_probe_cache(payload: &Value) -> Value {
    let path = root().join("config").join("gateway.yaml");
    let raw = read_yaml_json(&path);
    let mut m = raw.as_object().cloned().unwrap_or_default();
    let mut gw = m.get("gateway").cloned().unwrap_or(Value::Object(Default::default()));
    let mut pc = gw.get("probe_cache").cloned().unwrap_or(Value::Object(Default::default()));
    let host = payload.get("host").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if !host.is_empty() {
        if let Some(pcm) = pc.as_object_mut() {
            if payload.get("result").map(|v| v.is_null()).unwrap_or(false) {
                pcm.remove(&host); // 删除缓存条目
            } else {
                let mut entry = serde_json::Map::new();
                if let Some(r) = payload.get("result") { entry.insert("result".into(), r.clone()); }
                if let Some(t) = payload.get("probed_at") { entry.insert("probed_at".into(), t.clone()); }
                pcm.insert(host, Value::Object(entry));
            }
        }
    }
    if let Some(g) = gw.as_object_mut() { g.insert("probe_cache".into(), pc); }
    m.insert("gateway".into(), gw);
    match write_yaml_json(&path, &Value::Object(m)) {
        Ok(_) => json!({"ok": true}),
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    }
}

/// 对应 get_models()：CPA 模型 + 覆盖
pub async fn get_models() -> Value {
    let cfg = match crate::core::config::SyncConfig::load(None) {
        Ok(c) => c, Err(e) => return json!({"ok": false, "error": e, "models": [], (crate::adapters::DEFAULT_PROVIDER_ID): {}}),
    };
    let cli = crate::core::cpa_client::CPAClient::new(&cfg.myapi.base_url, &cfg.myapi.api_key, 8);
    let models = match cli.list_models().await {
        Ok(m) => m, Err(e) => return json!({"ok": false, "error": e.to_string(), "models": [], (crate::adapters::DEFAULT_PROVIDER_ID): {}}),
    };
    let myapi = json!({"context_window": cfg.myapi.context_window, "max_tokens": cfg.myapi.max_tokens,
                     "auto_compact_token_limit": cfg.myapi.auto_compact_token_limit,
                     "default_input_modes": cfg.myapi.default_input_modes, "image_models": cfg.myapi.image_models});
    let mut list = Vec::new();
    for m in models {
        let ov = cfg.myapi.model_overrides.get(&m.id).cloned().unwrap_or(serde_yaml::Value::Null);
        list.push(json!({"id": m.id, "override": ov}));
    }
    json!({"ok": true, (crate::adapters::DEFAULT_PROVIDER_ID): myapi, "models": list})
}

/// 对应 get_adapters_info()
pub fn get_adapters_info() -> Value {
    let names = crate::adapters::available_names();
    let cfg = read_yaml_json(&config_path());
    json!({"names": names, "hosts": cfg.get("hosts").cloned().unwrap_or_default(),
           "adapter_instances": cfg.get("adapter_instances").cloned().unwrap_or_default(),
           "targets": cfg.get("targets").cloned().unwrap_or_default(),
           "strategies": ["replace", "merge"]})
}

/// 探测选中 host 上的 agent 安装情况（web 面板「嗅探」用）。
pub fn probe_hosts(hosts_csv: &str) -> Value {
    crate::core::probe::probe("", hosts_csv)
}

/// 对应 get_version()
pub fn get_version() -> Value {
    let mut ver = String::from("?");
    if let Ok(text) = std::fs::read_to_string(root().join(crate::adapters::DEFAULT_PROVIDER_ID)) {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VERSION=") {
                let v: String = rest.chars().filter(|c| *c != '"' && *c != ' ').collect();
                if !v.is_empty() { ver = v; break; }
            }
        }
    }
    json!({(crate::adapters::DEFAULT_PROVIDER_ID): ver, "agent_sync": "1.0.0", "adapters": crate::adapters::available_names().len()})
}