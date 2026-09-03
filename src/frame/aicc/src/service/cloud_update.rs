use async_trait::async_trait;
use ndn_lib::ObjId;
use ndn_toolkit::cyfs_ndn_client::CyfsNdnClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{broadcast, watch, Mutex, Notify, RwLock};

use crate::catalog::{CatalogBuildOptions, CatalogKind, CatalogSnapshot, CurrentCatalogFile};
use crate::matching::{CompiledMatchRule, MatchContext, MatchRule, RELEASE_TRACK_MATCH_SCHEMA};
use crate::runtime::{RuntimeError, RuntimeInputs};
use crate::settings::{
    MetadataFile, MetadataOverrideLoader, MetadataSource, MetadataSources,
    StaticMetadataOverrideLoader,
};

const INDEX_VERSION: u32 = 2;
const PROTOCOL_VERSION: u32 = 2;
const INDEX_FORMAT: &str = "buckyos.aicc.provider-catalog-index";
const MANIFEST_FORMAT: &str = "buckyos.aicc.provider-catalog-manifest";
const INDEX_PATH: &str = "aicc/provider-catalog/index.json";
const STATE_FILE: &str = "state.json";
const REVISIONS_DIR: &str = "revisions";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCatalogIndex {
    pub format: String,
    pub index_version: u32,
    pub index_revision_seq: u64,
    #[serde(default)]
    pub tracks: Vec<ProviderCatalogTrack>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCatalogTrack {
    pub revision_seq: u64,
    pub manifest_path: String,
    pub manifest_obj_id: String,
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub required_features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCatalogManifest {
    pub format: String,
    pub protocol_version: u32,
    pub revision_seq: u64,
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub required_features: Vec<String>,
    #[serde(default)]
    pub files: Vec<ProviderCatalogManifestFile>,
    #[serde(default)]
    pub tombstones: Vec<ProviderCatalogTombstone>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCatalogManifestFile {
    pub catalog_kind: CloudCatalogKind,
    pub catalog_id: String,
    pub path: String,
    pub schema_version: u32,
    pub revision_seq: u64,
    pub obj_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCatalogTombstone {
    pub catalog_kind: CloudCatalogKind,
    pub catalog_id: String,
    pub revision_seq: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CloudCatalogKind {
    ModelDriver,
    ProviderRules,
    KnownProvider,
}

impl From<CloudCatalogKind> for CatalogKind {
    fn from(value: CloudCatalogKind) -> Self {
        match value {
            CloudCatalogKind::ModelDriver => Self::ModelDriver,
            CloudCatalogKind::ProviderRules => Self::ProviderRules,
            CloudCatalogKind::KnownProvider => Self::KnownProvider,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloudUpdateClientProfile {
    pub client_version: String,
    pub update_channel: String,
    pub rollout_group: String,
    pub supported_features: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloudUpdateConfig {
    pub enabled: bool,
    pub source_url: Option<String>,
    pub interval_secs: u64,
}

impl Default for CloudUpdateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source_url: None,
            interval_secs: 900,
        }
    }
}

impl CloudUpdateConfig {
    pub(crate) fn validate(&self) -> Result<(), CloudUpdateError> {
        if self.enabled && self.source_url.as_deref().is_none_or(str::is_empty) {
            return Err(CloudUpdateError::InvalidConfig(
                "enabled cloud update requires source_url".to_string(),
            ));
        }
        if self.interval_secs == 0 {
            return Err(CloudUpdateError::InvalidConfig(
                "interval_secs must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudCacheState {
    target_seq: u64,
    index_revision_seq: u64,
    manifest_obj_id: String,
    files: Vec<CachedCatalogFile>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedCatalogFile {
    catalog_kind: CloudCatalogKind,
    catalog_id: String,
    file_name: String,
    obj_id: String,
    sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloudUpdateEvent {
    pub target_seq: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CloudUpdateRuntimeStatus {
    pub updating: bool,
    pub active_revision: Option<u64>,
    pub last_attempt_at_ms: Option<u64>,
    pub last_success_at_ms: Option<u64>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Error)]
pub(crate) enum CloudUpdateError {
    #[error("invalid cloud update config: {0}")]
    InvalidConfig(String),
    #[error("invalid cloud update protocol: {0}")]
    InvalidProtocol(String),
    #[error("cloud update download failed: {0}")]
    Download(String),
    #[error("cloud update cache I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("cloud update JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cloud catalog is invalid: {0}")]
    Catalog(String),
}

#[async_trait]
pub(crate) trait CloudObjectFetcher: Send + Sync {
    async fn fetch(
        &self,
        url: &str,
        expected_obj_id: Option<&str>,
    ) -> Result<Vec<u8>, CloudUpdateError>;
}

pub(crate) struct NdnCloudObjectFetcher {
    session_token: String,
}

impl NdnCloudObjectFetcher {
    pub(crate) fn new(session_token: impl Into<String>) -> Self {
        Self {
            session_token: session_token.into(),
        }
    }
}

#[async_trait]
impl CloudObjectFetcher for NdnCloudObjectFetcher {
    async fn fetch(
        &self,
        url: &str,
        expected_obj_id: Option<&str>,
    ) -> Result<Vec<u8>, CloudUpdateError> {
        let url = url.to_string();
        let token = self.session_token.clone();
        let expected = expected_obj_id.map(str::to_string);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("aicc-cloud-fetch".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let result = match runtime {
                    Ok(runtime) => {
                        let local = tokio::task::LocalSet::new();
                        local.block_on(&runtime, fetch_ndn_object(url, token, expected))
                    }
                    Err(error) => Err(CloudUpdateError::Download(error.to_string())),
                };
                let _ = sender.send(result);
            })
            .map_err(|error| CloudUpdateError::Download(error.to_string()))?;
        receiver
            .await
            .map_err(|_| CloudUpdateError::Download("NDN fetch worker stopped".to_string()))?
    }
}

async fn fetch_ndn_object(
    url: String,
    token: String,
    expected_obj_id: Option<String>,
) -> Result<Vec<u8>, CloudUpdateError> {
    let mut builder = CyfsNdnClient::builder();
    if !token.is_empty() {
        builder = builder.session_token(token);
    }
    let client = builder
        .build()
        .map_err(|error| CloudUpdateError::Download(error.to_string()))?;
    let mut request = client.get(url);
    if let Some(expected) = expected_obj_id.as_deref() {
        let obj_id = ObjId::new(expected)
            .map_err(|error| CloudUpdateError::InvalidProtocol(error.to_string()))?;
        request = request.obj_id(obj_id);
    }
    let (actual_obj_id, contents) = request
        .send()
        .await
        .map_err(|error| CloudUpdateError::Download(error.to_string()))?
        .object_string()
        .await
        .map_err(|error| CloudUpdateError::Download(error.to_string()))?;
    if expected_obj_id
        .as_deref()
        .is_some_and(|expected| expected != actual_obj_id.to_string())
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "downloaded object identity does not match manifest".to_string(),
        ));
    }
    Ok(contents.into_bytes())
}

pub(crate) struct CloudUpdateManager {
    cache_root: PathBuf,
    fetcher: Arc<dyn CloudObjectFetcher>,
    profile: CloudUpdateClientProfile,
    config: RwLock<CloudUpdateConfig>,
    update_lock: Mutex<()>,
    events: broadcast::Sender<CloudUpdateEvent>,
    stop: watch::Sender<bool>,
    wake: Notify,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    status: RwLock<CloudUpdateRuntimeStatus>,
    builtin: Vec<CurrentCatalogFile>,
    overrides: Arc<dyn MetadataOverrideLoader>,
}

impl CloudUpdateManager {
    pub(crate) fn new(
        cache_root: impl Into<PathBuf>,
        fetcher: Arc<dyn CloudObjectFetcher>,
        profile: CloudUpdateClientProfile,
        config: CloudUpdateConfig,
        builtin: Vec<CurrentCatalogFile>,
        local: Vec<MetadataFile>,
        system_config: Vec<MetadataFile>,
    ) -> Result<Arc<Self>, CloudUpdateError> {
        Self::new_with_override_loader(
            cache_root,
            fetcher,
            profile,
            config,
            builtin,
            Arc::new(StaticMetadataOverrideLoader::new(local, system_config)),
        )
    }

    pub(crate) fn new_with_override_loader(
        cache_root: impl Into<PathBuf>,
        fetcher: Arc<dyn CloudObjectFetcher>,
        profile: CloudUpdateClientProfile,
        config: CloudUpdateConfig,
        builtin: Vec<CurrentCatalogFile>,
        overrides: Arc<dyn MetadataOverrideLoader>,
    ) -> Result<Arc<Self>, CloudUpdateError> {
        config.validate()?;
        let (events, _) = broadcast::channel(16);
        let (stop, _) = watch::channel(false);
        Ok(Arc::new(Self {
            cache_root: cache_root.into(),
            fetcher,
            profile,
            config: RwLock::new(config),
            update_lock: Mutex::new(()),
            events,
            stop,
            wake: Notify::new(),
            task: Mutex::new(None),
            status: RwLock::new(CloudUpdateRuntimeStatus::default()),
            builtin,
            overrides,
        }))
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<CloudUpdateEvent> {
        self.events.subscribe()
    }

    pub(crate) async fn config(&self) -> CloudUpdateConfig {
        self.config.read().await.clone()
    }

    pub(crate) async fn set_config(
        &self,
        config: CloudUpdateConfig,
    ) -> Result<(), CloudUpdateError> {
        config.validate()?;
        *self.config.write().await = config;
        self.wake.notify_one();
        Ok(())
    }

    pub(crate) async fn status(&self) -> CloudUpdateRuntimeStatus {
        let mut status = self.status.read().await.clone();
        status.active_revision = self
            .load_state()
            .await
            .ok()
            .flatten()
            .map(|state| state.target_seq);
        status
    }

    pub(crate) async fn start(self: &Arc<Self>) {
        let mut task = self.task.lock().await;
        if task.is_some() {
            return;
        }
        let this = self.clone();
        let mut stop = self.stop.subscribe();
        *task = Some(tokio::spawn(async move {
            loop {
                let config = this.config().await;
                if config.enabled {
                    let _ = this.check_once().await;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(config.interval_secs.max(1))) => {}
                    _ = this.wake.notified() => {}
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() { break; }
                    }
                }
            }
        }));
    }

    pub(crate) async fn shutdown(&self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }

    pub(crate) async fn check_once(&self) -> Result<Option<u64>, CloudUpdateError> {
        {
            let mut status = self.status.write().await;
            status.updating = true;
            status.last_attempt_at_ms = now_ms();
        }
        let result = self.check_once_inner().await;
        let mut status = self.status.write().await;
        status.updating = false;
        match &result {
            Ok(_) => {
                status.last_success_at_ms = status.last_attempt_at_ms;
                status.last_error = None;
                status.consecutive_failures = 0;
            }
            Err(error) => {
                status.last_error = Some(error.to_string());
                status.consecutive_failures = status.consecutive_failures.saturating_add(1);
            }
        }
        result
    }

    async fn check_once_inner(&self) -> Result<Option<u64>, CloudUpdateError> {
        let _guard = self.update_lock.lock().await;
        let config = self.config().await;
        config.validate()?;
        if !config.enabled {
            return Ok(None);
        }
        let source = config.source_url.as_deref().unwrap();
        let index_url = join_url(source, INDEX_PATH)?;
        let index: ProviderCatalogIndex =
            serde_json::from_slice(&self.fetcher.fetch(&index_url, None).await?)?;
        validate_index(&index)?;
        let current = self.load_state().await?;
        let current_seq = current.as_ref().map_or(0, |state| state.target_seq);
        reject_reused_revision(&index, &self.profile, current.as_ref())?;
        let Some(track) = select_track(&index, &self.profile, current_seq)? else {
            return Ok(None);
        };
        let manifest_url = join_url(source, &track.manifest_path)?;
        let manifest: ProviderCatalogManifest = serde_json::from_slice(
            &self
                .fetcher
                .fetch(&manifest_url, Some(&track.manifest_obj_id))
                .await?,
        )?;
        validate_manifest(&manifest, track, &self.profile, current_seq)?;
        validate_cloud_transition(current.as_ref(), &manifest)?;
        let mut downloaded = Vec::with_capacity(manifest.files.len());
        for file in &manifest.files {
            let url = join_url(source, &file.path)?;
            let contents = self.fetcher.fetch(&url, Some(&file.obj_id)).await?;
            verify_file(file, &contents)?;
            downloaded.push((file.clone(), contents));
        }
        self.validate_effective_catalog(manifest.revision_seq, &downloaded)
            .await?;
        if self.config().await != config {
            return Ok(None);
        }
        self.commit(
            index.index_revision_seq,
            &track.manifest_obj_id,
            manifest.revision_seq,
            downloaded,
        )
        .await?;
        let _ = self.events.send(CloudUpdateEvent {
            target_seq: manifest.revision_seq,
        });
        Ok(Some(manifest.revision_seq))
    }

    async fn commit(
        &self,
        index_revision_seq: u64,
        manifest_obj_id: &str,
        target_seq: u64,
        files: Vec<(ProviderCatalogManifestFile, Vec<u8>)>,
    ) -> Result<(), CloudUpdateError> {
        tokio::fs::create_dir_all(self.cache_root.join(REVISIONS_DIR)).await?;
        let staging = self.cache_root.join(format!(".staging-{target_seq}"));
        if tokio::fs::try_exists(&staging).await? {
            tokio::fs::remove_dir_all(&staging).await?;
        }
        tokio::fs::create_dir(&staging).await?;
        let mut cached = Vec::with_capacity(files.len());
        for (position, (file, contents)) in files.into_iter().enumerate() {
            let file_name = format!(
                "{position:04}-{}-{}.json",
                kind_name(file.catalog_kind),
                file.catalog_id
            );
            validate_file_name(&file_name)?;
            let path = staging.join(&file_name);
            tokio::fs::write(&path, &contents).await?;
            sync_file(&path).await?;
            cached.push(CachedCatalogFile {
                catalog_kind: file.catalog_kind,
                catalog_id: file.catalog_id,
                file_name,
                obj_id: file.obj_id,
                sha256: hex_sha256(&contents),
            });
        }
        let state = CloudCacheState {
            target_seq,
            index_revision_seq,
            manifest_obj_id: manifest_obj_id.to_string(),
            files: cached,
        };
        let staged_state = staging.join(STATE_FILE);
        tokio::fs::write(&staged_state, serde_json::to_vec_pretty(&state)?).await?;
        sync_file(&staged_state).await?;
        sync_directory(&staging).await?;
        let revision_dir = self
            .cache_root
            .join(REVISIONS_DIR)
            .join(target_seq.to_string());
        if tokio::fs::try_exists(&revision_dir).await? {
            let existing = load_state_file(&revision_dir.join(STATE_FILE)).await?;
            if existing != state {
                return Err(CloudUpdateError::InvalidProtocol(
                    "target revision already exists with different contents".to_string(),
                ));
            }
            verify_cached_revision(&revision_dir, &existing).await?;
            tokio::fs::remove_dir_all(&staging).await?;
        } else {
            tokio::fs::rename(&staging, &revision_dir).await?;
            sync_directory(&self.cache_root.join(REVISIONS_DIR)).await?;
        }
        let state_tmp = self.cache_root.join("state.json.tmp");
        tokio::fs::write(&state_tmp, serde_json::to_vec_pretty(&state)?).await?;
        sync_file(&state_tmp).await?;
        tokio::fs::rename(&state_tmp, self.cache_root.join(STATE_FILE)).await?;
        sync_directory(&self.cache_root).await?;
        Ok(())
    }

    async fn load_state(&self) -> Result<Option<CloudCacheState>, CloudUpdateError> {
        let path = self.cache_root.join(STATE_FILE);
        if !tokio::fs::try_exists(&path).await? {
            return Ok(None);
        }
        let state = load_state_file(&path).await?;
        validate_cached_state(&state)?;
        Ok(Some(state))
    }

    async fn load_cloud_files(
        &self,
        target_seq: u64,
    ) -> Result<Vec<MetadataFile>, CloudUpdateError> {
        if target_seq == 0 {
            return Ok(Vec::new());
        }
        let revision_dir = self
            .cache_root
            .join(REVISIONS_DIR)
            .join(target_seq.to_string());
        let state = load_state_file(&revision_dir.join(STATE_FILE)).await?;
        validate_cached_state(&state)?;
        if state.target_seq != target_seq {
            return Err(CloudUpdateError::InvalidProtocol(
                "cached cloud revision does not match requested target".to_string(),
            ));
        }
        verify_cached_revision(&revision_dir, &state).await?;
        let mut files = Vec::with_capacity(state.files.len());
        for cached in state.files {
            let contents = tokio::fs::read(revision_dir.join(&cached.file_name)).await?;
            files.push(
                MetadataFile::parse(MetadataSource::Cloud, cached.catalog_kind.into(), contents)
                    .map_err(|error| CloudUpdateError::Catalog(error.to_string()))?,
            );
        }
        Ok(files)
    }

    async fn validate_effective_catalog(
        &self,
        target_seq: u64,
        files: &[(ProviderCatalogManifestFile, Vec<u8>)],
    ) -> Result<(), CloudUpdateError> {
        let builtin = self
            .builtin
            .iter()
            .map(|file| {
                MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents.clone())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CloudUpdateError::Catalog(error.to_string()))?;
        let cloud = files
            .iter()
            .map(|(file, contents)| {
                MetadataFile::parse(
                    MetadataSource::Cloud,
                    file.catalog_kind.into(),
                    contents.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CloudUpdateError::Catalog(error.to_string()))?;
        let overrides = self
            .overrides
            .load()
            .await
            .map_err(|error| CloudUpdateError::Catalog(error.to_string()))?;
        MetadataSources {
            builtin,
            cloud,
            local: overrides.local,
            system_config: overrides.system_config,
        }
        .build_snapshot(target_seq, &CatalogBuildOptions::default())
        .map_err(|error| CloudUpdateError::Catalog(error.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl RuntimeInputs for CloudUpdateManager {
    async fn metadata_target_seq(&self) -> Result<u64, RuntimeError> {
        self.load_state()
            .await
            .map(|state| state.map_or(0, |state| state.target_seq))
            .map_err(|error| RuntimeError::Backend(error.to_string()))
    }

    async fn load_catalog(&self, target_seq: u64) -> Result<Arc<CatalogSnapshot>, RuntimeError> {
        let cloud = self
            .load_cloud_files(target_seq)
            .await
            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
        let builtin = self
            .builtin
            .iter()
            .map(|file| {
                MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents.clone())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
        let overrides = self
            .overrides
            .load()
            .await
            .map_err(|error| RuntimeError::Backend(error.to_string()))?;
        MetadataSources {
            builtin,
            cloud,
            local: overrides.local,
            system_config: overrides.system_config,
        }
        .build_snapshot(target_seq, &CatalogBuildOptions::default())
        .map_err(|error| RuntimeError::Backend(error.to_string()))
    }
}

fn validate_index(index: &ProviderCatalogIndex) -> Result<(), CloudUpdateError> {
    if index.format != INDEX_FORMAT
        || index.index_version != INDEX_VERSION
        || index.index_revision_seq == 0
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "unsupported or invalid catalog index".to_string(),
        ));
    }
    let mut revisions = BTreeSet::new();
    for track in &index.tracks {
        if track.revision_seq == 0
            || !revisions.insert(track.revision_seq)
            || track.manifest_obj_id.is_empty()
        {
            return Err(CloudUpdateError::InvalidProtocol(
                "catalog index contains an invalid track".to_string(),
            ));
        }
        validate_relative_path(&track.manifest_path)?;
        CompiledMatchRule::compile(track.match_rule.clone(), &RELEASE_TRACK_MATCH_SCHEMA)
            .map_err(|error| CloudUpdateError::InvalidProtocol(error.to_string()))?;
    }
    Ok(())
}

fn select_track<'a>(
    index: &'a ProviderCatalogIndex,
    profile: &CloudUpdateClientProfile,
    current_seq: u64,
) -> Result<Option<&'a ProviderCatalogTrack>, CloudUpdateError> {
    let context = release_context(profile);
    let mut selected = None;
    for track in &index.tracks {
        if track.revision_seq <= current_seq
            || !track
                .required_features
                .iter()
                .all(|feature| profile.supported_features.contains(feature))
        {
            continue;
        }
        let matcher =
            CompiledMatchRule::compile(track.match_rule.clone(), &RELEASE_TRACK_MATCH_SCHEMA)
                .map_err(|error| CloudUpdateError::InvalidProtocol(error.to_string()))?;
        if matcher.matches(&context)
            && selected.is_none_or(|current: &ProviderCatalogTrack| {
                current.revision_seq < track.revision_seq
            })
        {
            selected = Some(track);
        }
    }
    Ok(selected)
}

fn validate_manifest(
    manifest: &ProviderCatalogManifest,
    track: &ProviderCatalogTrack,
    profile: &CloudUpdateClientProfile,
    current_seq: u64,
) -> Result<(), CloudUpdateError> {
    if manifest.format != MANIFEST_FORMAT
        || manifest.protocol_version != PROTOCOL_VERSION
        || manifest.revision_seq != track.revision_seq
        || manifest.revision_seq <= current_seq
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "manifest version or revision does not match selected track".to_string(),
        ));
    }
    if !manifest
        .required_features
        .iter()
        .all(|feature| profile.supported_features.contains(feature))
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "manifest requires unsupported features".to_string(),
        ));
    }
    let matcher =
        CompiledMatchRule::compile(manifest.match_rule.clone(), &RELEASE_TRACK_MATCH_SCHEMA)
            .map_err(|error| CloudUpdateError::InvalidProtocol(error.to_string()))?;
    if !matcher.matches(&release_context(profile)) {
        return Err(CloudUpdateError::InvalidProtocol(
            "manifest is incompatible with this client".to_string(),
        ));
    }
    let mut identities = BTreeSet::new();
    for file in &manifest.files {
        validate_relative_path(&file.path)?;
        if file.catalog_id.is_empty()
            || file.obj_id.is_empty()
            || file.schema_version == 0
            || file.revision_seq > manifest.revision_seq
            || !identities.insert((file.catalog_kind, file.catalog_id.clone()))
        {
            return Err(CloudUpdateError::InvalidProtocol(
                "manifest contains an invalid catalog file".to_string(),
            ));
        }
    }
    for tombstone in &manifest.tombstones {
        if tombstone.catalog_id.is_empty()
            || tombstone.revision_seq > manifest.revision_seq
            || !identities.insert((tombstone.catalog_kind, tombstone.catalog_id.clone()))
        {
            return Err(CloudUpdateError::InvalidProtocol(
                "manifest contains an invalid tombstone".to_string(),
            ));
        }
    }
    Ok(())
}

fn reject_reused_revision(
    index: &ProviderCatalogIndex,
    profile: &CloudUpdateClientProfile,
    current: Option<&CloudCacheState>,
) -> Result<(), CloudUpdateError> {
    let Some(current) = current else {
        return Ok(());
    };
    let context = release_context(profile);
    for track in &index.tracks {
        if track.revision_seq != current.target_seq
            || !track
                .required_features
                .iter()
                .all(|feature| profile.supported_features.contains(feature))
        {
            continue;
        }
        let matcher =
            CompiledMatchRule::compile(track.match_rule.clone(), &RELEASE_TRACK_MATCH_SCHEMA)
                .map_err(|error| CloudUpdateError::InvalidProtocol(error.to_string()))?;
        if matcher.matches(&context) && track.manifest_obj_id != current.manifest_obj_id {
            return Err(CloudUpdateError::InvalidProtocol(
                "manifest revision was reused with a different object identity".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_cloud_transition(
    current: Option<&CloudCacheState>,
    manifest: &ProviderCatalogManifest,
) -> Result<(), CloudUpdateError> {
    let previous = current
        .into_iter()
        .flat_map(|state| state.files.iter())
        .map(|file| (file.catalog_kind, file.catalog_id.as_str()))
        .collect::<BTreeSet<_>>();
    let next = manifest
        .files
        .iter()
        .map(|file| (file.catalog_kind, file.catalog_id.as_str()))
        .collect::<BTreeSet<_>>();
    let tombstones = manifest
        .tombstones
        .iter()
        .map(|file| (file.catalog_kind, file.catalog_id.as_str()))
        .collect::<BTreeSet<_>>();
    if previous
        .difference(&next)
        .any(|identity| !tombstones.contains(identity))
        || tombstones
            .iter()
            .any(|identity| !previous.contains(identity))
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "cloud catalog deletion does not match the previous release".to_string(),
        ));
    }
    Ok(())
}

fn verify_file(
    file: &ProviderCatalogManifestFile,
    contents: &[u8],
) -> Result<(), CloudUpdateError> {
    if file
        .sha256
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&hex_sha256(contents)))
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "downloaded catalog digest does not match manifest".to_string(),
        ));
    }
    MetadataFile::parse(
        MetadataSource::Cloud,
        file.catalog_kind.into(),
        contents.to_vec(),
    )
    .map_err(|error| CloudUpdateError::Catalog(error.to_string()))
    .and_then(|parsed| {
        if parsed.catalog_id == file.catalog_id {
            Ok(())
        } else {
            Err(CloudUpdateError::InvalidProtocol(
                "downloaded catalog identity does not match manifest".to_string(),
            ))
        }
    })
}

fn release_context(profile: &CloudUpdateClientProfile) -> MatchContext {
    BTreeMap::from([
        (
            "client_version".to_string(),
            serde_json::Value::String(profile.client_version.clone()),
        ),
        (
            "update_channel".to_string(),
            serde_json::Value::String(profile.update_channel.clone()),
        ),
        (
            "rollout_group".to_string(),
            serde_json::Value::String(profile.rollout_group.clone()),
        ),
    ])
}

fn join_url(base: &str, relative: &str) -> Result<String, CloudUpdateError> {
    validate_relative_path(relative)?;
    Ok(format!(
        "{}/{}",
        base.trim_end_matches('/'),
        relative.trim_start_matches('/')
    ))
}

fn validate_relative_path(path: &str) -> Result<(), CloudUpdateError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "cloud update path must be a safe relative path".to_string(),
        ));
    }
    Ok(())
}

fn validate_file_name(file_name: &str) -> Result<(), CloudUpdateError> {
    if Path::new(file_name)
        .file_name()
        .and_then(|value| value.to_str())
        != Some(file_name)
    {
        return Err(CloudUpdateError::InvalidProtocol(
            "invalid cached catalog filename".to_string(),
        ));
    }
    Ok(())
}

fn validate_cached_state(state: &CloudCacheState) -> Result<(), CloudUpdateError> {
    if state.target_seq == 0 || state.index_revision_seq == 0 || state.manifest_obj_id.is_empty() {
        return Err(CloudUpdateError::InvalidProtocol(
            "cached cloud state is incomplete".to_string(),
        ));
    }
    let mut identities = BTreeSet::new();
    for file in &state.files {
        validate_file_name(&file.file_name)?;
        if file.catalog_id.is_empty()
            || file.obj_id.is_empty()
            || file.sha256.len() != 64
            || !identities.insert((file.catalog_kind, file.catalog_id.clone()))
        {
            return Err(CloudUpdateError::InvalidProtocol(
                "cached cloud state contains an invalid file".to_string(),
            ));
        }
    }
    Ok(())
}

fn kind_name(kind: CloudCatalogKind) -> &'static str {
    match kind {
        CloudCatalogKind::ModelDriver => "model-driver",
        CloudCatalogKind::ProviderRules => "provider-rules",
        CloudCatalogKind::KnownProvider => "known-provider",
    }
}

fn hex_sha256(contents: &[u8]) -> String {
    Sha256::digest(contents)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

async fn sync_file(path: &Path) -> Result<(), std::io::Error> {
    tokio::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .await?
        .sync_all()
        .await
}

async fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    tokio::fs::File::open(path).await?.sync_all().await
}

async fn load_state_file(path: &Path) -> Result<CloudCacheState, CloudUpdateError> {
    Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
}

async fn verify_cached_revision(
    revision_dir: &Path,
    state: &CloudCacheState,
) -> Result<(), CloudUpdateError> {
    for cached in &state.files {
        validate_file_name(&cached.file_name)?;
        let contents = tokio::fs::read(revision_dir.join(&cached.file_name)).await?;
        if hex_sha256(&contents) != cached.sha256 {
            return Err(CloudUpdateError::InvalidProtocol(
                "cached catalog digest mismatch".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::builtin_provider_registry;

    struct FakeFetcher {
        objects: BTreeMap<String, Vec<u8>>,
    }

    #[async_trait]
    impl CloudObjectFetcher for FakeFetcher {
        async fn fetch(
            &self,
            url: &str,
            _expected_obj_id: Option<&str>,
        ) -> Result<Vec<u8>, CloudUpdateError> {
            self.objects
                .get(url)
                .cloned()
                .ok_or_else(|| CloudUpdateError::Download(format!("missing fixture {url}")))
        }
    }

    fn profile() -> CloudUpdateClientProfile {
        CloudUpdateClientProfile {
            client_version: "2.2.0".to_string(),
            update_channel: "stable".to_string(),
            rollout_group: "default".to_string(),
            supported_features: BTreeSet::new(),
        }
    }

    fn fixtures(
        source: &str,
        revision: u64,
        corrupt_first: bool,
    ) -> (FakeFetcher, Vec<CurrentCatalogFile>) {
        let files = builtin_provider_registry().unwrap().catalog_files();
        let mut objects = BTreeMap::new();
        let mut manifest_files = Vec::new();
        for (position, file) in files.iter().enumerate() {
            let parsed =
                MetadataFile::parse(MetadataSource::Cloud, file.kind, file.contents.clone())
                    .unwrap();
            let path = format!("aicc/provider-catalog/v2/catalog-{position}.json");
            let mut served = file.contents.clone();
            if corrupt_first && position == 0 {
                served = b"{}".to_vec();
            }
            objects.insert(join_url(source, &path).unwrap(), served);
            manifest_files.push(ProviderCatalogManifestFile {
                catalog_kind: match file.kind {
                    CatalogKind::ModelDriver => CloudCatalogKind::ModelDriver,
                    CatalogKind::ProviderRules => CloudCatalogKind::ProviderRules,
                    CatalogKind::KnownProvider => CloudCatalogKind::KnownProvider,
                },
                catalog_id: parsed.catalog_id,
                path,
                schema_version: 1,
                revision_seq: revision,
                obj_id: format!("obj-{position}"),
                sha256: Some(hex_sha256(&file.contents)),
            });
        }
        let manifest_path = format!("aicc/provider-catalog/v2/manifest-{revision}.json");
        let manifest = ProviderCatalogManifest {
            format: MANIFEST_FORMAT.to_string(),
            protocol_version: PROTOCOL_VERSION,
            revision_seq: revision,
            match_rule: MatchRule::Shorthand("2.2.*".to_string()),
            required_features: Vec::new(),
            files: manifest_files,
            tombstones: Vec::new(),
        };
        objects.insert(
            join_url(source, &manifest_path).unwrap(),
            serde_json::to_vec(&manifest).unwrap(),
        );
        let index = ProviderCatalogIndex {
            format: INDEX_FORMAT.to_string(),
            index_version: INDEX_VERSION,
            index_revision_seq: revision,
            tracks: vec![ProviderCatalogTrack {
                revision_seq: revision,
                manifest_path,
                manifest_obj_id: "manifest-object".to_string(),
                match_rule: MatchRule::Shorthand("2.2.*".to_string()),
                required_features: Vec::new(),
            }],
        };
        objects.insert(
            join_url(source, INDEX_PATH).unwrap(),
            serde_json::to_vec(&index).unwrap(),
        );
        (FakeFetcher { objects }, files)
    }

    fn manager(root: &Path, source: &str, fetcher: FakeFetcher) -> Arc<CloudUpdateManager> {
        CloudUpdateManager::new(
            root,
            Arc::new(fetcher),
            profile(),
            CloudUpdateConfig {
                enabled: true,
                source_url: Some(source.to_string()),
                interval_secs: 60,
            },
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn download_commits_complete_revision_then_emits_event() {
        let temp = tempfile::tempdir().unwrap();
        let source = "ndn://metadata.test";
        let (fetcher, _) = fixtures(source, 42, false);
        let manager = manager(temp.path(), source, fetcher);
        let mut events = manager.subscribe();

        assert_eq!(manager.check_once().await.unwrap(), Some(42));
        assert_eq!(events.recv().await.unwrap().target_seq, 42);
        assert_eq!(manager.metadata_target_seq().await.unwrap(), 42);
        assert!(temp.path().join("revisions/42/state.json").is_file());
        assert_eq!(manager.check_once().await.unwrap(), None);

        let snapshot = manager.load_catalog(42).await.unwrap();
        assert_eq!(snapshot.target_revision_seq(), 42);
        assert!(snapshot.known_providers().next().is_some());
    }

    #[tokio::test]
    async fn invalid_download_never_advances_target_or_emits_event() {
        let temp = tempfile::tempdir().unwrap();
        let source = "ndn://metadata.test";
        let (fetcher, _) = fixtures(source, 42, true);
        let manager = manager(temp.path(), source, fetcher);
        let mut events = manager.subscribe();

        assert!(manager.check_once().await.is_err());
        assert_eq!(manager.metadata_target_seq().await.unwrap(), 0);
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(!temp.path().join(STATE_FILE).exists());
    }

    #[tokio::test]
    async fn partial_cloud_release_is_validated_with_builtin_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let source = "ndn://metadata.test";
        let (mut fetcher, builtins) = fixtures(source, 42, false);
        let manifest_url = join_url(source, "aicc/provider-catalog/v2/manifest-42.json").unwrap();
        let mut manifest: ProviderCatalogManifest =
            serde_json::from_slice(fetcher.objects.get(&manifest_url).unwrap()).unwrap();
        manifest.files.truncate(1);
        fetcher
            .objects
            .insert(manifest_url, serde_json::to_vec(&manifest).unwrap());
        let manager = CloudUpdateManager::new(
            temp.path(),
            Arc::new(fetcher),
            profile(),
            CloudUpdateConfig {
                enabled: true,
                source_url: Some(source.to_string()),
                interval_secs: 60,
            },
            builtins,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        assert_eq!(manager.check_once().await.unwrap(), Some(42));
        let snapshot = manager.load_catalog(42).await.unwrap();
        assert!(snapshot.known_providers().count() > 1);
    }

    #[tokio::test]
    async fn captured_revision_remains_loadable_after_target_advances() {
        let temp = tempfile::tempdir().unwrap();
        let source = "ndn://metadata.test";
        let (first_fetcher, _) = fixtures(source, 42, false);
        let first = manager(temp.path(), source, first_fetcher);
        assert_eq!(first.check_once().await.unwrap(), Some(42));

        let (second_fetcher, _) = fixtures(source, 43, false);
        let second = manager(temp.path(), source, second_fetcher);
        assert_eq!(second.check_once().await.unwrap(), Some(43));
        assert_eq!(second.metadata_target_seq().await.unwrap(), 43);

        let captured = second.load_catalog(42).await.unwrap();
        assert_eq!(captured.target_revision_seq(), 42);
        let current = second.load_catalog(43).await.unwrap();
        assert_eq!(current.target_revision_seq(), 43);
    }

    #[tokio::test]
    async fn retries_activation_after_revision_files_were_committed() {
        let temp = tempfile::tempdir().unwrap();
        let source = "ndn://metadata.test";
        let (first_fetcher, _) = fixtures(source, 42, false);
        let first = manager(temp.path(), source, first_fetcher);
        assert_eq!(first.check_once().await.unwrap(), Some(42));
        tokio::fs::remove_file(temp.path().join(STATE_FILE))
            .await
            .unwrap();

        let (retry_fetcher, _) = fixtures(source, 42, false);
        let retry = manager(temp.path(), source, retry_fetcher);
        assert_eq!(retry.check_once().await.unwrap(), Some(42));
        assert_eq!(retry.metadata_target_seq().await.unwrap(), 42);
        assert_eq!(
            retry.load_catalog(42).await.unwrap().target_revision_seq(),
            42
        );
    }

    #[tokio::test]
    async fn concurrent_checks_publish_one_complete_revision() {
        let temp = tempfile::tempdir().unwrap();
        let source = "ndn://metadata.test";
        let (fetcher, _) = fixtures(source, 42, false);
        let manager = manager(temp.path(), source, fetcher);
        let mut events = manager.subscribe();

        let first_manager = manager.clone();
        let second_manager = manager.clone();
        let (first, second) = tokio::join!(
            async move { first_manager.check_once().await.unwrap() },
            async move { second_manager.check_once().await.unwrap() }
        );
        assert!(matches!(
            (first, second),
            (Some(42), None) | (None, Some(42))
        ));
        assert_eq!(events.recv().await.unwrap().target_seq, 42);
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert_eq!(manager.metadata_target_seq().await.unwrap(), 42);
    }

    #[test]
    fn protocol_rejects_path_traversal_and_incompatible_tracks() {
        assert!(validate_relative_path("../secret").is_err());
        let index = ProviderCatalogIndex {
            format: INDEX_FORMAT.to_string(),
            index_version: INDEX_VERSION,
            index_revision_seq: 1,
            tracks: vec![ProviderCatalogTrack {
                revision_seq: 7,
                manifest_path: "manifest.json".to_string(),
                manifest_obj_id: "obj".to_string(),
                match_rule: MatchRule::Shorthand("3.*".to_string()),
                required_features: Vec::new(),
            }],
        };
        validate_index(&index).unwrap();
        assert!(select_track(&index, &profile(), 0).unwrap().is_none());
    }

    #[test]
    fn protocol_rejects_revision_reuse_and_unannounced_deletion() {
        let current = CloudCacheState {
            target_seq: 7,
            index_revision_seq: 7,
            manifest_obj_id: "old-manifest".to_string(),
            files: vec![CachedCatalogFile {
                catalog_kind: CloudCatalogKind::ProviderRules,
                catalog_id: "openai".to_string(),
                file_name: "0000-provider-rules-openai.json".to_string(),
                obj_id: "rules-object".to_string(),
                sha256: "0".repeat(64),
            }],
        };
        let index = ProviderCatalogIndex {
            format: INDEX_FORMAT.to_string(),
            index_version: INDEX_VERSION,
            index_revision_seq: 8,
            tracks: vec![ProviderCatalogTrack {
                revision_seq: 7,
                manifest_path: "manifest.json".to_string(),
                manifest_obj_id: "different-manifest".to_string(),
                match_rule: MatchRule::Shorthand("2.2.*".to_string()),
                required_features: Vec::new(),
            }],
        };
        assert!(reject_reused_revision(&index, &profile(), Some(&current)).is_err());

        let manifest = ProviderCatalogManifest {
            format: MANIFEST_FORMAT.to_string(),
            protocol_version: PROTOCOL_VERSION,
            revision_seq: 8,
            match_rule: MatchRule::Shorthand("2.2.*".to_string()),
            required_features: Vec::new(),
            files: Vec::new(),
            tombstones: Vec::new(),
        };
        assert!(validate_cloud_transition(Some(&current), &manifest).is_err());
    }
}
