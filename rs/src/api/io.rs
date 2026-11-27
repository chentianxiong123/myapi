//! io（精确拆分）
use super::*;

pub fn root() -> PathBuf {
    // MYAPI_ROOT 环境变量可覆盖（Python 假测试框架用：隔离到临时目录，不污染真实配置）
    std::env::var("MYAPI_ROOT").map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub fn config_path() -> PathBuf { root().join("config").join("myapi-config.yaml") }

#[derive(Debug, Clone)]
pub struct GatewayCfg {
    pub listen: u16,
    pub panel_port: u16,
    pub key: String,
    pub upstreams: Vec<Value>,
}

/// 对应 _gateway_cfg()：读 config/gateway.yaml
/// yaml Value → json Value（配置层转换）
pub fn yaml_to_json(v: &serde_yaml::Value) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// json Value → yaml Value（写配置文件用）
pub fn json_to_yaml(v: &Value) -> serde_yaml::Value {
    serde_yaml::to_value(v).unwrap_or(serde_yaml::Value::Null)
}

/// 读 yaml 文件为 json Value（配置层统一入口）
pub fn read_yaml_json(path: &PathBuf) -> Value {
    std::fs::read_to_string(path).ok().and_then(|t| {
        serde_yaml::from_str::<serde_yaml::Value>(&t).ok().map(|v| yaml_to_json(&v))
    }).unwrap_or(Value::Null)
}

/// 写 json Value 为 yaml 文件
pub fn write_yaml_json(path: &PathBuf, v: &Value) -> std::io::Result<()> {
    let y = json_to_yaml(v);
    let text = serde_yaml::to_string(&y).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    std::fs::write(path, text)
}

pub fn gateway_cfg() -> GatewayCfg {
    let p = root().join("config").join("gateway.yaml");
    let raw = read_yaml_json(&p);
    let g = raw.get("gateway").cloned().unwrap_or(Value::Object(Default::default()));
    let mgmt = g.get("management").cloned().unwrap_or(Value::Object(Default::default()));
    GatewayCfg {
        listen: g.get("listen").and_then(|v| v.as_u64()).unwrap_or(8380) as u16,
        panel_port: g.get("panel_port").and_then(|v| v.as_u64()).unwrap_or(17834) as u16,
        key: mgmt.get("key").and_then(|v| v.as_str()).unwrap_or("").into(),
        upstreams: g.get("upstreams").and_then(|v| v.as_array())
            .map(|s| s.iter().cloned().collect()).unwrap_or_default(),
    }
}

/// 对应 _panel_cfg()
pub fn panel_cfg() -> (String, String) {
    let p = root().join("config").join("gateway.yaml");
    let raw = read_yaml_json(&p);
    let pl = raw.get("gateway").and_then(|g| g.get("panel")).cloned().unwrap_or(Value::Object(Default::default()));
    (
        pl.get("auth_key").and_then(|v| v.as_str()).unwrap_or("").into(),
        pl.get("bind").and_then(|v| v.as_str()).unwrap_or("127.0.0.1").into(),
    )
}