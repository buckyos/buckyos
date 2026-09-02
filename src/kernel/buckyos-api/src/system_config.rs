//TODO:
//  add WATCH,and load cached value automatically when the value is changed.

use ::kRPC::{kRPC, RPCContext};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use name_lib::{
    DIDDocumentTrait, EncodedDocument, NSError, NSResult, VerifyHubInfo, ZoneBootDocument,
    ZoneDocument, DID,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

use std::collections::HashMap;
use tokio::sync::{OnceCell, RwLock};

use crate::KVAction;

const CONFIG_CACHE_TIME: u64 = 10; //10s
pub const SYSTEM_CONFIG_BOOTSTRAP_AUDIENCE: &str = "system-config-bootstrap";
pub const BUCKYOS_INFO_KEY: &str = "system/buckyos_info";
pub const BUCKYOS_INFO_SCHEMA_VERSION: u32 = 1;
pub const BUCKYOS_DEV_CONFIG_KEY: &str = "system/buckyos_dev_config";
pub const BUCKYOS_DEV_CONFIG_SCHEMA_VERSION: u32 = 1;
type ConfigCache = HashMap<String, (String, u64, u64)>;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuckyOSDevConfig {
    pub schema_version: u32,
    pub enabled: bool,
    pub enabled_at: Option<u64>,
    pub enabled_by: Option<String>,
}

impl Default for BuckyOSDevConfig {
    fn default() -> Self {
        Self {
            schema_version: BUCKYOS_DEV_CONFIG_SCHEMA_VERSION,
            enabled: false,
            enabled_at: None,
            enabled_by: None,
        }
    }
}

impl BuckyOSDevConfig {
    pub fn set_enabled(&mut self, enabled: bool, actor: &str, now: u64) -> Result<(), String> {
        if actor.trim().is_empty() {
            return Err("BuckyOSDevConfig actor is empty".to_string());
        }
        if now == 0 {
            return Err("BuckyOSDevConfig enable timestamp is zero".to_string());
        }

        if enabled && !self.enabled {
            self.enabled_at = Some(now);
            self.enabled_by = Some(actor.to_string());
        }
        self.enabled = enabled;
        self.validate()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != BUCKYOS_DEV_CONFIG_SCHEMA_VERSION {
            return Err(format!(
                "unsupported BuckyOSDevConfig schema_version {}",
                self.schema_version
            ));
        }

        match (self.enabled_at, self.enabled_by.as_deref()) {
            (None, None) if !self.enabled => Ok(()),
            (Some(enabled_at), Some(enabled_by))
                if enabled_at > 0 && !enabled_by.trim().is_empty() =>
            {
                Ok(())
            }
            _ => Err(
                "BuckyOSDevConfig enabled_at and enabled_by must be present together".to_string(),
            ),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuckyOSInfo {
    pub schema_version: u32,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_version: Option<String>,
    pub release_channel: String,
    pub target: String,
    pub installed_at: u64,
    pub updated_at: u64,
}

impl BuckyOSInfo {
    pub fn from_runtime(
        runtime_version: &str,
        release_channel: &str,
        target: &str,
        installed_at: u64,
    ) -> Self {
        let channel_suffix = format!(" ({release_channel})");
        let version_without_channel = runtime_version
            .trim()
            .strip_suffix(channel_suffix.as_str())
            .unwrap_or(runtime_version)
            .trim();
        let (version, build_version) = version_without_channel
            .split_once('+')
            .map(|(version, build_version)| {
                (
                    version.to_string(),
                    (!build_version.is_empty()).then(|| build_version.to_string()),
                )
            })
            .unwrap_or_else(|| (version_without_channel.to_string(), None));

        Self {
            schema_version: BUCKYOS_INFO_SCHEMA_VERSION,
            version,
            build_version,
            release_channel: release_channel.to_string(),
            target: target.to_string(),
            installed_at,
            updated_at: installed_at,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != BUCKYOS_INFO_SCHEMA_VERSION {
            return Err(format!(
                "unsupported BuckyOSInfo schema_version {}",
                self.schema_version
            ));
        }
        if self.version.trim().is_empty() {
            return Err("BuckyOSInfo version is empty".to_string());
        }
        if self.release_channel.trim().is_empty() {
            return Err("BuckyOSInfo release_channel is empty".to_string());
        }
        if self.target.trim().is_empty() {
            return Err("BuckyOSInfo target is empty".to_string());
        }
        if self.installed_at == 0 {
            return Err("BuckyOSInfo installed_at is zero".to_string());
        }
        if self.updated_at < self.installed_at {
            return Err("BuckyOSInfo updated_at predates installed_at".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ZoneConfig {
    pub zone_document: String, // jwt or json-ld document
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_repo_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_hub_info: Option<VerifyHubInfo>,
}

impl ZoneConfig {
    pub fn new(zone_document: String) -> Self {
        Self {
            zone_document,
            docker_repo_base_url: None,
            verify_hub_info: None,
        }
    }

    pub fn from_zone_document(
        zone_document: &ZoneDocument,
        verify_hub_info: Option<VerifyHubInfo>,
    ) -> NSResult<Self> {
        let mut zone_document_value = serde_json::to_value(zone_document)
            .map_err(|err| NSError::Failed(format!("encode zone document failed: {}", err)))?;
        if let Some(zone_document_object) = zone_document_value.as_object_mut() {
            zone_document_object.remove("docker_repo_base_url");
            zone_document_object.remove("verify_hub_info");
        }
        let zone_document = serde_json::to_string(&zone_document_value)
            .map_err(|err| NSError::Failed(format!("encode zone document failed: {}", err)))?;
        Ok(Self {
            zone_document,
            docker_repo_base_url: None,
            verify_hub_info,
        })
    }

    pub fn zone_document(&self) -> NSResult<ZoneDocument> {
        let encoded = EncodedDocument::from_str(self.zone_document.clone())?;
        if let Ok(zone_document) = ZoneDocument::decode(&encoded, None) {
            return Ok(zone_document);
        }

        let zone_boot_document = ZoneBootDocument::decode(&encoded, None)?;
        let zone_id = zone_boot_document
            .id
            .clone()
            .ok_or_else(|| NSError::InvalidParam("zone boot document id is missing".to_string()))?;
        let owner_key = zone_boot_document.owner_key.clone().ok_or_else(|| {
            NSError::InvalidParam("zone boot document owner_key is missing".to_string())
        })?;
        let owner = zone_boot_document
            .owner
            .clone()
            .unwrap_or_else(DID::undefined);
        let mut zone_document = ZoneDocument::new(zone_id, owner, owner_key);
        zone_document.init_by_boot_document(&zone_boot_document, &self.zone_document);
        Ok(zone_document)
    }
}

fn select_zone_document_hostname(configured_hostname: &str, zone_id: &DID) -> String {
    let configured_hostname = configured_hostname.trim().trim_end_matches('.');
    if configured_hostname.is_empty() {
        zone_id.to_host_name()
    } else {
        configured_hostname.to_string()
    }
}

/// Return the hostname published by the ZoneDocument.
///
/// A BNS DID's `to_host_name()` result depends on the process-local web3
/// bridge configuration.  The document's explicit hostname is the authority
/// for routing, TLS and ACME; converting the DID again can silently move a
/// test/custom-SN zone onto a different bridge domain.  The fallback keeps
/// compatibility with legacy documents that did not publish a hostname.
pub fn zone_document_hostname(zone_document: &ZoneDocument) -> String {
    select_zone_document_hostname(&zone_document.hostname, &zone_document.id)
}

#[derive(Error, Debug)]
pub enum SystemConfigError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("key {0} not found")]
    KeyNotFound(String),
    #[error("NoPermission: {0}")]
    NoPermission(String),
    #[error("Timeout: {0}")]
    Timeout(String),
}

pub type SytemConfigResult<T> = std::result::Result<T, SystemConfigError>;
pub struct SystemConfigClient {
    client: kRPC,
    session_token: RwLock<Option<String>>,
    cache_key_control: OnceCell<Vec<String>>,
    config_cache: RwLock<ConfigCache>,
}

pub struct SystemConfigValue {
    pub value: String,
    pub version: u64,
    pub is_changed: bool,
}

fn summarize_session_token(session_token: Option<&str>) -> String {
    match session_token {
        Some(token) => format!("present(len={})", token.len()),
        None => "None".to_string(),
    }
}

impl SystemConfigValue {
    pub fn new(value: String, version: u64, is_changed: bool) -> Self {
        Self {
            value,
            version,
            is_changed,
        }
    }
}

impl SystemConfigClient {
    pub async fn get_buckyos_dev_config(&self) -> SytemConfigResult<BuckyOSDevConfig> {
        let value = self.get(BUCKYOS_DEV_CONFIG_KEY).await?;
        let config: BuckyOSDevConfig = serde_json::from_str(&value.value).map_err(|error| {
            SystemConfigError::ReasonError(format!("failed to parse BuckyOSDevConfig: {error}"))
        })?;
        config.validate().map_err(SystemConfigError::ReasonError)?;
        Ok(config)
    }

    pub async fn is_buckyos_dev_mode_enabled(&self) -> SytemConfigResult<bool> {
        Ok(self.get_buckyos_dev_config().await?.enabled)
    }

    pub async fn get_zone_owner_user_id(&self) -> SytemConfigResult<String> {
        let value = self.get(crate::ZONE_OWNER_USER_ID_KEY).await?;
        let owner_user_id: String = serde_json::from_str(&value.value).map_err(|error| {
            SystemConfigError::ReasonError(format!(
                "system/zone_owner_user_id is not a JSON string: {error}"
            ))
        })?;
        let owner_user_id = owner_user_id.trim();
        if owner_user_id.is_empty()
            || owner_user_id.contains('@')
            || !owner_user_id.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            })
        {
            return Err(SystemConfigError::ReasonError(
                "system/zone_owner_user_id is not a canonical user id".to_string(),
            ));
        }
        Ok(owner_user_id.to_string())
    }

    fn need_cache(&self, key: &str) -> bool {
        let cache_key_control = self.cache_key_control.get();
        if cache_key_control.is_none() {
            return false;
        }
        for k in cache_key_control.unwrap().iter() {
            if key.starts_with(k) {
                return true;
            }
        }
        false
    }

    pub async fn set_context(&self, context: RPCContext) -> SytemConfigResult<()> {
        self.client.set_context(context).await;
        Ok(())
    }

    async fn rpc_get_optional_value_with_revision(
        &self,
        key: &str,
    ) -> SytemConfigResult<Option<(String, u64)>> {
        let result = self
            .client
            .call("sys_config_get", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        if result.is_null() {
            return Ok(None);
        }

        let value = result
            .get("value")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                SystemConfigError::ReasonError(format!(
                    "sys_config_get missing string value for key {}",
                    key
                ))
            })?;
        let revision = result
            .get("version")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                SystemConfigError::ReasonError(format!(
                    "sys_config_get missing numeric version for key {}",
                    key
                ))
            })?;

        Ok(Some((value.to_string(), revision)))
    }

    async fn get_config_cache(&self, key: &str) -> Option<(String, u64)> {
        let cache_guard = self.config_cache.read().await;
        let v = cache_guard.get(key)?;
        let (value, revision, cached_at) = v;
        let now = buckyos_get_unix_timestamp();
        if *cached_at + CONFIG_CACHE_TIME < now {
            // 缓存过期，删除缓存项
            drop(cache_guard); // 释放读锁
            let mut cache_guard = self.config_cache.write().await;
            cache_guard.remove(key);
            return None;
        }
        debug!(
            "get system_config from CONFIG_CACHE {}=>{}@{}",
            &key, &value, revision
        );
        Some((value.clone(), *revision))
    }

    async fn set_config_cache(&self, key: &str, value: &str, revision: u64) -> bool {
        if !self.need_cache(key) {
            return true;
        }

        let mut cache_guard = self.config_cache.write().await;
        let cached_at = buckyos_get_unix_timestamp();
        let old_value =
            cache_guard.insert(key.to_string(), (value.to_string(), revision, cached_at));
        if old_value.is_none() {
            return true;
        }
        let old_value = old_value.unwrap();
        if old_value.0 == value && old_value.1 == revision {
            return false;
        }
        true
    }

    async fn remove_config_cache(&self, key: &str) {
        let mut cache_guard = self.config_cache.write().await;
        cache_guard.remove(key);
    }

    pub fn new(service_url: Option<&str>, session_token: Option<&str>) -> Self {
        let real_session_token = session_token.map(|token| token.to_string());
        //let default_sys_config_url =
        let client = kRPC::new(
            service_url.unwrap_or("http://127.0.0.1:3200/kapi/system_config"),
            real_session_token.clone(),
        );

        info!(
            "system config client is created,service_url:{},session_token:{}",
            service_url.unwrap_or("http://127.0.0.1:3200/kapi/system_config"),
            summarize_session_token(session_token)
        );
        let key_control = vec!["services/".to_string(), "system/rbac/".to_string()];
        let cache_key_control = OnceCell::new_with(Some(key_control));

        SystemConfigClient {
            client,
            session_token: RwLock::new(real_session_token),
            cache_key_control,
            config_cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_session_token(&self) -> Option<String> {
        let session_token = self.session_token.read().await;
        session_token.clone()
    }

    pub async fn sync_session_token(&self, session_token: Option<&str>) -> SytemConfigResult<()> {
        let next_token = session_token.map(|value| value.to_string());

        let context = RPCContext {
            token: next_token.clone(),
            ..Default::default()
        };
        self.client.set_context(context).await;

        let mut current_token = self.session_token.write().await;
        *current_token = next_token;
        Ok(())
    }

    fn get_krpc_client(&self) -> SytemConfigResult<&kRPC> {
        Ok(&self.client)
    }

    //return (value,version,is_changed)
    pub async fn get(&self, key: &str) -> SytemConfigResult<SystemConfigValue> {
        // 首先尝试从缓存获取
        if let Some((cached_value, cached_revision)) = self.get_config_cache(key).await {
            return Ok(SystemConfigValue::new(cached_value, cached_revision, false));
        }

        // 缓存中没有，从服务器获取
        let result = self.rpc_get_optional_value_with_revision(key).await?;
        let Some((value, revision)) = result else {
            return Err(SystemConfigError::KeyNotFound(key.to_string()));
        };

        // 将结果存入缓存
        let is_changed = self.set_config_cache(key, &value, revision).await;

        Ok(SystemConfigValue::new(value, revision, is_changed))
    }

    pub async fn set(&self, key: &str, value: &str) -> SytemConfigResult<u64> {
        if key.is_empty() || value.is_empty() {
            return Err(SystemConfigError::ReasonError(
                "key or value is empty".to_string(),
            ));
        }
        let client = self.get_krpc_client()?;
        let _result = client
            .call("sys_config_set", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        self.remove_config_cache(key).await;
        Ok(0)
    }

    pub async fn set_by_json_path(
        &self,
        key: &str,
        json_path: &str,
        value: &str,
    ) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        client
            .call(
                "sys_config_set_by_json_path",
                json!({"key": key, "json_path": json_path, "value": value}),
            )
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        self.remove_config_cache(key).await;

        Ok(0)
    }

    pub async fn create(&self, key: &str, value: &str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        let _result = client
            .call("sys_config_create", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        self.remove_config_cache(key).await;
        Ok(0)
    }

    pub async fn delete(&self, key: &str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        let _result = client
            .call("sys_config_delete", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        self.remove_config_cache(key).await;
        Ok(0)
    }

    pub async fn append(&self, key: &str, value: &str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        client
            .call(
                "sys_config_append",
                json!({"key": key, "append_value": value}),
            )
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        self.remove_config_cache(key).await;

        Ok(0)
    }

    //list direct children
    pub async fn list(&self, key: &str) -> SytemConfigResult<Vec<String>> {
        let client = self.get_krpc_client()?;
        client
            .call("sys_config_list", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))
            .map(|result| {
                let mut list = Vec::new();
                for item in result.as_array().unwrap() {
                    list.push(item.as_str().unwrap().to_string());
                }
                list
            })
    }

    pub async fn exec_tx(
        &self,
        tx_actions: HashMap<String, KVAction>,
        main_key: Option<(String, u64)>,
    ) -> SytemConfigResult<u64> {
        if tx_actions.is_empty() {
            return Ok(0);
        }
        let mut tx_json = Map::new();

        for (key, action) in tx_actions.iter() {
            match action {
                KVAction::Create(value) => {
                    tx_json.insert(
                        key.to_string(),
                        json!({
                            "action": "create",
                            "value": value
                        }),
                    );
                }
                KVAction::Update(value) => {
                    tx_json.insert(
                        key.to_string(),
                        json!({
                            "action": "update",
                            "value": value
                        }),
                    );
                }
                KVAction::Append(value) => {
                    tx_json.insert(
                        key.to_string(),
                        json!({
                            "action": "append",
                            "value": value
                        }),
                    );
                }
                KVAction::SetByJsonPath(value) => {
                    tx_json.insert(
                        key.to_string(),
                        json!({
                            "action": "set_by_path",
                            "all_set": value
                        }),
                    );
                }
                KVAction::Remove => {
                    tx_json.insert(
                        key.to_string(),
                        json!({
                            "action": "remove"
                        }),
                    );
                }
            }
        }

        let mut req_params = Map::new();
        req_params.insert("actions".to_string(), Value::Object(tx_json));

        if let Some((key, revision)) = main_key {
            req_params.insert(
                "main_key".to_string(),
                json!({
                    "key": key,
                    "revision": revision,
                }),
            );
        }

        let client = self.get_krpc_client()?;
        client
            .call("sys_config_exec_tx", Value::Object(req_params))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        for (key, _action) in tx_actions.iter() {
            self.remove_config_cache(key).await;
        }
        Ok(0)
    }

    pub async fn dump_configs_for_scheduler(&self) -> SytemConfigResult<Value> {
        let client = self.get_krpc_client()?;
        let result = client
            .call("dump_configs_for_scheduler", json!({}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(result)
    }

    pub async fn refresh_trust_keys(&self) -> SytemConfigResult<()> {
        let client = self.get_krpc_client()?;
        client
            .call("sys_refresh_trust_keys", json!({}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_document_hostname_prefers_the_published_hostname() {
        let bns_zone = DID::new("bns", "issue39e2e");
        assert_eq!(
            select_zone_document_hostname("issue39e2e.web3.devtests.org.", &bns_zone),
            "issue39e2e.web3.devtests.org"
        );

        let legacy_web_zone = DID::new("web", "legacy.example");
        assert_eq!(
            select_zone_document_hostname("", &legacy_web_zone),
            "legacy.example"
        );
    }

    #[test]
    fn buckyos_dev_config_tracks_the_last_enable_event() {
        let mut config = BuckyOSDevConfig::default();
        config.validate().expect("disabled default should be valid");
        let default_value = serde_json::to_value(&config).expect("default should serialize");
        assert_eq!(default_value["enabled_at"], Value::Null);
        assert_eq!(default_value["enabled_by"], Value::Null);

        config
            .set_enabled(true, "alice", 1_788_000_000)
            .expect("enable should be valid");
        assert!(config.enabled);
        assert_eq!(config.enabled_at, Some(1_788_000_000));
        assert_eq!(config.enabled_by.as_deref(), Some("alice"));

        config
            .set_enabled(false, "alice", 1_788_000_100)
            .expect("disable should be valid");
        assert!(!config.enabled);
        assert_eq!(config.enabled_at, Some(1_788_000_000));
        assert_eq!(config.enabled_by.as_deref(), Some("alice"));

        config
            .set_enabled(true, "bob", 1_788_000_200)
            .expect("re-enable should be valid");
        assert_eq!(config.enabled_at, Some(1_788_000_200));
        assert_eq!(config.enabled_by.as_deref(), Some("bob"));
    }

    #[test]
    fn enabled_buckyos_dev_config_requires_enable_audit_fields() {
        let config = BuckyOSDevConfig {
            enabled: true,
            ..BuckyOSDevConfig::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn buckyos_info_splits_runtime_version_and_preserves_install_time() {
        let info = BuckyOSInfo::from_runtime(
            "0.7.0+build260829.main.abcdef123456 (nightly)",
            "nightly",
            "x86_64-unknown-linux-gnu",
            1_788_000_000,
        );

        assert_eq!(info.schema_version, BUCKYOS_INFO_SCHEMA_VERSION);
        assert_eq!(info.version, "0.7.0");
        assert_eq!(
            info.build_version.as_deref(),
            Some("build260829.main.abcdef123456")
        );
        assert_eq!(info.release_channel, "nightly");
        assert_eq!(info.target, "x86_64-unknown-linux-gnu");
        assert_eq!(info.installed_at, 1_788_000_000);
        assert_eq!(info.updated_at, info.installed_at);
        info.validate().expect("BuckyOSInfo should be valid");
    }

    #[test]
    fn buckyos_info_accepts_runtime_version_without_build_metadata() {
        let info = BuckyOSInfo::from_runtime(
            "0.7.0 (stable)",
            "stable",
            "aarch64-apple-darwin",
            1_788_000_000,
        );

        assert_eq!(info.version, "0.7.0");
        assert_eq!(info.build_version, None);
        info.validate().expect("BuckyOSInfo should be valid");
    }

    #[tokio::test]
    async fn cache_is_scoped_to_client_instance() {
        let client_a = SystemConfigClient::new(None, Some("token-a"));
        let client_b = SystemConfigClient::new(None, Some("token-b"));

        assert!(
            client_a
                .set_config_cache("services/demo", "value-a", 1)
                .await
        );
        assert_eq!(
            client_a.get_config_cache("services/demo").await,
            Some(("value-a".to_string(), 1))
        );
        assert_eq!(client_b.get_config_cache("services/demo").await, None);

        client_a.remove_config_cache("services/demo").await;
        assert_eq!(client_a.get_config_cache("services/demo").await, None);
    }

    #[tokio::test]
    async fn sync_session_token_updates_underlying_krpc_client() {
        let client = SystemConfigClient::new(None, Some("token-a"));

        client
            .sync_session_token(Some("token-b"))
            .await
            .expect("sync session token");

        assert_eq!(
            client.get_session_token().await,
            Some("token-b".to_string())
        );
        assert_eq!(
            client.client.get_session_token().await,
            Some("token-b".to_string())
        );
    }
}
