//! 适配器公共逻辑（对照 adapters/base.py 的 diff）
use crate::core::ir::ProviderIR;

/// 统一品牌/网关标识（下游 agent 配置里的 provider 名、notes、cc-switch 匹配）。
/// 改名只需改这一处，避免硬编码散落各 adapter。
pub const DEFAULT_PROVIDER_ID: &str = "myapi";

pub fn base_diff(current: &Option<ProviderIR>, target: &ProviderIR, name: &str) -> Vec<String> {
    let mut lines = Vec::new();
    match current {
        None => lines.push(format!("  [{}] 当前无配置，将新建", name)),
        Some(cur) => {
            if cur.base_url != target.base_url {
                lines.push(format!("  [{}] base_url: {} -> {}", name, cur.base_url, target.base_url));
            }
            if cur.api_key != target.api_key {
                let ck = if cur.api_key.is_empty() { "(空)" } else { "****" };
                let tk = if target.api_key.is_empty() { "(空)" } else { "****" };
                lines.push(format!("  [{}] api_key: {} -> {}", name, ck, tk));
            }
            let cur_ids: std::collections::BTreeSet<String> = cur.models.iter().map(|m| m.id.clone()).collect();
            let tgt_ids: std::collections::BTreeSet<String> = target.models.iter().map(|m| m.id.clone()).collect();
            let added: Vec<&str> = tgt_ids.difference(&cur_ids).into_iter().map(|s| s.as_str()).collect();
            let removed: Vec<&str> = cur_ids.difference(&tgt_ids).into_iter().map(|s| s.as_str()).collect();
            if !added.is_empty() {
                lines.push(format!("  [{}] 新增模型: {}", name, added.join(", ")));
            }
            if !removed.is_empty() {
                lines.push(format!("  [{}] 移除模型: {}", name, removed.join(", ")));
            }
            let common: Vec<&str> = tgt_ids.intersection(&cur_ids).into_iter().map(|s| s.as_str()).collect();
            for mid in common {
                let cm = cur.models.iter().find(|m| m.id == *mid);
                let tm = target.models.iter().find(|m| m.id == *mid);
                if let (Some(cm), Some(tm)) = (cm, tm) {
                    if cm.context_window != tm.context_window {
                        lines.push(format!("  [{}] 模型 {}: contextWindow {} -> {}", name, mid, cm.context_window, tm.context_window));
                    }
                    if cm.max_tokens != tm.max_tokens {
                        lines.push(format!("  [{}] 模型 {}: maxTokens {} -> {}", name, mid, cm.max_tokens, tm.max_tokens));
                    }
                    if cm.input_modes != tm.input_modes {
                        lines.push(format!("  [{}] 模型 {}: input {} -> {}", name, mid, cm.input_modes.join("/"), tm.input_modes.join("/")));
                    }
                    if cm.reasoning != tm.reasoning {
                        lines.push(format!("  [{}] 模型 {}: reasoning {} -> {}", name, mid, cm.reasoning, tm.reasoning));
                    }
                }
            }
        }
    }
    lines
}

// ==================== 测试（base_diff 纯逻辑：新建/变更/模型增删）====================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ModelIR;

    fn ir(base_url: &str, key: &str, model_ids: &[&str]) -> ProviderIR {
        ProviderIR {
            provider_id: "myapi".into(),
            api: "openai-completions".into(),
            base_url: base_url.into(),
            api_key: key.into(),
            models: model_ids.iter().map(|id| ModelIR { id: id.to_string(), ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn diff_new_shows_create() {
        let target = ir("http://x/v1", "k", &["m1"]);
        let lines = base_diff(&None, &target, "pi");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("当前无配置，将新建"));
    }

    #[test]
    fn diff_no_change_is_empty() {
        let cur = ir("http://x/v1", "k", &["m1"]);
        let tgt = ir("http://x/v1", "k", &["m1"]);
        let lines = base_diff(&Some(cur), &tgt, "pi");
        assert!(lines.is_empty(), "无变更应空，实际: {:?}", lines);
    }

    #[test]
    fn diff_base_url_change() {
        let cur = ir("http://old/v1", "k", &["m1"]);
        let tgt = ir("http://new/v1", "k", &["m1"]);
        let lines = base_diff(&Some(cur), &tgt, "pi");
        assert!(lines.iter().any(|l| l.contains("base_url") && l.contains("old") && l.contains("new")));
    }

    #[test]
    fn diff_api_key_masks_secret() {
        let cur = ir("http://x/v1", "", &["m1"]);
        let tgt = ir("http://x/v1", "sk-secret", &["m1"]);
        let lines = base_diff(&Some(cur), &tgt, "pi");
        let key_line = lines.iter().find(|l| l.contains("api_key")).unwrap();
        assert!(key_line.contains("(空)"), "空 key 显示 (空)");
        assert!(!key_line.contains("sk-secret"), "不应泄漏新 key 明文");
    }

    #[test]
    fn diff_model_add_and_remove() {
        let cur = ir("http://x/v1", "k", &["m1", "m2"]);
        let tgt = ir("http://x/v1", "k", &["m1", "m3"]);
        let lines = base_diff(&Some(cur), &tgt, "pi");
        assert!(lines.iter().any(|l| l.contains("新增模型") && l.contains("m3")));
        assert!(lines.iter().any(|l| l.contains("移除模型") && l.contains("m2")));
    }

    #[test]
    fn diff_model_field_change() {
        let mut cur = ir("http://x/v1", "k", &["m1"]);
        cur.models[0].context_window = 100;
        let mut tgt = ir("http://x/v1", "k", &["m1"]);
        tgt.models[0].context_window = 200;
        let lines = base_diff(&Some(cur), &tgt, "pi");
        assert!(lines.iter().any(|l| l.contains("contextWindow") && l.contains("100") && l.contains("200")));
    }
}
