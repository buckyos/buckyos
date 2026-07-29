use crate::metadata_resolver::DriverMetadataDocument;
use anyhow::{anyhow, bail, Context, Result};
use buckyos_kit::get_buckyos_system_etc_dir;
use ndn_lib::ObjId;
use ndn_toolkit::cyfs_ndn_client::CyfsNdnClient;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const INDEX_FORMAT: &str = "buckyos.aicc.driver-metadata-index";
const MANIFEST_FORMAT: &str = "buckyos.aicc.driver-metadata-manifest";
const ACTIVATION_FORMAT: &str = "buckyos.aicc.driver-metadata-activation";
const PROTOCOL_VERSION: u32 = 1;
const METADATA_SCHEMA_VERSION: u32 = 1;
const MAX_PATH_OBJECT_TTL_SECS: u64 = 24 * 60 * 60;
const MAX_INDEX_BYTES: u64 = 256 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROVIDER_FILES: usize = 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct DriverMetadataUpdateSettings {
    #[serde(default)]
    pub enabled: bool,
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
        parsed.interval_secs = parsed.interval_secs.max(60);
        Ok(Some(parsed))
    }
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DriverMetadataActivation {
    format: String,
    storage_version: u32,
    index_revision_seq: u64,
    manifest_obj_id: String,
    manifest: DriverMetadataManifest,
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
}

impl DriverMetadataUpdater {
    pub fn new(settings: DriverMetadataUpdateSettings) -> Result<Self> {
        let client = CyfsNdnClient::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| anyhow!(err.to_string()))?;
        Ok(Self {
            settings,
            client,
            store: DriverMetadataStore::new(default_store_root()),
        })
    }

    pub async fn update_once(&self) -> Result<DriverMetadataUpdateOutcome> {
        self.store.prepare()?;
        let attempt = self.store.new_attempt_dir()?;
        let result = self.update_once_inner(attempt.as_path()).await;
        let _ = std::fs::remove_dir_all(attempt.as_path());
        let _ = self.store.cleanup_parts_and_orphans();
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
        let (manifest_bytes, downloaded_manifest_obj_id) = self
            .download_verified(
                manifest_url.as_str(),
                Some(&manifest_obj_id),
                MAX_MANIFEST_BYTES,
                attempt,
                "manifest",
            )
            .await?;
        if downloaded_manifest_obj_id != manifest_obj_id {
            bail!("manifest PathObject target does not match index ObjId");
        }
        let manifest: DriverMetadataManifest =
            serde_json::from_slice(manifest_bytes.as_slice()).context("parse metadata manifest")?;
        validate_manifest(&manifest, track)?;
        self.store
            .observe_manifest(&manifest, manifest_obj_id.to_string().as_str())?;

        let current = self.store.load_latest_activation();
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

        for file in manifest.files.iter() {
            if self.store.prepare_object_slot(file)? {
                continue;
            }
            let expected_obj_id = parse_obj_id(file.obj_id.as_str())?;
            let file_url = join_canonical_url(&source_url, file.path.as_str())?;
            let (bytes, downloaded_obj_id) = self
                .download_verified(
                    file_url.as_str(),
                    Some(&expected_obj_id),
                    MAX_METADATA_BYTES,
                    attempt,
                    file.provider_driver.as_str(),
                )
                .await?;
            if downloaded_obj_id != expected_obj_id {
                bail!(
                    "metadata PathObject target for {} does not match manifest ObjId",
                    file.provider_driver
                );
            }
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

        self.store.activate(
            index.index_revision_seq,
            manifest_obj_id.to_string().as_str(),
            manifest.clone(),
        )?;
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
        let response = self
            .client
            .get(url.to_string())
            .send()
            .await
            .map_err(|err| anyhow!(err.to_string()))?;
        let path_object = response
            .meta()
            .path_object
            .clone()
            .ok_or_else(|| anyhow!("{} response has no verified PathObject", label))?;
        if path_object.exp < path_object.iat
            || path_object.exp - path_object.iat > MAX_PATH_OBJECT_TTL_SECS
        {
            bail!("{} PathObject TTL exceeds 24 hours", label);
        }
        if let Some(expected) = expected_obj_id {
            if &path_object.target != expected {
                bail!("{} PathObject target does not match expected ObjId", label);
            }
        }
        if response
            .meta()
            .cyfs_headers
            .chunk_size
            .map(|size| size > max_bytes)
            .unwrap_or(false)
        {
            bail!("{} exceeds the maximum size", label);
        }

        let output = attempt.join(format!("{}.part", safe_label(label)));
        response
            .pull_to_local_file(output.as_path())
            .await
            .map_err(|err| anyhow!(err.to_string()))?;
        let metadata = std::fs::metadata(output.as_path())?;
        if metadata.len() > max_bytes {
            bail!("{} exceeds the maximum size", label);
        }
        let bytes = std::fs::read(output.as_path())?;
        Ok((bytes, path_object.target))
    }
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

    fn new_attempt_dir(&self) -> Result<PathBuf> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = self
            .staging_dir()
            .join(format!("{}-{}", std::process::id(), now));
        std::fs::create_dir(&path)?;
        Ok(path)
    }

    fn object_path(&self, obj_id: &ObjId) -> PathBuf {
        self.objects_dir()
            .join(format!("{}.json", obj_id.to_base32()))
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
        atomic_create_bytes(self.object_path(obj_id).as_path(), bytes)
    }

    fn prepare_object_slot(&self, file: &DriverMetadataManifestFile) -> Result<bool> {
        if self.load_valid_object(file).is_some() {
            return Ok(true);
        }
        let obj_id = parse_obj_id(file.obj_id.as_str())?;
        let path = self.object_path(&obj_id);
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        Ok(false)
    }

    fn load_valid_object(
        &self,
        file: &DriverMetadataManifestFile,
    ) -> Option<DriverMetadataDocument> {
        let obj_id = parse_obj_id(file.obj_id.as_str()).ok()?;
        let bytes = std::fs::read(self.object_path(&obj_id)).ok()?;
        validate_metadata_bytes(bytes.as_slice(), file).ok()
    }

    fn activate(
        &self,
        index_revision_seq: u64,
        manifest_obj_id: &str,
        manifest: DriverMetadataManifest,
    ) -> Result<()> {
        let activation = DriverMetadataActivation {
            format: ACTIVATION_FORMAT.to_string(),
            storage_version: 1,
            index_revision_seq,
            manifest_obj_id: manifest_obj_id.to_string(),
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
                        && existing.manifest == activation.manifest =>
                {
                    return Ok(())
                }
                Some(_) => bail!("activation revision conflict"),
                None => std::fs::remove_file(path.as_path())?,
            }
        }
        atomic_create_json(path.as_path(), &activation)
    }

    fn load_latest_activation(&self) -> Option<DriverMetadataActivation> {
        load_activations(self.activations_dir().as_path())
            .into_iter()
            .find(|activation| self.validate_activation(activation))
    }

    fn validate_activation(&self, activation: &DriverMetadataActivation) -> bool {
        if activation.format != ACTIVATION_FORMAT || activation.storage_version != 1 {
            return false;
        }
        activation
            .manifest
            .files
            .iter()
            .all(|file| self.load_valid_object(file).is_some())
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

        let referenced = load_activations(self.activations_dir().as_path())
            .into_iter()
            .flat_map(|activation| activation.manifest.files)
            .filter_map(|file| parse_obj_id(file.obj_id.as_str()).ok())
            .map(|obj_id| format!("{}.json", obj_id.to_base32()))
            .collect::<HashSet<_>>();
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
}

pub(crate) fn load_active_remote_metadata(provider_driver: &str) -> Option<DriverMetadataDocument> {
    load_active_remote_metadata_in(default_store_root().as_path(), provider_driver)
}

fn load_active_remote_metadata_in(
    root: &Path,
    provider_driver: &str,
) -> Option<DriverMetadataDocument> {
    let store = DriverMetadataStore::new(root.to_path_buf());
    let activation = store.load_latest_activation()?;
    let file = activation
        .manifest
        .files
        .iter()
        .find(|file| file.provider_driver == provider_driver)?;
    store.load_valid_object(file)
}

fn default_store_root() -> PathBuf {
    get_buckyos_system_etc_dir()
        .join("aicc")
        .join("driver_metadata")
        .join("remote_cache")
        .join("v1")
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
    let mut protocol_versions = HashSet::new();
    for track in index.tracks.iter() {
        if !protocol_versions.insert(track.protocol_version) {
            bail!("duplicate protocol track in metadata index");
        }
        validate_relative_path(track.manifest.path.as_str())?;
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
        validate_relative_path(file.path.as_str())?;
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
    match std::fs::hard_link(temp.as_path(), path) {
        Ok(()) => {
            std::fs::remove_file(temp.as_path())?;
            sync_parent_dir(parent)
        }
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
    let mut values = read_json_files::<DriverMetadataActivation>(dir);
    values.sort_by_key(|value| std::cmp::Reverse(value.manifest.revision_seq));
    values
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

fn read_json_files<T>(dir: &Path) -> Vec<T>
where
    T: for<'de> Deserialize<'de>,
{
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|entry| std::fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(bytes.as_slice()).ok())
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn obj_id(seed: &str) -> ObjId {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(seed.as_bytes());
        let chunk = ndn_lib::ChunkId::from_hash_result(&hash, ndn_lib::ChunkType::Sha256);
        chunk.to_obj_id()
    }

    fn file(driver: &str, revision: u64, obj_id: &ObjId) -> DriverMetadataManifestFile {
        DriverMetadataManifestFile {
            provider_driver: driver.to_string(),
            path: format!("v1/providers/{}-{}.json", driver, revision),
            schema_version: 1,
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
            schema_version: 1,
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
    fn deletion_requires_newer_tombstone() {
        let openai = obj_id("openai-v3");
        let current = DriverMetadataActivation {
            format: ACTIVATION_FORMAT.to_string(),
            storage_version: 1,
            index_revision_seq: 3,
            manifest_obj_id: obj_id("manifest-v3").to_string(),
            manifest: manifest(3, vec![file("openai", 3, &openai)]),
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
        assert!(!store.prepare_object_slot(&entry).unwrap());
        assert!(!store.object_path(&object_id).exists());
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
