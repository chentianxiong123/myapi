//! adapter 模块（对照 adapters/ 包）
//! 通用抽象：Adapter trait + AdapterInstance（host/path 参数化，不写死用户目录）
pub mod base;
pub mod claude;
pub mod ccswitch;
pub mod codex;
pub mod crush;
pub mod dsh;
pub mod hermes;
pub mod omp;
pub mod openclaw;
pub mod opencode;
pub mod picoclaw;
pub mod pi;
pub mod factory;

use crate::core::ir::ProviderIR;
use serde_json::Value;

/// 渲染产物（对照 Python render() 返回 dict/str 的 Any）
pub enum Rendered {
    Json(Value),
    Yaml(serde_yaml::Value),
    Text(String),
}

impl Rendered {
    pub fn to_text(&self) -> String {
        match self {
            Rendered::Json(v) => { let mut t = serde_json::to_string_pretty(v).unwrap_or_default(); t.push('\n'); t }
            Rendered::Yaml(v) => serde_yaml::to_string(v).unwrap_or_default(),
            Rendered::Text(s) => s.clone(),
        }
    }
}

/// 对应 config.AdapterInstance：{name, host, path, platform}
#[derive(Debug, Clone, Default)]
pub struct AdapterInstance {
    pub name: String,
    pub host: Option<String>,
    pub path: Option<String>,
    pub platform: Option<String>,
    /// omp 深度裁剪：enabledModels 白名单
    pub enabled_models: Option<Vec<String>>,
    /// omp config.yml 路径（写 enabledModels 用）
    pub config_path: Option<String>,
}

/// 对应 adapters.base.Adapter
pub trait Adapter: Send {
    fn name(&self) -> &str;
    /// 对应 iter_instances()：从 adapter_instances 展开多实例
    fn iter_instances(&self, instances: &[AdapterInstance]) -> Vec<(String, Box<dyn Adapter>)>;
    /// 对应 load_current()
    fn load_current(&self) -> Option<ProviderIR>;
    /// 对应 diff()
    fn diff(&self, current: &Option<ProviderIR>, target: &ProviderIR) -> Vec<String>;
    /// 对应 render()
    fn render(&self, ir: &ProviderIR) -> Rendered;
    /// 对应 write()
    fn write(&self, rendered: &Rendered, backup: bool) -> Result<(), String>;
    /// 总是同步的副作用（如 omp enabledModels 写 config.yml）；默认无。
    /// 即使主配置无 diff（无变更）也会调用，确保深度裁剪始终下发。
    fn side_effects(&self) -> Result<(), String> { Ok(()) }
}

/// 对应 adapters.available_names()
pub fn available_names() -> Vec<String> {
    ["opencode", "pi", "omp", "dsh", "codex", "claude", "crush", "hermes", "picoclaw", "openclaw"]
        .into_iter().map(String::from).collect()
}

pub fn base_diff(current: &Option<ProviderIR>, target: &ProviderIR, name: &str) -> Vec<String> {
    base::base_diff(current, target, name)
}

/// 统一品牌/网关标识（下游 provider 名/notes/cc-switch 匹配）。改名只改 base.rs。
pub const DEFAULT_PROVIDER_ID: &str = base::DEFAULT_PROVIDER_ID;

// ==================== 测试（available_names + Rendered::to_text）====================
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn available_names_has_10_adapters() {
        let names = available_names();
        assert_eq!(names.len(), 10);
        assert!(names.contains(&"opencode".to_string()));
        assert!(names.contains(&"pi".to_string()));
        assert!(names.contains(&"omp".to_string()));
    }

    #[test]
    fn rendered_json_to_text() {
        let r = Rendered::Json(json!({"a": 1}));
        let t = r.to_text();
        assert!(t.contains("\"a\""));
        assert!(t.contains("1"));
    }

    #[test]
    fn rendered_text_passthrough() {
        let r = Rendered::Text("hello world".into());
        assert_eq!(r.to_text(), "hello world");
    }

    #[test]
    fn rendered_yaml_to_text() {
        let y: serde_yaml::Value = serde_yaml::from_str("key: value").unwrap();
        let r = Rendered::Yaml(y);
        let t = r.to_text();
        assert!(t.contains("key"));
        assert!(t.contains("value"));
    }

    #[test]
    fn default_provider_id_is_myapi() {
        assert_eq!(DEFAULT_PROVIDER_ID, "myapi");
    }
}
