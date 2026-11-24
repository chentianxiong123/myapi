//! adapter 工厂（对照 adapters 包的 iter_instances 分发）
use crate::core::config::{AdapterInstance, SyncConfig};
use super::{Adapter, AdapterInstance as AI};
use super::claude::ClaudeAdapter;
use super::codex::CodexAdapter;
use super::crush::CrushAdapter;
use super::dsh::DshAdapter;
use super::hermes::HermesAdapter;
use super::omp::OmpAdapter;
use super::openclaw::OpenclawAdapter;
use super::opencode::OpencodeAdapter;
use super::picoclaw::PicoclawAdapter;
use super::pi::PiAdapter;

/// 从 SyncConfig 的 adapter_instances 构造某 agent 的所有实例
pub fn build(cfg: &SyncConfig, agent: &str) -> Vec<(String, Box<dyn Adapter>)> {
    build_filtered(cfg, agent, &[])
}

/// 只构造 host 在 only_hosts 里的实例（空 = 不过滤）。中心下发选目标用。
pub fn build_filtered(cfg: &SyncConfig, agent: &str, only_hosts: &[String]) -> Vec<(String, Box<dyn Adapter>)> {
    let instances: Vec<AI> = cfg.adapter_instances.get(agent).cloned().unwrap_or_default().into_iter()
        .filter(|i| only_hosts.is_empty() || only_hosts.contains(&i.host))
        .map(|i| AI {
            name: i.name,
            // 通用语义：host 为空 = 本机模式（对照 Python 的 `self.host or None`）
            host: if i.host.is_empty() { None } else { Some(i.host) },
            path: if i.path.is_empty() { None } else { Some(i.path) },
            platform: i.platform,
            enabled_models: i.enabled_models,
            config_path: i.config_path,
        }).collect();
    match agent {
        "opencode" => OpencodeAdapter::new(&AI::default()).iter_instances(&instances),
        "pi" => PiAdapter::new(&AI::default()).iter_instances(&instances),
        "openclaw" => OpenclawAdapter::new(&AI::default()).iter_instances(&instances),
        "picoclaw" => PicoclawAdapter::new(&AI::default()).iter_instances(&instances),
        "crush" => CrushAdapter::new(&AI::default()).iter_instances(&instances),
        "dsh" => DshAdapter::new(&AI::default()).iter_instances(&instances),
        "hermes" => HermesAdapter::new(&AI::default()).iter_instances(&instances),
        "omp" => OmpAdapter::new(&AI::default()).iter_instances(&instances),
        "claude" => ClaudeAdapter::new(&AI::default()).iter_instances(&instances),
        "codex" => CodexAdapter::new(&AI::default()).iter_instances(&instances),
        _ => Vec::new(),
    }
}

// ==================== 测试（build 分发 + build_filtered 按 host 过滤）====================
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 造一个 SyncConfig：opencode 有 2 个实例（本机/h1），pi 有 1 个（h2）
    /// AdapterInstance/SyncConfig 无 Default，手填全字段
    fn inst(name: &str, host: &str) -> AdapterInstance {
        AdapterInstance {
            name: name.into(), host: host.into(), path: String::new(),
            platform: None, enabled_models: None, config_path: None,
        }
    }
    fn test_cfg() -> SyncConfig {
        let mut instances: HashMap<String, Vec<AdapterInstance>> = HashMap::new();
        instances.insert("opencode".into(), vec![inst("local", ""), inst("remote", "h1")]);
        instances.insert("pi".into(), vec![inst("pi1", "h2")]);
        SyncConfig {
            myapi: Default::default(), targets: vec![], strategy: "merge".into(),
            hosts: HashMap::new(), adapter_instances: instances, path: String::new(),
        }
    }

    #[test]
    fn build_expands_all_instances_for_agent() {
        let cfg = test_cfg();
        let insts = build(&cfg, "opencode");
        assert_eq!(insts.len(), 2, "opencode 应展开 2 个实例");
    }

    #[test]
    fn build_unknown_agent_returns_empty() {
        let cfg = test_cfg();
        let insts = build(&cfg, "nonexistent");
        assert!(insts.is_empty());
    }

    #[test]
    fn build_filtered_by_host() {
        let cfg = test_cfg();
        // 只选 host=h1 的（remote），本机（host=""）被过滤掉
        let insts = build_filtered(&cfg, "opencode", &["h1".to_string()]);
        assert_eq!(insts.len(), 1);
        assert_eq!(insts[0].0, "remote");
    }

    #[test]
    fn build_filtered_empty_keeps_all() {
        let cfg = test_cfg();
        let insts = build_filtered(&cfg, "opencode", &[]);
        assert_eq!(insts.len(), 2, "空 only_hosts = 不过滤");
    }
}
