//! What verify-hub reads about the Zone to decide an issuance: accounts, their DID
//! documents, exclusive invites, SSO settings and App availability. Behind a trait so
//! the decisions can be tested without system-config.

use ::kRPC::RPCErrors;
use async_trait::async_trait;
use buckyos_api::{
    app_login_settings_key, check_app_runtime_session, exclusive_invite_key,
    owner_canonical_zone, user_did_from_doc_value, AccessDenial, AppAvailabilityDecision,
    AppAvailabilityResolver, AppInstanceId, AppLoginSettings, DenyReason, ExclusiveInvite,
    SsoSettings, SystemConfigError, UserSettings, SSO_SETTINGS_KEY,
};
use log::warn;
use name_lib::{OwnerDocument, DID};
use std::sync::Arc;

type Result<T> = std::result::Result<T, RPCErrors>;

/// Identity facts of `users/<id>/doc` (the account's OwnerDocument).
#[derive(Clone, Debug, PartialEq)]
pub struct UserDoc {
    pub did: DID,
    pub default_zone: Option<DID>,
}

#[async_trait]
pub trait ZoneDirectory: Send + Sync {
    async fn zone_did(&self) -> Result<DID>;
    /// The hostname this Zone serves under (its ZoneDocument).
    async fn zone_hostname(&self) -> Result<String> {
        Ok(self.zone_did().await?.to_host_name())
    }
    async fn user_settings(&self, user_id: &str) -> Result<Option<UserSettings>>;
    /// `None` when the account has no usable OwnerDocument.
    async fn user_doc(&self, user_id: &str) -> Result<Option<UserDoc>>;
    async fn native_login_binding(&self, _did: &DID) -> Result<Option<String>> {
        Ok(None)
    }
    async fn oauth_binding(&self, issuer: &str, sub: &str) -> Result<Option<String>> {
        read_optional(&buckyos_api::oauth_binding_key(issuer,sub)).await
    }
    async fn list_users(&self) -> Result<Vec<String>>;
    async fn exclusive_invite(&self, invite_id: &str) -> Result<Option<ExclusiveInvite>>;
    async fn sso_settings(&self) -> Result<SsoSettings>;
    async fn has_share_acceptance(
        &self,
        _invite_id: &str,
        _did: &str,
        _target: &buckyos_api::AuthTarget,
    ) -> Result<bool> {
        Ok(false)
    }
    async fn app_availability(
        &self,
        user_id: &str,
        app_instance_id: &AppInstanceId,
    ) -> Result<AppAvailabilityDecision>;
    /// Whether the App instance may sign external customers in right now: its owner's
    /// switch is on and the App is installed and running. The denial names what closed it.
    async fn app_login_available(
        &self,
        app_instance_id: &AppInstanceId,
    ) -> Result<std::result::Result<(), AccessDenial>>;
}

/// The live Zone: system-config read with verify-hub's own service session.
pub struct SystemConfigDirectory;

fn read_failure(key: &str, error: SystemConfigError) -> RPCErrors {
    RPCErrors::ReasonError(format!("read {key} failed: {error}"))
}

async fn read_optional(key: &str) -> Result<Option<String>> {
    let client = crate::get_system_config_client().await?;
    match client.get(key).await {
        Ok(value) => Ok(Some(value.value)),
        Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
        Err(error) => Err(read_failure(key, error)),
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(key: &str) -> Result<Option<T>> {
    read_optional(key)
        .await?
        .map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|error| RPCErrors::ReasonError(format!("invalid {key}: {error}")))
        })
        .transpose()
}

#[async_trait]
impl ZoneDirectory for SystemConfigDirectory {
    async fn zone_did(&self) -> Result<DID> {
        crate::VERIFY_SERVICE_CONFIG
            .lock()
            .await
            .as_ref()
            .map(|config| config.zone_document.id.clone())
            .ok_or_else(|| RPCErrors::ReasonError("verify_hub service config not loaded".into()))
    }

    async fn zone_hostname(&self) -> Result<String> {
        crate::VERIFY_SERVICE_CONFIG
            .lock()
            .await
            .as_ref()
            .map(|config| buckyos_api::zone_document_hostname(&config.zone_document))
            .ok_or_else(|| RPCErrors::ReasonError("verify_hub service config not loaded".into()))
    }

    async fn user_settings(&self, user_id: &str) -> Result<Option<UserSettings>> {
        read_json(&format!("users/{user_id}/settings")).await
    }

    async fn user_doc(&self, user_id: &str) -> Result<Option<UserDoc>> {
        let key = format!("users/{user_id}/doc");
        let Some(raw) = read_optional(&key).await? else {
            return Ok(None);
        };
        let did = match user_did_from_doc_value(&key, &raw) {
            Ok(did) => did,
            Err(error) => {
                warn!("unusable {key}: {error}");
                return Ok(None);
            }
        };
        let default_zone = serde_json::from_str::<OwnerDocument>(&raw)
            .ok()
            .map(|doc| owner_canonical_zone(&doc));
        Ok(Some(UserDoc { did, default_zone }))
    }

    async fn native_login_binding(&self, did: &DID) -> Result<Option<String>> {
        read_json(&buckyos_api::native_login_binding_key(&did.to_string())).await
    }

    async fn list_users(&self) -> Result<Vec<String>> {
        let client = crate::get_system_config_client().await?;
        match client.list("users").await {
            Ok(users) => Ok(users),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(Vec::new()),
            Err(error) => Err(read_failure("users", error)),
        }
    }

    async fn exclusive_invite(&self, invite_id: &str) -> Result<Option<ExclusiveInvite>> {
        read_json(&exclusive_invite_key(invite_id)).await
    }

    async fn has_share_acceptance(
        &self,
        invite_id: &str,
        did: &str,
        target: &buckyos_api::AuthTarget,
    ) -> Result<bool> {
        use buckyos_api::{
            resource_share_acceptance_key, valid_resource_share_id, AuthTarget,
            ResourceShareAcceptance,
        };
        if !valid_resource_share_id(invite_id) {
            return Ok(false);
        }
        let accepted =
            read_json::<ResourceShareAcceptance>(&resource_share_acceptance_key(invite_id, did))
                .await?;
        if !accepted
            .is_some_and(|a| a.invite_id == invite_id && a.did == did && &a.target == target)
        {
            return Ok(false);
        }
        if let AuthTarget::App { app_instance_id } = target {
            let client = crate::get_system_config_client().await?;
            check_app_runtime_session(&client, app_instance_id).await?;
        }
        // The resource service owns the live ACL. A lost connection must remain retryable.
        let token = crate::get_my_krpc_token().await?.to_string();
        let endpoint = format!(
            "http://127.0.0.1:{}/kapi/control-panel",
            buckyos_api::DEFAULT_NODE_GATEWAY_PORT
        );
        let client = ::kRPC::kRPC::new_with_timeout_secs(&endpoint, Some(token), 10);
        let result = client
            .call(
                "resource.share.verify_grant",
                serde_json::json!({"invite_id": invite_id,"did":did}),
            )
            .await?;
        result
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| RPCErrors::ReasonError("invalid resource grant response".into()))
    }

    async fn sso_settings(&self) -> Result<SsoSettings> {
        Ok(read_json(SSO_SETTINGS_KEY).await?.unwrap_or_default())
    }

    async fn app_availability(
        &self,
        user_id: &str,
        app_instance_id: &AppInstanceId,
    ) -> Result<AppAvailabilityDecision> {
        let client = crate::get_system_config_client().await?;
        AppAvailabilityResolver::new(Arc::new(client))
            .check_user(user_id, app_instance_id)
            .await
    }

    async fn app_login_available(
        &self,
        app_instance_id: &AppInstanceId,
    ) -> Result<std::result::Result<(), AccessDenial>> {
        let settings: AppLoginSettings =
            read_json(&app_login_settings_key(&app_instance_id.to_string()))
                .await?
                .unwrap_or_default();
        if !settings.enabled {
            return Ok(Err(
                AccessDenial::new(DenyReason::SourceRevoked).with_detail("app_login_disabled")
            ));
        }
        let client = crate::get_system_config_client().await?;
        match check_app_runtime_session(&client, app_instance_id).await {
            Ok(_) => Ok(Ok(())),
            Err(error) => match AccessDenial::from_rpc_error(&error) {
                Some(denial) => Ok(Err(AccessDenial::new(DenyReason::SourceRevoked)
                    .with_detail(denial.detail.unwrap_or_else(|| "app_unavailable".into())))),
                None => Err(error),
            },
        }
    }
}
