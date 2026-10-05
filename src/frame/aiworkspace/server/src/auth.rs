//! Caller identity. Identity is only ever what this module verified; no field
//! of a request body is consulted.

use aiworkspace_store::Caller;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, token: &str) -> Result<Caller, String>;
}

/// Standalone test mode: a fixed table of static tokens.
#[derive(Default, Deserialize)]
pub struct StaticTokens {
    /// `token → { principal, app_id? }`
    pub tokens: HashMap<String, StaticIdentity>,
}

#[derive(Clone, Deserialize)]
pub struct StaticIdentity {
    pub principal: String,
    #[serde(default)]
    pub app_id: Option<String>,
}

#[async_trait]
impl Authenticator for StaticTokens {
    async fn authenticate(&self, token: &str) -> Result<Caller, String> {
        match self.tokens.get(token) {
            Some(id) => Ok(Caller { principal: id.principal.clone(), app_id: id.app_id.clone() }),
            None => Err("invalid session token".to_string()),
        }
    }
}

/// System mode: session tokens issued by verify-hub, checked by the service
/// runtime. The gateway's generic `/kapi/<service>` forwarding does not
/// authenticate requests, so every method goes through this.
pub struct RuntimeAuth;

#[async_trait]
impl Authenticator for RuntimeAuth {
    async fn authenticate(&self, token: &str) -> Result<Caller, String> {
        use buckyos_api::{get_buckyos_api_runtime, validate_verify_hub_token_claims, TokenUse};
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("runtime unavailable: {e}"))?;
        let verified = runtime.verify_trusted_session_token(token).await.map_err(|e| format!("invalid session token: {e}"))?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session).map_err(|e| format!("invalid session token: {e}"))?;
        let principal = verified.sub.clone().filter(|s| !s.trim().is_empty()).ok_or("session token has no subject")?;
        Ok(Caller { principal, app_id: Some(claims.target.canonical_key()) })
    }
}
