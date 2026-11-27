//! sniff（从 gateway.rs 精确拆分）
use super::*;

/// 主动健康探测（学 LiteLLM /health 主动 ping deployment）：轻量 GET {base_url}/models
/// 不花 token，只探通道上游可达性。返回每通道 live/dead + 延迟 + HTTP status。
/// 拉取上游模型列表（学 new-api FetchUpstreamModels）：GET base_url/models，解析 OpenAI 格式
/// 返回模型 id 列表（供面板自动填充，避免手填写错）
/// 按协议拼上游模型列表 URL（学 new-api fetchChannelUpstreamModelIDs）。
/// 只支持最通用的几个：openai_chat/openai_responses/anthropic→{base}/models（OpenAI 形），
/// gemini→{base}/v1beta/models，ollama→{base}/api/tags。其余（阿里/智谱等专门路径）不做。
pub fn upstream_models_url(base: &str, protocol: &str) -> String {
    let base = base.trim_end_matches('/');
    match protocol {
        "gemini" => format!("{}/v1beta/models", base),
        "ollama" => format!("{}/api/tags", base),
        _ => format!("{}/models", base),
    }
}

/// 按协议剥离模型名前缀（学 new-api TrimPrefix）：gemini 的 name 带 `models/` 前缀，剥掉。
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

/// 上游模型差异检测（学 new-api collectPendingUpstreamModelChangesFromModels）。
/// 纯手动：只算差异不写库，由调用方决定加/删。
/// - add    = upstream − local − ignored（ignored 支持 `regex:` 前缀正则）
/// - remove = local − upstream（本地有、上游已消失）
/// - ignored= 被忽略规则命中的上游新模型（提示但不加入）
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

/// 应用上游模型变更到 gateway.yaml（纯手动：只写调用方勾选的 add/remove）。
/// add 是完整 ChannelModel JSON（[{name, alias?, protocol?}, ...]），remove 是模型名。
/// 返回更新后的 providers 段。
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