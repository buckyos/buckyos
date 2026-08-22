use crate::app_install_driver::pikg_staging_root;
use crate::pikg::PikgReader;
use buckyos_api::{
    get_buckyos_api_runtime, PikgStagingMetadata, PikgStagingPurpose, StagingHandle,
    APP_INSTALL_SCHEMA_VERSION,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use kRPC::RPCErrors;
use name_lib::DID;
use ndn_lib::{ChunkId, ObjId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
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

    async fn gc_locked(&self, now: u64) -> Result<(), RPCErrors> {
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
            if !still_referenced {
                let _ = tokio::fs::remove_file(self.content_path(&metadata.pikg_digest)).await;
            }
        }
        Ok(())
    }

    pub async fn gc(&self) -> Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;
        self.ensure_roots().await?;
        self.gc_locked(buckyos_get_unix_timestamp()).await
    }

    pub async fn finalize_named_object(
        &self,
        source: &ObjId,
        owner_user_id: &str,
        owner_app_id: &str,
        zone_did: &DID,
        purpose: PikgStagingPurpose,
    ) -> Result<PikgStagingMetadata, RPCErrors> {
        if !source.is_chunk() {
            return Err(RPCErrors::ParseRequestError(
                "staging source must be an uploaded chunk object".to_string(),
            ));
        }
        self.ensure_roots().await?;
        let runtime = get_buckyos_api_runtime()?;
        let named_store = runtime.get_named_store().await?;
        let chunk_id = ChunkId::from_obj_id(source);
        let (mut reader, _) = named_store
            .open_chunk_reader(&chunk_id, 0)
            .await
            .map_err(|error| Self::error(format!("uploaded staging chunk unavailable: {error}")))?;
        let tmp = self
            .root
            .join(format!(".finalize-{}.tmp", uuid::Uuid::new_v4().simple()));
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|error| Self::error(format!("create staging temp file failed: {error}")))?;
        tokio::io::copy(&mut reader, &mut file)
            .await
            .map_err(|error| Self::error(format!("copy staging upload failed: {error}")))?;
        file.flush().await.ok();
        drop(file);
        let staged = PikgReader::stage_pikg_file(&tmp, &self.root).await;
        let _ = tokio::fs::remove_file(&tmp).await;
        let (digest, path) = staged
            .map_err(|error| Self::error(format!("finalize pikg validation failed: {error}")))?;
        let size = tokio::fs::metadata(&path)
            .await
            .map_err(|error| Self::error(format!("read staged pikg size failed: {error}")))?
            .len();

        let _guard = self.lock.lock().await;
        let now = buckyos_get_unix_timestamp();
        self.gc_locked(now).await?;
        let all = self.list_metadata().await?;
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
