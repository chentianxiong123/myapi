//! main.rs 集成测试（按主题拆分）

    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt; // collect 读 body
    use tower::ServiceExt; // oneshot
    use tempfile::TempDir;
    use crate::gateway::{hit_disable_keyword, record_outcome, HealthMap, CircuitState};


mod helpers;
mod panel_api;
mod tier_routing;
mod forward_failover;
mod stress_boundary;
mod multi_turn_stream;
mod protocol_endpoints;
mod routing_policy;
mod circuit_keyword;
mod thinking_caps;
mod channel_probe;