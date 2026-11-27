//! 网关转发核心（照 LiteLLM router-core 建模）
//! 网关转发核心（照 LiteLLM router-core 建模，最小可跑通管道）
//!
//! 数据流：/v1/chat/completions → 选通道（按模型）→ 转发上游 → 流式回传
//! 智能路由（黏性/熔断/延迟感知）先留接口，管道通了再加。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use axum::response::IntoResponse;

// ==================== 数据结构（对接前端 types.ts 的 Provider/Channel）====================

/// 列出所有对外可用模型名（/v1/models 用）：聚合所有通道的 alias/name
pub fn list_models(config: &GatewayConfig) -> Vec<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in &config.providers {
        if p.disabled { continue; }
        for c in &p.channels {
            for m in &c.models {
                let external = m.alias.clone().unwrap_or_else(|| m.name.clone());
                if seen.insert(external.clone()) {
                    out.push(serde_json::json!({
                        "id": external,
                        "object": "model",
                        "owned_by": p.name,
                    }));
                }
            }
        }
    }
    out
}

mod config;
mod circuit;
mod routing;
mod cache;
mod usage;
mod forward;
mod sniff;
mod nginx;
pub(crate) use config::*;
pub(crate) use circuit::*;
pub(crate) use routing::*;
pub(crate) use cache::*;
pub(crate) use usage::*;
pub(crate) use forward::*;
pub(crate) use sniff::*;
pub(crate) use nginx::*;

#[cfg(test)]
mod tests;
