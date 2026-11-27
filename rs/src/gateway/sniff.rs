//! 上游模型嗅探（从 forward.rs 拆出，纯手动 detect/apply）
use super::*;

pub fn upstream_models_url(base: &str, protocol: &str) -> String {
    let base = base.trim_end_matches('/');
    match protocol {
        "gemini" => format!("{}/v1beta/models", base),
        "ollama" => format!("{}/api/tags", base),
        _ => format!("{}/models", base),
    }
}

pub fn strip_model_prefix(id: &str, protocol: &str) -> String {
    if protocol == "gemini" {
        id.strip_prefix("models/").unwrap_or(id).to_string()
    } else {
        id.to_string()
    }
}

pub async fn fetch_upstream_models(base_url: &str, api_key: &str, headers: &[(String, String)], protocol: &str) -> Result<Vec<String>, String> {
    let url = upstream_models_url(base_url, protocol);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client 构造失败: {}", e))?;
    let mut req = client.get(&url).header("Authorization", format!("Bearer {}", api_key));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("请求上游失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("上游返回 {}", status.as_u16()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    // OpenAI 格式 {data: [{id: "gpt-4", ...}, ...]}；兼容 {models: [...]}（gemini/ollama）/ 裸数组
    let ids: Vec<String> = if let Some(data) = body.get("data").and_then(|v| v.as_array()) {
        data.iter().filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())).collect()
    } else if let Some(models) = body.get("models").and_then(|v| v.as_array()) {
        models.iter().filter_map(|m| {
            m.get("id").or_else(|| m.get("name")).and_then(|x| x.as_str()).map(|s| s.to_string())
        }).collect()
    } else if let Some(arr) = body.as_array() {
        arr.iter().filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())).collect()
    } else {
        vec![]
    };
    Ok(ids.into_iter().map(|id| strip_model_prefix(&id, protocol)).collect())
}

pub fn diff_upstream_models(upstream: &[String], local: &[String], ignored: &[String])
    -> (Vec<String>, Vec<String>, Vec<String>) {
    use regex::Regex;
    use std::collections::HashSet;

    let local_set: HashSet<&str> = local.iter().map(|s| s.as_str()).collect();
    // 预编译忽略规则（`regex:` 前缀走正则，否则精确匹配）
    let rules: Vec<(String, Option<Regex>)> = ignored
        .iter()
        .filter_map(|r| {
            let r = r.trim().to_string();
            if r.is_empty() { return None; }
            if let Some(pat) = r.strip_prefix("regex:") {
                let re = Regex::new(pat).ok();
                Some((r, re))
            } else {
                Some((r, None))
            }
        })
        .collect();

    let mut ignored_hit = Vec::new();
    let mut add = Vec::new();
    for m in upstream {
        if local_set.contains(m.as_str()) { continue; }
        if rules.iter().any(|(r, re)| match re {
            Some(rx) => rx.is_match(m),
            None => r == m,
        }) {
            ignored_hit.push(m.clone());
            continue;
        }
        add.push(m.clone());
    }

    let up_set: HashSet<&str> = upstream.iter().map(|s| s.as_str()).collect();
    let remove: Vec<String> = local.iter()
        .filter(|m| !up_set.contains(m.as_str()))
        .cloned()
        .collect();

    (add, remove, ignored_hit)
}

pub fn apply_upstream_models(
    root: &std::path::Path,
    provider: &str,
    channel_id: &str,
    add: &[serde_json::Value],
    remove: &[String],
) -> Result<serde_json::Value, String> {
    use std::collections::HashSet;
    let path = root.join("config").join("gateway.yaml");
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读 gateway.yaml 失败: {}", e))?;
    let mut doc: serde_yaml::Value = serde_yaml::from_str(&raw)
        .map_err(|e| format!("解析 gateway.yaml 失败: {}", e))?;

    let provs = doc.get_mut("providers").and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| "gateway.yaml 缺 providers 段".to_string())?;
    let p = provs.iter_mut()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(provider))
        .ok_or_else(|| format!("供应商不存在: {}", provider))?;
    let chs = p.get_mut("channels").and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| format!("供应商 {} 缺 channels 段", provider))?;
    let c = chs.iter_mut()
        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(channel_id))
        .ok_or_else(|| format!("通道不存在: {}", channel_id))?;
    let models = c.get_mut("models").and_then(|v| v.as_sequence_mut())
        .ok_or_else(|| format!("通道 {} 缺 models 段", channel_id))?;

    let to_remove: HashSet<&str> = remove.iter().map(|s| s.as_str()).collect();
    let before = models.len();
    models.retain(|m| !to_remove.contains(
        m.get("name").and_then(|v| v.as_str()).unwrap_or(""),
    ));
    let removed = before - models.len();
    for m in add {
        let yv = serde_yaml::to_value(m).map_err(|e| format!("模型项转 YAML 失败: {}", e))?;
        models.push(yv);
    }

    std::fs::write(&path, serde_yaml::to_string(&doc).map_err(|e| format!("序列化失败: {}", e))?)
        .map_err(|e| format!("写 gateway.yaml 失败: {}", e))?;

    let provs_val = doc.get("providers").cloned().unwrap_or(serde_yaml::Value::Null);
    serde_json::to_value(provs_val).map_err(|e| format!("转 JSON 失败: {}", e))
}
