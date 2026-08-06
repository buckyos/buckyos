use crate::metadata_resolver::{validate_driver_metadata_document, DriverMetadataDocument};
use anyhow::{anyhow, bail, Context, Result};
use buckyos_kit::get_buckyos_service_home_dir;
use ndn_lib::{ChunkId, ChunkList, FileObject, ObjId, OBJ_TYPE_CHUNK_LIST};
use ndn_toolkit::cyfs_ndn_client::{CyfsNdnClient, CyfsResponseMeta, VerifiedPathObject};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INDEX_FORMAT: &str = "buckyos.aicc.driver-metadata-index";
const MANIFEST_FORMAT: &str = "buckyos.aicc.driver-metadata-manifest";
const ACTIVATION_FORMAT: &str = "buckyos.aicc.driver-metadata-activation";
const PROTOCOL_VERSION: u32 = 1;
const METADATA_SCHEMA_VERSION: u32 = 2;
const MAX_INDEX_BYTES: u64 = 256 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_METADATA_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PROVIDER_FILES: usize = 1024;
const MAX_RETAINED_ACTIVATIONS: usize = 2;
const MAX_RETAINED_WATERMARKS: usize = 2;
const MAX_RETAINED_SOURCE_NAMESPACES: usize = 4;
const MIN_UPDATE_INTERVAL_SECS: u64 = 60;
const MAX_UPDATE_INTERVAL_SECS: u64 = 24 * 60 * 60;

pub(crate) fn normalize_update_interval_secs(value: u64) -> u64 {
    value.clamp(MIN_UPDATE_INTERVAL_SECS, MAX_UPDATE_INTERVAL_SECS)
}

static CONFIGURED_SOURCE_KEY: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static METADATA_STORE_LOCK: OnceLock<RwLock<()>> = OnceLock::new();
static ACTIVATION_CACHE: OnceLock<RwLock<HashMap<PathBuf, CachedActivation>>> = OnceLock::new();
static EFFECTIVE_METADATA_SELECTION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static EFFECTIVE_METADATA_IDENTITY: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static DRIVER_METADATA_GENERATION: AtomicU64 = AtomicU64::new(0);

fn advance_driver_metadata_generation() -> u64 {
    DRIVER_METADATA_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
}

fn observe_effective_metadata_identity(identity: String) -> u64 {
    let Ok(mut current) = EFFECTIVE_METADATA_IDENTITY
        .get_or_init(|| RwLock::new(None))
        .write()
    else {
        return DRIVER_METADATA_GENERATION.load(Ordering::Acquire);
    };
    if current.as_ref() != Some(&identity) {
        *current = Some(identity);
        return advance_driver_metadata_generation();
    }
    DRIVER_METADATA_GENERATION.load(Ordering::Acquire)
}

fn activation_identity(source_key: &str, activation: &DriverMetadataActivation) -> String {
    activation_identity_from_parts(
        source_key,
        activation.manifest.revision_seq,
        activation.manifest_obj_id.as_str(),
        activation.manifest_sha256.as_str(),
    )
}

fn activation_identity_from_parts(
    source_key: &str,
    revision_seq: u64,
    manifest_obj_id: &str,
    manifest_sha256: &str,
) -> String {
    format!(
        "source:{}:activation:{}:{}:{}",
        source_key, revision_seq, manifest_obj_id, manifest_sha256
    )
}

fn observe_effective_activation(
    source_key: &str,
    activation: Option<&DriverMetadataActivation>,
) -> u64 {
    let identity = activation
        .map(|activation| activation_identity(source_key, activation))
        .unwrap_or_else(|| format!("source:{}:no-valid-activation", source_key));
    observe_effective_metadata_identity(identity)
}

fn observe_effective_identity_if_configured(source_key: &str, identity: String) -> u64 {
    let Ok(configured_source) = CONFIGURED_SOURCE_KEY
        .get_or_init(|| RwLock::new(None))
        .read()
    else {
        return DRIVER_METADATA_GENERATION.load(Ordering::Acquire);
    };
    if configured_source.as_deref() != Some(source_key) {
        return DRIVER_METADATA_GENERATION.load(Ordering::Acquire);
    }
    let Ok(_selection_guard) = EFFECTIVE_METADATA_SELECTION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
    else {
        return DRIVER_METADATA_GENERATION.load(Ordering::Acquire);
    };
    observe_effective_metadata_identity(identity)
}

#[derive(Clone, Debug, Deserialize)]
pub struct DriverMetadataUpdateSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub source_url: String,
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
}

fn default_interval_secs() -> u64 {
    3600
}

impl DriverMetadataUpdateSettings {
    pub fn from_aicc_settings(settings: &Value) -> Result<Option<Self>> {
        let Some(value) = settings.get("driver_metadata_update") else {
            return Ok(None);
        };
        let mut parsed: Self = serde_json::from_value(value.clone())
            .context("parse driver_metadata_update settings")?;
        if !parsed.enabled {
            return Ok(None);
        }
        parsed.source_url = parsed.source_url.trim().to_string();
        if parsed.source_url.is_empty() {
            bail!("driver_metadata_update.source_url is empty");
        }
        let url = Url::parse(parsed.source_url.as_str()).context("parse metadata source_url")?;
        if url.scheme() != "https" || url.host_str().is_none() {
            bail!("metadata source_url must be an https URL with a host");
        }
        if url.path() != "/aicc/driver-metadata/index.json"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("metadata source_url must use /aicc/driver-metadata/index.json");
        }
        parsed.interval_secs = normalize_update_interval_secs(parsed.interval_secs);
        Ok(Some(parsed))
    }

    fn source_key(&self) -> Result<String> {
        let url = Url::parse(self.source_url.as_str()).context("parse metadata source_url")?;
        Ok(content_sha256(url.as_str().as_bytes()))
    }
}

pub fn configure_remote_metadata_source(settings: &Value) -> Result<()> {
    let parsed = DriverMetadataUpdateSettings::from_aicc_settings(settings);
    let source_key = match parsed.as_ref() {
        Ok(Some(settings)) => Some(settings.source_key()?),
        _ => None,
    };
    {
        let mut configured_source = CONFIGURED_SOURCE_KEY
            .get_or_init(|| RwLock::new(None))
            .write()
            .map_err(|_| anyhow!("configured metadata source lock poisoned"))?;
        if *configured_source != source_key {
            *configured_source = source_key.clone();
            let identity = source_key
                .as_deref()
                .map(|source_key| format!("source:{}:unresolved", source_key))
                .unwrap_or_else(|| "disabled".to_string());
            observe_effective_metadata_identity(identity);
        }
    }
    parsed.map(|_| ())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataIndex {
    pub format: String,
    pub index_version: u32,
    #[serde(default)]
    pub index_revision: u32,
    pub index_revision_seq: u64,
    #[serde(default)]
    pub required_features: Vec<String>,
    pub tracks: Vec<DriverMetadataTrack>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataTrack {
    pub protocol_version: u32,
    #[serde(default)]
    pub protocol_revision: u32,
    pub revision_seq: u64,
    #[serde(default)]
    pub required_features: Vec<String>,
    pub manifest: DriverMetadataIndexManifest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataIndexManifest {
    pub path: String,
    pub obj_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataManifest {
    pub format: String,
    pub protocol_version: u32,
    #[serde(default)]
    pub protocol_revision: u32,
    pub revision_seq: u64,
    #[serde(default)]
    pub required_features: Vec<String>,
    #[serde(default)]
    pub files: Vec<DriverMetadataManifestFile>,
    #[serde(default)]
    pub tombstones: Vec<DriverMetadataTombstone>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataManifestFile {
    pub provider_driver: String,
    pub path: String,
    pub schema_version: u32,
    pub revision_seq: u64,
    pub obj_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataTombstone {
    pub provider_driver: String,
    pub revision_seq: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ObservedIndex {
    obj_id: String,
    index: DriverMetadataIndex,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ObservedManifest {
    obj_id: String,
    manifest: DriverMetadataManifest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct DriverMetadataActivation {
    format: String,
    storage_version: u32,
    index_revision_seq: u64,
    manifest_obj_id: String,
    manifest_sha256: String,
    manifest: DriverMetadataManifest,
}

#[derive(Clone)]
struct CachedActivation {
    head_revision: u64,
    activation: DriverMetadataActivation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverMetadataUpdateOutcome {
    Activated { revision_seq: u64 },
    Unchanged { revision_seq: u64 },
}

pub struct DriverMetadataUpdater {
    settings: DriverMetadataUpdateSettings,
    client: CyfsNdnClient,
    store: DriverMetadataStore,
    source_key: String,
}

struct DriverMetadataAttempt {
    path: PathBuf,
}

impl DriverMetadataAttempt {
    fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl Drop for DriverMetadataAttempt {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.as_path());
    }
}

impl DriverMetadataUpdater {
    pub fn new(settings: DriverMetadataUpdateSettings) -> Result<Self> {
        let source_key = settings.source_key()?;
        let client = CyfsNdnClient::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| anyhow!(err.to_string()))?;
        Ok(Self {
            settings,
            client,
            store: DriverMetadataStore::new(source_store_root(source_key.as_str())),
            source_key,
        })
    }

    pub async fn update_once(&self) -> Result<DriverMetadataUpdateOutcome> {
        {
            let _guard = METADATA_STORE_LOCK
                .get_or_init(|| RwLock::new(()))
                .write()
                .map_err(|_| anyhow!("metadata store lock poisoned"))?;
            self.store.prepare()?;
            touch_source_namespace(self.store.root.as_path())?;
            prune_source_namespaces(
                default_store_root().as_path(),
                self.source_key.as_str(),
                MAX_RETAINED_SOURCE_NAMESPACES,
            )?;
        }
        let attempt = self.store.new_attempt_dir()?;
        let result = self.update_once_inner(attempt.path()).await;
        if let Ok(_guard) = METADATA_STORE_LOCK.get_or_init(|| RwLock::new(())).write() {
            let _ = self.store.prune_history();
            let _ = self.store.cleanup_parts_and_orphans();
        }
        result
    }

    async fn update_once_inner(&self, attempt: &Path) -> Result<DriverMetadataUpdateOutcome> {
        let source_url = Url::parse(self.settings.source_url.as_str())?;
        let (index_bytes, index_obj_id) = self
            .download_verified(source_url.as_str(), None, MAX_INDEX_BYTES, attempt, "index")
            .await?;
        let index: DriverMetadataIndex =
            serde_json::from_slice(index_bytes.as_slice()).context("parse metadata index")?;
        validate_index(&index)?;
        self.store.observe_index(&index, &index_obj_id)?;
        let track = select_supported_track(&index)?;

        let manifest_obj_id = parse_obj_id(track.manifest.obj_id.as_str())?;
        let manifest_url = join_canonical_url(&source_url, track.manifest.path.as_str())?;
        let (manifest_bytes, _) = self
            .download_verified(
                manifest_url.as_str(),
                Some(&manifest_obj_id),
                MAX_MANIFEST_BYTES,
                attempt,
                "manifest",
            )
            .await?;
        let manifest: DriverMetadataManifest =
            serde_json::from_slice(manifest_bytes.as_slice()).context("parse metadata manifest")?;
        validate_manifest(&manifest, track)?;
        self.store
            .observe_manifest(&manifest, manifest_obj_id.to_string().as_str())?;

        let current = self.store.load_latest_activation();
        let current_identity = current
            .as_ref()
            .map(|activation| activation_identity(self.source_key.as_str(), activation))
            .unwrap_or_else(|| format!("source:{}:no-valid-activation", self.source_key.as_str()));
        observe_effective_identity_if_configured(self.source_key.as_str(), current_identity);
        validate_manifest_transition(current.as_ref(), &manifest, &manifest_obj_id)?;
        if let Some(current) = current.as_ref() {
            if current.manifest.revision_seq == manifest.revision_seq
                && current.manifest_obj_id == manifest_obj_id.to_string()
            {
                return Ok(DriverMetadataUpdateOutcome::Unchanged {
                    revision_seq: manifest.revision_seq,
                });
            }
        }

        let mut metadata_bytes = 0u64;
        for file in manifest.files.iter() {
            if let Some(size) = self.store.prepare_object_slot(file)? {
                metadata_bytes = checked_manifest_metadata_bytes(metadata_bytes, size)?;
                continue;
            }
            let expected_obj_id = parse_obj_id(file.obj_id.as_str())?;
            let file_url = join_canonical_url(&source_url, file.path.as_str())?;
            let (bytes, _) = self
                .download_verified(
                    file_url.as_str(),
                    Some(&expected_obj_id),
                    MAX_METADATA_BYTES,
                    attempt,
                    file.provider_driver.as_str(),
                )
                .await?;
            metadata_bytes = checked_manifest_metadata_bytes(metadata_bytes, bytes.len() as u64)?;
            validate_metadata_bytes(bytes.as_slice(), file)?;
            self.store
                .store_object(&expected_obj_id, bytes.as_slice())?;
        }

        for file in manifest.files.iter() {
            if self.store.load_valid_object(file).is_none() {
                bail!(
                    "prepared metadata object for {} is unavailable",
                    file.provider_driver
                );
            }
        }

        let candidate_identity = activation_identity_from_parts(
            self.source_key.as_str(),
            manifest.revision_seq,
            manifest_obj_id.to_string().as_str(),
            manifest_sha256(&manifest)?.as_str(),
        );
        {
            let _guard = METADATA_STORE_LOCK
                .get_or_init(|| RwLock::new(()))
                .write()
                .map_err(|_| anyhow!("metadata store lock poisoned"))?;
            self.store.activate(
                index.index_revision_seq,
                manifest_obj_id.to_string().as_str(),
                manifest.clone(),
            )?;
        }
        observe_effective_identity_if_configured(self.source_key.as_str(), candidate_identity);
        Ok(DriverMetadataUpdateOutcome::Activated {
            revision_seq: manifest.revision_seq,
        })
    }

    async fn download_verified(
        &self,
        url: &str,
        expected_obj_id: Option<&ObjId>,
        max_bytes: u64,
        attempt: &Path,
        label: &str,
    ) -> Result<(Vec<u8>, ObjId)> {
        let mut request = self.client.get(url.to_string());
        if let Some(expected) = expected_obj_id {
            request = request.obj_id(expected.clone());
        }
        let response = request
            .send()
            .await
            .map_err(|err| anyhow!("download {} through NDN failed: {}", label, err))?;
        let path_object = validate_verified_path_response(response.meta(), expected_obj_id, label)?;
        validate_verified_ndn_size(response.meta(), &path_object.target, max_bytes, label)?;

        let output = attempt.join(format!("{}.part", safe_label(label)));
        let pull_result = response
            .pull_to_local_file(output.as_path())
            .await
            .map_err(|err| anyhow!("download and verify {} through NDN failed: {}", label, err))?;
        if pull_result.total_size > max_bytes {
            bail!("{} exceeds the maximum size", label);
        }
        let metadata = std::fs::metadata(output.as_path())?;
        if metadata.len() > max_bytes {
            bail!("{} exceeds the maximum size", label);
        }
        let bytes = std::fs::read(output.as_path())?;
        std::fs::remove_file(output.as_path())?;
        Ok((bytes, path_object.target))
    }
}

fn validate_verified_path_response(
    meta: &CyfsResponseMeta,
    expected_obj_id: Option<&ObjId>,
    label: &str,
) -> Result<VerifiedPathObject> {
    let path_object = meta
        .path_object
        .clone()
        .ok_or_else(|| anyhow!("{} response has no verified PathObject", label))?;
    if let Some(expected) = expected_obj_id {
        if &path_object.target != expected {
            bail!("{} PathObject target does not match expected ObjId", label);
        }
    }
    Ok(path_object)
}

fn validate_verified_ndn_size(
    meta: &CyfsResponseMeta,
    target: &ObjId,
    max_bytes: u64,
    label: &str,
) -> Result<()> {
    let declared_size = if target.is_chunk() {
        ChunkId::from_obj_id(target).get_length().ok_or_else(|| {
            anyhow!(
                "{} NDN ChunkId does not carry a verified content length",
                label
            )
        })?
    } else {
        let parent = meta
            .parents
            .iter()
            .rev()
            .find(|parent| &parent.obj_id == target)
            .ok_or_else(|| anyhow!("{} response has no verified size-bearing parent", label))?;
        let value = parent.obj_json.as_ref().ok_or_else(|| {
            anyhow!(
                "{} verified parent does not include canonical object data",
                label
            )
        })?;
        if target.is_file_object() {
            serde_json::from_value::<FileObject>(value.clone())
                .with_context(|| format!("parse verified {} FileObject", label))?
                .size
        } else if target.obj_type == OBJ_TYPE_CHUNK_LIST {
            let chunk_list = ChunkList::from_json_value(value.clone())
                .map_err(|err| anyhow!("parse verified {} ChunkList: {}", label, err))?;
            chunk_list.body.iter().try_fold(0u64, |total, chunk_id| {
                let size = chunk_id.get_length().ok_or_else(|| {
                    anyhow!(
                        "{} ChunkList contains a chunk without verified length",
                        label
                    )
                })?;
                total
                    .checked_add(size)
                    .ok_or_else(|| anyhow!("{} verified content size overflow", label))
            })?
        } else {
            bail!("{} NDN target type has no trusted size declaration", label);
        }
    };
    if declared_size > max_bytes {
        bail!("{} exceeds the maximum size", label);
    }
    Ok(())
}

fn checked_manifest_metadata_bytes(current: u64, next: u64) -> Result<u64> {
    let total = current
        .checked_add(next)
        .ok_or_else(|| anyhow!("manifest metadata size overflow"))?;
    if total > MAX_MANIFEST_METADATA_BYTES {
        bail!("manifest metadata exceeds the maximum total size");
    }
    Ok(total)
}

#[derive(Clone)]
struct DriverMetadataStore {
    root: PathBuf,
}

impl DriverMetadataStore {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn prepare(&self) -> Result<()> {
        std::fs::create_dir_all(self.objects_dir())?;
        std::fs::create_dir_all(self.activations_dir())?;
        std::fs::create_dir_all(self.observed_index_dir())?;
        std::fs::create_dir_all(self.observed_manifest_dir())?;
        std::fs::create_dir_all(self.staging_dir())?;
        self.cleanup_parts_and_orphans()
    }

    fn objects_dir(&self) -> PathBuf {
        self.root.join("objects")
    }

    fn activations_dir(&self) -> PathBuf {
        self.root.join("activations")
    }

    fn observed_index_dir(&self) -> PathBuf {
        self.root.join("observed").join("index")
    }

    fn observed_manifest_dir(&self) -> PathBuf {
        self.root.join("observed").join("manifest")
    }

    fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }

    fn new_attempt_dir(&self) -> Result<DriverMetadataAttempt> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = self
            .staging_dir()
            .join(format!("{}-{}", std::process::id(), now));
        std::fs::create_dir(&path)?;
        Ok(DriverMetadataAttempt { path })
    }

    fn object_path(&self, obj_id: &ObjId) -> PathBuf {
        self.objects_dir()
            .join(format!("{}.json", obj_id.to_base32()))
    }

    fn object_digest_path(&self, obj_id: &ObjId) -> PathBuf {
        self.objects_dir()
            .join(format!("{}.sha256", obj_id.to_base32()))
    }

    fn observe_index(&self, index: &DriverMetadataIndex, obj_id: &ObjId) -> Result<()> {
        if let Some(latest) =
            latest_json_strict::<ObservedIndex>(self.observed_index_dir().as_path())?
        {
            if index.index_revision_seq < latest.index.index_revision_seq {
                bail!("index revision rollback");
            }
            if index.index_revision_seq == latest.index.index_revision_seq
                && (latest.obj_id != obj_id.to_string() || latest.index != *index)
            {
                bail!("index revision conflict");
            }
            if let Some(previous_track) = latest
                .index
                .tracks
                .iter()
                .find(|track| track.protocol_version == PROTOCOL_VERSION)
            {
                let next_track = index
                    .tracks
                    .iter()
                    .find(|track| track.protocol_version == PROTOCOL_VERSION)
                    .ok_or_else(|| anyhow!("supported protocol track disappeared"))?;
                if next_track.revision_seq < previous_track.revision_seq
                    || next_track.protocol_revision < previous_track.protocol_revision
                {
                    bail!("supported protocol track revision rollback");
                }
                if next_track.revision_seq == previous_track.revision_seq
                    && next_track != previous_track
                {
                    bail!("supported protocol track revision conflict");
                }
            }
        }
        let observed = ObservedIndex {
            obj_id: obj_id.to_string(),
            index: index.clone(),
        };
        atomic_create_json(
            self.observed_index_dir()
                .join(format!("{}.json", index.index_revision_seq))
                .as_path(),
            &observed,
        )
    }

    fn observe_manifest(&self, manifest: &DriverMetadataManifest, obj_id: &str) -> Result<()> {
        if let Some(latest) =
            latest_json_strict::<ObservedManifest>(self.observed_manifest_dir().as_path())?
        {
            if manifest.revision_seq < latest.manifest.revision_seq {
                bail!("manifest revision rollback");
            }
            if manifest.revision_seq == latest.manifest.revision_seq
                && (latest.obj_id != obj_id || latest.manifest != *manifest)
            {
                bail!("manifest revision conflict");
            }
            let observed_state = DriverMetadataActivation {
                format: ACTIVATION_FORMAT.to_string(),
                storage_version: 1,
                index_revision_seq: latest.manifest.revision_seq,
                manifest_obj_id: latest.obj_id,
                manifest_sha256: manifest_sha256(&latest.manifest)?,
                manifest: latest.manifest,
            };
            validate_manifest_transition(Some(&observed_state), manifest, &parse_obj_id(obj_id)?)?;
        }
        let observed = ObservedManifest {
            obj_id: obj_id.to_string(),
            manifest: manifest.clone(),
        };
        atomic_create_json(
            self.observed_manifest_dir()
                .join(format!("{}.json", manifest.revision_seq))
                .as_path(),
            &observed,
        )
    }

    fn store_object(&self, obj_id: &ObjId, bytes: &[u8]) -> Result<()> {
        atomic_create_bytes(self.object_path(obj_id).as_path(), bytes)?;
        atomic_create_bytes(
            self.object_digest_path(obj_id).as_path(),
            content_sha256(bytes).as_bytes(),
        )
    }

    fn prepare_object_slot(&self, file: &DriverMetadataManifestFile) -> Result<Option<u64>> {
        if let Some((_, size)) = self.load_valid_object_with_size(file) {
            return Ok(Some(size));
        }
        let obj_id = parse_obj_id(file.obj_id.as_str())?;
        let path = self.object_path(&obj_id);
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        let digest_path = self.object_digest_path(&obj_id);
        if digest_path.is_file() {
            std::fs::remove_file(digest_path)?;
        }
        self.invalidate_activation_cache();
        Ok(None)
    }

    fn load_valid_object(
        &self,
        file: &DriverMetadataManifestFile,
    ) -> Option<DriverMetadataDocument> {
        self.load_valid_object_with_size(file)
            .map(|(document, _)| document)
    }

    fn load_valid_object_with_size(
        &self,
        file: &DriverMetadataManifestFile,
    ) -> Option<(DriverMetadataDocument, u64)> {
        let obj_id = parse_obj_id(file.obj_id.as_str()).ok()?;
        let bytes = std::fs::read(self.object_path(&obj_id)).ok()?;
        let size = bytes.len() as u64;
        if size > MAX_METADATA_BYTES {
            return None;
        }
        let stored_digest = std::fs::read_to_string(self.object_digest_path(&obj_id)).ok()?;
        if stored_digest != content_sha256(bytes.as_slice()) {
            return None;
        }
        validate_metadata_bytes(bytes.as_slice(), file)
            .ok()
            .map(|document| (document, size))
    }

    fn activate(
        &self,
        index_revision_seq: u64,
        manifest_obj_id: &str,
        manifest: DriverMetadataManifest,
    ) -> Result<bool> {
        let manifest_sha256 = manifest_sha256(&manifest)?;
        let activation = DriverMetadataActivation {
            format: ACTIVATION_FORMAT.to_string(),
            storage_version: 1,
            index_revision_seq,
            manifest_obj_id: manifest_obj_id.to_string(),
            manifest_sha256,
            manifest,
        };
        let path = self
            .activations_dir()
            .join(format!("{}.json", activation.manifest.revision_seq));
        if path.is_file() {
            match std::fs::read(path.as_path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<DriverMetadataActivation>(&bytes).ok())
            {
                Some(existing)
                    if existing.manifest_obj_id == activation.manifest_obj_id
                        && existing.manifest_sha256 == activation.manifest_sha256
                        && existing.manifest == activation.manifest =>
                {
                    return Ok(false)
                }
                Some(_) => bail!("activation revision conflict"),
                None => std::fs::remove_file(path.as_path())?,
            }
        }
        atomic_create_json(path.as_path(), &activation)?;
        if self.validate_activation(&activation) {
            self.cache_activation(activation.clone());
        }
        Ok(true)
    }

    fn load_latest_activation(&self) -> Option<DriverMetadataActivation> {
        let activation = load_activations(self.activations_dir().as_path())
            .into_iter()
            .find(|activation| self.validate_activation(activation));
        if let Some(activation) = activation.as_ref() {
            self.cache_activation(activation.clone());
        } else {
            self.invalidate_activation_cache();
        }
        activation
    }

    fn load_latest_activation_cached(&self) -> Option<DriverMetadataActivation> {
        let cached = ACTIVATION_CACHE
            .get_or_init(|| RwLock::new(HashMap::new()))
            .read()
            .ok()
            .and_then(|cache| cache.get(&self.root).cloned());
        if let Some(cached) = cached {
            let latest_revision = latest_activation_revision(self.activations_dir().as_path());
            let wrapper = load_activation(
                self.activations_dir()
                    .join(format!("{}.json", cached.activation.manifest.revision_seq))
                    .as_path(),
            );
            if latest_revision == Some(cached.head_revision)
                && wrapper.as_ref() == Some(&cached.activation)
                && self.validate_activation_wrapper(&cached.activation)
            {
                return Some(cached.activation);
            }
            self.invalidate_activation_cache();
        }
        self.load_latest_activation()
    }

    fn validate_activation(&self, activation: &DriverMetadataActivation) -> bool {
        self.validate_activation_wrapper(activation)
            && activation
                .manifest
                .files
                .iter()
                .all(|file| self.load_valid_object(file).is_some())
    }

    fn validate_activation_wrapper(&self, activation: &DriverMetadataActivation) -> bool {
        if activation.format != ACTIVATION_FORMAT || activation.storage_version != 1 {
            return false;
        }
        if manifest_sha256(&activation.manifest)
            .map(|sha256| sha256 != activation.manifest_sha256)
            .unwrap_or(true)
        {
            return false;
        }
        let track = DriverMetadataTrack {
            protocol_version: activation.manifest.protocol_version,
            protocol_revision: activation.manifest.protocol_revision,
            revision_seq: activation.manifest.revision_seq,
            required_features: activation.manifest.required_features.clone(),
            manifest: DriverMetadataIndexManifest {
                path: format!(
                    "v{}/manifest-{}.json",
                    PROTOCOL_VERSION, activation.manifest.revision_seq
                ),
                obj_id: activation.manifest_obj_id.clone(),
            },
        };
        if parse_obj_id(activation.manifest_obj_id.as_str()).is_err()
            || validate_manifest(&activation.manifest, &track).is_err()
        {
            return false;
        }
        true
    }

    fn cache_activation(&self, activation: DriverMetadataActivation) {
        let Some(head_revision) = latest_activation_revision(self.activations_dir().as_path())
        else {
            return;
        };
        if let Ok(mut cache) = ACTIVATION_CACHE
            .get_or_init(|| RwLock::new(HashMap::new()))
            .write()
        {
            cache.insert(
                self.root.clone(),
                CachedActivation {
                    head_revision,
                    activation,
                },
            );
        }
    }

    fn invalidate_activation_cache(&self) {
        if let Ok(mut cache) = ACTIVATION_CACHE
            .get_or_init(|| RwLock::new(HashMap::new()))
            .write()
        {
            cache.remove(&self.root);
        }
    }

    fn cleanup_parts_and_orphans(&self) -> Result<()> {
        if self.staging_dir().is_dir() {
            for entry in std::fs::read_dir(self.staging_dir())? {
                let path = entry?.path();
                if path.is_dir() {
                    std::fs::remove_dir_all(path)?;
                } else {
                    std::fs::remove_file(path)?;
                }
            }
        }
        remove_part_files(self.root.as_path())?;

        let mut referenced = load_activations(self.activations_dir().as_path())
            .into_iter()
            .flat_map(|activation| activation.manifest.files)
            .filter_map(|file| parse_obj_id(file.obj_id.as_str()).ok())
            .flat_map(|obj_id| {
                [
                    format!("{}.json", obj_id.to_base32()),
                    format!("{}.sha256", obj_id.to_base32()),
                ]
            })
            .collect::<HashSet<_>>();
        if let Some(observed) =
            latest_json_strict::<ObservedManifest>(self.observed_manifest_dir().as_path())?
        {
            referenced.extend(
                observed
                    .manifest
                    .files
                    .iter()
                    .filter_map(|file| parse_obj_id(file.obj_id.as_str()).ok())
                    .flat_map(|obj_id| {
                        [
                            format!("{}.json", obj_id.to_base32()),
                            format!("{}.sha256", obj_id.to_base32()),
                        ]
                    }),
            );
        }
        if self.objects_dir().is_dir() {
            for entry in std::fs::read_dir(self.objects_dir())? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_file() && !referenced.contains(name.as_str()) {
                    std::fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    fn prune_history(&self) -> Result<()> {
        self.prune_activations()?;
        latest_json_strict::<ObservedIndex>(self.observed_index_dir().as_path())?;
        latest_json_strict::<ObservedManifest>(self.observed_manifest_dir().as_path())?;
        prune_revision_files(self.observed_index_dir().as_path(), MAX_RETAINED_WATERMARKS)?;
        prune_revision_files(
            self.observed_manifest_dir().as_path(),
            MAX_RETAINED_WATERMARKS,
        )
    }

    fn prune_activations(&self) -> Result<()> {
        let mut entries = std::fs::read_dir(self.activations_dir())?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .filter_map(|entry| {
                let path = entry.path();
                let activation = std::fs::read(path.as_path()).ok().and_then(|bytes| {
                    serde_json::from_slice::<DriverMetadataActivation>(&bytes).ok()
                });
                Some((path, activation))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(path, _)| {
            std::cmp::Reverse(
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.parse::<u64>().ok())
                    .unwrap_or(0),
            )
        });
        let mut retained = 0usize;
        for (path, activation) in entries {
            let valid = activation
                .as_ref()
                .map(|activation| self.validate_activation(activation))
                .unwrap_or(false);
            if valid && retained < MAX_RETAINED_ACTIVATIONS {
                retained += 1;
            } else {
                std::fs::remove_file(path)?;
            }
        }
        sync_parent_dir(self.activations_dir().as_path())
    }
}

pub(crate) fn load_active_remote_metadata(
    provider_driver: &str,
) -> (Option<DriverMetadataDocument>, u64) {
    let source_key_guard = match CONFIGURED_SOURCE_KEY
        .get_or_init(|| RwLock::new(None))
        .read()
    {
        Ok(guard) => guard,
        Err(_) => return (None, DRIVER_METADATA_GENERATION.load(Ordering::Acquire)),
    };
    let Some(source_key) = source_key_guard.as_ref() else {
        return (None, DRIVER_METADATA_GENERATION.load(Ordering::Acquire));
    };
    let _store_guard = match METADATA_STORE_LOCK.get_or_init(|| RwLock::new(())).read() {
        Ok(guard) => guard,
        Err(_) => return (None, DRIVER_METADATA_GENERATION.load(Ordering::Acquire)),
    };
    let _selection_guard = match EFFECTIVE_METADATA_SELECTION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
    {
        Ok(guard) => guard,
        Err(_) => return (None, DRIVER_METADATA_GENERATION.load(Ordering::Acquire)),
    };
    load_active_remote_metadata_for_source(
        source_store_root(source_key.as_str()).as_path(),
        provider_driver,
        source_key.as_str(),
    )
}

pub fn active_remote_metadata_revision(settings: &Value) -> Option<u64> {
    let settings = DriverMetadataUpdateSettings::from_aicc_settings(settings)
        .ok()
        .flatten()?;
    let source_key = settings.source_key().ok()?;
    let _store_guard = METADATA_STORE_LOCK
        .get_or_init(|| RwLock::new(()))
        .read()
        .ok()?;
    DriverMetadataStore::new(source_store_root(source_key.as_str()))
        .load_latest_activation_cached()
        .map(|activation| activation.manifest.revision_seq)
}

#[cfg(test)]
fn load_active_remote_metadata_in(
    root: &Path,
    provider_driver: &str,
) -> Option<DriverMetadataDocument> {
    let store = DriverMetadataStore::new(root.to_path_buf());
    let activation = store.load_latest_activation_cached()?;
    let file = activation
        .manifest
        .files
        .iter()
        .find(|file| file.provider_driver == provider_driver)?;
    if let Some(document) = store.load_valid_object(file) {
        return Some(document);
    }
    store.invalidate_activation_cache();
    let activation = store.load_latest_activation()?;
    let file = activation
        .manifest
        .files
        .iter()
        .find(|file| file.provider_driver == provider_driver)?;
    store.load_valid_object(file)
}

fn load_active_remote_metadata_for_source(
    root: &Path,
    provider_driver: &str,
    source_key: &str,
) -> (Option<DriverMetadataDocument>, u64) {
    let store = DriverMetadataStore::new(root.to_path_buf());
    let Some(activation) = store.load_latest_activation_cached() else {
        return (None, observe_effective_activation(source_key, None));
    };
    let mut generation = observe_effective_activation(source_key, Some(&activation));
    let Some(file) = activation
        .manifest
        .files
        .iter()
        .find(|file| file.provider_driver == provider_driver)
    else {
        return (None, generation);
    };
    if let Some(document) = store.load_valid_object(file) {
        return (Some(document), generation);
    }
    store.invalidate_activation_cache();
    let Some(activation) = store.load_latest_activation() else {
        return (None, observe_effective_activation(source_key, None));
    };
    generation = observe_effective_activation(source_key, Some(&activation));
    let Some(file) = activation
        .manifest
        .files
        .iter()
        .find(|file| file.provider_driver == provider_driver)
    else {
        return (None, generation);
    };
    (store.load_valid_object(file), generation)
}

fn default_store_root() -> PathBuf {
    get_buckyos_service_home_dir("aicc")
        .join("driver_metadata")
        .join("remote_cache")
        .join("v1")
}

fn source_store_root(source_key: &str) -> PathBuf {
    default_store_root().join(source_key)
}

fn touch_source_namespace(root: &Path) -> Result<()> {
    let path = root.join("last_used");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path.as_path())?;
    file.write_all(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .to_string()
            .as_bytes(),
    )?;
    file.sync_all()?;
    sync_parent_dir(root)
}

fn prune_source_namespaces(root: &Path, active_source_key: &str, retain: usize) -> Result<()> {
    if !root.is_dir() || retain == 0 {
        return Ok(());
    }
    let mut namespaces = std::fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map(|entry| {
            let path = entry.path();
            let source_key = entry.file_name().to_string_lossy().to_string();
            let last_used = std::fs::read_to_string(path.join("last_used"))
                .ok()
                .and_then(|value| value.parse::<u128>().ok())
                .unwrap_or(0);
            (source_key, path, last_used)
        })
        .collect::<Vec<_>>();
    namespaces.sort_by_key(|(source_key, _, last_used)| {
        (
            std::cmp::Reverse(source_key == active_source_key),
            std::cmp::Reverse(*last_used),
        )
    });
    for (_, path, _) in namespaces.into_iter().skip(retain) {
        std::fs::remove_dir_all(path)?;
    }
    sync_parent_dir(root)
}

fn validate_index(index: &DriverMetadataIndex) -> Result<()> {
    if index.format != INDEX_FORMAT {
        bail!("unsupported metadata index format");
    }
    if index.index_version != 1 {
        bail!("unsupported metadata index version");
    }
    if !index.required_features.is_empty() {
        bail!("metadata index requires unsupported features");
    }
    if index.index_revision_seq == 0 {
        bail!("metadata index revision must be positive");
    }
    let mut protocol_versions = HashSet::new();
    for track in index.tracks.iter() {
        if !protocol_versions.insert(track.protocol_version) {
            bail!("duplicate protocol track in metadata index");
        }
        validate_relative_path(track.manifest.path.as_str())?;
        if track.revision_seq == 0 {
            bail!("metadata track revision must be positive");
        }
        if track.protocol_version == PROTOCOL_VERSION {
            let expected = format!("v{}/manifest-{}.json", PROTOCOL_VERSION, track.revision_seq);
            if track.manifest.path != expected {
                bail!("supported metadata track uses a non-canonical manifest path");
            }
        }
        parse_obj_id(track.manifest.obj_id.as_str())?;
    }
    Ok(())
}

fn select_supported_track(index: &DriverMetadataIndex) -> Result<&DriverMetadataTrack> {
    let track = index
        .tracks
        .iter()
        .find(|track| track.protocol_version == PROTOCOL_VERSION)
        .ok_or_else(|| anyhow!("metadata index has no supported protocol track"))?;
    if !track.required_features.is_empty() {
        bail!("metadata track requires unsupported features");
    }
    Ok(track)
}

fn validate_manifest(manifest: &DriverMetadataManifest, track: &DriverMetadataTrack) -> Result<()> {
    if manifest.format != MANIFEST_FORMAT {
        bail!("unsupported metadata manifest format");
    }
    if manifest.protocol_version != PROTOCOL_VERSION
        || manifest.protocol_version != track.protocol_version
    {
        bail!("unsupported manifest protocol version");
    }
    if manifest.revision_seq != track.revision_seq {
        bail!("track and manifest revisions differ");
    }
    if manifest.protocol_revision != track.protocol_revision {
        bail!("track and manifest protocol revisions differ");
    }
    if !manifest.required_features.is_empty() {
        bail!("metadata manifest requires unsupported features");
    }
    if manifest.files.len() > MAX_PROVIDER_FILES {
        bail!("metadata manifest has too many provider files");
    }
    let mut identities = HashSet::new();
    for file in manifest.files.iter() {
        validate_provider_driver(file.provider_driver.as_str())?;
        if !identities.insert(file.provider_driver.as_str()) {
            bail!("duplicate provider_driver in manifest");
        }
        if file.schema_version != METADATA_SCHEMA_VERSION {
            bail!("unsupported provider metadata schema version");
        }
        if file.revision_seq == 0 {
            bail!("provider metadata revision must be positive");
        }
        validate_relative_path(file.path.as_str())?;
        let expected = format!(
            "v{}/providers/{}-{}.json",
            PROTOCOL_VERSION, file.provider_driver, file.revision_seq
        );
        if file.path != expected {
            bail!("provider metadata uses a non-canonical path");
        }
        parse_obj_id(file.obj_id.as_str())?;
    }
    let mut tombstones = HashSet::new();
    for tombstone in manifest.tombstones.iter() {
        validate_provider_driver(tombstone.provider_driver.as_str())?;
        if identities.contains(tombstone.provider_driver.as_str())
            || !tombstones.insert(tombstone.provider_driver.as_str())
        {
            bail!("duplicate or active tombstone provider_driver");
        }
        if tombstone.revision_seq == 0 {
            bail!("provider tombstone revision must be positive");
        }
    }
    Ok(())
}

fn validate_manifest_transition(
    current: Option<&DriverMetadataActivation>,
    candidate: &DriverMetadataManifest,
    candidate_obj_id: &ObjId,
) -> Result<()> {
    let Some(current) = current else {
        return Ok(());
    };
    if candidate.revision_seq < current.manifest.revision_seq {
        bail!("candidate manifest revision rollback");
    }
    if candidate.revision_seq == current.manifest.revision_seq {
        if current.manifest_obj_id != candidate_obj_id.to_string() || current.manifest != *candidate
        {
            bail!("candidate manifest revision conflict");
        }
        return Ok(());
    }

    let candidate_files = candidate
        .files
        .iter()
        .map(|file| (file.provider_driver.as_str(), file))
        .collect::<HashMap<_, _>>();
    let candidate_tombstones = candidate
        .tombstones
        .iter()
        .map(|item| (item.provider_driver.as_str(), item.revision_seq))
        .collect::<HashMap<_, _>>();
    let current_tombstones = current
        .manifest
        .tombstones
        .iter()
        .map(|item| (item.provider_driver.as_str(), item.revision_seq))
        .collect::<HashMap<_, _>>();

    for old in current.manifest.files.iter() {
        match candidate_files.get(old.provider_driver.as_str()) {
            Some(next) if next.revision_seq < old.revision_seq => {
                bail!(
                    "provider metadata revision rollback for {}",
                    old.provider_driver
                )
            }
            Some(next) if next.revision_seq == old.revision_seq && next.obj_id != old.obj_id => {
                bail!(
                    "provider metadata revision conflict for {}",
                    old.provider_driver
                )
            }
            Some(_) => {}
            None => match candidate_tombstones.get(old.provider_driver.as_str()) {
                Some(revision) if *revision > old.revision_seq => {}
                _ => bail!(
                    "provider {} disappeared without a newer tombstone",
                    old.provider_driver
                ),
            },
        }
    }
    for (driver, old_revision) in current_tombstones {
        if let Some(file) = candidate_files.get(driver) {
            if file.revision_seq <= old_revision {
                bail!("provider {} resurrected without a newer revision", driver);
            }
        } else if candidate_tombstones.get(driver).copied().unwrap_or(0) < old_revision {
            bail!("tombstone revision rollback for {}", driver);
        }
    }
    Ok(())
}

fn validate_metadata_bytes(
    bytes: &[u8],
    file: &DriverMetadataManifestFile,
) -> Result<DriverMetadataDocument> {
    let document: DriverMetadataDocument =
        serde_json::from_slice(bytes).context("parse provider metadata")?;
    if document.format != "buckyos.aicc.provider-driver-metadata"
        || document.schema_version != file.schema_version
        || document.provider_driver != file.provider_driver
        || document.revision_seq != file.revision_seq
        || !document.required_features.is_empty()
    {
        bail!("provider metadata identity or schema does not match manifest");
    }
    validate_driver_metadata_document(&document).map_err(anyhow::Error::msg)?;
    Ok(document)
}

fn validate_provider_driver(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("invalid provider_driver");
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<()> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.contains('%')
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("invalid metadata relative path");
    }
    Ok(())
}

fn join_canonical_url(base: &Url, relative: &str) -> Result<Url> {
    validate_relative_path(relative)?;
    let joined = base.join(relative)?;
    if joined.scheme() != base.scheme()
        || joined.host_str() != base.host_str()
        || joined.port_or_known_default() != base.port_or_known_default()
    {
        bail!("metadata path changed the source origin");
    }
    let base_directory = base
        .path()
        .rsplit_once('/')
        .map(|(directory, _)| format!("{}/", directory))
        .ok_or_else(|| anyhow!("metadata source URL has no base directory"))?;
    if !joined.path().starts_with(base_directory.as_str()) {
        bail!("metadata path escaped the source directory");
    }
    Ok(joined)
}

fn parse_obj_id(value: &str) -> Result<ObjId> {
    ObjId::new(value).map_err(|err| anyhow!(err.to_string()))
}

fn safe_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn manifest_sha256(manifest: &DriverMetadataManifest) -> Result<String> {
    Ok(content_sha256(serde_json::to_vec(manifest)?.as_slice()))
}

fn content_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn atomic_create_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_create_bytes(path, bytes.as_slice())
}

fn atomic_create_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.is_file() {
        let existing = std::fs::read(path)?;
        if existing == bytes {
            return Ok(());
        }
        bail!("immutable state conflict at {}", path.display());
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("state path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.part",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(temp.as_path());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp.as_path())?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    match durable_commit_create(temp.as_path(), path) {
        Ok(()) => Ok(()),
        Err(err) if path.is_file() => {
            let _ = std::fs::remove_file(temp.as_path());
            let existing = std::fs::read(path)?;
            if existing == bytes {
                Ok(())
            } else {
                Err(anyhow!(
                    "immutable state conflict at {}: {}",
                    path.display(),
                    err
                ))
            }
        }
        Err(err) => {
            let _ = std::fs::remove_file(temp.as_path());
            Err(err.into())
        }
    }
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<()> {
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<()> {
    Ok(())
}

fn load_activations(dir: &Path) -> Vec<DriverMetadataActivation> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut values = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let revision = entry
                .path()
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u64>().ok())?;
            let bytes = std::fs::read(entry.path()).ok()?;
            let activation = serde_json::from_slice::<DriverMetadataActivation>(&bytes).ok()?;
            (activation.manifest.revision_seq == revision).then_some(activation)
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.manifest.revision_seq));
    values
}

fn load_activation(path: &Path) -> Option<DriverMetadataActivation> {
    let revision = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.parse::<u64>().ok())?;
    let bytes = std::fs::read(path).ok()?;
    let activation = serde_json::from_slice::<DriverMetadataActivation>(&bytes).ok()?;
    (activation.manifest.revision_seq == revision).then_some(activation)
}

fn latest_activation_revision(dir: &Path) -> Option<u64> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u64>().ok())
        })
        .max()
}

#[cfg(not(windows))]
fn durable_commit_create(source: &Path, destination: &Path) -> Result<()> {
    std::fs::hard_link(source, destination)?;
    std::fs::remove_file(source)?;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("state path has no parent"))?;
    sync_parent_dir(parent)
}

#[cfg(windows)]
fn durable_commit_create(source: &Path, destination: &Path) -> Result<()> {
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    fn wide_path(path: &Path) -> Result<Vec<u16>> {
        let mut value = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if value.contains(&0) {
            bail!("state path contains an embedded null");
        }
        value.push(0);
        Ok(value)
    }

    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn latest_json_strict<T>(dir: &Path) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de> + Revisioned,
{
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut latest: Option<T> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let expected_revision = entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u64>().ok())
            .ok_or_else(|| anyhow!("invalid observed watermark filename"))?;
        let bytes = std::fs::read(entry.path())?;
        let value: T = serde_json::from_slice(bytes.as_slice())
            .with_context(|| format!("parse observed watermark {}", entry.path().display()))?;
        if value.revision_seq() != expected_revision {
            bail!("observed watermark filename and revision differ");
        }
        if latest
            .as_ref()
            .map(|current| value.revision_seq() > current.revision_seq())
            .unwrap_or(true)
        {
            latest = Some(value);
        }
    }
    Ok(latest)
}

trait Revisioned {
    fn revision_seq(&self) -> u64;
}

impl Revisioned for ObservedIndex {
    fn revision_seq(&self) -> u64 {
        self.index.index_revision_seq
    }
}

impl Revisioned for ObservedManifest {
    fn revision_seq(&self) -> u64 {
        self.manifest.revision_seq
    }
}

fn remove_part_files(root: &Path) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            remove_part_files(path.as_path())?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".part"))
            .unwrap_or(false)
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn prune_revision_files(dir: &Path, retain: usize) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut paths = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| {
        std::cmp::Reverse(
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u64>().ok())
                .unwrap_or(0),
        )
    });
    for path in paths.into_iter().skip(retain) {
        std::fs::remove_file(path)?;
    }
    sync_parent_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    static GENERATION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn response_meta(path_object: Option<VerifiedPathObject>) -> CyfsResponseMeta {
        CyfsResponseMeta {
            requested_url: "https://metadata.example/aicc/driver-metadata/index.json".to_string(),
            transport_url: "https://metadata.example/aicc/driver-metadata/index.json".to_string(),
            known_obj_id: None,
            url_obj_id: None,
            url_inner_path_steps: vec![],
            resp_raw: false,
            cyfs_headers: ndn_lib::CYFSHttpRespHeaders::default(),
            path_object,
            parents: vec![],
        }
    }

    fn obj_id(seed: &str) -> ObjId {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(seed.as_bytes());
        let chunk = ndn_lib::ChunkId::from_hash_result(&hash, ndn_lib::ChunkType::Sha256);
        chunk.to_obj_id()
    }

    fn sized_chunk(bytes: &[u8]) -> ObjId {
        ndn_lib::ChunkHasher::new(None)
            .unwrap()
            .calc_mix_chunk_id_from_bytes(bytes)
            .unwrap()
            .to_obj_id()
    }

    #[test]
    fn unsigned_path_response_is_rejected() {
        let meta = response_meta(None);
        assert!(validate_verified_path_response(&meta, None, "index").is_err());
    }

    #[test]
    fn path_response_enforces_expected_target_without_app_ttl_policy() {
        let target = obj_id("target");
        let long_lived = VerifiedPathObject {
            path: "/aicc/driver-metadata/index.json".to_string(),
            target: target.clone(),
            iat: 100,
            exp: 100 + 3 * 365 * 24 * 60 * 60,
        };
        let meta = response_meta(Some(long_lived));
        validate_verified_path_response(&meta, Some(&target), "index").unwrap();
        assert!(validate_verified_path_response(&meta, Some(&obj_id("other")), "index").is_err());
    }

    #[test]
    fn verified_chunk_size_is_checked_before_pull() {
        let target = sized_chunk(b"verified metadata");
        let meta = response_meta(Some(VerifiedPathObject {
            path: "/aicc/driver-metadata/index.json".to_string(),
            target: target.clone(),
            iat: 100,
            exp: 101,
        }));
        validate_verified_ndn_size(&meta, &target, 64, "index").unwrap();
        assert!(validate_verified_ndn_size(&meta, &target, 1, "index").is_err());
    }

    fn file(driver: &str, revision: u64, obj_id: &ObjId) -> DriverMetadataManifestFile {
        DriverMetadataManifestFile {
            provider_driver: driver.to_string(),
            path: format!("v1/providers/{}-{}.json", driver, revision),
            schema_version: 2,
            revision_seq: revision,
            obj_id: obj_id.to_string(),
        }
    }

    fn manifest(revision: u64, files: Vec<DriverMetadataManifestFile>) -> DriverMetadataManifest {
        DriverMetadataManifest {
            format: MANIFEST_FORMAT.to_string(),
            protocol_version: 1,
            protocol_revision: 0,
            revision_seq: revision,
            required_features: vec![],
            files,
            tombstones: vec![],
        }
    }

    fn document(driver: &str, revision: u64) -> DriverMetadataDocument {
        DriverMetadataDocument {
            format: "buckyos.aicc.provider-driver-metadata".to_string(),
            schema_version: 2,
            schema_revision: 0,
            provider_driver: driver.to_string(),
            revision_seq: revision,
            required_features: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn one_changed_provider_reuses_other_object() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let openai_v1 = obj_id("openai-v1");
        let claude_v1 = obj_id("claude-v1");
        let first = manifest(
            1,
            vec![file("openai", 1, &openai_v1), file("claude", 1, &claude_v1)],
        );
        for entry in first.files.iter() {
            let bytes = serde_json::to_vec(&document(&entry.provider_driver, 1)).unwrap();
            store
                .store_object(
                    &parse_obj_id(entry.obj_id.as_str()).unwrap(),
                    bytes.as_slice(),
                )
                .unwrap();
        }
        store
            .activate(1, obj_id("manifest-v1").to_string().as_str(), first.clone())
            .unwrap();

        let openai_v2 = obj_id("openai-v2");
        let second = manifest(
            2,
            vec![file("openai", 2, &openai_v2), file("claude", 1, &claude_v1)],
        );
        validate_manifest_transition(
            store.load_latest_activation().as_ref(),
            &second,
            &obj_id("manifest-v2"),
        )
        .unwrap();
        assert!(store.load_valid_object(&second.files[0]).is_none());
        assert!(store.load_valid_object(&second.files[1]).is_some());
    }

    #[test]
    fn interrupted_update_keeps_completed_candidate_objects() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let openai = obj_id("candidate-openai");
        let claude = obj_id("candidate-claude");
        let candidate = manifest(
            7,
            vec![file("openai", 3, &openai), file("claude", 2, &claude)],
        );
        store
            .observe_manifest(
                &candidate,
                obj_id("candidate-manifest").to_string().as_str(),
            )
            .unwrap();
        store
            .store_object(
                &openai,
                serde_json::to_vec(&document("openai", 3))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();

        store.cleanup_parts_and_orphans().unwrap();

        assert!(store
            .prepare_object_slot(&candidate.files[0])
            .unwrap()
            .is_some());
        assert!(store
            .prepare_object_slot(&candidate.files[1])
            .unwrap()
            .is_none());
        assert!(store.load_latest_activation().is_none());
    }

    #[test]
    fn manifest_total_size_is_bounded() {
        assert_eq!(
            checked_manifest_metadata_bytes(
                MAX_MANIFEST_METADATA_BYTES - MAX_METADATA_BYTES,
                MAX_METADATA_BYTES,
            )
            .unwrap(),
            MAX_MANIFEST_METADATA_BYTES
        );
        assert!(checked_manifest_metadata_bytes(MAX_MANIFEST_METADATA_BYTES, 1).is_err());
    }

    #[test]
    fn history_pruning_keeps_two_complete_activations() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        for revision in 1..=3 {
            let object = obj_id(format!("openai-{revision}").as_str());
            store
                .store_object(
                    &object,
                    serde_json::to_vec(&document("openai", revision))
                        .unwrap()
                        .as_slice(),
                )
                .unwrap();
            let value = manifest(revision, vec![file("openai", revision, &object)]);
            store
                .activate(
                    revision,
                    obj_id(format!("manifest-{revision}").as_str())
                        .to_string()
                        .as_str(),
                    value,
                )
                .unwrap();
        }

        store.prune_activations().unwrap();

        let revisions = load_activations(store.activations_dir().as_path())
            .into_iter()
            .map(|activation| activation.manifest.revision_seq)
            .collect::<Vec<_>>();
        assert_eq!(revisions, vec![3, 2]);
    }

    #[test]
    fn deletion_requires_newer_tombstone() {
        let openai = obj_id("openai-v3");
        let current_manifest = manifest(3, vec![file("openai", 3, &openai)]);
        let current = DriverMetadataActivation {
            format: ACTIVATION_FORMAT.to_string(),
            storage_version: 1,
            index_revision_seq: 3,
            manifest_obj_id: obj_id("manifest-v3").to_string(),
            manifest_sha256: manifest_sha256(&current_manifest).unwrap(),
            manifest: current_manifest,
        };
        let missing = manifest(4, vec![]);
        assert!(validate_manifest_transition(Some(&current), &missing, &obj_id("m4")).is_err());
        let mut deleted = missing;
        deleted.tombstones.push(DriverMetadataTombstone {
            provider_driver: "openai".to_string(),
            revision_seq: 4,
        });
        validate_manifest_transition(Some(&current), &deleted, &obj_id("m4")).unwrap();
    }

    #[test]
    fn incomplete_new_activation_falls_back_to_lkgs() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let old_obj = obj_id("old");
        let old_file = file("openai", 1, &old_obj);
        store
            .store_object(
                &old_obj,
                serde_json::to_vec(&document("openai", 1))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        store
            .activate(
                1,
                obj_id("m1").to_string().as_str(),
                manifest(1, vec![old_file]),
            )
            .unwrap();
        let missing_obj = obj_id("missing");
        store
            .activate(
                2,
                obj_id("m2").to_string().as_str(),
                manifest(2, vec![file("openai", 2, &missing_obj)]),
            )
            .unwrap();
        assert_eq!(
            store
                .load_latest_activation()
                .unwrap()
                .manifest
                .revision_seq,
            1
        );
    }

    #[test]
    fn immutable_revision_conflict_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("1.json");
        atomic_create_bytes(path.as_path(), b"first").unwrap();
        assert!(atomic_create_bytes(path.as_path(), b"second").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"first");
    }

    #[test]
    fn corrupted_observed_watermark_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        std::fs::write(store.observed_index_dir().join("8.json"), b"broken").unwrap();
        let index = DriverMetadataIndex {
            format: INDEX_FORMAT.to_string(),
            index_version: 1,
            index_revision: 0,
            index_revision_seq: 9,
            required_features: vec![],
            tracks: vec![DriverMetadataTrack {
                protocol_version: 1,
                protocol_revision: 0,
                revision_seq: 9,
                required_features: vec![],
                manifest: DriverMetadataIndexManifest {
                    path: "v1/manifest-9.json".to_string(),
                    obj_id: obj_id("manifest-9").to_string(),
                },
            }],
        };
        assert!(store.observe_index(&index, &obj_id("index-9")).is_err());
    }

    #[test]
    fn settings_require_canonical_secure_index_path() {
        let valid = serde_json::json!({
            "driver_metadata_update": {
                "enabled": true,
                "source_url": "https://metadata.example/aicc/driver-metadata/index.json"
            }
        });
        assert!(DriverMetadataUpdateSettings::from_aicc_settings(&valid)
            .unwrap()
            .is_some());
        let invalid = serde_json::json!({
            "driver_metadata_update": {
                "enabled": true,
                "source_url": "http://metadata.example/other.json"
            }
        });
        assert!(DriverMetadataUpdateSettings::from_aicc_settings(&invalid).is_err());
    }

    #[test]
    fn settings_bound_update_interval() {
        let settings = |interval_secs| {
            serde_json::json!({
                "driver_metadata_update": {
                    "enabled": true,
                    "source_url": "https://metadata.example/aicc/driver-metadata/index.json",
                    "interval_secs": interval_secs
                }
            })
        };

        let too_short = DriverMetadataUpdateSettings::from_aicc_settings(&settings(0))
            .unwrap()
            .unwrap();
        assert_eq!(too_short.interval_secs, MIN_UPDATE_INTERVAL_SECS);

        let too_long = DriverMetadataUpdateSettings::from_aicc_settings(&settings(u64::MAX))
            .unwrap()
            .unwrap();
        assert_eq!(too_long.interval_secs, MAX_UPDATE_INTERVAL_SECS);
    }

    #[test]
    fn metadata_paths_reject_encoded_traversal() {
        let base = Url::parse("https://metadata.example/aicc/driver-metadata/index.json").unwrap();
        assert!(join_canonical_url(&base, "v1/providers/openai-1.json").is_ok());
        assert!(join_canonical_url(&base, "%2e%2e/escape.json").is_err());
        assert!(join_canonical_url(&base, "v1/%2E%2E/escape.json").is_err());
    }

    #[test]
    fn metadata_protocol_rejects_non_canonical_object_paths() {
        let mut index = DriverMetadataIndex {
            format: INDEX_FORMAT.to_string(),
            index_version: 1,
            index_revision: 0,
            index_revision_seq: 1,
            required_features: vec![],
            tracks: vec![DriverMetadataTrack {
                protocol_version: 1,
                protocol_revision: 0,
                revision_seq: 1,
                required_features: vec![],
                manifest: DriverMetadataIndexManifest {
                    path: "v1/manifest-1.json".to_string(),
                    obj_id: obj_id("manifest-1").to_string(),
                },
            }],
        };
        assert!(validate_index(&index).is_ok());
        index.tracks[0].manifest.path = "v1/other.json".to_string();
        assert!(validate_index(&index).is_err());

        let object_id = obj_id("openai-1");
        let mut candidate = manifest(1, vec![file("openai", 1, &object_id)]);
        let track = DriverMetadataTrack {
            protocol_version: 1,
            protocol_revision: 0,
            revision_seq: 1,
            required_features: vec![],
            manifest: DriverMetadataIndexManifest {
                path: "v1/manifest-1.json".to_string(),
                obj_id: obj_id("manifest-1").to_string(),
            },
        };
        assert!(validate_manifest(&candidate, &track).is_ok());
        candidate.files[0].path = "v1/providers/other.json".to_string();
        assert!(validate_manifest(&candidate, &track).is_err());
    }

    #[test]
    fn provider_metadata_rejects_invalid_rules_and_unknown_fields() {
        let object_id = obj_id("openai-v1");
        let entry = file("openai", 1, &object_id);
        let duplicate_rules = serde_json::json!({
            "format": "buckyos.aicc.provider-driver-metadata",
            "schema_version": 2,
            "schema_revision": 0,
            "provider_driver": "openai",
            "revision_seq": 1,
            "required_features": [],
            "models": [
                {"id": "gpt-test"},
                {"id": "GPT-TEST"}
            ],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        });
        assert!(validate_metadata_bytes(
            serde_json::to_vec(&duplicate_rules).unwrap().as_slice(),
            &entry,
        )
        .is_err());

        let mut unknown_field = duplicate_rules;
        unknown_field["models"] = serde_json::json!([]);
        unknown_field["unexpected"] = serde_json::json!(true);
        assert!(validate_metadata_bytes(
            serde_json::to_vec(&unknown_field).unwrap().as_slice(),
            &entry,
        )
        .is_err());

        let mut invalid_variant = unknown_field;
        invalid_variant
            .as_object_mut()
            .unwrap()
            .remove("unexpected");
        invalid_variant["variants"] = serde_json::json!([{
            "name": "reasoning.high",
            "provider_options": {"reasoning": {"effort": "high"}}
        }]);
        assert!(validate_metadata_bytes(
            serde_json::to_vec(&invalid_variant).unwrap().as_slice(),
            &entry,
        )
        .is_err());

        let mut invalid_cost = invalid_variant.clone();
        invalid_cost["variants"] = serde_json::json!([]);
        invalid_cost["models"] = serde_json::json!([{
            "id": "gpt-test",
            "input_token_usd": -0.01
        }]);
        assert!(validate_metadata_bytes(
            serde_json::to_vec(&invalid_cost).unwrap().as_slice(),
            &entry,
        )
        .is_err());

        let mut invalid_version_limit = invalid_variant;
        invalid_version_limit["variants"] = serde_json::json!([]);
        invalid_version_limit["models"] = serde_json::json!([]);
        invalid_version_limit["version_rules"] = serde_json::json!([{
            "family": "gpt",
            "capabilities": {"max_context_tokens": 0}
        }]);
        assert!(validate_metadata_bytes(
            serde_json::to_vec(&invalid_version_limit)
                .unwrap()
                .as_slice(),
            &entry,
        )
        .is_err());
    }

    #[test]
    fn metadata_source_change_advances_generation_once() {
        let _guard = GENERATION_TEST_LOCK.lock().unwrap();
        let settings = serde_json::json!({
            "driver_metadata_update": {
                "enabled": true,
                "source_url": "https://generation.example/aicc/driver-metadata/index.json",
                "interval_secs": 3600
            }
        });
        configure_remote_metadata_source(&settings).unwrap();
        let configured_generation = DRIVER_METADATA_GENERATION.load(Ordering::Acquire);

        configure_remote_metadata_source(&settings).unwrap();
        assert_eq!(
            DRIVER_METADATA_GENERATION.load(Ordering::Acquire),
            configured_generation
        );

        configure_remote_metadata_source(&serde_json::json!({})).unwrap();
        assert!(DRIVER_METADATA_GENERATION.load(Ordering::Acquire) > configured_generation);
    }

    #[test]
    fn metadata_sources_have_independent_storage_namespaces() {
        let first = DriverMetadataUpdateSettings {
            enabled: true,
            source_url: "https://metadata-a.example/aicc/driver-metadata/index.json".to_string(),
            interval_secs: 3600,
        };
        let second = DriverMetadataUpdateSettings {
            enabled: true,
            source_url: "https://metadata-b.example/aicc/driver-metadata/index.json".to_string(),
            interval_secs: 3600,
        };
        assert_ne!(first.source_key().unwrap(), second.source_key().unwrap());
        assert_eq!(first.source_key().unwrap(), first.source_key().unwrap());
    }

    #[test]
    fn stable_index_selects_v1_track_alongside_future_track() {
        let index = DriverMetadataIndex {
            format: INDEX_FORMAT.to_string(),
            index_version: 1,
            index_revision: 0,
            index_revision_seq: 12,
            required_features: vec![],
            tracks: vec![
                DriverMetadataTrack {
                    protocol_version: 2,
                    protocol_revision: 0,
                    revision_seq: 3,
                    required_features: vec!["future".to_string()],
                    manifest: DriverMetadataIndexManifest {
                        path: "v2/manifest-3.json".to_string(),
                        obj_id: obj_id("manifest-v2").to_string(),
                    },
                },
                DriverMetadataTrack {
                    protocol_version: 1,
                    protocol_revision: 0,
                    revision_seq: 9,
                    required_features: vec![],
                    manifest: DriverMetadataIndexManifest {
                        path: "v1/manifest-9.json".to_string(),
                        obj_id: obj_id("manifest-v1").to_string(),
                    },
                },
            ],
        };
        validate_index(&index).unwrap();
        assert_eq!(select_supported_track(&index).unwrap().revision_seq, 9);

        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        store.observe_index(&index, &obj_id("index-12")).unwrap();
        let mut without_v1 = index;
        without_v1.index_revision_seq = 13;
        without_v1
            .tracks
            .retain(|track| track.protocol_version != 1);
        assert!(store
            .observe_index(&without_v1, &obj_id("index-13"))
            .is_err());
    }

    #[test]
    fn corrupted_cached_object_is_removed_for_full_redownload() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let object_id = obj_id("corrupt-object");
        let entry = file("openai", 1, &object_id);
        store.store_object(&object_id, b"broken").unwrap();
        assert!(store.prepare_object_slot(&entry).unwrap().is_none());
        assert!(!store.object_path(&object_id).exists());
        assert!(!store.object_digest_path(&object_id).exists());
    }

    #[test]
    fn semantically_valid_cached_object_corruption_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let object_id = obj_id("silently-corrupted-object");
        let entry = file("openai", 1, &object_id);
        let original = serde_json::to_vec(&document("openai", 1)).unwrap();
        store.store_object(&object_id, original.as_slice()).unwrap();

        let mut changed: serde_json::Value = serde_json::from_slice(original.as_slice()).unwrap();
        changed["schema_revision"] = serde_json::json!(1);
        std::fs::write(
            store.object_path(&object_id),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();

        assert!(store.load_valid_object(&entry).is_none());
        assert!(store.prepare_object_slot(&entry).unwrap().is_none());
        assert!(!store.object_path(&object_id).exists());
        assert!(!store.object_digest_path(&object_id).exists());
    }

    #[test]
    fn cached_activation_revalidates_only_requested_provider() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let openai_id = obj_id("cached-openai");
        let claude_id = obj_id("cached-claude");
        let activation_manifest = manifest(
            1,
            vec![file("openai", 1, &openai_id), file("claude", 1, &claude_id)],
        );
        store
            .store_object(
                &openai_id,
                serde_json::to_vec(&document("openai", 1))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        store
            .store_object(
                &claude_id,
                serde_json::to_vec(&document("claude", 1))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        store
            .activate(
                1,
                obj_id("cached-manifest").to_string().as_str(),
                activation_manifest,
            )
            .unwrap();

        std::fs::write(store.object_path(&claude_id), b"broken").unwrap();

        assert_eq!(
            load_active_remote_metadata_in(temp.path(), "openai")
                .unwrap()
                .provider_driver,
            "openai"
        );
        assert!(load_active_remote_metadata_in(temp.path(), "claude").is_none());
    }

    #[test]
    fn effective_activation_fallback_and_recovery_advance_generation() {
        let _guard = GENERATION_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let source_key = "generation-fallback-source";

        let old_obj_id = obj_id("generation-openai-v1");
        let old_file = file("openai", 1, &old_obj_id);
        store
            .store_object(
                &old_obj_id,
                serde_json::to_vec(&document("openai", 1))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        store
            .activate(
                1,
                obj_id("generation-manifest-v1").to_string().as_str(),
                manifest(1, vec![old_file]),
            )
            .unwrap();

        let new_obj_id = obj_id("generation-openai-v2");
        let new_file = file("openai", 2, &new_obj_id);
        let new_manifest = manifest(2, vec![new_file.clone()]);
        let new_manifest_obj_id = obj_id("generation-manifest-v2");
        store
            .store_object(
                &new_obj_id,
                serde_json::to_vec(&document("openai", 2))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        store
            .activate(
                2,
                new_manifest_obj_id.to_string().as_str(),
                new_manifest.clone(),
            )
            .unwrap();

        let (current, current_generation) =
            load_active_remote_metadata_for_source(temp.path(), "openai", source_key);
        assert_eq!(current.unwrap().revision_seq, 2);

        std::fs::write(store.object_path(&new_obj_id), b"broken").unwrap();
        let (fallback, fallback_generation) =
            load_active_remote_metadata_for_source(temp.path(), "openai", source_key);
        assert_eq!(fallback.unwrap().revision_seq, 1);
        assert!(fallback_generation > current_generation);

        std::fs::write(store.object_path(&old_obj_id), b"broken").unwrap();
        let (unavailable, unavailable_generation) =
            load_active_remote_metadata_for_source(temp.path(), "openai", source_key);
        assert!(unavailable.is_none());
        assert!(unavailable_generation > fallback_generation);

        store.prepare_object_slot(&new_file).unwrap();
        store
            .store_object(
                &new_obj_id,
                serde_json::to_vec(&document("openai", 2))
                    .unwrap()
                    .as_slice(),
            )
            .unwrap();
        store
            .activate(
                2,
                new_manifest_obj_id.to_string().as_str(),
                new_manifest.clone(),
            )
            .unwrap();
        let recovered_generation =
            observe_effective_metadata_identity(activation_identity_from_parts(
                source_key,
                new_manifest.revision_seq,
                new_manifest_obj_id.to_string().as_str(),
                manifest_sha256(&new_manifest).unwrap().as_str(),
            ));
        let (recovered, loaded_generation) =
            load_active_remote_metadata_for_source(temp.path(), "openai", source_key);
        assert_eq!(recovered.unwrap().revision_seq, 2);
        assert!(recovered_generation > unavailable_generation);
        assert!(loaded_generation >= recovered_generation);
    }

    #[test]
    fn corrupted_activation_can_be_recreated_without_touching_lkgs() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let path = store.activations_dir().join("2.json");
        std::fs::write(path.as_path(), b"broken").unwrap();
        let candidate = manifest(2, vec![]);
        store
            .activate(2, obj_id("manifest-2").to_string().as_str(), candidate)
            .unwrap();
        assert_eq!(
            store
                .load_latest_activation()
                .unwrap()
                .manifest
                .revision_seq,
            2
        );
    }

    #[test]
    fn activation_manifest_digest_rejects_semantic_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        store
            .activate(
                1,
                obj_id("manifest-1").to_string().as_str(),
                manifest(1, vec![]),
            )
            .unwrap();
        let path = store.activations_dir().join("1.json");
        let mut activation: DriverMetadataActivation =
            serde_json::from_slice(std::fs::read(path.as_path()).unwrap().as_slice()).unwrap();
        activation.manifest.protocol_revision = 1;
        std::fs::write(path, serde_json::to_vec_pretty(&activation).unwrap()).unwrap();

        assert!(store.load_latest_activation().is_none());
    }

    #[test]
    fn semantically_invalid_activation_falls_back_to_lkgs() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        store
            .activate(
                1,
                obj_id("manifest-1").to_string().as_str(),
                manifest(1, vec![]),
            )
            .unwrap();

        let mut invalid = manifest(2, vec![]);
        invalid.protocol_version = 2;
        store
            .activate(2, obj_id("manifest-2").to_string().as_str(), invalid)
            .unwrap();

        assert_eq!(
            store
                .load_latest_activation()
                .unwrap()
                .manifest
                .revision_seq,
            1
        );
    }

    #[test]
    fn activation_filename_must_match_manifest_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let activation_manifest = manifest(9, vec![]);
        let activation = DriverMetadataActivation {
            format: ACTIVATION_FORMAT.to_string(),
            storage_version: 1,
            index_revision_seq: 9,
            manifest_obj_id: obj_id("manifest-9").to_string(),
            manifest_sha256: manifest_sha256(&activation_manifest).unwrap(),
            manifest: activation_manifest,
        };
        std::fs::write(
            store.activations_dir().join("2.json"),
            serde_json::to_vec(&activation).unwrap(),
        )
        .unwrap();

        assert!(store.load_latest_activation().is_none());
        assert!(load_activations(store.activations_dir().as_path()).is_empty());
    }

    #[test]
    fn source_namespace_gc_keeps_active_and_most_recent_namespaces() {
        let temp = tempfile::tempdir().unwrap();
        let active_key = "a".repeat(64);
        let recent_key = "b".repeat(64);
        let old_key = "c".repeat(64);
        for (key, last_used) in [(&active_key, 30), (&recent_key, 20), (&old_key, 10)] {
            let namespace = temp.path().join(key);
            std::fs::create_dir_all(namespace.as_path()).unwrap();
            std::fs::write(namespace.join("last_used"), last_used.to_string()).unwrap();
        }

        prune_source_namespaces(temp.path(), active_key.as_str(), 2).unwrap();

        assert!(temp.path().join(active_key).is_dir());
        assert!(temp.path().join(recent_key).is_dir());
        assert!(!temp.path().join(old_key).exists());
    }

    #[test]
    fn dropping_update_attempt_removes_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let attempt = store.new_attempt_dir().unwrap();
        let path = attempt.path().to_path_buf();
        std::fs::write(path.join("index.part"), b"partial").unwrap();

        drop(attempt);

        assert!(!path.exists());
    }

    #[test]
    fn observed_provider_watermark_blocks_later_file_rollback() {
        let temp = tempfile::tempdir().unwrap();
        let store = DriverMetadataStore::new(temp.path().to_path_buf());
        store.prepare().unwrap();
        let high = manifest(10, vec![file("openai", 8, &obj_id("openai-8"))]);
        store
            .observe_manifest(&high, obj_id("manifest-10").to_string().as_str())
            .unwrap();
        let rollback = manifest(11, vec![file("openai", 7, &obj_id("openai-7"))]);
        assert!(store
            .observe_manifest(&rollback, obj_id("manifest-11").to_string().as_str())
            .is_err());
    }
}
