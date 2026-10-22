//! Zone SSO settings (contract §8.1): the self-registration and DID login
//! switches and the jump entry, at `system/sso/settings`.

use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::user_mgr::require_control_panel_admin;
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{SsoSettings, SSO_SETTINGS_KEY};
use buckyos_kit::KVAction;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

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
    })
}

impl ControlPanelServer {
    /// `auth.sso_config {}` (public) → `SsoPublicConfig`.
    pub(crate) async fn handle_auth_sso_config(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let settings = load_sso_settings(&ZoneKv::connect().await?).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!(settings.public_config())),
            req.seq,
        ))
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
    /// dev_federation_http?}` (admin, no sudo) → same as `sso.settings.get`. Fields left
    /// out keep the stored value; `jump_base_url` null or "" returns to the default.
    pub(crate) async fn handle_sso_settings_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let patch: SsoSettingsPatch = Self::typed_params(&req)?;
        let kv = ZoneKv::connect().await?;
        let settings = patch.apply(load_sso_settings(&kv).await?)?;
        let raw = serde_json::to_string(&settings)
            .map_err(|error| RPCErrors::ReasonError(format!("serialize SSO settings: {error}")))?;
        let action = match kv.get(SSO_SETTINGS_KEY).await? {
            Some(_) => KVAction::Update(raw),
            None => KVAction::Create(raw),
        };
        kv.exec_tx(
            HashMap::from([(SSO_SETTINGS_KEY.to_string(), action)]),
            None,
        )
        .await?;
        log::info!(
            "Zone SSO settings changed by '{}': self_register={} did_login={}",
            principal.username,
            settings.self_register_enabled,
            settings.did_login_enabled
        );
        Ok(RPCResponse::new(
            RPCResult::Success(settings_value(&settings)),
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
