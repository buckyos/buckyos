//! Caller identity. Identity is only ever what this module verified; no field
//! of a request body is consulted. Besides the token, the identity backend answers what grant matching needs per
//! request: membership of `group:` / `contact:` subjects and the Zone's sign-in switches.

use aiworkspace_store::workspace::{CONTACT_PREFIX, GROUP_PREFIX};
use aiworkspace_store::{Caller, CallerClass};
use async_trait::async_trait;
use buckyos_api::{AccessDenial, DenyReason, SsoSettings};
use serde::Deserialize;
use std::collections::HashMap;

/// Why a token was not accepted: it is no session of this Zone, or a session this service refuses (D-4).
#[derive(Debug)]
pub enum AuthError {
    Invalid(String),
    Denied(AccessDenial),
}

fn unavailable() -> AccessDenial {
    AccessDenial::new(DenyReason::AuthorizationUnavailable).at("aiworkspace", "*")
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, token: &str) -> Result<Caller, AuthError>;
    /// Principals a grant can name (phase two §6.3 "主体选择"): users of the zone, or the static
    /// identities in standalone mode. Best effort; an empty list means "type the subject yourself".
    async fn subjects(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }
    /// Settle the caller's membership of the `group:` / `contact:` `subjects` a decision may depend on, now (never
    /// from a cache): members go into the caller, a check that fails puts the subject into `caller.unconfirmed`.
    async fn resolve_subjects(&self, caller: &mut Caller, subjects: &[String]);
    /// Make sure an account caller carries its DID (writing a `contact:` grant needs it); `Err` when it cannot be read.
    async fn load_did(&self, _caller: &mut Caller) -> Result<(), AccessDenial> {
        Ok(())
    }
    /// The Zone's sign-in switches (`system/sso/settings`, defaults when absent).
    async fn sso_settings(&self) -> Result<SsoSettings, AccessDenial>;
}

/// Standalone test mode: a fixed table of static tokens. Memberships (`groups`, `contacts`) are read again from
/// `source` at each request, as the Zone reads them from system-config and msg-center.
#[derive(Default, Deserialize)]
pub struct StaticTokens {
    /// `token → { principal, app_id?, class?, restricted?, did?, groups?, contacts?, contacts_unavailable?, auth_method? }`
    pub tokens: HashMap<String, StaticIdentity>,
    /// The Zone's sign-in switches.
    #[serde(default)]
    pub sso: SsoSettings,
    #[serde(skip)]
    pub source: Option<std::path::PathBuf>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StaticClass {
    #[default]
    Account,
    /// The principal is the DID.
    External,
    Service,
}

#[derive(Clone, Deserialize)]
pub struct StaticIdentity {
    pub principal: String,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub class: StaticClass,
    /// An account session with a ceiling (a Limited account).
    #[serde(default)]
    pub restricted: bool,
    #[serde(default)]
    pub did: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    /// `contact:<owner_did>#<tag>` subjects the identity belongs to.
    #[serde(default)]
    pub contacts: Vec<String>,
    /// Every contact check fails, as with an unreachable msg-center.
    #[serde(default)]
    pub contacts_unavailable: bool,
    #[serde(default)]
    pub auth_method: Option<String>,
}

#[async_trait]
impl Authenticator for StaticTokens {
    async fn authenticate(&self, token: &str) -> Result<Caller, AuthError> {
        let id = self.tokens.get(token).ok_or_else(|| AuthError::Invalid("invalid session token".to_string()))?;
        let mut caller = match id.class {
            StaticClass::Account => {
                let mut c = Caller::account(&id.principal, id.restricted);
                c.class = CallerClass::Account { restricted: id.restricted, did: id.did.clone(), groups: id.groups.clone() };
                c
            }
            StaticClass::External => Caller::external(&id.principal),
            StaticClass::Service => Caller::service(&id.principal),
        };
        caller.app_id = id.app_id.clone();
        caller.auth_method = id.auth_method.clone();
        Ok(caller)
    }
    async fn subjects(&self) -> Vec<serde_json::Value> {
        let mut names: Vec<(&String, &str)> = self
            .tokens
            .values()
            .map(|i| (&i.principal, if i.class == StaticClass::External { "external" } else { "user" }))
            .collect();
        names.sort();
        names.dedup();
        names.into_iter().map(|(n, kind)| serde_json::json!({ "subject": n, "kind": kind })).collect()
    }
    async fn resolve_subjects(&self, caller: &mut Caller, subjects: &[String]) {
        let fresh: Option<StaticTokens> = self.source.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok());
        let table = fresh.as_ref().unwrap_or(self);
        let Some(id) = table.tokens.values().find(|i| i.principal == caller.principal) else { return };
        if let CallerClass::Account { groups, .. } = &mut caller.class {
            if subjects.iter().any(|s| s.starts_with(GROUP_PREFIX)) {
                *groups = id.groups.clone();
            }
        }
        for subject in subjects.iter().filter(|s| s.starts_with(CONTACT_PREFIX)) {
            if id.contacts_unavailable {
                caller.unconfirmed.insert(subject.clone());
            } else if id.contacts.contains(subject) {
                caller.contacts.insert(subject.clone());
            }
        }
    }
    async fn sso_settings(&self) -> Result<SsoSettings, AccessDenial> {
        Ok(self.sso.clone())
    }
}

/// System mode: session tokens issued by verify-hub, checked by the service
/// runtime. The gateway's generic `/kapi/<service>` forwarding does not
/// authenticate requests, so every method goes through this.
pub struct RuntimeAuth;

#[async_trait]
impl Authenticator for RuntimeAuth {
    /// Restricted sessions (Limited, External) are accepted when their ceiling includes `aiworkspace`; what they
    /// reach is then decided by grants (and they never match `*` or own anything).
    async fn authenticate(&self, token: &str) -> Result<Caller, AuthError> {
        use buckyos_api::{
            get_buckyos_api_runtime, token_auth_method, token_is_external, token_session_ceiling, validate_verify_hub_token_claims,
            SessionScope, TokenPrincipalKind, TokenUse,
        };
        let runtime = get_buckyos_api_runtime().map_err(|e| AuthError::Invalid(format!("runtime unavailable: {e}")))?;
        let verified = runtime.verify_session_token_opened(token, SessionScope::AiWorkspace).await.map_err(|e| match AccessDenial::from_rpc_error(&e) {
            Some(denial) => AuthError::Denied(denial),
            None => AuthError::Invalid(format!("invalid session token: {e}")),
        })?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session).map_err(|e| AuthError::Invalid(format!("invalid session token: {e}")))?;
        let ceiling = token_session_ceiling(&verified).map_err(|e| AuthError::Invalid(format!("invalid session token: {e}")))?;
        let principal = verified.sub.clone().filter(|s| !s.trim().is_empty()).ok_or_else(|| AuthError::Invalid("session token has no subject".to_string()))?;
        let mut caller = match claims.principal_kind {
            TokenPrincipalKind::User if token_is_external(&verified) => Caller::external(&principal),
            TokenPrincipalKind::User => Caller::account(&principal, ceiling.restricted),
            _ => Caller::service(&principal),
        };
        caller.app_id = Some(claims.target.canonical_key());
        caller.auth_method = token_auth_method(&verified).map(|m| m.as_str().to_string());
        Ok(caller)
    }
    /// Zone users from system-config `users/` and the system user groups (whether a non-owner may read those lists
    /// is decided by system-config; a refusal just leaves them out).
    async fn subjects(&self) -> Vec<serde_json::Value> {
        use buckyos_api::get_buckyos_api_runtime;
        let Ok(runtime) = get_buckyos_api_runtime() else { return Vec::new() };
        let Ok(client) = runtime.get_control_panel_client().await else { return Vec::new() };
        let mut out: Vec<serde_json::Value> = match client.get_user_list().await {
            Ok(users) => users.into_iter().map(|u| serde_json::json!({ "subject": u, "kind": "user" })).collect(),
            Err(e) => {
                log::debug!("user list unavailable: {:?}", e);
                Vec::new()
            }
        };
        match client.load_user_groups().await {
            Ok(dir) => out.extend(dir.groups.into_iter().map(|g| serde_json::json!({ "subject": format!("{GROUP_PREFIX}{}", g.group_id), "kind": "group", "name": g.name }))),
            Err(e) => log::debug!("user groups unavailable: {:?}", e),
        }
        out
    }
    /// Groups from `users/<id>/settings`, the account's DID from `users/<id>/doc` (never the profile), contact
    /// membership from msg-center `contact.check_membership`.
    async fn resolve_subjects(&self, caller: &mut Caller, subjects: &[String]) {
        use buckyos_api::{get_buckyos_api_runtime, ContactCheckMembershipReq};
        let groups: Vec<&String> = subjects.iter().filter(|s| s.starts_with(GROUP_PREFIX)).collect();
        let contacts: Vec<&String> = subjects.iter().filter(|s| s.starts_with(CONTACT_PREFIX)).collect();
        if groups.is_empty() && contacts.is_empty() {
            return;
        }
        let runtime = get_buckyos_api_runtime().ok();
        let principal = caller.principal.clone();
        if let CallerClass::Account { did, groups: member_of, .. } = &mut caller.class {
            let cp = match &runtime {
                Some(r) => r.get_control_panel_client().await.ok(),
                None => None,
            };
            let mut failed: Vec<&String> = Vec::new();
            if !groups.is_empty() {
                match &cp {
                    Some(cp) => match cp.get_user_settings_by_username(&principal).await {
                        Ok(settings) => *member_of = settings.groups,
                        Err(e) => {
                            log::warn!("groups of {principal} unavailable: {e:?}");
                            failed.extend(groups.iter().copied());
                        }
                    },
                    None => failed.extend(groups.iter().copied()),
                }
            }
            if !contacts.is_empty() && did.is_none() {
                match &cp {
                    Some(cp) => match cp.get_user_did(&principal).await {
                        Ok(d) => *did = Some(d.to_string()),
                        Err(e) => {
                            log::warn!("DID of {principal} unavailable: {e:?}");
                            failed.extend(contacts.iter().copied());
                        }
                    },
                    None => failed.extend(contacts.iter().copied()),
                }
            }
            let failed: Vec<String> = failed.into_iter().cloned().collect();
            caller.unconfirmed.extend(failed);
        }
        let Some(did) = caller.did().map(str::to_string) else { return };
        let contacts: Vec<String> = contacts.into_iter().filter(|s| !caller.unconfirmed.contains(*s)).cloned().collect();
        if contacts.is_empty() {
            return;
        }
        let msg = match &runtime {
            Some(r) => r.get_msg_center_client().await.ok(),
            None => None,
        };
        for subject in contacts {
            let Some((owner, tag)) = subject[CONTACT_PREFIX.len()..].split_once('#') else { continue };
            let req: Option<ContactCheckMembershipReq> = serde_json::from_value(serde_json::json!({ "owner": owner, "did": did, "tag": tag })).ok();
            let answer = match (&msg, req) {
                (Some(msg), Some(req)) => msg.contact_check_membership(req).await.map(|r| r.member).map_err(|e| format!("{e:?}")),
                (None, _) => Err("msg-center client unavailable".to_string()),
                (_, None) => Ok(false),
            };
            match answer {
                Ok(true) => {
                    caller.contacts.insert(subject);
                }
                Ok(false) => {}
                Err(e) => {
                    log::warn!("contact membership {subject} of {did} unavailable: {e}");
                    caller.unconfirmed.insert(subject);
                }
            }
        }
    }
    async fn load_did(&self, caller: &mut Caller) -> Result<(), AccessDenial> {
        use buckyos_api::get_buckyos_api_runtime;
        let principal = caller.principal.clone();
        let CallerClass::Account { did: did @ None, .. } = &mut caller.class else { return Ok(()) };
        let runtime = get_buckyos_api_runtime().map_err(|_| unavailable())?;
        let cp = runtime.get_control_panel_client().await.map_err(|_| unavailable())?;
        match cp.get_user_did(&principal).await {
            Ok(d) => {
                *did = Some(d.to_string());
                Ok(())
            }
            Err(e) => {
                log::warn!("DID of {principal} unavailable: {e:?}");
                Err(unavailable())
            }
        }
    }
    async fn sso_settings(&self) -> Result<SsoSettings, AccessDenial> {
        use buckyos_api::{get_buckyos_api_runtime, SystemConfigError, SSO_SETTINGS_KEY};
        let runtime = get_buckyos_api_runtime().map_err(|_| unavailable())?;
        let client = runtime.get_system_config_client().await.map_err(|_| unavailable())?;
        match client.get(SSO_SETTINGS_KEY).await {
            Ok(v) => serde_json::from_str(&v.value).map_err(|e| {
                log::warn!("{SSO_SETTINGS_KEY} is invalid: {e}");
                unavailable()
            }),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(SsoSettings::default()),
            Err(e) => {
                log::warn!("{SSO_SETTINGS_KEY} unavailable: {e}");
                Err(unavailable())
            }
        }
    }
}
