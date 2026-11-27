//! auth（精确拆分）
use super::super::*;

/// 下游鉴权配置（agent 调网关时校验 Bearer key，防裸奍）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 允许的下游 key 列表（空=不鉴权，本机内网可用；配了=严格校验 Bearer）。
    /// 支持两种形态：字符串 `sk-xxx`（无设备名）或对象 `{name: "手机", key: "sk-xxx"}`（多设备身份区分）。
    #[serde(default)]
    pub api_keys: Vec<AuthKey>,
    /// 每个 key 每日 token 上限（input+output+cache；None/0=不限）。超限返 429。学 LiteLLM max_budget
    #[serde(default)]
    pub daily_token_budget: Option<i64>,
}

/// 下游 key：裸字符串（无设备名）或带设备名对象。untagged 向后兼容 `api_keys: ["sk-1"]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthKey {
    Plain(String),
    Named { name: String, key: String },
}

impl AuthKey {
    pub(crate) fn key(&self) -> &str {
        match self { AuthKey::Plain(k) => k, AuthKey::Named { key, .. } => key }
    }
    pub(crate) fn device_name(&self) -> &str {
        match self { AuthKey::Plain(k) => k, AuthKey::Named { name, .. } => name }
    }
    pub(crate) fn matches(&self, token: &str) -> bool { self.key() == token }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { api_keys: Vec::new(), daily_token_budget: None }
    }
}

/// 校验下游 Bearer key（api_keys 空=不鉴权内网可用；配了=严格校验）。
/// 返回设备名（多设备身份区分：Named 的 name；Plain 用 key 本身）；未配返回 None。
pub fn validate_auth(auth: &AuthConfig, auth_header: Option<&str>) -> Result<Option<String>, String> {
    if auth.api_keys.is_empty() {
        return Ok(None); // 未配 key → 不鉴权（本机/内网）
    }
    let token = auth_header
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    if token.is_empty() {
        return Err("缺少 Authorization Bearer key".into());
    }
    for k in &auth.api_keys {
        if k.matches(token) {
            return Ok(Some(k.device_name().to_string()));
        }
    }
    Err("无效的 API key".into())
}

/// 取下游设备名（多设备身份区分：Named→name；Plain→key 本身；未配→原始 key 兜底）
pub(crate) fn downstream_key(auth: &AuthConfig, auth_header: Option<&str>) -> Option<String> {
    let token = auth_header.and_then(|h| h.strip_prefix("Bearer "))?;
    for k in &auth.api_keys {
        if k.matches(token) {
            return Some(k.device_name().to_string());
        }
    }
    Some(token.to_string())
}