//! config_io（精确拆分）
use super::*;

pub(crate) fn root() -> PathBuf {
    // MYAPI_ROOT 环境变量可覆盖（Python 假测试框架用：隔离到临时目录，不污染真实 rs/config/）
    std::env::var("MYAPI_ROOT").map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// gateway.yaml 路径（网关配置，热加载：网关每请求重读）
pub(crate) fn gateway_yaml(root: &Path) -> PathBuf { root.join("config").join("gateway.yaml") }

/// 读 gateway.yaml 某一段（providers/routing/proxypools…），缺段返回空
pub(crate) fn read_gateway_section(root: &Path, section: &str) -> Value {
    let doc = api::read_yaml_json(&gateway_yaml(root));
    doc.get(section).cloned().unwrap_or(Value::Null)
}

/// 写 gateway.yaml 某一段（保留其他段）
pub(crate) fn write_gateway_section(root: &Path, section: &str, val: Value) -> std::io::Result<()> {
    let mut doc = api::read_yaml_json(&gateway_yaml(root));
    if !doc.is_object() {
        doc = serde_json::json!({});
    }
    if let Some(obj) = doc.as_object_mut() {
        obj.insert(section.to_string(), val);
    }
    api::write_yaml_json(&gateway_yaml(root), &doc)
}

pub(crate) fn chrono_now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{:02}:{:02}:{:02}Z", h, m, s)
}

pub(crate) fn gateway_yaml_path(root: &PathBuf) -> PathBuf { root.join("config").join("gateway.yaml") }

/// 读 auth.api_keys（AuthKey Plain/Named → [{name, key}]）
pub(crate) fn read_auth_keys(root: &PathBuf) -> Vec<Value> {
    let path = gateway_yaml_path(root);
    let doc = api::read_yaml_json(&path);
    let arr = doc.pointer("/auth/api_keys").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    arr.iter().map(|k| {
        if let Some(name) = k.get("name").and_then(|v| v.as_str()) {
            let key = k.get("key").and_then(|v| v.as_str()).unwrap_or("");
            json!({"name": name, "key": key})
        } else {
            let key = k.as_str().unwrap_or("");
            json!({"name": key, "key": key})
        }
    }).collect()
}

/// 写 auth.api_keys 回 gateway.yaml（保留其他段）
pub(crate) fn write_auth_keys(root: &PathBuf, keys: &[Value]) -> std::io::Result<()> {
    let path = gateway_yaml_path(root);
    let mut doc = api::read_yaml_json(&path);
    if !doc.is_object() { doc = json!({}); }
    if let Some(obj) = doc.as_object_mut() {
        let arr: Vec<Value> = keys.iter().map(|k| {
            json!({"name": k.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                   "key": k.get("key").and_then(|v| v.as_str()).unwrap_or("")})
        }).collect();
        let auth = obj.entry("auth").or_insert_with(|| json!({}));
        if let Some(a) = auth.as_object_mut() {
            a.insert("api_keys".into(), json!(arr));
        }
    }
    api::write_yaml_json(&path, &doc)
}