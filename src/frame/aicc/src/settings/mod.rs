#![allow(dead_code)]

use crate::catalog::{
    CatalogBuildError, CatalogBuildOptions, CatalogKind, CatalogSnapshot, CurrentCatalogFile,
    KnownProviderCatalog, ModelDriverCatalog, ProviderRulesCatalog,
};
use async_trait::async_trait;
use buckyos_api::{AiccRouteOverlay, SystemConfigClient, SystemConfigError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

pub(crate) const AICC_SETTINGS_KEY: &str = "services/aicc/settings";
pub(crate) const SYSTEM_CONFIG_METADATA_KEY: &str = "services/aicc/driver_metadata";
pub(crate) const LOCAL_METADATA_RELATIVE_DIR: &str = "etc/aicc/driver_metadata/local";
const SYSTEM_CONFIG_METADATA_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiccSettings {
    #[serde(default)]
    pub providers: Vec<ProviderSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_config: Option<AiccRouteOverlay>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderSettings {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub base_url: String,
    pub credentials: Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_rules: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_sync_models: Option<bool>,
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderSettings")
            .field("provider_instance_name", &self.provider_instance_name)
            .field("provider_type", &self.provider_type)
            .field("provider_profile_id", &self.provider_profile_id)
            .field("protocol_adapter_id", &self.protocol_adapter_id)
            .field("base_url", &self.base_url)
            .field("credentials", &"<redacted>")
            .field("enabled", &self.enabled)
            .field("region", &self.region)
            .field("workspace", &self.workspace)
            .field("account", &self.account)
            .field("provider_rules_id", &self.provider_rules_id)
            .field("auth", &self.auth.as_ref().map(|_| "<redacted>"))
            .field(
                "discovery",
                &self.discovery.as_ref().map(|_| "<configured>"),
            )
            .field(
                "instance_rules",
                &self.instance_rules.as_ref().map(|_| "<configured>"),
            )
            .field("timeout_ms", &self.timeout_ms)
            .field("auto_sync_models", &self.auto_sync_models)
            .finish()
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsDocument {
    pub revision: u64,
    pub settings: Arc<AiccSettings>,
}

impl SettingsDocument {
    pub(crate) fn parse(revision: u64, contents: &str) -> Result<Self, SettingsError> {
        let settings: AiccSettings = serde_json::from_str(contents)?;
        settings.validate()?;
        Ok(Self {
            revision,
            settings: Arc::new(settings),
        })
    }

    pub(crate) fn new(revision: u64, settings: AiccSettings) -> Result<Self, SettingsError> {
        settings.validate()?;
        Ok(Self {
            revision,
            settings: Arc::new(settings),
        })
    }
}

impl AiccSettings {
    pub(crate) fn validate(&self) -> Result<(), SettingsError> {
        let mut names = BTreeSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if !names.insert(provider.provider_instance_name.clone()) {
                return Err(SettingsError::DuplicateProvider(
                    provider.provider_instance_name.clone(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn enabled_provider_names(&self) -> BTreeSet<String> {
        self.providers
            .iter()
            .filter(|provider| provider.enabled)
            .map(|provider| provider.provider_instance_name.clone())
            .collect()
    }
}

impl ProviderSettings {
    fn validate(&self) -> Result<(), SettingsError> {
        validate_id("provider_instance_name", &self.provider_instance_name)?;
        validate_id("provider_profile_id", &self.provider_profile_id)?;
        validate_id("protocol_adapter_id", &self.protocol_adapter_id)?;
        validate_nonempty("provider_type", &self.provider_type)?;
        if let Some(provider_rules_id) = &self.provider_rules_id {
            validate_id("provider_rules_id", provider_rules_id)?;
        }
        validate_optional_nonempty("region", self.region.as_deref())?;
        validate_optional_nonempty("workspace", self.workspace.as_deref())?;
        validate_optional_nonempty("account", self.account.as_deref())?;
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| SettingsError::InvalidField {
            field: "base_url",
            reason: "must be an absolute URL".into(),
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
            return Err(SettingsError::InvalidField {
                field: "base_url",
                reason: "must use http or https and support relative paths".into(),
            });
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(SettingsError::InvalidField {
                field: "base_url",
                reason: "must not contain credentials, query, or fragment".into(),
            });
        }
        let credentials = self
            .credentials
            .as_object()
            .ok_or(SettingsError::InvalidField {
                field: "credentials",
                reason: "must be an object containing locked values or credential references"
                    .into(),
            })?;
        if credentials.is_empty() {
            return Err(SettingsError::InvalidField {
                field: "credentials",
                reason: "must not be empty".into(),
            });
        }
        reject_legacy_fields(&self.credentials)?;
        if !contains_protected_credential(&self.credentials) {
            return Err(SettingsError::InvalidField {
                field: "credentials",
                reason: "must contain a locked value or credential reference".into(),
            });
        }
        if self.timeout_ms == Some(0) {
            return Err(SettingsError::InvalidField {
                field: "timeout_ms",
                reason: "must be greater than zero".into(),
            });
        }
        Ok(())
    }
}

fn reject_legacy_fields(value: &Value) -> Result<(), SettingsError> {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if matches!(
                    name.as_str(),
                    "instance_id"
                        | "provider_driver"
                        | "endpoint"
                        | "api_key"
                        | "apiKey"
                        | "secret"
                ) {
                    return Err(SettingsError::InvalidField {
                        field: "credentials",
                        reason: format!("legacy or plaintext field `{name}` is not accepted"),
                    });
                }
                reject_legacy_fields(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_legacy_fields(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn contains_protected_credential(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(name, value)| {
            ((name == "locked" || name.ends_with("_ref"))
                && value.as_str().is_some_and(|value| !value.trim().is_empty()))
                || contains_protected_credential(value)
        }),
        Value::Array(values) => values.iter().any(contains_protected_credential),
        _ => false,
    }
}

fn validate_nonempty(field: &'static str, value: &str) -> Result<(), SettingsError> {
    if value.trim().is_empty() {
        return Err(SettingsError::InvalidField {
            field,
            reason: "must not be empty".into(),
        });
    }
    Ok(())
}

fn validate_optional_nonempty(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), SettingsError> {
    if let Some(value) = value {
        validate_nonempty(field, value)?;
    }
    Ok(())
}

fn validate_id(field: &'static str, value: &str) -> Result<(), SettingsError> {
    validate_nonempty(field, value)?;
    if value.contains('@') || value.chars().any(char::is_whitespace) {
        return Err(SettingsError::InvalidField {
            field,
            reason: "must not contain `@` or whitespace".into(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum MetadataSource {
    Builtin,
    Cloud,
    Local,
    SystemConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetadataFile {
    pub source: MetadataSource,
    pub kind: CatalogKind,
    pub catalog_id: String,
    pub contents: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MetadataOverrideSnapshot {
    pub local_revision: String,
    pub system_config_revision: u64,
    pub local: Vec<MetadataFile>,
    pub system_config: Vec<MetadataFile>,
}

#[async_trait]
pub(crate) trait MetadataOverrideLoader: Send + Sync {
    async fn load(&self) -> Result<MetadataOverrideSnapshot, SettingsError>;
}

pub(crate) struct StaticMetadataOverrideLoader {
    snapshot: MetadataOverrideSnapshot,
}

impl StaticMetadataOverrideLoader {
    pub(crate) fn new(local: Vec<MetadataFile>, system_config: Vec<MetadataFile>) -> Self {
        Self {
            snapshot: MetadataOverrideSnapshot {
                local,
                system_config,
                ..Default::default()
            },
        }
    }
}

#[async_trait]
impl MetadataOverrideLoader for StaticMetadataOverrideLoader {
    async fn load(&self) -> Result<MetadataOverrideSnapshot, SettingsError> {
        Ok(self.snapshot.clone())
    }
}

pub(crate) struct ProductionMetadataOverrideLoader {
    local_root: PathBuf,
    system_config_url: String,
    session_token: String,
}

impl ProductionMetadataOverrideLoader {
    pub(crate) fn new(
        buckyos_root: impl AsRef<Path>,
        system_config_url: impl Into<String>,
        session_token: impl Into<String>,
    ) -> Self {
        Self {
            local_root: buckyos_root.as_ref().join(LOCAL_METADATA_RELATIVE_DIR),
            system_config_url: system_config_url.into(),
            session_token: session_token.into(),
        }
    }

    pub(crate) fn local_root(&self) -> &Path {
        &self.local_root
    }

    async fn load_local(&self) -> Result<(String, Vec<MetadataFile>), SettingsError> {
        let first = read_local_metadata(&self.local_root).await?;
        let second = read_local_metadata(&self.local_root).await?;
        if first != second {
            return Err(SettingsError::MetadataSourceChanged(MetadataSource::Local));
        }
        let revision = metadata_content_revision(&first);
        let files = first
            .into_iter()
            .map(|file| MetadataFile::parse(MetadataSource::Local, file.kind, file.contents))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((revision, files))
    }

    async fn load_system_config(&self) -> Result<(u64, Vec<MetadataFile>), SettingsError> {
        let client =
            SystemConfigClient::new(Some(&self.system_config_url), Some(&self.session_token));
        let value = match client.get(SYSTEM_CONFIG_METADATA_KEY).await {
            Ok(value) => value,
            Err(SystemConfigError::KeyNotFound(_)) => return Ok((0, Vec::new())),
            Err(error) => return Err(SettingsError::SystemConfig(error.to_string())),
        };
        let document: SystemConfigMetadataDocument = serde_json::from_str(&value.value)?;
        if document.schema_version != SYSTEM_CONFIG_METADATA_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedMetadataSchema(
                document.schema_version,
            ));
        }
        Ok((value.version, document.into_files()?))
    }
}

#[async_trait]
impl MetadataOverrideLoader for ProductionMetadataOverrideLoader {
    async fn load(&self) -> Result<MetadataOverrideSnapshot, SettingsError> {
        let (local_revision, local) = self.load_local().await?;
        let (system_config_revision, system_config) = self.load_system_config().await?;
        Ok(MetadataOverrideSnapshot {
            local_revision,
            system_config_revision,
            local,
            system_config,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemConfigMetadataDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub model_drivers: Vec<Value>,
    #[serde(default)]
    pub provider_rules: Vec<Value>,
    #[serde(default)]
    pub known_providers: Vec<Value>,
}

impl SystemConfigMetadataDocument {
    fn into_files(self) -> Result<Vec<MetadataFile>, SettingsError> {
        let mut files = Vec::with_capacity(
            self.model_drivers.len() + self.provider_rules.len() + self.known_providers.len(),
        );
        for (kind, documents) in [
            (CatalogKind::ModelDriver, self.model_drivers),
            (CatalogKind::ProviderRules, self.provider_rules),
            (CatalogKind::KnownProvider, self.known_providers),
        ] {
            for document in documents {
                files.push(MetadataFile::parse(
                    MetadataSource::SystemConfig,
                    kind,
                    serde_json::to_vec(&document)?,
                )?);
            }
        }
        Ok(files)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalMetadataDocument {
    relative_path: String,
    kind: CatalogKind,
    contents: Vec<u8>,
}

async fn read_local_metadata(root: &Path) -> Result<Vec<LocalMetadataDocument>, SettingsError> {
    let mut documents = Vec::new();
    for (directory, kind) in [
        ("models", CatalogKind::ModelDriver),
        ("providers", CatalogKind::ProviderRules),
        ("known-providers", CatalogKind::KnownProvider),
    ] {
        let path = root.join(directory);
        let mut entries = match tokio::fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(SettingsError::MetadataIo(error)),
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(SettingsError::MetadataIo)?
        {
            let file_type = entry.file_type().await.map_err(SettingsError::MetadataIo)?;
            let file_name = entry.file_name();
            let file_name =
                file_name
                    .to_str()
                    .ok_or_else(|| SettingsError::InvalidMetadataPath {
                        path: entry.path(),
                        reason: "filename is not valid UTF-8".to_string(),
                    })?;
            if file_name.starts_with('.') {
                continue;
            }
            if !file_type.is_file()
                || Path::new(file_name)
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("json")
            {
                return Err(SettingsError::InvalidMetadataPath {
                    path: entry.path(),
                    reason: "only direct .json files are allowed".to_string(),
                });
            }
            documents.push(LocalMetadataDocument {
                relative_path: format!("{directory}/{file_name}"),
                kind,
                contents: tokio::fs::read(entry.path())
                    .await
                    .map_err(SettingsError::MetadataIo)?,
            });
        }
    }
    documents.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(documents)
}

fn metadata_content_revision(documents: &[LocalMetadataDocument]) -> String {
    let mut digest = Sha256::new();
    for document in documents {
        digest.update((document.relative_path.len() as u64).to_be_bytes());
        digest.update(document.relative_path.as_bytes());
        digest.update((document.contents.len() as u64).to_be_bytes());
        digest.update(&document.contents);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl MetadataFile {
    pub(crate) fn parse(
        source: MetadataSource,
        kind: CatalogKind,
        contents: impl Into<Vec<u8>>,
    ) -> Result<Self, SettingsError> {
        let contents = contents.into();
        let catalog_id = match kind {
            CatalogKind::ModelDriver => {
                serde_json::from_slice::<ModelDriverCatalog>(&contents)?.model_driver_id
            }
            CatalogKind::ProviderRules => {
                serde_json::from_slice::<ProviderRulesCatalog>(&contents)?.provider_profile_id
            }
            CatalogKind::KnownProvider => {
                serde_json::from_slice::<KnownProviderCatalog>(&contents)?.catalog_id
            }
        };
        validate_id("catalog_id", &catalog_id)?;
        Ok(Self {
            source,
            kind,
            catalog_id,
            contents,
        })
    }

    fn identity(&self) -> (CatalogKind, String) {
        (self.kind, self.catalog_id.clone())
    }

    fn into_current_file(self) -> CurrentCatalogFile {
        CurrentCatalogFile {
            kind: self.kind,
            contents: self.contents,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MetadataSources {
    pub builtin: Vec<MetadataFile>,
    pub cloud: Vec<MetadataFile>,
    pub local: Vec<MetadataFile>,
    pub system_config: Vec<MetadataFile>,
}

impl MetadataSources {
    pub(crate) fn resolve(self) -> Result<Vec<CurrentCatalogFile>, SettingsError> {
        let mut selected = BTreeMap::new();
        for (source, files) in [
            (MetadataSource::Builtin, self.builtin),
            (MetadataSource::Cloud, self.cloud),
            (MetadataSource::Local, self.local),
            (MetadataSource::SystemConfig, self.system_config),
        ] {
            let mut identities = BTreeSet::new();
            for file in files {
                if file.source != source {
                    return Err(SettingsError::MetadataSourceMismatch {
                        expected: source,
                        actual: file.source,
                        catalog_id: file.catalog_id,
                    });
                }
                let identity = file.identity();
                if !identities.insert(identity.clone()) {
                    return Err(SettingsError::DuplicateMetadataFile {
                        metadata_source: source,
                        kind: identity.0,
                        catalog_id: identity.1,
                    });
                }
                selected.insert(identity, file);
            }
        }
        Ok(selected
            .into_values()
            .map(MetadataFile::into_current_file)
            .collect())
    }

    pub(crate) fn build_snapshot(
        self,
        target_seq: u64,
        options: &CatalogBuildOptions,
    ) -> Result<Arc<CatalogSnapshot>, SettingsError> {
        Ok(Arc::new(CatalogSnapshot::from_current_files(
            target_seq,
            self.resolve()?,
            options,
        )?))
    }
}

#[derive(Debug, Error)]
pub(crate) enum SettingsError {
    #[error("invalid AICC settings JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("invalid settings field `{field}`: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("provider instance `{0}` appears more than once")]
    DuplicateProvider(String),
    #[error("duplicate {kind} catalog `{catalog_id}` in {metadata_source:?} metadata source")]
    DuplicateMetadataFile {
        metadata_source: MetadataSource,
        kind: CatalogKind,
        catalog_id: String,
    },
    #[error(
        "metadata file `{catalog_id}` declares {actual:?} source but was placed in {expected:?}"
    )]
    MetadataSourceMismatch {
        expected: MetadataSource,
        actual: MetadataSource,
        catalog_id: String,
    },
    #[error("effective catalog is invalid: {0}")]
    InvalidCatalog(#[from] CatalogBuildError),
    #[error("metadata filesystem I/O failed: {0}")]
    MetadataIo(#[source] std::io::Error),
    #[error("invalid metadata path `{path}`: {reason}")]
    InvalidMetadataPath { path: PathBuf, reason: String },
    #[error("{0:?} metadata changed while a reload snapshot was being captured")]
    MetadataSourceChanged(MetadataSource),
    #[error("unsupported system-config metadata schema version {0}")]
    UnsupportedMetadataSchema(u32),
    #[error("system-config metadata read failed: {0}")]
    SystemConfig(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(name: &str) -> Value {
        json!({
            "provider_instance_name": name,
            "provider_type": "cloud_api",
            "provider_profile_id": "openai",
            "protocol_adapter_id": "openai-responses",
            "base_url": "https://api.example/v1",
            "credentials": {
                "api_token": {"locked": "opaque"}
            }
        })
    }

    #[test]
    fn parses_only_unified_provider_array() {
        let document = SettingsDocument::parse(
            12,
            &json!({"providers": [provider("primary")], "session_config": {}}).to_string(),
        )
        .unwrap();
        assert_eq!(document.revision, 12);
        assert_eq!(
            document.settings.providers[0].provider_instance_name,
            "primary"
        );
        assert!(document.settings.providers[0].enabled);

        assert!(
            SettingsDocument::parse(1, &json!({"openai": {"instances": []}}).to_string()).is_err()
        );
        for legacy_field in ["provider_driver", "endpoint", "instance_id", "api_key"] {
            let mut legacy_provider = provider("legacy");
            legacy_provider[legacy_field] = json!("old-value");
            let legacy = json!({"providers": [legacy_provider]});
            assert!(SettingsDocument::parse(1, &legacy.to_string()).is_err());
        }
    }

    #[test]
    fn rejects_duplicate_names_invalid_urls_and_empty_credentials() {
        let duplicate = json!({"providers": [provider("same"), provider("same")]});
        assert!(matches!(
            SettingsDocument::parse(1, &duplicate.to_string()),
            Err(SettingsError::DuplicateProvider(name)) if name == "same"
        ));

        let mut invalid = provider("invalid");
        invalid["base_url"] = json!("file:///tmp/models");
        assert!(SettingsDocument::parse(1, &json!({"providers": [invalid]}).to_string()).is_err());

        let mut empty = provider("empty");
        empty["credentials"] = json!({});
        assert!(SettingsDocument::parse(1, &json!({"providers": [empty]}).to_string()).is_err());

        let mut plaintext = provider("plaintext");
        plaintext["credentials"] = json!({"api_key": "not-allowed"});
        assert!(
            SettingsDocument::parse(1, &json!({"providers": [plaintext]}).to_string()).is_err()
        );
    }

    #[test]
    fn settings_debug_redacts_credentials_and_private_auth() {
        let secret = "must-not-appear";
        let mut value = provider("primary");
        value["workspace"] = json!("engineering");
        value["credentials"] = json!({"api_token": {"locked": secret}});
        value["auth"] = json!({"credential_ref": secret});
        let document =
            SettingsDocument::parse(1, &json!({"providers": [value]}).to_string()).unwrap();
        let debug = format!("{document:?}");
        assert!(!debug.contains(secret));
        assert!(debug.contains("engineering"));

        let mut unsafe_url = provider("unsafe-url");
        unsafe_url["base_url"] = json!("https://token@example.test/v1?api_key=secret");
        assert!(
            SettingsDocument::parse(1, &json!({"providers": [unsafe_url]}).to_string()).is_err()
        );
    }

    #[test]
    fn provider_settings_workspace_round_trips_and_rejects_empty_values() {
        let mut value = provider("primary");
        value["region"] = json!("us-west");
        value["workspace"] = json!("engineering");
        value["account"] = json!("production");
        let document =
            SettingsDocument::parse(7, &json!({"providers": [value]}).to_string()).unwrap();
        assert_eq!(
            document.settings.providers[0].workspace.as_deref(),
            Some("engineering")
        );

        let serialized = serde_json::to_string(document.settings.as_ref()).unwrap();
        let round_tripped = SettingsDocument::parse(7, &serialized).unwrap();
        assert_eq!(round_tripped, document);

        let mut invalid = provider("invalid");
        invalid["workspace"] = json!("  ");
        assert!(matches!(
            SettingsDocument::parse(1, &json!({"providers": [invalid]}).to_string()),
            Err(SettingsError::InvalidField {
                field: "workspace",
                ..
            })
        ));
    }

    fn model_driver_document(id: &str, revision: u64) -> Value {
        json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": id,
            "revision_seq": revision,
            "required_features": [],
            "models": [],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        })
    }

    #[tokio::test]
    async fn local_metadata_loader_enumerates_three_fixed_directories() {
        let temp = tempfile::tempdir().unwrap();
        let local_root = temp.path().join(LOCAL_METADATA_RELATIVE_DIR);
        tokio::fs::create_dir_all(local_root.join("models"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(local_root.join("providers"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(local_root.join("known-providers"))
            .await
            .unwrap();
        tokio::fs::write(
            local_root.join("models/vendor.json"),
            serde_json::to_vec(&model_driver_document("vendor", 1)).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            local_root.join("providers/vendor.json"),
            serde_json::to_vec(&json!({
                "format": "buckyos.aicc.provider-rules-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "revision_seq": 1,
                "provider_profile_id": "vendor",
                "models": [],
                "patterns": [],
                "variants": []
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            local_root.join("known-providers/vendor.json"),
            serde_json::to_vec(&json!({
                "format": "buckyos.aicc.known-provider-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "revision_seq": 1,
                "catalog_id": "vendor",
                "providers": []
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loader = ProductionMetadataOverrideLoader::new(
            temp.path(),
            "http://system-config.invalid",
            "token",
        );
        assert_eq!(loader.local_root(), local_root);
        let (first_revision, files) = loader.load_local().await.unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(
            files.iter().map(|file| file.kind).collect::<BTreeSet<_>>(),
            BTreeSet::from([
                CatalogKind::ModelDriver,
                CatalogKind::ProviderRules,
                CatalogKind::KnownProvider,
            ])
        );

        tokio::fs::write(
            loader.local_root().join("models/vendor.json"),
            serde_json::to_vec(&model_driver_document("vendor", 2)).unwrap(),
        )
        .await
        .unwrap();
        let (second_revision, _) = loader.load_local().await.unwrap();
        assert_ne!(first_revision, second_revision);
    }

    #[tokio::test]
    async fn local_metadata_loader_rejects_non_document_entries() {
        let temp = tempfile::tempdir().unwrap();
        let local_root = temp.path().join(LOCAL_METADATA_RELATIVE_DIR).join("models");
        tokio::fs::create_dir_all(local_root.join("nested"))
            .await
            .unwrap();
        let loader = ProductionMetadataOverrideLoader::new(
            temp.path(),
            "http://system-config.invalid",
            "token",
        );
        assert!(matches!(
            loader.load_local().await,
            Err(SettingsError::InvalidMetadataPath { .. })
        ));
    }

    #[test]
    fn system_config_metadata_document_enumerates_all_catalog_kinds() {
        let document: SystemConfigMetadataDocument = serde_json::from_value(json!({
            "schema_version": 1,
            "model_drivers": [model_driver_document("vendor", 1)],
            "provider_rules": [{
                "format": "buckyos.aicc.provider-rules-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "revision_seq": 1,
                "provider_profile_id": "vendor",
                "models": [],
                "patterns": [],
                "variants": []
            }],
            "known_providers": [{
                "format": "buckyos.aicc.known-provider-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "revision_seq": 1,
                "catalog_id": "vendor",
                "providers": []
            }]
        }))
        .unwrap();
        let files = document.into_files().unwrap();
        assert_eq!(files.len(), 3);
        assert!(files
            .iter()
            .all(|file| file.source == MetadataSource::SystemConfig));
    }

    fn model_driver(source: MetadataSource, id: &str, revision: u64, marker: &str) -> MetadataFile {
        MetadataFile::parse(
            source,
            CatalogKind::ModelDriver,
            serde_json::to_vec(&json!({
                "format": "buckyos.aicc.model-driver-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "model_driver_id": id,
                "revision_seq": revision,
                "required_features": [],
                "models": [],
                "patterns": [],
                "defaults": {"parameter_scale": marker},
                "variants": [],
                "version_rules": []
            }))
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn metadata_resolution_selects_whole_file_by_identity_and_priority() {
        let sources = MetadataSources {
            builtin: vec![
                model_driver(MetadataSource::Builtin, "openai", 1, "builtin"),
                model_driver(MetadataSource::Builtin, "claude", 1, "builtin"),
            ],
            cloud: vec![model_driver(MetadataSource::Cloud, "openai", 2, "cloud")],
            local: vec![model_driver(MetadataSource::Local, "openai", 3, "local")],
            system_config: vec![model_driver(
                MetadataSource::SystemConfig,
                "openai",
                4,
                "system-config",
            )],
        };
        let snapshot = sources
            .build_snapshot(4, &CatalogBuildOptions::default())
            .unwrap();
        assert_eq!(
            snapshot
                .model_driver("openai")
                .unwrap()
                .defaults
                .parameter_scale
                .as_deref(),
            Some("system-config")
        );
        assert!(snapshot.model_driver("claude").is_some());
    }

    #[test]
    fn duplicate_identity_inside_one_source_is_rejected() {
        let sources = MetadataSources {
            builtin: vec![
                model_driver(MetadataSource::Builtin, "openai", 1, "one"),
                model_driver(MetadataSource::Builtin, "openai", 1, "two"),
            ],
            ..MetadataSources::default()
        };
        assert!(matches!(
            sources.resolve(),
            Err(SettingsError::DuplicateMetadataFile { catalog_id, .. }) if catalog_id == "openai"
        ));
    }
}
