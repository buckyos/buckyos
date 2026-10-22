//! Caller identity. Owner methods take zone session tokens; protocol reads take a session
//! token (same zone) or a self-signed reader proof (other zones, §4.4 读者身份), or nothing
//! (anonymous). Only what this module verified counts as identity.

use crate::audience::Reader;
use crate::directory::Directory;
use crate::error::{denied, HsError, HsResult};
use crate::sign::{decode_unverified, kid_did, verify_signature, Signer};
use crate::{new_id, now_s};
use async_trait::async_trait;
use buckyos_api::{
    token_is_external, token_session_ceiling, validate_verify_hub_token_claims, AccessDenial, ControlPanelClient, DenyReason, SessionScope,
    TokenPrincipalKind, TokenUse,
};
use kRPC::{RPCErrors, RPCSessionToken};
use name_lib::DID;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct Caller {
    /// Session `sub`: the zone user id, or the DID of an External session.
    pub principal: String,
    /// The DID the session speaks for (contract §2.1): bound by `users/<id>/doc` for an
    /// account, the subject itself for an External session.
    pub did: String,
    pub app_id: Option<String>,
    /// Restricted session (Limited, External): reads and comments only.
    pub restricted: bool,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, token: &str) -> HsResult<Caller>;
}

#[derive(Default, Deserialize)]
pub struct StaticTokens {
    /// `token → { principal, did, app_id?, restricted? }`
    pub tokens: HashMap<String, StaticIdentity>,
}

#[derive(Clone, Deserialize)]
pub struct StaticIdentity {
    pub principal: String,
    pub did: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub restricted: bool,
}

#[async_trait]
impl Authenticator for StaticTokens {
    async fn authenticate(&self, token: &str) -> HsResult<Caller> {
        match self.tokens.get(token) {
            Some(id) => Ok(Caller { principal: id.principal.clone(), did: id.did.clone(), app_id: id.app_id.clone(), restricted: id.restricted }),
            None => Err(HsError::Unauthorized("invalid session token".to_string())),
        }
    }
}

/// Account → DID by `users/<id>/doc`, the only trusted binding; the profile the user edits
/// never decides who a session is.
#[async_trait]
pub trait AccountDids: Send + Sync {
    async fn account_did(&self, user_id: &str) -> Result<DID, RPCErrors>;
}

#[async_trait]
impl AccountDids for ControlPanelClient {
    async fn account_did(&self, user_id: &str) -> Result<DID, RPCErrors> {
        self.get_user_did(user_id).await
    }
}

/// The zone's accounts as the service runtime reads them.
pub struct RuntimeAccounts;

#[async_trait]
impl AccountDids for RuntimeAccounts {
    async fn account_did(&self, user_id: &str) -> Result<DID, RPCErrors> {
        buckyos_api::get_buckyos_api_runtime()?.get_control_panel_client().await?.get_user_did(user_id).await
    }
}

/// Whom a verified verify-hub session speaks for (contract §2.1): an account (or an App
/// acting for it) as the DID of its `users/<id>/doc`, an External session as its subject.
/// A session that resolves no DID is refused, never read as anonymous.
pub async fn session_caller(token: &RPCSessionToken, accounts: &dyn AccountDids) -> HsResult<Caller> {
    let claims = validate_verify_hub_token_claims(token, TokenUse::Session).map_err(session_error)?;
    let restricted = token_session_ceiling(token).map_err(session_error)?.restricted;
    let principal = token.sub.clone().filter(|s| !s.trim().is_empty()).ok_or_else(|| HsError::Unauthorized("session token has no subject".into()))?;
    let did = match claims.principal_kind {
        TokenPrincipalKind::User if token_is_external(token) => {
            name_lib::parse_canonical_did(&principal).map_err(|_| HsError::Unauthorized("external session subject is not a DID".into()))?;
            principal.clone()
        }
        TokenPrincipalKind::User | TokenPrincipalKind::App => match accounts.account_did(&principal).await {
            Ok(did) => did.to_string(),
            Err(RPCErrors::KeyNotExist(_)) => return Err(denied(DenyReason::AccountState, "session")),
            Err(e) => {
                log::warn!("DID of account {principal} unavailable: {e}");
                return Err(denied(DenyReason::AuthorizationUnavailable, "session"));
            }
        },
        _ => return Err(HsError::Unauthorized("this kind of session has no reader identity".into())),
    };
    Ok(Caller { principal, did, app_id: Some(claims.target.canonical_key()), restricted })
}

/// A denial (restricted session beyond its ceiling) keeps its reason; anything else is an
/// invalid session.
fn session_error(e: RPCErrors) -> HsError {
    match AccessDenial::from_rpc_error(&e) {
        Some(denial) => HsError::Denied(denial),
        None => HsError::Unauthorized(format!("invalid session token: {e}")),
    }
}

/// System mode: verify-hub session tokens, restricted ones included for the HomeStation
/// scope (contract §3.3); `Host` keeps restricted callers to reading and commenting.
pub struct RuntimeAuth {
    accounts: Box<dyn AccountDids>,
    /// User ids are never reused, so the DID their doc binds is kept once read.
    dids: Mutex<HashMap<String, DID>>,
}

impl RuntimeAuth {
    pub fn new(accounts: Box<dyn AccountDids>) -> Self {
        Self { accounts, dids: Mutex::new(HashMap::new()) }
    }
}

impl Default for RuntimeAuth {
    fn default() -> Self {
        Self::new(Box::new(RuntimeAccounts))
    }
}

#[async_trait]
impl AccountDids for RuntimeAuth {
    async fn account_did(&self, user_id: &str) -> Result<DID, RPCErrors> {
        if let Some(did) = self.dids.lock().unwrap().get(user_id).cloned() {
            return Ok(did);
        }
        let did = self.accounts.account_did(user_id).await?;
        self.dids.lock().unwrap().insert(user_id.to_string(), did.clone());
        Ok(did)
    }
}

#[async_trait]
impl Authenticator for RuntimeAuth {
    async fn authenticate(&self, token: &str) -> HsResult<Caller> {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| HsError::Unavailable(format!("runtime unavailable: {e}")))?;
        let verified = runtime.verify_session_token_opened(token, SessionScope::HomeStation).await.map_err(session_error)?;
        session_caller(&verified, self).await
    }
}

pub const READER_PROOF_TTL: u64 = 300;

/// A short-lived statement "I am `iss` reading `aud`", signed by a key `iss` authorizes.
pub fn make_reader_proof(signer: &Signer, reader: &str, target_zone: &str) -> Result<String, String> {
    let iat = now_s();
    let claims = json!({
        "iss": reader,
        "aud": format!("cyfs://{target_zone}/home"),
        "iat": iat,
        "exp": iat + READER_PROOF_TTL,
        "jti": new_id("r"),
    });
    signer.sign(&claims)
}

pub async fn verify_reader_proof(directory: &dyn Directory, jwt: &str, own_zone: &str) -> Result<String, String> {
    let decoded = decode_unverified(jwt)?;
    let iss = decoded.claims.get("iss").and_then(Value::as_str).ok_or("reader proof has no iss")?.to_string();
    let aud = decoded.claims.get("aud").and_then(Value::as_str).unwrap_or_default();
    if aud != format!("cyfs://{own_zone}/home") {
        return Err("reader proof is for another zone".into());
    }
    let iat = decoded.claims.get("iat").and_then(Value::as_u64).ok_or("reader proof has no iat")?;
    let exp = decoded.claims.get("exp").and_then(Value::as_u64).ok_or("reader proof has no exp")?;
    let now = now_s();
    if exp < now || iat > now + 60 || exp.saturating_sub(iat) > 2 * READER_PROOF_TTL {
        return Err("reader proof expired or too long-lived".into());
    }
    let signer = kid_did(&decoded.kid).to_string();
    if !directory.authorizes(&iss, &signer, &decoded.kid).await.map_err(|e| e.to_string())? {
        return Err(format!("{signer} may not sign for {iss}"));
    }
    let key = directory.signer_key(&signer, &decoded.kid).await.map_err(|e| e.to_string())?;
    verify_signature(jwt, &key)?;
    Ok(iss)
}

/// Reader of a protocol request: owner session, another zone user, a reader proof, or anonymous.
pub async fn reader_from_request(
    auth: &dyn Authenticator,
    directory: &dyn Directory,
    owner: &str,
    zone: &str,
    authorization: Option<&str>,
    access_token: Option<&str>,
) -> HsResult<Reader> {
    if let Some(value) = authorization {
        if let Some(proof) = value.strip_prefix("DID ") {
            let did = verify_reader_proof(directory, proof.trim(), zone).await.map_err(HsError::Unauthorized)?;
            return Ok(if did == owner { Reader::Owner } else { Reader::Did(did) });
        }
        if let Some(token) = value.strip_prefix("Bearer ") {
            return session_reader(auth, owner, token.trim()).await;
        }
        return Err(HsError::Unauthorized("unsupported authorization scheme".into()));
    }
    if let Some(token) = access_token {
        return session_reader(auth, owner, token).await;
    }
    Ok(Reader::Anonymous)
}

async fn session_reader(auth: &dyn Authenticator, owner: &str, token: &str) -> HsResult<Reader> {
    let caller = auth.authenticate(token).await?;
    Ok(if caller.did == owner { Reader::Owner } else { Reader::Did(caller.did) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use axum::{Json, Router};
    use buckyos_api::{
        bind_token_ceiling, bind_token_external, bind_token_principal_kind, bind_token_target, AuthTarget, SessionCeiling, SystemConfigClient,
        SystemServiceId, UserType, VERIFY_HUB_TOKEN_ISSUER,
    };
    use kRPC::RPCSessionTokenType;
    use std::sync::Arc;

    fn session(sub: &str, kind: TokenPrincipalKind, ceiling: &SessionCeiling) -> RPCSessionToken {
        let mut token = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            appid: None,
            jti: None,
            aud: None,
            sub: Some(sub.to_string()),
            token: None,
            iss: Some(VERIFY_HUB_TOKEN_ISSUER.to_string()),
            exp: None,
            sudo: false,
            extra: HashMap::new(),
        };
        bind_token_principal_kind(&mut token, kind);
        bind_token_target(&mut token, &AuthTarget::system(SystemServiceId::parse("control-panel").unwrap()), TokenUse::Session).unwrap();
        bind_token_ceiling(&mut token, ceiling);
        token
    }

    /// A system config answering `sys_config_get` / `sys_config_list` from `entries`.
    async fn system_config(entries: &[(&str, Value)]) -> Arc<SystemConfigClient> {
        let entries: Arc<HashMap<String, String>> = Arc::new(entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
        let app = Router::new().route(
            "/kapi/system_config",
            post(move |Json(req): Json<Value>| {
                let entries = entries.clone();
                async move {
                    let key = req["params"]["key"].as_str().unwrap_or_default().to_string();
                    let result = match req["method"].as_str() {
                        Some("sys_config_get") => entries.get(&key).map(|v| json!({ "value": v, "version": 1 })).unwrap_or(Value::Null),
                        Some("sys_config_list") => {
                            let mut children: Vec<String> = entries
                                .keys()
                                .filter_map(|k| k.strip_prefix(&format!("{key}/")))
                                .filter_map(|rest| rest.split('/').next())
                                .map(str::to_string)
                                .collect();
                            children.sort();
                            children.dedup();
                            json!(children)
                        }
                        _ => Value::Null,
                    };
                    Json(json!({ "result": result, "sys": [req["sys"][0].clone()] }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/kapi/system_config", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Arc::new(SystemConfigClient::new(Some(&url), None))
    }

    fn reason(result: HsResult<Caller>) -> DenyReason {
        match result {
            Err(HsError::Denied(denial)) => denial.reason,
            other => panic!("expected a denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn account_sessions_speak_for_the_did_their_doc_binds() {
        let client = system_config(&[
            ("users/carol/doc", json!({ "id": "did:bns:carol" })),
            // The profile the account edits does not decide who it is.
            ("users/carol/profile", json!({ "did": "did:bns:mallory", "name": "Carol" })),
        ])
        .await;
        let accounts = ControlPanelClient::from_shared(client);
        let limited = session("carol", TokenPrincipalKind::User, &SessionCeiling::for_user_type(UserType::Limited));
        let caller = session_caller(&limited, &accounts).await.unwrap();
        assert_eq!(caller.did, "did:bns:carol");
        assert_eq!(caller.principal, "carol");
        assert!(caller.restricted);
        let user = session("carol", TokenPrincipalKind::User, &SessionCeiling::unrestricted());
        assert!(!session_caller(&user, &accounts).await.unwrap().restricted);
        let unknown = session("dave", TokenPrincipalKind::User, &SessionCeiling::unrestricted());
        assert_eq!(reason(session_caller(&unknown, &accounts).await), DenyReason::AccountState);
    }

    #[tokio::test]
    async fn an_unreachable_account_record_refuses_rather_than_reading_anonymous() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/kapi/system_config", closed.local_addr().unwrap());
        drop(closed);
        let accounts = ControlPanelClient::new(SystemConfigClient::new(Some(&url), None));
        let token = session("carol", TokenPrincipalKind::User, &SessionCeiling::unrestricted());
        assert_eq!(reason(session_caller(&token, &accounts).await), DenyReason::AuthorizationUnavailable);
    }

    struct NoAccounts;

    #[async_trait]
    impl AccountDids for NoAccounts {
        async fn account_did(&self, user_id: &str) -> Result<DID, RPCErrors> {
            panic!("{user_id} is not an account here")
        }
    }

    #[tokio::test]
    async fn external_sessions_speak_for_their_subject_did() {
        let mut external = session("did:bns:alice", TokenPrincipalKind::User, &SessionCeiling::external());
        bind_token_external(&mut external);
        let caller = session_caller(&external, &NoAccounts).await.unwrap();
        assert_eq!(caller.did, "did:bns:alice");
        assert!(caller.restricted);

        let mut not_a_did = session("alice", TokenPrincipalKind::User, &SessionCeiling::external());
        bind_token_external(&mut not_a_did);
        assert!(matches!(session_caller(&not_a_did, &NoAccounts).await, Err(HsError::Unauthorized(_))));
        let service = session("kernel", TokenPrincipalKind::System, &SessionCeiling::unrestricted());
        assert!(matches!(session_caller(&service, &NoAccounts).await, Err(HsError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn zone_users_carry_the_did_of_their_doc() {
        let settings = |id: &str, user_type: &str| json!({ "user_id": id, "type": user_type, "password": "", "state": "active", "res_pool_id": "default" });
        let client = system_config(&[
            ("users/carol/doc", json!({ "id": "did:bns:carol" })),
            ("users/carol/profile", json!({ "did": "did:bns:bob", "name": "Carol" })),
            ("users/carol/settings", settings("carol", "limited")),
            ("users/bob/doc", json!({ "id": "did:bns:bob" })),
            ("users/bob/settings", settings("bob", "user")),
        ])
        .await;
        let users = crate::users::load_users(client).await.unwrap();
        let carol = users.iter().find(|u| u.user == "carol").unwrap();
        assert_eq!(carol.did, "did:bns:carol");
        assert_eq!(carol.name, "Carol");
        assert!(!carol.zone_feed_writer);
        let bob = users.iter().find(|u| u.user == "bob").unwrap();
        assert_eq!((bob.did.as_str(), bob.name.as_str()), ("did:bns:bob", "bob"));
    }
}
