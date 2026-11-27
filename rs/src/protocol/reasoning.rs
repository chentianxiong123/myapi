//! reasoning（精确拆分）
use super::*;

pub(crate) fn resolve_reasoning_effort(body: &Value) -> Option<&'static str> {
    // 学 cc-switch resolve_reasoning_effort：模型感知的精细映射。
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or_default();
    resolve_requested_reasoning_effort(body, model)
        .map(|effort| clamp_reasoning_effort_for_model(model, effort))
}

/// 优先级：
/// 1. 显式 `output_config.effort`（low/medium/high/xhigh/max；max 仅模型支持保留，否则回退 xhigh；thinking off 时 floor 到 low）
/// 2. `thinking.type` + `budget_tokens`（adaptive→xhigh；enabled+<4000→low/<16000→medium/其余→high；无 budget→high；disabled/缺→None）
pub(crate) fn resolve_requested_reasoning_effort(body: &Value, model: &str) -> Option<&'static str> {
    if let Some(effort) = body.pointer("/output_config/effort").and_then(|v| v.as_str()) {
        let mapped = match effort {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" if supports_max_reasoning_effort(model) => Some("max"),
            "max" => Some("xhigh"),
            _ => None, // 未知值不注入
        };
        // 学 cc-switch make_default_adaptive_thinking_explicit：Claude 5.x 省略 thinking = adaptive 默认（不算 off）；
        // 否则省略或显式 disabled 都视为 off → effort floor 到 low。
        let thinking_omitted = body.get("thinking").is_none_or(|t| t.is_null());
        let thinking_off = if thinking_omitted {
            !adaptive_thinking_is_default(model)
        } else {
            matches!(
                body.pointer("/thinking/type").and_then(|t| t.as_str()),
                Some("disabled")
            )
        };
        return if thinking_off {
            mapped.map(|_| "low")
        } else {
            mapped
        };
    }
    let thinking = body.get("thinking")?;
    match thinking.get("type").and_then(|t| t.as_str()) {
        Some("adaptive") => Some("xhigh"),
        Some("enabled") => {
            let budget = thinking.get("budget_tokens").and_then(|b| b.as_u64());
            match budget {
                Some(b) if b < 4_000 => Some("low"),
                Some(b) if b < 16_000 => Some("medium"),
                Some(_) => Some("high"),
                None => Some("high"), // enabled 无 budget — 假设强推理
            }
        }
        _ => None, // disabled 或缺失
    }
}

/// Claude 5.x 系模型省略 thinking 时默认 adaptive（学 cc-switch adaptive_thinking_is_default）
pub(crate) fn adaptive_thinking_is_default(model: &str) -> bool {
    let n = model.to_ascii_lowercase();
    ["fable-5", "mythos-5", "mythos-preview", "opus-5", "sonnet-5"]
        .iter()
        .any(|needle| n.contains(needle))
}

/// 模型是否支持独立的 max 推理档（学 cc-switch supports_max_reasoning_effort）
pub(crate) fn supports_max_reasoning_effort(model: &str) -> bool {
    let n = model.to_ascii_lowercase();
    matches!(n.as_str(), "gpt-5.6" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna")
        || matches!(n.as_str(), "gpt-6-astra" | "gpt-6-sol" | "gpt-6-luna")
}

/// 模型感知钳制（学 cc-switch clamp_reasoning_effort_for_model）
pub(crate) fn clamp_reasoning_effort_for_model(model: &str, effort: &'static str) -> &'static str {
    let n = model.to_ascii_lowercase();
    if n == "gpt-5-pro" || n.starts_with("gpt-5-pro-") {
        return "high";
    }
    let is_gpt_pro = n.starts_with("gpt-") && (n.ends_with("-pro") || n.contains("-pro-"));
    if is_gpt_pro && effort == "low" {
        return "medium";
    }
    effort
}