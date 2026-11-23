//! Zone SSO settings (contract §8.1): the self-registration and DID login
//! switches and the jump entry, at `system/sso/settings`.

use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::exclusive_invite::get_json;
use crate::user_mgr::require_control_panel_admin;
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    oauth_binding_key, OAuthSource, OAuthSourceKind, SsoSettings, UserSettings, UserState,
    SSO_SETTINGS_KEY,
};
use buckyos_kit::KVAction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Deleted login sources. Sessions, grants and account bindings name a source by
/// its ID, identity domain and issuer, so a deleted source's namespace stays
/// reserved: only the same source can be added again, and then its existing
/// bindings sign in again.
const RETIRED_OAUTH_SOURCES_KEY: &str = "services/control_panel/oauth/retired_sources";
const OAUTH_SUBJECTS_PREFIX: &str = "services/control_panel/oauth/subjects";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct RetiredOAuthSource {
    id: String,
    kind: OAuthSourceKind,
    webhost: String,
    issuer: String,
}

impl RetiredOAuthSource {
    fn of(source: &OAuthSource) -> Self {
        Self {
            id: source.id.clone(),
            kind: source.kind,
            webhost: source.webhost.clone(),
            issuer: source.issuer.clone(),
        }
    }

    fn claims(&self, source: &OAuthSource) -> bool {
        self.id == source.id || self.webhost == source.webhost || self.issuer == source.issuer
    }
}

/// Checks validated `new` sources against the stored `old` ones: a kept source keeps
/// its namespace, a removed one is retired, and an added one may only take a
/// retired namespace back unchanged. Returns whether `retired` changed.
fn reconcile_oauth_sources(
    old: &[OAuthSource],
    new: &[OAuthSource],
    retired: &mut Vec<RetiredOAuthSource>,
) -> Result<bool, RPCErrors> {
    let mut changed = false;
    for source in old {
        match new.iter().find(|s| s.id == source.id) {
            Some(kept) if RetiredOAuthSource::of(kept) != RetiredOAuthSource::of(source) => {
                return Err(RPCErrors::ParseRequestError(
                    "oauth_source_identity_fixed: a login source's ID, type, identity domain and issuer cannot change".into(),
                ));
            }
            Some(_) => {}
            None => {
                let record = RetiredOAuthSource::of(source);
                if !retired.contains(&record) {
                    retired.push(record);
                    changed = true;
                }
            }
        }
    }
    for source in new.iter().filter(|s| !old.iter().any(|o| o.id == s.id)) {
        let record = RetiredOAuthSource::of(source);
        if retired.iter().any(|r| r.claims(source) && *r != record) {
            return Err(RPCErrors::ParseRequestError(
                "oauth_source_retired: this ID, identity domain or issuer belonged to a deleted login source; only that same source can be added again".into(),
            ));
        }
        if let Some(index) = retired.iter().position(|r| *r == record) {
            retired.remove(index);
            changed = true;
        }
    }
    Ok(changed)
}

/// Applies `patch` to the stored settings and writes them, with the retired
/// sources in the same transaction.
pub(crate) async fn save_sso_settings(
    kv: &dyn AgentKv,
    patch: SsoSettingsPatch,
    zone: &str,
) -> Result<SsoSettings, RPCErrors> {
    let previous = kv.get(SSO_SETTINGS_KEY).await?;
    let current: SsoSettings = match &previous {
        Some((raw, _)) => serde_json::from_str(raw)
            .map_err(|e| RPCErrors::ReasonError(format!("invalid SSO settings: {e}")))?,
        None => SsoSettings::default(),
    };
    let mut settings = patch.apply(current.clone())?;
    buckyos_api::validate_oauth_sources(&mut settings.oauth_sources, zone)
        .map_err(RPCErrors::ParseRequestError)?;
    let stored_retired = get_json::<Vec<RetiredOAuthSource>>(kv, RETIRED_OAUTH_SOURCES_KEY).await?;
    let mut retired = stored_retired
        .as_ref()
        .map(|(list, _)| list.clone())
        .unwrap_or_default();
    let retired_changed =
        reconcile_oauth_sources(&current.oauth_sources, &settings.oauth_sources, &mut retired)?;
    let raw = serde_json::to_string(&settings)
        .map_err(|error| RPCErrors::ReasonError(format!("serialize SSO settings: {error}")))?;
    let mut actions = HashMap::from([(
        SSO_SETTINGS_KEY.to_string(),
        match &previous {
            Some(_) => KVAction::Update(raw),
            None => KVAction::Create(raw),
        },
    )]);
    if retired_changed {
        let raw = serde_json::to_string(&retired)
            .map_err(|error| RPCErrors::ReasonError(format!("serialize retired sources: {error}")))?;
        actions.insert(
            RETIRED_OAUTH_SOURCES_KEY.to_string(),
            match stored_retired {
                Some(_) => KVAction::Update(raw),
                None => KVAction::Create(raw),
            },
        );
    }
    kv.exec_tx(actions, previous.map(|(_, v)| (SSO_SETTINGS_KEY.into(), v)))
        .await?;
    Ok(settings)
}

/// The local accounts bound to a subject of the source with `issuer`, so the
/// ones that lose this way in when it is deleted. Deleted accounts are left out.
pub(crate) async fn oauth_source_accounts(
    kv: &dyn AgentKv,
    issuer: &str,
) -> Result<Vec<Value>, RPCErrors> {
    let mut accounts = BTreeMap::new();
    for key in kv.list(OAUTH_SUBJECTS_PREFIX).await? {
        let subject = match get_json::<crate::oauth::OAuthSubject>(
            kv,
            &format!("{OAUTH_SUBJECTS_PREFIX}/{key}"),
        )
        .await
        {
            Ok(Some((subject, _))) => subject,
            Ok(None) => continue,
            Err(error) => {
                log::warn!("skip OAuth subject {key}: {error:?}");
                continue;
            }
        };
        if subject.identity.issuer != issuer {
            continue;
        }
        let Some((user, _)) = kv
            .get(&oauth_binding_key(issuer, &subject.identity.sub))
            .await?
        else {
            continue;
        };
        let Some((settings, _)) =
            get_json::<UserSettings>(kv, &format!("users/{user}/settings")).await?
        else {
            continue;
        };
        if matches!(settings.state, UserState::Deleted) {
            continue;
        }
        let show_name = get_json::<Value>(kv, &format!("users/{user}/profile"))
            .await
            .ok()
            .flatten()
            .and_then(|(profile, _)| {
                ["display_name", "name"].iter().find_map(|field| {
                    profile[*field]
                        .as_str()
                        .filter(|name| !name.trim().is_empty())
                        .map(str::to_owned)
                })
            })
            .unwrap_or_else(|| user.clone());
        accounts.insert(
            user.clone(),
            json!({"user_id": user, "show_name": show_name, "email": subject.email, "state": settings.state}),
        );
    }
    Ok(accounts.into_values().collect())
}

/// The stored settings; a missing key means every default.
pub(crate) async fn load_sso_settings(kv: &dyn AgentKv) -> Result<SsoSettings, RPCErrors> {
    match kv.get(SSO_SETTINGS_KEY).await? {
        Some((raw, _)) => serde_json::from_str(&raw).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid `{SSO_SETTINGS_KEY}`: {error}"))
        }),
        None => Ok(SsoSettings::default()),
    }
}

/// `sso.settings.set`: fields left out keep their value; `jump_base_url` null or
/// blank returns to the default entry.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct SsoSettingsPatch {
    #[serde(default)]
    self_register_enabled: Option<bool>,
    #[serde(default)]
    oauth_sources: Option<Vec<buckyos_api::OAuthSource>>,
    #[serde(default)]
    did_login_enabled: Option<bool>,
    #[serde(default, deserialize_with = "patch_field")]
    jump_base_url: Option<Option<String>>,
    #[serde(default)]
    dev_federation_http: Option<bool>,
}

fn patch_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

impl SsoSettingsPatch {
    pub(crate) fn apply(self, mut settings: SsoSettings) -> Result<SsoSettings, RPCErrors> {
        if let Some(sources) = self.oauth_sources {
            settings.oauth_sources = sources;
        }
        if let Some(value) = self.self_register_enabled {
            settings.self_register_enabled = value;
        }
        if let Some(value) = self.did_login_enabled {
            settings.did_login_enabled = value;
        }
        if let Some(value) = self.dev_federation_http {
            settings.dev_federation_http = value;
        }
        if let Some(jump) = self.jump_base_url {
            settings.jump_base_url = match jump.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(url) => Some(validate_jump_base_url(url)?),
            };
        }
        Ok(settings)
    }
}

/// An https origin with an optional path; the SDK appends the action path and
/// puts the parameters in the fragment.
fn validate_jump_base_url(raw: &str) -> Result<String, RPCErrors> {
    let invalid = |reason: &str| RPCErrors::ParseRequestError(format!("jump_base_url {reason}"));
    let url = url::Url::parse(raw).map_err(|_| invalid("is not a URL"))?;
    if url.scheme() != "https" {
        return Err(invalid("must use https"));
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("needs a host and no credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("cannot carry a query or fragment"));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Every field explicit, so the page can show what is stored.
fn settings_value(settings: &SsoSettings) -> Value {
    json!({
        "self_register_enabled": settings.self_register_enabled,
        "did_login_enabled": settings.did_login_enabled,
        "jump_base_url": settings.jump_base_url,
        "dev_federation_http": settings.dev_federation_http,
        "oauth_sources": settings.oauth_sources,
    })
}

impl ControlPanelServer {
    /// `auth.sso_config {}` (public) → `SsoPublicConfig`.
    pub(crate) async fn handle_auth_sso_config(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let settings = load_sso_settings(&ZoneKv::connect().await?).await?;
        let mut public = json!(settings.public_config());
        let runtime = buckyos_api::get_buckyos_api_runtime()?;
        public["zone_origin"] = json!(Self::zone_root_origin(&req, &Self::sso_zone_host(runtime)?));
        Ok(RPCResponse::new(RPCResult::Success(public), req.seq))
    }

    /// `sso.settings.get {}` (admin) → `{self_register_enabled, did_login_enabled,
    /// jump_base_url: string|null, dev_federation_http}`.
    pub(crate) async fn handle_sso_settings_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_control_panel_admin(Self::require_rpc_principal(principal)?)?;
        let settings = load_sso_settings(&ZoneKv::connect().await?).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(settings_value(&settings)),
            req.seq,
        ))
    }

    /// `sso.settings.set {self_register_enabled?, did_login_enabled?, jump_base_url?,
    /// dev_federation_http?, oauth_sources?}` (admin, no sudo) → same as
    /// `sso.settings.get`. Fields left out keep the stored value; `jump_base_url`
    /// null or "" returns to the default. A source left out of `oauth_sources` is
    /// deleted and its namespace reserved (see `RETIRED_OAUTH_SOURCES_KEY`).
    pub(crate) async fn handle_sso_settings_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let patch: SsoSettingsPatch = Self::typed_params(&req)?;
        let zone = Self::sso_zone_host(buckyos_api::get_buckyos_api_runtime()?)?;
        let settings = save_sso_settings(&ZoneKv::connect().await?, patch, &zone).await?;
        log::info!(
            "Zone SSO settings changed by '{}': self_register={} did_login={} oauth_sources={:?}",
            principal.username,
            settings.self_register_enabled,
            settings.did_login_enabled,
            settings.oauth_sources.iter().map(|s| (&s.id, s.enabled)).collect::<Vec<_>>()
        );
        Ok(RPCResponse::new(
            RPCResult::Success(settings_value(&settings)),
            req.seq,
        ))
    }

    /// `sso.oauth_source.accounts {source_id}` (admin) → `{accounts: [{user_id,
    /// show_name, email, state}]}`: the local accounts that sign in through it.
    pub(crate) async fn handle_sso_oauth_source_accounts(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        require_control_panel_admin(Self::require_rpc_principal(principal)?)?;
        let id = Self::require_param_str(&req, "source_id")?;
        let kv = ZoneKv::connect().await?;
        let settings = load_sso_settings(&kv).await?;
        let source = settings
            .oauth_sources
            .iter()
            .find(|source| source.id == id)
            .ok_or_else(|| RPCErrors::ParseRequestError(format!("unknown login source `{id}`")))?;
        let accounts = oauth_source_accounts(&kv, &source.issuer).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "accounts": accounts })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive_invite::tests::MemKv;
    use buckyos_api::DEFAULT_JUMP_BASE_URL;

    #[tokio::test]
    async fn missing_settings_are_the_defaults() {
        let kv = MemKv::default();
        let settings = load_sso_settings(&kv).await.unwrap();
        assert!(settings.self_register_enabled && settings.did_login_enabled);
        assert_eq!(
            settings.public_config().jump_base_url,
            DEFAULT_JUMP_BASE_URL
        );
        kv.put_raw(SSO_SETTINGS_KEY, r#"{"did_login_enabled":false}"#);
        let settings = load_sso_settings(&kv).await.unwrap();
        assert!(settings.self_register_enabled && !settings.did_login_enabled);
    }

    fn oidc(id: &str, domain: &str) -> Value {
        json!({
            "id": id, "display_name": id, "enabled": true, "kind": "oidc",
            "client_id": "desktop", "secret_ref": id, "webhost": domain,
            "issuer": format!("https://login.{domain}"),
            "authorization_endpoint": format!("https://login.{domain}/authorize"),
            "token_endpoint": format!("https://login.{domain}/token"),
            "jwks_uri": format!("https://login.{domain}/keys"),
            "scopes": ["openid", "email"],
        })
    }

    fn google() -> Value {
        json!({"id": "google", "display_name": "Google", "enabled": true, "kind": "google",
               "client_id": "test.apps.googleusercontent.com", "secret_ref": "google-demo"})
    }

    async fn set_sources(kv: &MemKv, sources: Vec<Value>) -> Result<SsoSettings, RPCErrors> {
        let patch: SsoSettingsPatch =
            serde_json::from_value(json!({ "oauth_sources": sources })).unwrap();
        save_sso_settings(kv, patch, "zone.example").await
    }

    #[tokio::test]
    async fn deleted_sources_reserve_their_namespace_until_added_back_unchanged() {
        let kv = MemKv::default();
        set_sources(&kv, vec![google(), oidc("acme", "acme.example")])
            .await
            .unwrap();

        let mut renamed = oidc("acme", "other.example");
        renamed["display_name"] = json!("Acme");
        assert!(set_sources(&kv, vec![google(), renamed]).await.is_err());
        let mut edited = oidc("acme", "acme.example");
        edited["display_name"] = json!("Acme SSO");
        edited["client_id"] = json!("desktop-2");
        let saved = set_sources(&kv, vec![google(), edited]).await.unwrap();
        assert_eq!(saved.oauth_sources[1].client_id, "desktop-2");

        // Delete both; their namespaces stay reserved.
        let saved = set_sources(&kv, vec![]).await.unwrap();
        assert!(saved.oauth_sources.is_empty());
        let retired: Vec<RetiredOAuthSource> = kv.read(RETIRED_OAUTH_SOURCES_KEY);
        assert_eq!(retired.len(), 2);
        for reuse in [oidc("acme", "elsewhere.example"), oidc("fresh", "acme.example")] {
            let error = set_sources(&kv, vec![reuse]).await.unwrap_err();
            assert!(format!("{error:?}").contains("oauth_source_retired"));
        }

        // The same sources come back (other fields may differ) and leave the list.
        let mut back = oidc("acme", "acme.example");
        back["client_id"] = json!("desktop-3");
        set_sources(&kv, vec![google(), back]).await.unwrap();
        let retired: Vec<RetiredOAuthSource> = kv.read(RETIRED_OAUTH_SOURCES_KEY);
        assert!(retired.is_empty());
        set_sources(&kv, vec![google(), oidc("acme", "acme.example"), oidc("beta", "beta.example")])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn source_accounts_are_the_live_local_accounts_bound_to_its_issuer() {
        let kv = MemKv::default();
        let subject = |issuer: &str, sub: &str, email: &str| {
            json!({"identity": {"source_id": "x", "issuer": issuer, "sub": sub, "email": email,
                    "label": "x", "did": format!("did:web:{sub}.x"), "webhost": "x"},
                   "did": format!("did:web:{sub}.x"), "email": email})
        };
        let google = "https://accounts.google.com";
        for (key, issuer, sub, email, user) in [
            ("a", google, "1", "alice@gmail.com", Some(("alice", "active", Some("Alice")))),
            ("b", google, "2", "bob@gmail.com", Some(("bob", "deleted", None))),
            ("c", google, "3", "guest@gmail.com", None),
            ("d", "https://login.acme.example", "4", "carol@acme.example", Some(("carol", "active", None))),
            ("e", google, "5", "dave@gmail.com", Some(("dave", "active", None))),
        ] {
            kv.put(&format!("{OAUTH_SUBJECTS_PREFIX}/{key}"), &subject(issuer, sub, email));
            if let Some((user, state, name)) = user {
                kv.put_raw(&oauth_binding_key(issuer, sub), user);
                kv.put(&format!("users/{user}/settings"), &json!({
                    "user_id": user, "type": "user", "password": "", "state": state, "res_pool_id": "default",
                }));
                if let Some(name) = name {
                    kv.put(&format!("users/{user}/profile"), &json!({"display_name": name}));
                }
            }
        }
        let accounts = oauth_source_accounts(&kv, google).await.unwrap();
        assert_eq!(
            accounts,
            vec![
                json!({"user_id": "alice", "show_name": "Alice", "email": "alice@gmail.com", "state": "active"}),
                json!({"user_id": "dave", "show_name": "dave", "email": "dave@gmail.com", "state": "active"}),
            ]
        );
    }

    #[test]
    fn patch_keeps_unnamed_fields_and_validates_the_jump_entry() {
        let patch: SsoSettingsPatch = serde_json::from_value(
            json!({"did_login_enabled": false, "jump_base_url": "https://jump.example/"}),
        )
        .unwrap();
        let settings = patch.apply(SsoSettings::default()).unwrap();
        assert!(settings.self_register_enabled);
        assert!(!settings.did_login_enabled);
        assert_eq!(
            settings.jump_base_url.as_deref(),
            Some("https://jump.example")
        );

        let reset: SsoSettingsPatch =
            serde_json::from_value(json!({"jump_base_url": null})).unwrap();
        assert_eq!(reset.apply(settings.clone()).unwrap().jump_base_url, None);
        let keep: SsoSettingsPatch = serde_json::from_value(json!({})).unwrap();
        assert_eq!(keep.apply(settings.clone()).unwrap(), settings);

        for bad in [
            "http://jump.example",
            "https://u:p@jump.example",
            "https://jump.example/?a=1",
            "jump",
        ] {
            let patch: SsoSettingsPatch =
                serde_json::from_value(json!({"jump_base_url": bad})).unwrap();
            assert!(
                matches!(
                    patch.apply(SsoSettings::default()),
                    Err(RPCErrors::ParseRequestError(_))
                ),
                "{bad}"
            );
        }
        assert!(
            serde_json::from_value::<SsoSettingsPatch>(json!({"guest_enabled": true})).is_err()
        );
    }
}
