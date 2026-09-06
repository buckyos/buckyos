use crate::app_install_driver::pikg_staging_root;
use crate::pikg::PikgReader;
use buckyos_api::{
    get_buckyos_api_runtime, PikgStagingMetadata, PikgStagingPurpose, StagingHandle,
    APP_INSTALL_SCHEMA_VERSION,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use kRPC::RPCErrors;
use name_lib::DID;
use named_store::NamedDataMgr;
use ndn_lib::{ChunkReader, FileObject, ObjId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

const INSPECT_TTL_SECS: u64 = 60 * 60;
const INSTALL_TTL_SECS: u64 = 24 * 60 * 60;
const PRINCIPAL_QUOTA_BYTES: u64 = 1024 * 1024 * 1024;
const ZONE_QUOTA_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub struct PikgStagingStore {
    root: PathBuf,
    metadata_root: PathBuf,
    lock: Mutex<()>,
}

impl PikgStagingStore {
    pub fn new() -> Self {
        Self::with_root(pikg_staging_root())
    }

    pub(crate) fn with_root(root: PathBuf) -> Self {
        Self {
            metadata_root: root.join("metadata"),
            root,
            lock: Mutex::new(()),
        }
    }

    fn error(message: impl Into<String>) -> RPCErrors {
        RPCErrors::ReasonError(message.into())
    }

    async fn ensure_roots(&self) -> Result<(), RPCErrors> {
        tokio::fs::create_dir_all(&self.metadata_root)
            .await
            .map_err(|error| Self::error(format!("create pikg staging root failed: {error}")))
    }

    fn metadata_path(&self, handle: &StagingHandle) -> PathBuf {
        self.metadata_root.join(format!("{}.json", handle.as_str()))
    }

    fn content_path(&self, digest: &str) -> PathBuf {
        self.root.join(format!("{digest}.pikg"))
    }

    async fn read_metadata_path(path: &Path) -> Result<PikgStagingMetadata, RPCErrors> {
        let raw = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| Self::error(format!("read staging metadata failed: {error}")))?;
        serde_json::from_str(&raw)
            .map_err(|error| Self::error(format!("invalid staging metadata: {error}")))
    }

    async fn write_metadata(&self, metadata: &PikgStagingMetadata) -> Result<(), RPCErrors> {
        let target = self.metadata_path(&metadata.handle);
        let tmp = self
            .metadata_root
            .join(format!(".{}.tmp", metadata.handle.as_str()));
        let raw = serde_json::to_vec(metadata)
            .map_err(|error| Self::error(format!("serialize staging metadata failed: {error}")))?;
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|error| Self::error(format!("create staging metadata failed: {error}")))?;
        file.write_all(&raw)
            .await
            .map_err(|error| Self::error(format!("write staging metadata failed: {error}")))?;
        file.sync_all().await.ok();
        tokio::fs::rename(&tmp, &target)
            .await
            .map_err(|error| Self::error(format!("commit staging metadata failed: {error}")))
    }

    async fn list_metadata(&self) -> Result<Vec<PikgStagingMetadata>, RPCErrors> {
        self.ensure_roots().await?;
        let mut result = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.metadata_root)
            .await
            .map_err(|error| Self::error(format!("list staging metadata failed: {error}")))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| Self::error(format!("read staging metadata entry failed: {error}")))?
        {
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Ok(metadata) = Self::read_metadata_path(&entry.path()).await {
                result.push(metadata);
            }
        }
        Ok(result)
    }

    async fn gc_locked(&self, now: u64, preserved_digest: Option<&str>) -> Result<(), RPCErrors> {
        let all = self.list_metadata().await?;
        for metadata in all
            .iter()
            .filter(|metadata| metadata.expires_at <= now && metadata.leases.is_empty())
        {
            let _ = tokio::fs::remove_file(self.metadata_path(&metadata.handle)).await;
            let still_referenced = all.iter().any(|other| {
                other.handle != metadata.handle
                    && other.pikg_digest == metadata.pikg_digest
                    && (other.expires_at > now || !other.leases.is_empty())
            });
            if !still_referenced && preserved_digest != Some(metadata.pikg_digest.as_str()) {
                let _ = tokio::fs::remove_file(self.content_path(&metadata.pikg_digest)).await;
            }
        }
        let incoming = self.root.join("incoming");
        if let Ok(mut entries) = tokio::fs::read_dir(incoming).await {
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
        Ok(())
    }

    pub async fn gc(&self) -> Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;
        self.ensure_roots().await?;
        self.gc_locked(buckyos_get_unix_timestamp(), None).await
    }

    async fn register_staged_content(
        &self,
        digest: String,
        path: PathBuf,
        size: u64,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let _guard = self.lock.lock().await;
        let now = buckyos_get_unix_timestamp();
        self.gc_locked(now, Some(&digest)).await?;
        let all = self.list_metadata().await?;
        if let Some(existing) = all.iter().find(|item| {
            item.owner_user_id == owner_user_id
                && item.owner_app_id == owner_app_id
                && item.zone_did == *zone_did
                && item.pikg_digest == digest
                && item.purpose == purpose
                && (item.expires_at > now || !item.leases.is_empty())
        }) {
            self.commit_content(&path, &digest).await?;
            return Ok(existing.clone());
        }

        let principal_content = all
            .iter()
            .filter(|item| {
                item.owner_user_id == owner_user_id
                    && item.owner_app_id == owner_app_id
                    && (item.expires_at > now || !item.leases.is_empty())
            })
            .fold(HashMap::<&str, u64>::new(), |mut content, item| {
                content
                    .entry(item.pikg_digest.as_str())
                    .or_insert(item.size);
                content
            });
        let zone_content = all
            .iter()
            .filter(|item| {
                item.zone_did == *zone_did && (item.expires_at > now || !item.leases.is_empty())
            })
            .fold(HashMap::<&str, u64>::new(), |mut content, item| {
                content
                    .entry(item.pikg_digest.as_str())
                    .or_insert(item.size);
                content
            });
        let principal_usage = principal_content.values().copied().sum::<u64>();
        let zone_usage = zone_content.values().copied().sum::<u64>();
        let principal_additional = (!principal_content.contains_key(digest.as_str()))
            .then_some(size)
            .unwrap_or(0);
        let zone_additional = (!zone_content.contains_key(digest.as_str()))
            .then_some(size)
            .unwrap_or(0);
        if principal_usage.saturating_add(principal_additional) > PRINCIPAL_QUOTA_BYTES {
            if !all.iter().any(|item| item.pikg_digest == digest) {
                let _ = tokio::fs::remove_file(&path).await;
            }
            return Err(Self::error("principal pikg staging quota exceeded"));
        }
        if zone_usage.saturating_add(zone_additional) > ZONE_QUOTA_BYTES {
            if !all.iter().any(|item| item.pikg_digest == digest) {
                let _ = tokio::fs::remove_file(&path).await;
            }
            return Err(Self::error("zone pikg staging quota exceeded"));
        }
        self.commit_content(&path, &digest).await?;
        let handle = StagingHandle::new_opaque(&uuid::Uuid::new_v4().simple().to_string())
            .map_err(Self::error)?;
        let metadata = PikgStagingMetadata {
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
        };
        self.write_metadata(&metadata).await?;
        Ok(metadata)
    }

    async fn commit_content(&self, source: &Path, digest: &str) -> Result<(), RPCErrors> {
        let target = self.content_path(digest);
        if source != target
            && !tokio::fs::try_exists(&target)
                .await
                .map_err(|error| Self::error(error.to_string()))?
        {
            tokio::fs::rename(source, target)
                .await
                .map_err(|error| Self::error(format!("commit staged pikg failed: {error}")))?;
        }
        Ok(())
    }

    pub(crate) async fn stage_preinstall_file(
        &self,
        source: &Path,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        self.ensure_roots().await?;
        let (digest, path) = PikgReader::stage_pikg_file(source, &self.root)
            .await
            .map_err(|error| Self::error(format!("stage pre-install pikg failed: {error}")))?;
        PikgReader::open(&path, Some(&digest))
            .await
            .map_err(|error| {
                Self::error(format!("open staged pre-install pikg failed: {error}"))
            })?;
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|error| Self::error(format!("read staged pikg size failed: {error}")))?
            .len();
        self.register_staged_content(
            digest,
            path,
            size,
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
        expected_digest: &str,
        expected_size: u64,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await?;
        self.finalize_file_object(
            &named_store,
            source,
            expected_digest,
            expected_size,
            owner_user_id,
            owner_app_id,
            zone_did,
            purpose,
        )
        .await
    }

    async fn finalize_file_object(
        &self,
        named_store: &NamedDataMgr,
        source: &ObjId,
        expected_digest: &str,
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
        let object = named_store
            .get_object(source)
            .await
            .map_err(|error| Self::error(format!("read staging FileObject failed: {error}")))?;
        let file_object: FileObject = serde_json::from_str(&object)
            .map_err(|error| Self::error(format!("invalid staging FileObject: {error}")))?;
        if file_object.size != expected_size {
            return Err(Self::error("PIKG_SIZE_MISMATCH: FileObject size differs"));
        }
        let (reader, size) = named_store
            .open_reader(source, None)
            .await
            .map_err(|error| Self::error(format!("uploaded staging file unavailable: {error}")))?;
        if size != expected_size {
            return Err(Self::error(
                "PIKG_SIZE_MISMATCH: uploaded file size differs",
            ));
        }
        self.finalize_reader(
            reader,
            expected_digest,
            expected_size,
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
        Self::validate_expected_content(expected_digest, expected_size)?;
        if file_id.len() != 32 || !file_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(RPCErrors::ParseRequestError(
                "invalid local_file_id".to_string(),
            ));
        }
        let source = self.root.join("incoming").join(format!("{file_id}.pikg"));
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
        let root = tokio::fs::canonicalize(&self.root)
            .await
            .map_err(unavailable)?;
        if canonical.parent() != Some(root.join("incoming").as_path()) {
            return Err(Self::error(
                "local staging source escaped incoming directory",
            ));
        }
        let file = tokio::fs::File::open(&canonical)
            .await
            .map_err(unavailable)?;
        let result = self
            .finalize_reader(
                Box::pin(file),
                expected_digest,
                expected_size,
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

    fn validate_expected_content(digest: &str, size: u64) -> Result<(), RPCErrors> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
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

    async fn finalize_reader(
        &self,
        reader: ChunkReader,
        expected_digest: &str,
        expected_size: u64,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        use sha2::{Digest, Sha256};

        self.ensure_roots().await?;
        let tmp = self
            .root
            .join(format!(".finalize-{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = async {
            let mut reader = reader.take(expected_size + 1);
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .await
                .map_err(|error| Self::error(format!("create staging file failed: {error}")))?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            let mut buffer = vec![0u8; 256 * 1024];
            loop {
                let read = reader
                    .read(&mut buffer)
                    .await
                    .map_err(|error| Self::error(format!("read staging source failed: {error}")))?;
                if read == 0 {
                    break;
                }
                size += read as u64;
                if size > expected_size {
                    return Err(Self::error(
                        "PIKG_SIZE_MISMATCH: staging source is larger than declared",
                    ));
                }
                hasher.update(&buffer[..read]);
                file.write_all(&buffer[..read])
                    .await
                    .map_err(|error| Self::error(format!("write staging file failed: {error}")))?;
            }
            if size != expected_size || hex::encode(hasher.finalize()) != expected_digest {
                return Err(Self::error(
                    "PIKG_DIGEST_MISMATCH: staging source differs from client snapshot",
                ));
            }
            file.sync_all()
                .await
                .map_err(|error| Self::error(format!("sync staging file failed: {error}")))?;
            drop(file);
            let reader = PikgReader::open(&tmp, Some(expected_digest))
                .await
                .map_err(|error| Self::error(format!("validate staged pikg failed: {error}")))?;
            reader
                .verify_all_contents()
                .await
                .map_err(|error| Self::error(format!("verify staged pikg failed: {error}")))?;
            self.register_staged_content(
                expected_digest.to_string(),
                tmp.clone(),
                size,
                owner_user_id,
                owner_app_id,
                zone_did,
                purpose,
            )
            .await
        }
        .await;
        let _ = tokio::fs::remove_file(&tmp).await;
        result
    }

    pub async fn resolve(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        required_purpose: PikgStagingPurpose,
        lease: Option<&str>,
    ) -> Result<(PikgStagingMetadata, PathBuf), RPCErrors> {
        let handle = StagingHandle::parse(raw_handle).map_err(RPCErrors::ParseRequestError)?;
        let _guard = self.lock.lock().await;
        let mut metadata = Self::read_metadata_path(&self.metadata_path(&handle)).await?;
        if metadata.schema_version != APP_INSTALL_SCHEMA_VERSION
            || metadata.owner_user_id != owner_user_id
            || metadata.owner_app_id != owner_app_id
            || metadata.zone_did != *zone_did
        {
            return Err(RPCErrors::NoPermission(
                "staging handle is not owned by this principal and zone".to_string(),
            ));
        }
        if required_purpose == PikgStagingPurpose::Install
            && metadata.purpose != PikgStagingPurpose::Install
        {
            return Err(Self::error(
                "inspect-only staging handle cannot be consumed by an install task",
            ));
        }
        let now = buckyos_get_unix_timestamp();
        if metadata.expires_at <= now && metadata.leases.is_empty() {
            return Err(Self::error("staging handle expired"));
        }
        if let Some(lease) = lease {
            if !metadata.leases.iter().any(|existing| existing == lease) {
                metadata.leases.push(lease.to_string());
                self.write_metadata(&metadata).await?;
            }
        }
        let path = self.content_path(&metadata.pikg_digest);
        let canonical = path
            .canonicalize()
            .map_err(|error| Self::error(format!("staged pikg is unavailable: {error}")))?;
        let root = self
            .root
            .canonicalize()
            .map_err(|error| Self::error(format!("staging root is unavailable: {error}")))?;
        if !canonical.starts_with(root) {
            return Err(Self::error("staged pikg escaped controlled root"));
        }
        Ok((metadata, canonical))
    }

    pub async fn release(
        &self,
        raw_handle: &str,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        lease: Option<&str>,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        let handle = StagingHandle::parse(raw_handle).map_err(RPCErrors::ParseRequestError)?;
        let _guard = self.lock.lock().await;
        let mut metadata = Self::read_metadata_path(&self.metadata_path(&handle)).await?;
        if metadata.owner_user_id != owner_user_id
            || metadata.owner_app_id != owner_app_id
            || metadata.zone_did != *zone_did
        {
            return Err(RPCErrors::NoPermission(
                "staging handle is not owned by this principal and zone".to_string(),
            ));
        }
        if let Some(lease) = lease {
            metadata.leases.retain(|existing| existing != lease);
        } else if metadata.leases.is_empty() {
            metadata.expires_at = buckyos_get_unix_timestamp();
        } else {
            return Err(Self::error(
                "staging handle is referenced by an active task",
            ));
        }
        self.write_metadata(&metadata).await?;
        Ok(metadata)
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
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "buckyos-pikg-staging-test-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    async fn test_named_store(root: &Path) -> NamedDataMgr {
        use named_store::{NamedLocalStore, StoreLayout, StoreTarget};
        use std::sync::Arc;
        let store = NamedLocalStore::get_named_store_by_path(root.join("named_store"))
            .await
            .unwrap();
        let store_id = store.store_id().to_string();
        let manager = NamedDataMgr::new();
        manager.register_store(Arc::new(Mutex::new(store))).await;
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
    async fn large_pikg_local_fileobject_and_payload_roundtrip() {
        use crate::app_install_driver::store_pikg_payload;
        use ndn_lib::{FileObject, StoreMode, CHUNK_DEFAULT_SIZE};
        use ndn_toolkit::{cacl_file_object, CheckMode};
        use sha2::{Digest, Sha256};

        let root = temp_root();
        let (pikg, payload, content, digest, size) = large_pikg(&root).await;
        assert!(size > CHUNK_DEFAULT_SIZE);
        let manager = test_named_store(&root).await;
        let staging = PikgStagingStore::with_root(root.join("staging"));
        let zone = DID::new("bns", "zone");
        let file_id = uuid::Uuid::new_v4().simple().to_string();
        let incoming = staging.root.join("incoming");
        tokio::fs::create_dir_all(&incoming).await.unwrap();
        let local = incoming.join(format!("{file_id}.pikg"));
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
            .finalize_file_object(
                &manager,
                &object,
                &digest,
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
        let (metadata, path) = staging
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
        assert_eq!(
            hex::encode(Sha256::digest(tokio::fs::read(path).await.unwrap())),
            digest
        );

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
    async fn staging_rejects_bad_identity_and_truncated_content_without_leaving_files() {
        use sha2::{Digest, Sha256};
        let root = temp_root();
        let store = PikgStagingStore::with_root(root.clone());
        let zone = DID::new("bns", "zone");
        let bytes = b"PK\x03\x04payload";
        let digest = hex::encode(Sha256::digest(bytes));
        assert!(store
            .finalize_local_file(
                "../escape",
                &digest,
                bytes.len() as u64,
                "alice",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Inspect
            )
            .await
            .is_err());
        for (data, expected_size, expected_digest) in [
            (bytes.to_vec(), bytes.len() as u64, "a".repeat(64)),
            (bytes[..4].to_vec(), bytes.len() as u64, digest.clone()),
            (bytes.to_vec(), 4, digest.clone()),
            (bytes.to_vec(), bytes.len() as u64, digest.clone()),
        ] {
            assert!(store
                .finalize_reader(
                    Box::pin(std::io::Cursor::new(data)),
                    &expected_digest,
                    expected_size,
                    "alice",
                    "buckyos-tool",
                    &zone,
                    PikgStagingPurpose::Inspect
                )
                .await
                .is_err());
            assert!(store.list_metadata().await.unwrap().is_empty());
            let mut entries = tokio::fs::read_dir(&root).await.unwrap();
            while let Some(entry) = entries.next_entry().await.unwrap() {
                assert_eq!(entry.file_name(), "metadata");
            }
        }
        let manager = test_named_store(&root).await;
        let chunk = ndn_lib::ChunkId::new(&format!("sha256:{digest}"))
            .unwrap()
            .to_obj_id();
        assert!(store
            .finalize_file_object(
                &manager,
                &chunk,
                &digest,
                bytes.len() as u64,
                "alice",
                "buckyos-tool",
                &zone,
                PikgStagingPurpose::Inspect
            )
            .await
            .is_err());
        drop(manager);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn restaging_expired_content_commits_the_file_with_its_new_handle() {
        let root = temp_root();
        let store = PikgStagingStore::with_root(root.clone());
        let old = seed(&store, true).await;
        let incoming = root.join(".finalize-test.tmp");
        tokio::fs::write(&incoming, b"pikg").await.unwrap();
        let metadata = store
            .register_staged_content(
                old.pikg_digest.clone(),
                incoming.clone(),
                4,
                "alice",
                "buckyos-tool",
                &old.zone_did,
                PikgStagingPurpose::Install,
            )
            .await
            .unwrap();
        assert_ne!(metadata.handle, old.handle);
        let _ = tokio::fs::remove_file(&incoming).await;
        assert_eq!(
            tokio::fs::read(store.content_path(&old.pikg_digest))
                .await
                .unwrap(),
            b"pikg"
        );
        store
            .resolve(
                metadata.handle.as_str(),
                "alice",
                "buckyos-tool",
                &old.zone_did,
                PikgStagingPurpose::Install,
                Some("task"),
            )
            .await
            .unwrap();
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_staging_rejects_symlinks() {
        let root = temp_root();
        let store = PikgStagingStore::with_root(root.join("staging"));
        tokio::fs::create_dir_all(store.root.join("incoming"))
            .await
            .unwrap();
        let source = root.join("source.pikg");
        tokio::fs::write(&source, b"PK\x03\x04").await.unwrap();
        let file_id = "1".repeat(32);
        std::os::unix::fs::symlink(
            source,
            store.root.join("incoming").join(format!("{file_id}.pikg")),
        )
        .unwrap();
        assert!(store
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

    async fn seed(store: &PikgStagingStore, expired: bool) -> PikgStagingMetadata {
        store.ensure_roots().await.unwrap();
        let handle = StagingHandle::new_opaque("0123456789abcdef0123456789abcdef").unwrap();
        let now = buckyos_get_unix_timestamp();
        let metadata = PikgStagingMetadata {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            handle,
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
        };
        tokio::fs::write(store.content_path(&metadata.pikg_digest), b"pikg")
            .await
            .unwrap();
        store.write_metadata(&metadata).await.unwrap();
        metadata
    }

    #[tokio::test]
    async fn staging_is_principal_and_zone_scoped() {
        let root = temp_root();
        let store = PikgStagingStore::with_root(root.clone());
        let metadata = seed(&store, false).await;
        let error = store
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
        let store = PikgStagingStore::with_root(root.clone());
        let metadata = seed(&store, false).await;
        store
            .resolve(
                metadata.handle.as_str(),
                "alice",
                "buckyos-tool",
                &metadata.zone_did,
                PikgStagingPurpose::Install,
                Some("task-1"),
            )
            .await
            .unwrap();
        let mut leased = store
            .status(
                metadata.handle.as_str(),
                "alice",
                "buckyos-tool",
                &metadata.zone_did,
            )
            .await
            .unwrap();
        leased.expires_at = buckyos_get_unix_timestamp().saturating_sub(1);
        store.write_metadata(&leased).await.unwrap();
        store.gc().await.unwrap();
        assert!(store.content_path(&metadata.pikg_digest).exists());

        store
            .release(
                metadata.handle.as_str(),
                "alice",
                "buckyos-tool",
                &metadata.zone_did,
                Some("task-1"),
            )
            .await
            .unwrap();
        store.gc().await.unwrap();
        assert!(!store.content_path(&metadata.pikg_digest).exists());
        assert!(!store.metadata_path(&metadata.handle).exists());
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
