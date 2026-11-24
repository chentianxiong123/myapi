//! IR: 中间表示（对应 core/ir.py）
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn d_ctx() -> u64 { 512000 }
fn d_max() -> u32 { 16384 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelIR {
    pub id: String,
    #[serde(default = "d_ctx")]
    pub context_window: u64,
    #[serde(default = "d_max")]
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, serde_json::Value>,
}

fn d_api() -> String { "openai-completions".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderIR {
    pub provider_id: String,
    #[serde(default = "d_api")]
    pub api: String,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<ModelIR>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub meta: HashMap<String, serde_json::Value>,
}

// ==================== 测试（ModelIR/ProviderIR Default + 序列化往返）====================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ir_defaults() {
        // serde default（d_ctx/d_max）才给 512000/16384；derive Default 给 0
        // 真实行为：反序列化空 map 触发 #[serde(default = "d_ctx")]
        let m: ModelIR = serde_json::from_str("{\"id\":\"gpt-x\"}").unwrap();
        assert_eq!(m.id, "gpt-x");
        assert_eq!(m.context_window, 512000, "serde 默认上下文窗口");
        assert_eq!(m.max_tokens, 16384, "serde 默认最大输出");
        assert!(!m.reasoning);
        assert!(m.input_modes.is_empty());
    }

    #[test]
    fn provider_ir_serialization_roundtrip() {
        let p = ProviderIR {
            provider_id: "myapi".into(),
            api: "openai-completions".into(),
            base_url: "http://x/v1".into(),
            api_key: "sk-1".into(),
            models: vec![ModelIR { id: "m1".into(), reasoning: true, ..Default::default() }],
            ..Default::default()
        };
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j.get("provider_id").and_then(|v| v.as_str()), Some("myapi"));
        assert_eq!(j.get("base_url").and_then(|v| v.as_str()), Some("http://x/v1"));
        let back: ProviderIR = serde_json::from_value(j).unwrap();
        assert_eq!(back.models[0].id, "m1");
        assert!(back.models[0].reasoning);
    }

    #[test]
    fn model_ir_skip_empty_fields() {
        // input_modes 空 / reasoning false / extra 空 → 不序列化
        let m = ModelIR { id: "m1".into(), ..Default::default() };
        let j = serde_json::to_value(&m).unwrap();
        assert!(j.get("input_modes").is_none(), "空 input_modes 应 skip");
        assert!(j.get("reasoning").is_none(), "false reasoning 应 skip");
        assert!(j.get("extra").is_none(), "空 extra 应 skip");
    }
}
