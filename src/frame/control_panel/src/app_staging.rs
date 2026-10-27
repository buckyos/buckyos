//! PIKG staging (doc/App 安装协议.md D5): between finalize and install a PIKG is a
//! FileObject in the named store, bound in control-panel's `pikg_staging` temp bucket
//! under its opaque handle. The binding carries the staging metadata and expires with
//! it; an install task that leases the handle keeps it. Only externally dropped files
//! (`incoming/`) are on the file system.

use crate::app_install_driver::pikg_staging_root;
use crate::buckets::{BucketKeys, ZoneBucket};
use crate::pikg::PikgReader;
use buckyos_api::bucket_client::{Binding, PutRequest};
use buckyos_api::{
    get_buckyos_api_runtime, PikgStagingMetadata, PikgStagingPurpose, StagingHandle,
    APP_INSTALL_SCHEMA_VERSION, CONTROL_PANEL_PIKG_STAGING_BUCKET,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use kRPC::RPCErrors;
use name_lib::DID;
use named_store::{NamedDataMgr, PinScope};
use ndn_lib::{load_named_obj_and_verify, FileObject, ObjId, StoreMode};
use ndn_toolkit::{cacl_file_object, CheckMode};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

const INSPECT_TTL_SECS: u64 = 60 * 60;
const INSTALL_TTL_SECS: u64 = 24 * 60 * 60;
/// A lease of a running install task keeps the handle this long past its last renewal.
const LEASED_TTL_SECS: u64 = 7 * 24 * 60 * 60;
const IMPORT_LEASE_SECS: u64 = 24 * 60 * 60;
const PRINCIPAL_QUOTA_BYTES: u64 = 1024 * 1024 * 1024;
const ZONE_QUOTA_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const METADATA_KEY: &str = "staging";

/// A live staging entry: its metadata and the FileObject it binds.
struct Staged {
    metadata: PikgStagingMetadata,
    obj_id: ObjId,
}

impl Staged {
    fn from_binding(binding: &Binding) -> Result<Self, RPCErrors> {
        let raw = binding
            .metadata
            .get(METADATA_KEY)
            .ok_or_else(|| PikgStagingStore::error("staging binding has no metadata"))?;
        let metadata: PikgStagingMetadata = serde_json::from_str(raw).map_err(|error| {
            PikgStagingStore::error(format!("invalid staging metadata: {error}"))
        })?;
        let obj_id = binding
            .obj_id
            .as_deref()
            .ok_or_else(|| PikgStagingStore::error("staging binding has no object"))
            .and_then(|raw| {
                ObjId::new(raw).map_err(|error| PikgStagingStore::error(error.to_string()))
            })?;
        Ok(Self { metadata, obj_id })
    }

    fn live(&self, now: u64) -> bool {
        self.metadata.expires_at > now || !self.metadata.leases.is_empty()
    }

    fn owned_by(&self, owner_user_id: &str, owner_app_id: &str, zone_did: &DID) -> bool {
        self.metadata.owner_user_id == owner_user_id
            && self.metadata.owner_app_id == owner_app_id
            && self.metadata.zone_did == *zone_did
    }

    fn request(&self) -> Result<PutRequest, RPCErrors> {
        let raw = serde_json::to_string(&self.metadata).map_err(|error| {
            PikgStagingStore::error(format!("serialize staging metadata failed: {error}"))
        })?;
        Ok(PutRequest {
            key: self.metadata.handle.as_str().to_string(),
            obj_id: self.obj_id.to_string(),
            metadata: BTreeMap::from([(METADATA_KEY.to_string(), raw)]),
            expires_at: Some((self.metadata.expires_at * 1000) as i64),
            ..Default::default()
        })
    }
}

pub struct PikgStagingStore {
    incoming: PathBuf,
    index: Arc<dyn BucketKeys>,
    named_store: Option<NamedDataMgr>,
    lock: Mutex<()>,
}

impl PikgStagingStore {
    pub fn new() -> Self {
        Self {
            incoming: pikg_staging_root().join("incoming"),
            index: Arc::new(ZoneBucket(CONTROL_PANEL_PIKG_STAGING_BUCKET)),
            named_store: None,
            lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_parts(
        incoming: PathBuf,
        index: Arc<dyn BucketKeys>,
        named_store: NamedDataMgr,
    ) -> Self {
        Self {
            incoming,
            index,
            named_store: Some(named_store),
            lock: Mutex::new(()),
        }
    }

    fn error(message: impl Into<String>) -> RPCErrors {
        RPCErrors::ReasonError(message.into())
    }

    pub(crate) async fn named_store(&self) -> Result<NamedDataMgr, RPCErrors> {
        match &self.named_store {
            Some(store) => Ok(store.clone()),
            None => get_buckyos_api_runtime()?.get_local_named_store().await,
        }
    }

    /// The staged PIKG of a resolved handle, read from the named store.
    pub(crate) async fn reader(
        &self,
        obj_id: &ObjId,
        digest: &str,
    ) -> Result<PikgReader, RPCErrors> {
        PikgReader::open_named(self.named_store().await?, obj_id, Some(digest))
            .await
            .map_err(|error| Self::error(format!("open staged pikg failed: {error}")))
    }

    async fn load(&self, handle: &StagingHandle) -> Result<Option<Staged>, RPCErrors> {
        self.index
            .get(handle.as_str(), None)
            .await?
            .map(|binding| Staged::from_binding(&binding))
            .transpose()
    }

    async fn save(&self, staged: &Staged) -> Result<(), RPCErrors> {
        self.index.put(staged.request()?).await.map(|_| ())
    }

    /// Live entries; expired, unleased ones are unbound on the way.
    async fn live_entries(&self, now: u64) -> Result<Vec<Staged>, RPCErrors> {
        let mut live = Vec::new();
        for binding in self.index.list("").await? {
            let staged = match Staged::from_binding(&binding) {
                Ok(staged) => staged,
                Err(error) => {
                    log::warn!("drop staging binding {}: {error}", binding.key);
                    self.index.delete(&binding.key).await?;
                    continue;
                }
            };
            if staged.live(now) {
                live.push(staged);
            } else {
                self.index.delete(&binding.key).await?;
            }
        }
        Ok(live)
    }

    async fn gc_incoming(&self, now: u64) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.incoming).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(id) = name.strip_suffix(".pikg") else {
                continue;
            };
            if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(metadata) = tokio::fs::symlink_metadata(entry.path()).await else {
                continue;
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_secs())
                .unwrap_or(now);
            if metadata.is_file() && now.saturating_sub(modified) > INSTALL_TTL_SECS {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }

    pub async fn gc(&self) -> Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;
        let now = buckyos_get_unix_timestamp();
        self.live_entries(now).await?;
        self.gc_incoming(now).await;
        Ok(())
    }

    async fn register(
        &self,
        obj_id: &ObjId,
        digest: String,
        size: u64,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let _guard = self.lock.lock().await;
        let now = buckyos_get_unix_timestamp();
        let live = self.live_entries(now).await?;
        if let Some(existing) = live.iter().find(|item| {
            item.owned_by(owner_user_id, owner_app_id, zone_did)
                && item.metadata.pikg_digest == digest
                && item.metadata.purpose == purpose
        }) {
            return Ok(existing.metadata.clone());
        }

        let distinct = |filter: &dyn Fn(&Staged) -> bool| {
            live.iter().filter(|item| filter(item)).fold(
                HashMap::<&str, u64>::new(),
                |mut content, item| {
                    content
                        .entry(item.metadata.pikg_digest.as_str())
                        .or_insert(item.metadata.size);
                    content
                },
            )
        };
        let principal_content = distinct(&|item| {
            item.metadata.owner_user_id == owner_user_id
                && item.metadata.owner_app_id == owner_app_id
        });
        let zone_content = distinct(&|item| item.metadata.zone_did == *zone_did);
        let additional = |content: &HashMap<&str, u64>| {
            (!content.contains_key(digest.as_str()))
                .then_some(size)
                .unwrap_or(0)
        };
        if principal_content.values().sum::<u64>() + additional(&principal_content)
            > PRINCIPAL_QUOTA_BYTES
        {
            return Err(Self::error("principal pikg staging quota exceeded"));
        }
        if zone_content.values().sum::<u64>() + additional(&zone_content) > ZONE_QUOTA_BYTES {
            return Err(Self::error("zone pikg staging quota exceeded"));
        }

        let handle = StagingHandle::new_opaque(&uuid::Uuid::new_v4().simple().to_string())
            .map_err(Self::error)?;
        let staged = Staged {
            metadata: PikgStagingMetadata {
                schema_version: APP_INSTALL_SCHEMA_VERSION,
                handle,
                owner_user_id: owner_user_id.to_string(),
                owner_app_id: owner_app_id.to_string(),
                zone_did: zone_did.clone(),
                pikg_digest: digest,
                size,
                purpose,
                created_at: now,
                expires_at: now
                    + match purpose {
                        PikgStagingPurpose::Inspect => INSPECT_TTL_SECS,
                        PikgStagingPurpose::Install => INSTALL_TTL_SECS,
                    },
                leases: Vec::new(),
            },
            obj_id: obj_id.clone(),
        };
        self.save(&staged).await?;
        Ok(staged.metadata)
    }

    /// Validates a PIKG FileObject already in the named store and stages it. The
    /// object is held by a short import lease until the staging binding holds it.
    async fn stage_object(
        &self,
        store: &NamedDataMgr,
        source: &ObjId,
        expected_digest: Option<&str>,
        expected_size: Option<u64>,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let lease = format!("control-panel:pikg-import:{}", uuid::Uuid::new_v4());
        store
            .pin(
                source,
                &lease,
                PinScope::Recursive,
                Some(std::time::Duration::from_secs(IMPORT_LEASE_SECS)),
            )
            .await
            .map_err(|error| Self::error(format!("retain staging source failed: {error}")))?;
        let result = async {
            let object = store
                .get_object(source)
                .await
                .map_err(|error| Self::error(format!("read staging FileObject failed: {error}")))?;
            let file: FileObject = load_named_obj_and_verify(source, &object)
                .map_err(|error| Self::error(format!("invalid staging FileObject: {error}")))?;
            if expected_size.is_some_and(|size| size != file.size) {
                return Err(Self::error("PIKG_SIZE_MISMATCH: FileObject size differs"));
            }
            Self::validate_expected_content(expected_digest, file.size)?;
            let reader = PikgReader::open_named(store.clone(), source, expected_digest)
                .await
                .map_err(|error| Self::error(format!("validate staged pikg failed: {error}")))?;
            reader
                .verify_all_contents()
                .await
                .map_err(|error| Self::error(format!("verify staged pikg failed: {error}")))?;
            self.register(
                source,
                reader.pikg_digest().to_string(),
                file.size,
                owner_user_id,
                owner_app_id,
                zone_did,
                purpose,
            )
            .await
        }
        .await;
        if let Err(error) = store.unpin(source, &lease).await {
            log::warn!("release PIKG import lease failed: {error}");
        }
        result
    }

    /// Stores a local PIKG file in the named store as a FileObject.
    async fn import_file(store: &NamedDataMgr, path: &Path) -> Result<ObjId, RPCErrors> {
        let (_, obj_id, _) = cacl_file_object(
            Some(store),
            path,
            &FileObject::default(),
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .map_err(|error| Self::error(format!("import pikg into named store failed: {error}")))?;
        Ok(obj_id)
    }

    pub(crate) async fn stage_preinstall_file(
        &self,
        source: &Path,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let store = self.named_store().await?;
        let obj_id = Self::import_file(&store, source).await?;
        self.stage_object(
            &store,
            &obj_id,
            None,
            None,
            owner_user_id,
            owner_app_id,
            zone_did,
            PikgStagingPurpose::Install,
        )
        .await
    }

    pub async fn finalize_named_object(
        &self,
        source: &ObjId,
        expected_digest: Option<&str>,
        expected_size: u64,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        if !source.is_file_object() {
            return Err(RPCErrors::ParseRequestError(
                "staging source must be a FileObject".to_string(),
            ));
        }
        Self::validate_expected_content(expected_digest, expected_size)?;
        let store = self.named_store().await?;
        self.stage_object(
            &store,
            source,
            expected_digest,
            Some(expected_size),
            owner_user_id,
            owner_app_id,
            zone_did,
            purpose,
        )
        .await
    }

    pub async fn finalize_local_file(
        &self,
        file_id: &str,
        expected_digest: &str,
        expected_size: u64,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        Self::validate_expected_content(Some(expected_digest), expected_size)?;
        if file_id.len() != 32 || !file_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RPCErrors::ParseRequestError(
                "invalid local_file_id".to_string(),
            ));
        }
        let source = self.incoming.join(format!("{file_id}.pikg"));
        let unavailable = |error: std::io::Error| {
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) {
                Self::error("LOCAL_PIKG_UNAVAILABLE: local staging file is not present")
            } else {
                Self::error(format!("read local staging file failed: {error}"))
            }
        };
        let metadata = tokio::fs::symlink_metadata(&source)
            .await
            .map_err(unavailable)?;
        if !metadata.is_file() || metadata.len() != expected_size {
            return Err(Self::error(
                "local staging source must be a regular file of the expected size",
            ));
        }
        let canonical = tokio::fs::canonicalize(&source)
            .await
            .map_err(unavailable)?;
        let incoming = tokio::fs::canonicalize(&self.incoming)
            .await
            .map_err(unavailable)?;
        if canonical.parent() != Some(incoming.as_path()) {
            return Err(Self::error(
                "local staging source escaped incoming directory",
            ));
        }
        let store = self.named_store().await?;
        let obj_id = Self::import_file(&store, &canonical).await?;
        let result = self
            .stage_object(
                &store,
                &obj_id,
                Some(expected_digest),
                Some(expected_size),
                owner_user_id,
                owner_app_id,
                zone_did,
                purpose,
            )
            .await;
        if result.is_ok() {
            let _ = tokio::fs::remove_file(&source).await;
        }
        result
    }

    fn validate_expected_content(digest: Option<&str>, size: u64) -> Result<(), RPCErrors> {
        if digest.is_some_and(|digest| {
            digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err(RPCErrors::ParseRequestError(
                "invalid pikg_digest".to_string(),
            ));
        }
        if size < 4 || size > PRINCIPAL_QUOTA_BYTES {
            return Err(Self::error(
                "pikg size exceeds staging quota or is too small",
            ));
        }
        Ok(())
    }

    async fn owned(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
    ) -> Result<Option<Staged>, RPCErrors> {
        let handle = StagingHandle::parse(raw_handle).map_err(RPCErrors::ParseRequestError)?;
        let Some(staged) = self.load(&handle).await? else {
            return Ok(None);
        };
        if !staged.owned_by(owner_user_id, owner_app_id, zone_did) {
            return Err(RPCErrors::NoPermission(
                "staging handle is not owned by this principal and zone".to_string(),
            ));
        }
        Ok(Some(staged))
    }

    /// The handle's metadata and FileObject; `lease` (an install task id) keeps it past its expiry.
    pub async fn resolve(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        required_purpose: PikgStagingPurpose,
        lease: Option<&str>,
    ) -> Result<(PikgStagingMetadata, ObjId), RPCErrors> {
        let _guard = self.lock.lock().await;
        let mut staged = self
            .owned(raw_handle, owner_user_id, owner_app_id, zone_did)
            .await?
            .ok_or_else(|| Self::error("staging handle expired"))?;
        if staged.metadata.schema_version != APP_INSTALL_SCHEMA_VERSION {
            return Err(Self::error("staging handle has an unknown schema"));
        }
        if required_purpose == PikgStagingPurpose::Install
            && staged.metadata.purpose != PikgStagingPurpose::Install
        {
            return Err(Self::error(
                "inspect-only staging handle cannot be consumed by an install task",
            ));
        }
        let now = buckyos_get_unix_timestamp();
        if !staged.live(now) {
            return Err(Self::error("staging handle expired"));
        }
        if let Some(lease) = lease {
            if !staged
                .metadata
                .leases
                .iter()
                .any(|existing| existing == lease)
            {
                staged.metadata.leases.push(lease.to_string());
            }
            staged.metadata.expires_at = staged.metadata.expires_at.max(now + LEASED_TTL_SECS);
            self.save(&staged).await?;
        }
        Ok((staged.metadata, staged.obj_id))
    }

    /// Keeps `staged` when it is still wanted, unbinds it otherwise.
    async fn settle(&self, staged: &Staged) -> Result<(), RPCErrors> {
        if staged.live(buckyos_get_unix_timestamp()) {
            self.save(staged).await
        } else {
            self.index.delete(staged.metadata.handle.as_str()).await
        }
    }

    pub async fn release(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        lease: Option<&str>,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let _guard = self.lock.lock().await;
        let mut staged = self
            .owned(raw_handle, owner_user_id, owner_app_id, zone_did)
            .await?
            .ok_or_else(|| Self::error("staging handle expired"))?;
        if let Some(lease) = lease {
            staged.metadata.leases.retain(|existing| existing != lease);
        } else if staged.metadata.leases.is_empty() {
            staged.metadata.expires_at = buckyos_get_unix_timestamp();
        } else {
            return Err(Self::error(
                "staging handle is referenced by an active task",
            ));
        }
        self.settle(&staged).await?;
        Ok(staged.metadata)
    }

    pub async fn release_task_lease(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        task_id: &str,
    ) -> Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;
        let Some(mut staged) = self
            .owned(raw_handle, owner_user_id, owner_app_id, zone_did)
            .await?
        else {
            return Ok(());
        };
        staged.metadata.leases.retain(|lease| lease != task_id);
        if staged.metadata.leases.is_empty() {
            staged.metadata.expires_at = buckyos_get_unix_timestamp();
        }
        self.settle(&staged).await
    }

    pub async fn reconcile_task_leases(
        &self,
        tasks: &buckyos_api::TaskManagerClient,
    ) -> Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;
        let now = buckyos_get_unix_timestamp();
        for mut staged in self.live_entries(now).await? {
            let mut finished = Vec::new();
            for lease in &staged.metadata.leases {
                if crate::app_mutation::task_is_finished(
                    tasks.get_task(lease).await.map(|task| task.phase),
                )? {
                    finished.push(lease.clone());
                }
            }
            if !finished.is_empty() {
                staged
                    .metadata
                    .leases
                    .retain(|lease| !finished.contains(lease));
                if staged.metadata.leases.is_empty() {
                    staged.metadata.expires_at = now;
                }
                self.settle(&staged).await?;
            }
        }
        self.gc_incoming(now).await;
        Ok(())
    }

    pub async fn status(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        self.resolve(
            raw_handle,
            owner_user_id,
            owner_app_id,
            zone_did,
            PikgStagingPurpose::Inspect,
            None,
        )
        .await
        .map(|(metadata, _)| metadata)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::buckets::tests::MemoryBucket;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "buckyos-pikg-staging-test-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    pub(crate) async fn test_named_store(root: &Path) -> NamedDataMgr {
        use named_store::{NamedLocalStore, StoreLayout, StoreTarget};
        let store = NamedLocalStore::get_named_store_by_path(root.join("named_store"))
            .await
            .unwrap();
        let store_id = store.store_id().to_string();
        let manager = NamedDataMgr::new();
        manager
            .register_store(Arc::new(tokio::sync::Mutex::new(store)))
            .await;
        manager
            .add_layout(StoreLayout::new(
                1,
                vec![StoreTarget {
                    store_id,
                    device_did: String::new(),
                    capacity: None,
                    used: None,
                    readonly: false,
                    enabled: true,
                    weight: 1,
                }],
                0,
                0,
            ))
            .await;
        manager
    }

    async fn staging(root: &Path) -> (PikgStagingStore, Arc<MemoryBucket>, NamedDataMgr) {
        let index = Arc::new(MemoryBucket::new(CONTROL_PANEL_PIKG_STAGING_BUCKET));
        let store = test_named_store(root).await;
        let staging =
            PikgStagingStore::with_parts(root.join("incoming"), index.clone(), store.clone());
        (staging, index, store)
    }

    async fn large_pikg(root: &Path) -> (PathBuf, PathBuf, ndn_lib::ChunkId, String, u64) {
        use crate::pikg::PikgBuilder;
        use buckyos_api::{AppDoc, AppType, SubPkgDesc};
        use flate2::{write::GzEncoder, Compression};
        use ndn_lib::{build_named_object_by_json, ChunkId, ChunkType, OBJ_TYPE_PKG};
        use package_lib::PackageMeta;
        use sha2::{Digest, Sha256};
        use std::io::Read;

        tokio::fs::create_dir_all(root).await.unwrap();
        let payload = root.join("web.tar.gz");
        let encoder = GzEncoder::new(
            std::fs::File::create(&payload).unwrap(),
            Compression::none(),
        );
        let mut archive = tar::Builder::new(encoder);
        let size = 33 * 1024 * 1024;
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "payload.bin", std::io::repeat(7).take(size))
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        let payload_bytes = tokio::fs::read(&payload).await.unwrap();
        let content = ChunkId::from_mix_hash_result(
            payload_bytes.len() as u64,
            &Sha256::digest(&payload_bytes),
            ChunkType::Mix256,
        );
        let owner = DID::new("bns", "tester");
        let mut meta = PackageMeta::new(
            "all.web.demo.tester.bns.did",
            "0.1.0",
            "tester",
            &owner,
            None,
        );
        meta.size = payload_bytes.len() as u64;
        meta.content = content.to_string();
        let value = serde_json::to_value(meta).unwrap();
        let (meta_id, _) = build_named_object_by_json(OBJ_TYPE_PKG, &value);
        let mut desc = SubPkgDesc::new("all.web.demo.tester.bns.did#0.1.0");
        desc.pkg_objid = Some(meta_id);
        let app = AppDoc::builder(AppType::Web, "demo", "0.1.0", "tester", &owner)
            .web_pkg(desc)
            .build()
            .unwrap();
        let (builder, _) = PikgBuilder::new()
            .app_doc(&app)
            .unwrap()
            .add_package_meta_value(value)
            .unwrap();
        let pikg = root.join("demo.pikg");
        builder
            .add_payload_file("web", &payload)
            .unwrap()
            .write_to(&pikg)
            .await
            .unwrap();
        let bytes = tokio::fs::read(&pikg).await.unwrap();
        let digest = hex::encode(Sha256::digest(&bytes));
        (pikg, payload, content, digest, bytes.len() as u64)
    }

    #[tokio::test]
    async fn large_pikg_stages_from_named_store_and_payload_roundtrip() {
        use crate::app_install_driver::store_pikg_payload;
        use ndn_lib::CHUNK_DEFAULT_SIZE;
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncReadExt;

        let root = temp_root();
        let (pikg, payload, content, digest, size) = large_pikg(&root).await;
        assert!(size > CHUNK_DEFAULT_SIZE);
        let (staging, index, manager) = staging(&root).await;
        let zone = DID::new("bns", "zone");

        let file_id = uuid::Uuid::new_v4().simple().to_string();
        tokio::fs::create_dir_all(&staging.incoming).await.unwrap();
        let local = staging.incoming.join(format!("{file_id}.pikg"));
        tokio::fs::copy(&pikg, &local).await.unwrap();
        let local_metadata = staging
            .finalize_local_file(
                &file_id,
                &digest,
                size,
                "alice",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Install,
            )
            .await
            .unwrap();
        assert!(!local.exists());
        assert_eq!(local_metadata.size, size);
        assert_eq!(local_metadata.pikg_digest, digest);

        let (file, object, _) = cacl_file_object(
            Some(&manager),
            &pikg,
            &FileObject::default(),
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .unwrap();
        assert!(ObjId::new(&file.content).unwrap().is_chunk_list());
        let remote_metadata = staging
            .finalize_named_object(
                &object,
                None,
                size,
                "bob",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Install,
            )
            .await
            .unwrap();
        assert_eq!(remote_metadata.pikg_digest, local_metadata.pikg_digest);
        assert_ne!(remote_metadata.handle, local_metadata.handle);
        assert_eq!(index.list("").await.unwrap().len(), 2);
        let (metadata, obj_id) = staging
            .resolve(
                remote_metadata.handle.as_str(),
                "bob",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Install,
                Some("task"),
            )
            .await
            .unwrap();
        assert_eq!(metadata.size, size);
        assert_eq!(metadata.leases, vec!["task".to_string()]);
        let reader = staging.reader(&obj_id, &digest).await.unwrap();
        assert_eq!(reader.pikg_digest(), digest);
        let (mut pikg_reader, _) = manager.open_reader(&obj_id, None).await.unwrap();
        let mut staged_bytes = Vec::new();
        pikg_reader.read_to_end(&mut staged_bytes).await.unwrap();
        assert_eq!(hex::encode(Sha256::digest(&staged_bytes)), digest);

        store_pikg_payload(&manager, &payload, &content)
            .await
            .unwrap();
        let (mut reader, actual_size) = manager.open_chunk_reader(&content, 0).await.unwrap();
        let mut actual = Vec::new();
        reader.read_to_end(&mut actual).await.unwrap();
        assert_eq!(
            actual_size,
            tokio::fs::metadata(&payload).await.unwrap().len()
        );
        assert_eq!(actual, tokio::fs::read(&payload).await.unwrap());
        assert!(matches!(
            manager.query_chunk_state(&content).await.unwrap().0,
            named_store::ChunkStoreState::SameAs(_)
        ));
        drop(reader);
        drop(manager);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn staging_rejects_bad_identity_and_mismatched_content() {
        let root = temp_root();
        let (staging, index, manager) = staging(&root).await;
        let zone = DID::new("bns", "zone");
        assert!(staging
            .finalize_local_file(
                "../escape",
                &"a".repeat(64),
                8,
                "alice",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Inspect
            )
            .await
            .is_err());
        let (pikg, _, _, digest, size) = large_pikg(&root.join("pkg")).await;
        let (_, object, _) = cacl_file_object(
            Some(&manager),
            &pikg,
            &FileObject::default(),
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .unwrap();
        for (expected_digest, expected_size) in [
            (Some("a".repeat(64)), size),
            (Some(digest.clone()), size - 1),
        ] {
            assert!(staging
                .finalize_named_object(
                    &object,
                    expected_digest.as_deref(),
                    expected_size,
                    "alice",
                    "buckyos-tool",
                    &zone,
                    PikgStagingPurpose::Inspect
                )
                .await
                .is_err());
        }
        let chunk = ndn_lib::ChunkId::new(&format!("sha256:{digest}"))
            .unwrap()
            .to_obj_id();
        assert!(staging
            .finalize_named_object(
                &chunk,
                Some(&digest),
                size,
                "alice",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Inspect
            )
            .await
            .is_err());
        assert!(index.list("").await.unwrap().is_empty());
        assert!(manager
            .list_pins_by_owner("control-panel:pikg-import:", None, 10)
            .await
            .unwrap()
            .is_empty());
        drop(manager);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_staging_rejects_symlinks() {
        let root = temp_root();
        let (staging, _, _) = staging(&root).await;
        tokio::fs::create_dir_all(&staging.incoming).await.unwrap();
        let source = root.join("source.pikg");
        tokio::fs::write(&source, b"PK\x03\x04").await.unwrap();
        let file_id = "1".repeat(32);
        std::os::unix::fs::symlink(source, staging.incoming.join(format!("{file_id}.pikg")))
            .unwrap();
        assert!(staging
            .finalize_local_file(
                &file_id,
                &"a".repeat(64),
                4,
                "alice",
                "buckyos-tool",
                &DID::new("bns", "zone"),
                PikgStagingPurpose::Inspect
            )
            .await
            .is_err());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    async fn seed(staging: &PikgStagingStore, expired: bool) -> PikgStagingMetadata {
        let now = buckyos_get_unix_timestamp();
        let staged = Staged {
            metadata: PikgStagingMetadata {
                schema_version: APP_INSTALL_SCHEMA_VERSION,
                handle: StagingHandle::new_opaque("0123456789abcdef0123456789abcdef").unwrap(),
                owner_user_id: "alice".to_string(),
                owner_app_id: "buckyos-tool".to_string(),
                zone_did: DID::new("bns", "test-zone"),
                pikg_digest: "sha256-test-content".to_string(),
                size: 4,
                purpose: PikgStagingPurpose::Install,
                created_at: now.saturating_sub(10),
                expires_at: if expired {
                    now.saturating_sub(1)
                } else {
                    now + 60
                },
                leases: Vec::new(),
            },
            obj_id: ObjId::new(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap(),
        };
        staging.save(&staged).await.unwrap();
        staged.metadata
    }

    #[tokio::test]
    async fn cleanup_after_restart_releases_only_finished_task_leases() {
        let root = temp_root();
        let (staging, index, _) = staging(&root).await;
        let mut metadata = seed(&staging, true).await;
        let key = metadata.handle.as_str().to_string();
        let mut staged =
            Staged::from_binding(&index.get(&key, None).await.unwrap().unwrap()).unwrap();
        staged.metadata.leases = vec![
            "waiting".to_string(),
            "canceled".to_string(),
            "missing".to_string(),
        ];
        staging.save(&staged).await.unwrap();
        metadata.leases = staged.metadata.leases.clone();
        let tasks = buckyos_api::TaskManagerClient::new_in_process(Box::new(
            crate::app_mutation::tests::TestTasks {
                states: HashMap::from([
                    ("waiting".to_string(), buckyos_api::TaskPhase::Waiting),
                    ("canceled".to_string(), buckyos_api::TaskPhase::Terminal),
                ]),
                ..Default::default()
            },
        ));
        staging.reconcile_task_leases(&tasks).await.unwrap();
        let remaining =
            Staged::from_binding(&index.get(&key, None).await.unwrap().unwrap()).unwrap();
        assert_eq!(remaining.metadata.leases, vec!["waiting".to_string()]);
        let unavailable = buckyos_api::TaskManagerClient::new_in_process(Box::new(
            crate::app_mutation::tests::TestTasks {
                fail_reads: true,
                ..Default::default()
            },
        ));
        assert!(staging.reconcile_task_leases(&unavailable).await.is_err());
        let done = buckyos_api::TaskManagerClient::new_in_process(Box::new(
            crate::app_mutation::tests::TestTasks::default(),
        ));
        staging.reconcile_task_leases(&done).await.unwrap();
        assert!(index.get(&key, None).await.unwrap().is_none());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn task_lease_release_is_idempotent_and_preserves_other_tasks() {
        let root = temp_root();
        let (staging, index, _) = staging(&root).await;
        let metadata = seed(&staging, false).await;
        let key = metadata.handle.as_str().to_string();
        let mut staged =
            Staged::from_binding(&index.get(&key, None).await.unwrap().unwrap()).unwrap();
        staged.metadata.leases = vec!["old".to_string(), "new".to_string()];
        staging.save(&staged).await.unwrap();
        for _ in 0..2 {
            staging
                .release_task_lease(&key, "alice", "buckyos-tool", &metadata.zone_did, "old")
                .await
                .unwrap();
        }
        let remaining =
            Staged::from_binding(&index.get(&key, None).await.unwrap().unwrap()).unwrap();
        assert_eq!(remaining.metadata.leases, vec!["new".to_string()]);
        staging
            .release_task_lease(&key, "alice", "buckyos-tool", &metadata.zone_did, "new")
            .await
            .unwrap();
        assert!(index.get(&key, None).await.unwrap().is_none());
        staging
            .release_task_lease(&key, "alice", "buckyos-tool", &metadata.zone_did, "old")
            .await
            .unwrap();
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn staging_is_principal_and_zone_scoped() {
        let root = temp_root();
        let (staging, _, _) = staging(&root).await;
        let metadata = seed(&staging, false).await;
        let error = staging
            .resolve(
                metadata.handle.as_str(),
                "mallory",
                "buckyos-tool",
                &metadata.zone_did,
                PikgStagingPurpose::Install,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, RPCErrors::NoPermission(_)));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn active_lease_protects_expired_content_until_release() {
        let root = temp_root();
        let (staging, index, _) = staging(&root).await;
        let metadata = seed(&staging, false).await;
        let key = metadata.handle.as_str().to_string();
        let (leased, _) = staging
            .resolve(
                &key,
                "alice",
                "buckyos-tool",
                &metadata.zone_did,
                PikgStagingPurpose::Install,
                Some("task-1"),
            )
            .await
            .unwrap();
        assert!(leased.expires_at >= buckyos_get_unix_timestamp() + LEASED_TTL_SECS - 5);
        let mut staged =
            Staged::from_binding(&index.get(&key, None).await.unwrap().unwrap()).unwrap();
        staged.metadata.expires_at = buckyos_get_unix_timestamp().saturating_sub(1);
        staging.save(&staged).await.unwrap();
        staging.gc().await.unwrap();
        assert!(index.get(&key, None).await.unwrap().is_some());

        staging
            .release(
                &key,
                "alice",
                "buckyos-tool",
                &metadata.zone_did,
                Some("task-1"),
            )
            .await
            .unwrap();
        assert!(index.get(&key, None).await.unwrap().is_none());
        assert!(staging
            .resolve(
                &key,
                "alice",
                "buckyos-tool",
                &metadata.zone_did,
                PikgStagingPurpose::Inspect,
                None
            )
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
