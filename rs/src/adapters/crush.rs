//! crush 适配器（对照 adapters/crush.py）
//! 同步 CPA 到 crushrc 的 marker 段（provider add / model add）
//!
//! 注意：本 adapter 与 pi/openclaw/dsh 完全同构——path/host 由 AdapterInstance 声明，
//! RemoteIO 自动决定本机文件还是 ssh/scp 远程。crush 只定义了自己的"渲染方言"
//! （crushrc 的 provider add 语法），不含任何主机专属逻辑。
use crate::core::ir::ProviderIR;
use crate::core::remote::RemoteIO;
use crate::adapters::{Adapter, AdapterInstance, Rendered, base_diff, DEFAULT_PROVIDER_ID};

const MYAPI_ID: &str = DEFAULT_PROVIDER_ID;
const MYAPI_TYPE: &str = "openai-compat"; // crush 旧 catwalk 值；main/catwalk v3 是 completions
const BEGIN_MARK: &str = "# --- myapi provider BEGIN ---";
const END_MARK: &str = "# --- myapi provider END ---";

pub struct CrushAdapter { pub path: String, pub io: RemoteIO }

impl CrushAdapter {
    pub fn new(inst: &AdapterInstance) -> Self {
        let path = inst.path.clone().unwrap_or_else(|| Self::default_path());
        CrushAdapter { path, io: RemoteIO::new(inst.host.as_deref(), inst.platform.as_deref()) }
    }
    fn default_path() -> String {
        if let Ok(p) = std::env::var("CRUSH_RC") { return p; }
        let home = std::env::var("USERPROFILE").ok().or_else(|| std::env::var("HOME").ok()).unwrap_or_default();
        format!("{}/.config/crush/crushrc", home)
    }
    fn iter(&self, instances: &[AdapterInstance]) -> Vec<(String, Box<dyn Adapter>)> {
        if instances.is_empty() {
            let i = AdapterInstance { name: "default".into(), ..Default::default() };
            vec![("default".into(), Box::new(CrushAdapter::new(&i)))]
        } else {
            instances.iter().map(|i| (i.name.clone(), Box::new(CrushAdapter::new(i)) as Box<dyn Adapter>)).collect()
        }
    }

    /// 对应 _loopback_url()：crush 与 myapi 同机，base_url 用回环
    fn loopback_url(url: &str) -> String {
        // http://192.168.31.213:8317/v1 -> http://127.0.0.1:8317/v1
        if let Some(rest) = url.strip_prefix("http://") {
            let port_path: String = rest.chars().skip_while(|c| c != &':' && c != &'/').collect();
            return format!("http://127.0.0.1{}", port_path);
        }
        if let Some(rest) = url.strip_prefix("https://") {
            let port_path: String = rest.chars().skip_while(|c| c != &':' && c != &'/').collect();
            return format!("http://127.0.0.1{}", port_path);
        }
        url.to_string()
    }

    /// 对应 _extract_segment()
    fn extract_segment(text: &str) -> Option<String> {
        let b = text.find(BEGIN_MARK)?;
        let e = text[b..].find(END_MARK)? + b;
        Some(text[b..e].to_string())
    }

    /// 对应 _flag()
    fn flag(line: &str, name: &str) -> String {
        let idx = match line.find(name) { Some(i) => i + name.len(), None => return String::new() };
        let rest = &line[idx..];
        let start = rest.trim_start().trim_start_matches('"').as_ptr() as usize;
        let end = rest.as_ptr() as usize + rest.len();
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'"' || bytes[i] == b'\t') { i += 1; }
        let mut j = i;
        while j < bytes.len() && bytes[j] != b'"' && bytes[j] != b' ' && bytes[j] != b'\t' { j += 1; }
        let _ = (start, end);
        String::from_utf8_lossy(&bytes[i..j]).to_string()
    }

    /// 对应 _segment_to_ir()
    fn segment_to_ir(seg: &str) -> Option<ProviderIR> {
        let mut base_url = String::new();
        let mut api_key = String::new();
        let mut models = Vec::new();
        for line in seg.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix(&format!("provider add {}", MYAPI_ID)) {
                let _ = rest;
                base_url = Self::flag(line, "--base-url");
                api_key = Self::flag(line, "--api-key");
            } else if line.starts_with(&format!("model add {}/", MYAPI_ID)) {
                let tail = &line[format!("model add {}/", MYAPI_ID).len()..];
                let mid = tail.split_whitespace().next().unwrap_or("");
                if mid.is_empty() { continue; }
                let cw = Self::flag(line, "--context-window").parse().unwrap_or(256000);
                let mt = Self::flag(line, "--default-max-tokens").parse().unwrap_or(16384);
                models.push(crate::core::ir::ModelIR {
                    id: mid.into(), context_window: cw, max_tokens: mt,
                    input_modes: vec!["text".into()], reasoning: false, extra: Default::default(),
                });
            }
        }
        if models.is_empty() && base_url.is_empty() { return None; }
        Some(ProviderIR {
            provider_id: MYAPI_ID.into(), api: "openai-completions".into(),
            base_url, api_key, models, meta: Default::default(),
        })
    }
}

impl Adapter for CrushAdapter {
    fn name(&self) -> &str { "crush" }
    fn iter_instances(&self, instances: &[AdapterInstance]) -> Vec<(String, Box<dyn Adapter>)> { self.iter(instances) }

    fn load_current(&self) -> Option<ProviderIR> {
        let text = self.io.read(&self.path).unwrap_or_default();
        let seg = Self::extract_segment(&text)?;
        Self::segment_to_ir(&seg)
    }

    fn diff(&self, current: &Option<ProviderIR>, target: &ProviderIR) -> Vec<String> {
        // crush 与 myapi 同机（回环），base_url 主机段差异不参与对比
        base_diff(current, target, self.name())
            .into_iter().filter(|l| !l.starts_with(&format!("  [{}] base_url:", self.name()))).collect()
    }

    /// 对应 render()：生成 crushrc marker 段
    fn render(&self, ir: &ProviderIR) -> Rendered {
        let mut lines = vec![BEGIN_MARK.to_string()];
        lines.push(format!(
            "provider add {} --type {} --base-url \"{}\" --api-key \"{}\"",
            MYAPI_ID, MYAPI_TYPE, Self::loopback_url(&ir.base_url), ir.api_key));
        // 统一只下发 3 个档位名（auto/pro/flash），调度在网关黑盒
        let cw = 200000;
        let mt = 8192;
        for t in ["auto", "pro", "flash"] {
            lines.push(format!(
                "model add {}/{} --name \"{}\" --context-window {} --default-max-tokens {}",
                MYAPI_ID, t, t, cw, mt));
        }
        // crush 的 large/small（大/小任务）映射档位：large→pro（质量）、small→flash（速度）
        lines.push(format!("model large {}/pro", MYAPI_ID));
        lines.push(format!("model small {}/flash", MYAPI_ID));
        lines.push(END_MARK.to_string());
        Rendered::Text(lines.join("\n"))
    }

    /// 对应 write()：替换 marker 段（保留其它内容）
    fn write(&self, rendered: &Rendered, backup: bool) -> Result<(), String> {
        if backup { self.io.backup(&self.path); }
        let seg = match rendered { Rendered::Text(t) => t.clone(), _ => return Err("crush 需要文本渲染".into()) };
        let cur = self.io.read(&self.path).unwrap_or_default();
        let new = if cur.contains(BEGIN_MARK) {
            let b = cur.find(BEGIN_MARK).unwrap_or(0);
            let e = cur[b..].find(END_MARK).map(|x| x + b + END_MARK.len()).unwrap_or(cur.len());
            format!("{}{}\n{}", &cur[..b], seg, &cur[e..])
        } else {
            let base = cur.trim_end();
            if base.is_empty() { format!("{}\n", seg) } else { format!("{}\n{}\n", base, seg) }
        };
        self.io.write(&self.path, &new).map_err(|e| e.to_string())
    }
}

// ==================== 测试（render 纯逻辑：命令行文本 + loopback_url）====================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ModelIR;
    use crate::adapters::{Adapter, AdapterInstance, Rendered};

    fn adapter() -> CrushAdapter {
        CrushAdapter::new(&AdapterInstance::default())
    }
    fn ir(base_url: &str, ids: &[&str]) -> ProviderIR {
        ProviderIR {
            provider_id: MYAPI_ID.into(),
            base_url: base_url.into(),
            api_key: "sk-crush".into(),
            models: ids.iter().map(|id| ModelIR { id: id.to_string(), ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn render_has_begin_end_marks() {
        let out = adapter().render(&ir("http://192.168.31.213:8317/v1", &["m1"]));
        match out {
            Rendered::Text(t) => {
                assert!(t.contains(BEGIN_MARK));
                assert!(t.contains(END_MARK));
            }
            _ => panic!("crush 应返 Text"),
        }
    }

    #[test]
    fn render_provider_add_line() {
        let out = adapter().render(&ir("http://192.168.31.213:8317/v1", &["m1"]));
        match out {
            Rendered::Text(t) => {
                assert!(t.contains(&format!("provider add {} --type {}", MYAPI_ID, MYAPI_TYPE)));
                assert!(t.contains("--api-key \"sk-crush\""));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_loopback_converts_ip() {
        // 远程 IP → 127.0.0.1
        let out = adapter().render(&ir("http://192.168.31.213:8317/v1", &["m1"]));
        match out {
            Rendered::Text(t) => {
                assert!(t.contains("http://127.0.0.1:8317/v1"), "IP 应转 loopback: {t}");
                assert!(!t.contains("192.168.31.213"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_model_lines_and_large_small() {
        let out = adapter().render(&ir("http://x:8317/v1", &["m1", "m2", "m3"]));
        match out {
            Rendered::Text(t) => {
                assert!(t.contains("model add myapi/auto"));
                assert!(t.contains("model add myapi/pro"));
                assert!(t.contains("model add myapi/flash"));
                assert!(t.contains("model large myapi/pro"), "large→pro（质量）");
                assert!(t.contains("model small myapi/flash"), "small→flash（速度）");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn loopback_url_direct() {
        assert_eq!(CrushAdapter::loopback_url("http://1.2.3.4:8317/v1"), "http://127.0.0.1:8317/v1");
        assert_eq!(CrushAdapter::loopback_url("https://1.2.3.4:8317/v1"), "http://127.0.0.1:8317/v1");
        assert_eq!(CrushAdapter::loopback_url("http://127.0.0.1:8317/v1"), "http://127.0.0.1:8317/v1");
    }
}
