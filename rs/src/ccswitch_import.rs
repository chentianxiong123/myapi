//! 从 cc-switch 导入 provider 资产（单向：cc-switch → myapi gateway.yaml）
//!
//! 分析背景（docs/cc-switch依赖分析.md）：
//! - cc-switch 是 client 侧单选切换器（ProviderManager{providers, current}），
//!   provider 元数据存 `~/.cc-switch/cc-switch.db` 的 `providers` 表（settings_config JSON）。
//! - myapi 是 server 侧网关。本导入器**只吸收 cc-switch 的 provider 资产**
//!   （base_url / api_key / 模型清单），转成 myapi 的 Provider + Channel。
//! - **运行时绝不依赖 cc-switch**；本导入是一次性的（用户显式执行），不反向耦合。
//!
//! 目标模型（gateway.yaml）：
//! - Provider { name, base_url, keys:[NamedKey], channels:[Channel] }
//! - Channel  { id, name, key_name, models:[ChannelModel] }
//! 每个 cc-switch provider → 1 个 myapi Provider + 1 个 Channel（key 引用密钥池）。

use crate::gateway::{Channel, ChannelModel, NamedKey, Provider};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;

/// cc-switch db 里一行 provider（只取导入所需字段）
#[derive(Debug, Clone)]
struct CcProvider {
    id: String,
    app_type: String,
    name: String,
    settings: Value,
    is_current: bool,
}

/// 提取 (base_url, api_key)。按 app_type 走 cc-switch 的字段路径：
/// - openclaw/dsh/gemini/grokbuild：`baseUrl`/`baseURL` + `apiKey`/`apiKeyEnv`
/// - claude/claude-desktop：`env.ANTHROPIC_BASE_URL` + `env.ANTHROPIC_AUTH_TOKEN`
/// - codex：`config`(toml 字符串) 里的 base_url + `auth`
fn extract_credentials(app_type: &str, s: &Value) -> (Option<String>, Option<String>) {
    let str_at = |v: Option<&Value>| v.and_then(|x| x.as_str()).map(|x| x.to_string());
    // 顶层 baseUrl / baseURL（openclaw/dsh/gemini/grokbuild 用）
    let top_base = str_at(s.get("baseUrl")).or_else(|| str_at(s.get("baseURL")));
    // env 形态（claude 用）
    let env_base = s
        .pointer("/env/ANTHROPIC_BASE_URL")
        .and_then(|v| v.as_str())
        .map(|x| x.to_string());
    let env_key = s
        .pointer("/env/ANTHROPIC_AUTH_TOKEN")
        .and_then(|v| v.as_str())
        .map(|x| x.to_string());

    let base_url = top_base.or(env_base);

    // apiKey 顶层（openclaw 用）；apiKeyEnv 是环境变量名（dsh 用）
    let api_key = str_at(s.get("apiKey")).or_else(|| {
        s.get("apiKeyEnv")
            .and_then(|v| v.as_str())
            .and_then(|env_name| std::env::var(env_name).ok())
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
    });
    let api_key = api_key.or(env_key);

    (base_url, api_key)
}

/// 提取模型清单（多种形态：models[].id / env.ANTHROPIC_MODEL / codex config.toml model / modelCatalog）
fn extract_models(s: &Value) -> Vec<ChannelModel> {
    let mut out: Vec<ChannelModel> = Vec::new();
    let mut push = |id: String| {
        if !id.trim().is_empty() && !out.iter().any(|m| m.name == id) {
            out.push(ChannelModel { name: id, alias: None, capabilities: Vec::new() });
        }
    };
    // 1) models[].id（openclaw / dsh / gemini 用）
    if let Some(arr) = s.get("models").and_then(|v| v.as_array()) {
        for m in arr {
            if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                push(id.to_string());
            }
        }
    }
    // 2) env.ANTHROPIC_MODEL（claude 用）
    if let Some(m) = s.pointer("/env/ANTHROPIC_MODEL").and_then(|v| v.as_str()) {
        push(m.to_string());
    }
    // 3) codex：config toml 的 model 字段 + modelCatalog.models[].model
    if let Some(cfg) = s.get("config").and_then(|v| v.as_str()) {
        // 简单提取顶层 model = "..."（cc-switch codex config 的 model 是顶层键）
        for line in cfg.lines() {
            let line = line.trim();
            if line.starts_with("model = ") && !line.starts_with("model_provider") {
                if let Some(m) = line.split('=').nth(1).map(|v| v.trim().trim_matches('"')) {
                    push(m.to_string());
                }
                break; // 只需顶层 model
            }
        }
    }
    if let Some(arr) = s.get("modelCatalog").and_then(|v| v.get("models")).and_then(|v| v.as_array()) {
        for m in arr {
            if let Some(id) = m.get("model").and_then(|v| v.as_str()) {
                push(id.to_string());
            }
        }
    }
    out
}

/// 读 cc-switch db 的 providers 表 → 转成 myapi Provider 列表（跳过无凭据/官方壳）
pub fn import_from_db(db_path: &str) -> Result<Vec<Provider>, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开 cc-switch db 失败: {e}"))?;
    let mut stmt = conn
        .prepare("SELECT id, app_type, name, settings_config, is_current FROM providers")
        .map_err(|e| format!("查询 providers 表失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CcProvider {
                id: row.get::<_, String>(0)?,
                app_type: row.get::<_, String>(1)?,
                name: row.get::<_, String>(2)?,
                settings: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
                is_current: row.get::<_, i64>(4)? != 0,
            })
        })
        .map_err(|e| format!("遍历 providers 失败: {e}"))?;

    let mut out: Vec<Provider> = Vec::new();
    // (name, base_url) → 索引：同名同 base（claude/codex 各一份同上游）合并成一个 Provider
    let mut idx: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    for r in rows {
        let p = r.map_err(|e| format!("读 provider 行失败: {e}"))?;
        let (base_url, api_key) = extract_credentials(&p.app_type, &p.settings);
        let (Some(base_url), Some(api_key)) = (base_url, api_key) else {
            continue; // 官方壳 / 无凭据（如 claude-desktop-official env:{}）跳过
        };
        if base_url.trim().is_empty() || api_key.trim().is_empty() {
            continue;
        }
        let models = extract_models(&p.settings);
        // 同名同 base：合并到现有 Provider（追加 channel + 合并模型 + 补 key）
        let key = (p.name.clone(), base_url.clone());
        if let Some(&i) = idx.get(&key) {
            let prov = &mut out[i];
            // 追加通道（cc-switch 的 claude/codex 各一份 → myapi 多条通道，同一上游）
            prov.channels.push(Channel {
                id: format!("cc-{}", p.id),
                name: p.name.clone(),
                path: None,
                key_name: Some("主".into()),
                models: models.clone(),
                weight: 1,
                rpm: None,
                retry: None,
                max_input_tokens: None,
            });
            // 合并模型（去重，保留已有顺序）
            let existing: std::collections::HashSet<String> =
                prov.channels.iter().flat_map(|c| c.models.iter().map(|m| m.name.clone())).collect();
            for m in models {
                if !existing.contains(&m.name) {
                    if let Some(first) = prov.channels.first_mut() {
                        first.models.push(m);
                    }
                }
            }
            // 补 key（首通道用的「主」密钥；若现有为空则用新 key）
            if prov.keys.first().map(|k| k.api_key.trim().is_empty()).unwrap_or(true) {
                prov.keys = vec![NamedKey { name: "主".into(), api_key }];
            }
            continue;
        }
        let channel_id = format!("cc-{}", p.id);
        idx.insert(key, out.len());
        out.push(Provider {
            name: p.name.clone(),
            base_url,
            disabled: false,
            headers: HashMap::new(),
            keys: vec![NamedKey {
                name: "主".into(),
                api_key,
            }],
            channels: vec![Channel {
                id: channel_id,
                name: p.name,
                path: None,
                key_name: Some("主".into()),
                models,
                weight: 1,
                rpm: None,
                retry: None,
                max_input_tokens: None,
            }],
            proxy: String::new(),
        });
    }
    Ok(out)
}

/// 追加导入的 providers 到现有 gateway.yaml（保留原 providers，按 name 去重）
pub fn merge_into_yaml(
    yaml_path: &std::path::Path,
    imported: &[Provider],
) -> Result<usize, String> {
    use crate::api;
    let mut doc = api::read_yaml_json(&yaml_path.to_path_buf());
    if !doc.is_object() {
        doc = serde_json::json!({});
    }
    let mut existing_keys: std::collections::HashSet<(String, String)> = doc
        .get("providers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let name = p.get("name").and_then(|v| v.as_str());
                    let base = p.get("base_url").and_then(|v| v.as_str());
                    name.zip(base).map(|(n, b)| (n.to_string(), b.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut added = 0;
    let providers = doc
        .get_mut("providers")
        .and_then(|v| v.as_array_mut());
    let list: &mut Vec<Value> = match providers {
        Some(l) => l,
        None => {
            doc.as_object_mut()
                .unwrap()
                .insert("providers".into(), Value::Array(Vec::new()));
            doc.get_mut("providers").and_then(|v| v.as_array_mut()).unwrap()
        }
    };

    for p in imported {
        let dedup_key = (p.name.clone(), p.base_url.clone());
        if existing_keys.contains(&dedup_key) {
            continue; // 已有同名同 base provider，跳过（防重复导入）
        }
        existing_keys.insert(dedup_key);
        let v = serde_json::to_value(p).map_err(|e| format!("序列化 provider 失败: {e}"))?;
        list.push(v);
        added += 1;
    }
    api::write_yaml_json(&yaml_path.to_path_buf(), &doc)
        .map_err(|e| format!("写 gateway.yaml 失败: {e}"))?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_credentials_openclaw() {
        // openclaw 形态：baseUrl + apiKey + models
        let s = serde_json::json!({
            "baseUrl": "https://token.sensenova.cn/v1",
            "apiKey": "sk-fake-123",
            "models": [{"id": "sensenova-6.8-flash-lite"}, {"id": "deepseek-v4-flash"}]
        });
        let (base, key) = extract_credentials("openclaw", &s);
        assert_eq!(base.as_deref(), Some("https://token.sensenova.cn/v1"));
        assert_eq!(key.as_deref(), Some("sk-fake-123"));
        let models = extract_models(&s);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "sensenova-6.8-flash-lite");
    }

    #[test]
    fn test_extract_credentials_claude_env() {
        // claude 形态：env.ANTHROPIC_BASE_URL + env.ANTHROPIC_AUTH_TOKEN
        let s = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": "http://192.168.31.213:8317",
                "ANTHROPIC_AUTH_TOKEN": "sk-claude-fake"
            }
        });
        let (base, key) = extract_credentials("claude", &s);
        assert_eq!(base.as_deref(), Some("http://192.168.31.213:8317"));
        assert_eq!(key.as_deref(), Some("sk-claude-fake"));
    }

    #[test]
    fn test_skip_official_shell() {
        // 官方壳：env:{} 无凭据 → 提取为空 → import 跳过
        let s = serde_json::json!({"env": {}});
        let (base, key) = extract_credentials("claude-desktop", &s);
        assert!(base.is_none() && key.is_none());
    }

    #[test]
    fn test_extract_models_codex_config() {
        // codex 形态：config toml 的 model + modelCatalog
        let s = serde_json::json!({
            "config": "model_provider = \"custom\"\nmodel = \"sensenova-6.8-flash\"\nmodel_context_window = 256000\n\n[model_providers.custom]\nbase_url = \"http://x/v1\"\n",
            "modelCatalog": {"models": [{"model": "sensenova-6.8-flash"}, {"model": "deepseek-v4-flash"}]}
        });
        let models = extract_models(&s);
        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"sensenova-6.8-flash"), "config toml 的 model 应提取");
        assert!(names.contains(&"deepseek-v4-flash"), "modelCatalog 的 model 应提取");
    }

    #[test]
    fn test_extract_models_claude_env_model() {
        // claude 形态：env.ANTHROPIC_MODEL
        let s = serde_json::json!({"env": {"ANTHROPIC_MODEL": "claude-sonnet-4", "ANTHROPIC_BASE_URL": "http://x"}});
        let models = extract_models(&s);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "claude-sonnet-4");
    }

    #[test]
    fn test_merge_dedup() {
        // 合并去重：同名 provider 不重复
        let dir = tempfile::TempDir::new().unwrap();
        let yaml = dir.path().join("gateway.yaml");
        std::fs::write(&yaml, "listen: 8380\nproviders:\n  - name: existing\n    base_url: http://a/v1\n").unwrap();
        let imported = vec![
            Provider { name: "existing".into(), base_url: "http://a/v1".into(), disabled: false,
                headers: HashMap::new(), keys: vec![NamedKey{name:"主".into(),api_key:"k".into()}],
                channels: vec![Channel{id:"c1".into(),name:"existing".into(),path:None,key_name:Some("主".into()),
                    models:vec![],weight:1,rpm:None,retry:None,max_input_tokens:None}], proxy: String::new() },
            Provider { name: "new-provider".into(), base_url: "http://b/v1".into(), disabled: false,
                headers: HashMap::new(), keys: vec![NamedKey{name:"主".into(),api_key:"k2".into()}],
                channels: vec![Channel{id:"c2".into(),name:"new-provider".into(),path:None,key_name:Some("主".into()),
                    models:vec![],weight:1,rpm:None,retry:None,max_input_tokens:None}], proxy: String::new() },
        ];
        let added = merge_into_yaml(&yaml, &imported).unwrap();
        assert_eq!(added, 1, "同名 existing 应跳过，只加 new-provider");
        let doc = crate::api::read_yaml_json(&yaml);
        let names: Vec<&str> = doc["providers"].as_array().unwrap().iter()
            .filter_map(|p| p.get("name").and_then(|v| v.as_str())).collect();
        assert_eq!(names, vec!["existing", "new-provider"]);
    }
}
