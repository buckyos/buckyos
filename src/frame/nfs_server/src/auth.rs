//! Request authentication: turns the caller's verify-hub session token into an
//! [`Actor`]. The token comes from `Authorization: Bearer` (fetch-based
//! clients) or from the SSO cookies (`buckyos_session_token`, refreshed by
//! `/sso_refresh`, then the websdk's `<app_id>_token`) for `<img src>` and
//! downloads. No credentials means anonymous, which only reaches public paths.
//! The gateway forwards `/nfs/v1/*` unauthenticated, so this is the only check.

use crate::acl::Actor;
use crate::config::AuthMode;
use crate::error::{ErrorCode, NfsError, NfsResult};
use crate::state::AppState;
use axum::http::{header, HeaderMap};
use buckyos_api::TokenPrincipalKind;

#[derive(Debug, Default)]
pub struct Credentials {
    bearer: Option<String>,
    cookies: Vec<String>,
}

impl Credentials {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let mut cookies: Vec<(u8, String)> = Vec::new();
        for value in headers.get_all(header::COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            for pair in value.split(';') {
                if let Some((name, token)) = pair.trim().split_once('=') {
                    if name.ends_with("_token") && name != "buckyos_refresh_token" && !token.is_empty() {
                        cookies.push((cookie_rank(name), token.to_string()));
                    }
                }
            }
        }
        cookies.sort_by_key(|(rank, _)| *rank);
        Credentials { bearer, cookies: cookies.into_iter().map(|(_, t)| t).collect() }
    }

    pub fn bearer(&self) -> Option<&str> {
        self.bearer.as_deref()
    }
}

fn cookie_rank(name: &str) -> u8 {
    match name {
        "buckyos_session_token" => 0,
        "control-panel_token" => 1,
        _ => 2,
    }
}

fn unauthenticated(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::PermissionDenied, msg).with("reason", serde_json::json!("unauthenticated"))
}

/// An explicit Bearer token must be valid; stale cookies just fall back to
/// anonymous so public content keeps working for signed-out browsers.
pub async fn authenticate(state: &AppState, creds: &Credentials) -> NfsResult<Actor> {
    if matches!(state.config.auth, AuthMode::Disabled) {
        return Ok(Actor::Admin(None));
    }
    if let Some(token) = creds.bearer() {
        return verify(state, token).await;
    }
    for token in &creds.cookies {
        if let Ok(actor) = verify(state, token).await {
            return Ok(actor);
        }
    }
    Ok(Actor::Anonymous)
}

async fn verify(state: &AppState, token: &str) -> NfsResult<Actor> {
    let test_key = match &state.config.auth {
        AuthMode::TestKey { public_key, .. } => Some(public_key.as_str()),
        _ => None,
    };
    let claims = buckyos_api::verify_nfs_session_token(token, test_key)
        .await
        .map_err(|e| unauthenticated(format!("invalid session token: {}", e)))?;
    match claims.principal_kind {
        TokenPrincipalKind::User => {
            if is_zone_owner(state, &claims.subject).await {
                Ok(Actor::Admin(Some(claims.subject)))
            } else {
                Ok(Actor::User(claims.subject))
            }
        }
        // Zone services and zone devices (CLI on a node) are trusted, as in control-panel.
        TokenPrincipalKind::System | TokenPrincipalKind::Device => Ok(Actor::Admin(None)),
        other => Err(unauthenticated(format!(
            "{} sessions cannot access files directly",
            other.as_str()
        ))),
    }
}

pub(crate) async fn is_zone_owner(state: &AppState, user_id: &str) -> bool {
    match &state.config.auth {
        AuthMode::TestKey { zone_owner, .. } => zone_owner == user_id,
        AuthMode::Runtime => match zone_owner(state).await {
            Some(owner) => owner == user_id,
            None => false,
        },
        AuthMode::Disabled => true,
    }
}

/// `system/zone_owner_user_id`, cached once read successfully.
async fn zone_owner(state: &AppState) -> Option<String> {
    if let Some(owner) = state.zone_owner.read().unwrap().clone() {
        return Some(owner);
    }
    let runtime = buckyos_api::get_buckyos_api_runtime().ok()?;
    let client = runtime.get_system_config_client().await.ok()?;
    match client.get_zone_owner_user_id().await {
        Ok(owner) => {
            *state.zone_owner.write().unwrap() = Some(owner.clone());
            Some(owner)
        }
        Err(e) => {
            log::warn!("read zone owner failed: {:?}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_sources() {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        h.insert(
            header::COOKIE,
            "x=1; app_token=t3; control-panel_token=t2; empty_token=; buckyos_session_token=t1".parse().unwrap(),
        );
        let c = Credentials::from_headers(&h);
        assert_eq!(c.bearer(), Some("abc"));
        assert_eq!(c.cookies, vec!["t1".to_string(), "t2".to_string(), "t3".to_string()]);
        assert!(Credentials::from_headers(&HeaderMap::new()).bearer().is_none());
    }
}
