//! 响应缓存（从 gateway.rs 拆出）
use super::*;

pub struct CachedResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
    pub cached_at: std::time::Instant,
}

pub fn new_response_cache() -> ResponseCacheStore {
    std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))
}

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

pub async fn cache_get(store: &ResponseCacheStore, key: &str, ttl_seconds: u64) -> Option<CachedResponse> {
    let map = store.read().await;
    map.get(key).cloned().filter(|c| c.cached_at.elapsed().as_secs() < ttl_seconds)
}

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
