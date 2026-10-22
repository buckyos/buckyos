//! What verify-hub reads about the Zone to decide an issuance: accounts, their DID
//! documents, exclusive invites, SSO settings and App availability. Behind a trait so
//! the decisions can be tested without system-config.

use ::kRPC::RPCErrors;
use async_trait::async_trait;
use buckyos_api::{
    exclusive_invite_key, user_did_from_doc_value, AppAvailabilityDecision,
    AppAvailabilityResolver, AppInstanceId, ExclusiveInvite, SsoSettings, SystemConfigError,
    UserSettings, SSO_SETTINGS_KEY,
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
    async fn user_settings(&self, user_id: &str) -> Result<Option<UserSettings>>;
    /// `None` when the account has no usable OwnerDocument.
    async fn user_doc(&self, user_id: &str) -> Result<Option<UserDoc>>;
    async fn list_users(&self) -> Result<Vec<String>>;
    async fn exclusive_invite(&self, invite_id: &str) -> Result<Option<ExclusiveInvite>>;
    async fn sso_settings(&self) -> Result<SsoSettings>;
    async fn app_availability(
        &self,
        user_id: &str,
        app_instance_id: &AppInstanceId,
    ) -> Result<AppAvailabilityDecision>;
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
            .and_then(|doc| doc.get_default_zone_did());
        Ok(Some(UserDoc { did, default_zone }))
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
}
