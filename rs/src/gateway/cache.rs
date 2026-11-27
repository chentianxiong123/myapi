//! cache（从 gateway.rs 精确拆分）
use super::*;

/// 响应缓存条目
#[derive(Clone)]
pub struct CachedResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub cached_at: std::time::Instant,
}

/// 响应缓存 store（全局共享）
pub type ResponseCacheStore = std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, CachedResponse>>>;

pub fn new_response_cache() -> ResponseCacheStore {
    std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))
}

/// 缓存 key（学 LiteLLM cache-response：SHA256(model + messages)）
pub fn cache_key(model: &str, payload: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    if let Some(msgs) = payload.get("messages") {
        hasher.update(msgs.to_string().as_bytes());
    }
    let h = hasher.finalize();
    h.iter().map(|b| format!("{:02x}", b)).collect::<String>()
}

/// 查缓存（TTL 内有效）
pub async fn cache_get(store: &ResponseCacheStore, key: &str, ttl_seconds: u64) -> Option<CachedResponse> {
    let map = store.read().await;
    map.get(key).cloned().filter(|c| c.cached_at.elapsed().as_secs() < ttl_seconds)
}

/// 存缓存（超 max_entries 淘汰最旧）
pub async fn cache_put(store: &ResponseCacheStore, key: String, resp: CachedResponse, max_entries: usize) {
    let mut map = store.write().await;
    if map.len() >= max_entries {
        // 淘汰最旧（按 cached_at）
        if let Some(oldest) = map.iter().min_by_key(|(_, v)| v.cached_at).map(|(k, _)| k.clone()) {
            map.remove(&oldest);
        }
    }
    map.insert(key, resp);
}