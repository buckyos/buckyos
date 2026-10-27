//! Application object buckets (doc/arch/应用对象桶 Bucket 设计.md). The lib
//! (`bucket_store`, cyfs-ndn) owns the model; nfs-server hosts it: kRPC
//! `bucket.*` at `/kapi/nfs-server`, the data plane at `/bucket/v1/...`.

use crate::{RdbBackend, RdbInstanceConfig, RdbPartition};
use ::kRPC::{kRPC, RPCErrors};
pub use bucket_store::types::*;
pub use bucket_store::{
    AccessCheck, AccessToken, Action, Grant, GrantRequest, Grantee, GranteeKind, IssuedToken, PoolQuota, QuotaUpdate,
    QuotaView, Reference, RestoreReport, SnapshotInfo, TokenRequest, UsageReport,
};
use ndn_lib::{NamedObject, ObjId};
use ndn_toolkit::NamedObjectStore;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const BUCKET_INDEX_RDB_INSTANCE_ID: &str = "bucket-index";
pub const BUCKET_DATA_PATH: &str = "/bucket/v1/";
/// kevent id prefix of object events: `/bucket/<scope>/<appid>/<owner>/<name>`.
pub const BUCKET_EVENT_PREFIX: &str = "/bucket/";

/// `bucket-index` lives in nfs-server's `user_data` partition; bucket_store
/// creates its tables, the instance is registered so the system can list and back it up.
pub fn bucket_index_rdb_instance_config() -> RdbInstanceConfig {
    RdbInstanceConfig {
        backend: RdbBackend::Sqlite,
        version: 1,
        schema: HashMap::new(),
        connection: String::new(),
        partitions: vec![RdbPartition::UserData],
    }
}

/// Install-time checks of `install_config.buckets` (§4.3, D17).
pub fn validate_app_buckets(buckets: &HashMap<String, BucketDecl>, storage_quota_mib: Option<u64>) -> Vec<String> {
    let mut issues = Vec::new();
    for (name, decl) in buckets {
        if let Err(e) = decl.validate(name) {
            issues.push(format!("bucket `{name}`: {e}"));
        }
    }
    if storage_quota_mib.is_none() && buckets.values().any(|d| d.needs_app_pool()) {
        issues.push("storage_quota_mib is required when app_data or temp buckets are declared".to_string());
    }
    issues
}

pub fn bucket_event_id(bucket_id: &BucketId) -> String {
    format!("{BUCKET_EVENT_PREFIX}{bucket_id}")
}

/// What `bucket.release_app` did with one bucket of an uninstalled app instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseAction {
    Deleted,
    Orphaned,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReleasedBucket {
    pub bucket_id: BucketId,
    pub data_policy: DataPolicy,
    pub unique_bytes: u64,
    pub action: ReleaseAction,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppRelease {
    pub appid: String,
    pub owner: String,
    pub buckets: Vec<ReleasedBucket>,
}

/// kRPC client of the bucket service (nfs-server).
pub struct BucketClient {
    client: kRPC,
}

impl BucketClient {
    pub fn new(client: kRPC) -> Self {
        Self { client }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RPCErrors> {
        self.client.call(method, params).await
    }

    async fn call_as<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RPCErrors> {
        let v = self.client.call(method, params).await?;
        serde_json::from_value(v).map_err(|e| RPCErrors::ParserResponseError(format!("{method}: {e}")))
    }

    pub async fn list(&self, scope: Option<BucketScope>, appid: Option<&str>) -> Result<Vec<BucketInfo>, RPCErrors> {
        self.call_as("bucket.list", json!({"scope": scope, "appid": appid})).await
    }

    pub async fn stat(&self, bucket_id: &BucketId) -> Result<BucketInfo, RPCErrors> {
        self.call_as("bucket.stat", json!({"bucket_id": bucket_id})).await
    }

    pub async fn put(&self, bucket_id: &BucketId, req: &PutRequest) -> Result<PutResult, RPCErrors> {
        let mut params = serde_json::to_value(req).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        params["bucket_id"] = json!(bucket_id);
        let result: PutResult = self.call_as("bucket.put", params).await?;
        log::info!("bucket bind {bucket_id} key={} obj_id={} version={}", req.key, req.obj_id, result.version);
        Ok(result)
    }

    pub async fn get(&self, bucket_id: &BucketId, key: &str, version: Option<u64>) -> Result<Binding, RPCErrors> {
        self.call_as("bucket.get", json!({"bucket_id": bucket_id, "key": key, "version": version})).await
    }

    /// The binding and where its content is (`local`, `evicted`, `remote`).
    pub async fn head(&self, bucket_id: &BucketId, key: &str, version: Option<u64>) -> Result<(Binding, Availability), RPCErrors> {
        let v = self.call("bucket.head", json!({"bucket_id": bucket_id, "key": key, "version": version})).await?;
        let availability = serde_json::from_value(v["availability"].clone()).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
        let binding = serde_json::from_value(v["binding"].clone()).map_err(|e| RPCErrors::ParserResponseError(e.to_string()))?;
        Ok((binding, availability))
    }

    pub async fn delete(&self, bucket_id: &BucketId, key: &str, version: Option<u64>) -> Result<DeleteResult, RPCErrors> {
        let result: DeleteResult = self.call_as("bucket.delete", json!({"bucket_id": bucket_id, "key": key, "version": version})).await?;
        log::info!("bucket unbind {bucket_id} key={key} deleted={} version={:?}", result.deleted, result.version);
        Ok(result)
    }

    pub async fn list_objects(&self, bucket_id: &BucketId, req: &ListRequest) -> Result<ListResult, RPCErrors> {
        let mut params = serde_json::to_value(req).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        params["bucket_id"] = json!(bucket_id);
        self.call_as("bucket.list_objects", params).await
    }

    pub async fn copy(&self, req: &CopyRequest) -> Result<PutResult, RPCErrors> {
        let result: PutResult =
            self.call_as("bucket.copy", serde_json::to_value(req).map_err(|e| RPCErrors::ReasonError(e.to_string()))?).await?;
        log::info!(
            "bucket bind {} key={} copied from {}/{} etag={} version={}",
            req.dst_bucket_id, req.dst_key, req.src_bucket_id, req.src_key, result.etag, result.version
        );
        Ok(result)
    }

    /// Binds what `src` binds under `dst` (a separate commitment, §6.3); collected /
    /// ambient destinations need `origin`.
    pub async fn copy_to(
        &self,
        src: (&BucketId, &str),
        dst: (&BucketId, &str),
        origin: Option<Origin>,
        conditions: Conditions,
    ) -> Result<PutResult, RPCErrors> {
        let req = CopyRequest {
            src_bucket_id: src.0.to_string(),
            src_key: src.1.to_string(),
            dst_bucket_id: dst.0.to_string(),
            dst_key: dst.1.to_string(),
            origin,
            conditions,
            ..Default::default()
        };
        self.copy(&req).await
    }

    pub async fn batch(&self, bucket_id: &BucketId, ops: &[BatchOp]) -> Result<Vec<BatchResult>, RPCErrors> {
        self.call_as("bucket.batch", json!({"bucket_id": bucket_id, "ops": ops})).await
    }

    /// Stores a local file in `store` (`runtime.get_named_store()` in a zone) and
    /// binds it under `key`; success means the bucket retains it (design §12.4).
    pub async fn put_file(
        &self,
        store: &dyn NamedObjectStore,
        bucket_id: &BucketId,
        key: &str,
        path: &std::path::Path,
        content_type: Option<String>,
        conditions: Conditions,
    ) -> Result<(ObjId, PutResult), RPCErrors> {
        let mut template = ndn_lib::FileObject::default();
        template.content_obj.name = key_file_name(key);
        let (mut file, obj_id, _) = ndn_toolkit::cacl_file_object(
            Some(store),
            path,
            &template,
            true,
            &ndn_toolkit::CheckMode::ByFullHash,
            ndn_lib::StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .map_err(|e| RPCErrors::ReasonError(format!("store {}: {e}", path.display())))?;
        let obj_id = if file.content_obj.name == template.content_obj.name || template.content_obj.name.is_empty() {
            obj_id
        } else {
            file.content_obj.name = template.content_obj.name;
            let (obj_id, obj_str) = file.gen_obj_id();
            store.put_object(&obj_id, &obj_str).await.map_err(|e| RPCErrors::ReasonError(format!("store {}: {e}", path.display())))?;
            obj_id
        };
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), content_type, conditions, ..Default::default() };
        let result = self.put(bucket_id, &req).await?;
        Ok((obj_id, result))
    }

    /// Streams bytes into the named store (one chunk in memory at a time, no
    /// temporary file) as a FileObject named `name` and binds it under `key`.
    pub async fn put_reader<R: tokio::io::AsyncRead + Unpin + Send>(
        &self,
        store: &dyn NamedObjectStore,
        bucket_id: &BucketId,
        req: PutRequest,
        name: &str,
        reader: R,
    ) -> Result<(ObjId, PutResult), RPCErrors> {
        let obj_id = store_reader(store, name, reader).await.map_err(|e| RPCErrors::ReasonError(format!("store {}: {e}", req.key)))?;
        let req = PutRequest { obj_id: obj_id.to_string(), ..req };
        let result = self.put(bucket_id, &req).await?;
        Ok((obj_id, result))
    }

    pub async fn put_bytes(
        &self,
        store: &dyn NamedObjectStore,
        bucket_id: &BucketId,
        req: PutRequest,
        bytes: &[u8],
    ) -> Result<(ObjId, PutResult), RPCErrors> {
        let name = key_file_name(&req.key);
        self.put_reader(store, bucket_id, req, &name, bytes).await
    }

    /// Stores a JSON named object (`put_object`) and binds it under `key` in an
    /// `object` bucket, in one call.
    pub async fn put_json_object(
        &self,
        store: &dyn NamedObjectStore,
        bucket_id: &BucketId,
        key: &str,
        obj_type: &str,
        value: &Value,
        conditions: Conditions,
    ) -> Result<(ObjId, PutResult), RPCErrors> {
        let (obj_id, obj_str) = ndn_lib::try_build_named_object_by_json(obj_type, value)
            .map_err(|e| RPCErrors::ReasonError(format!("{obj_type} object: {e}")))?;
        let result = self.put_object_str(store, bucket_id, key, &obj_id, &obj_str, conditions).await?;
        Ok((obj_id, result))
    }

    /// Stores an already encoded named object and binds it under `key`.
    pub async fn put_object_str(
        &self,
        store: &dyn NamedObjectStore,
        bucket_id: &BucketId,
        key: &str,
        obj_id: &ObjId,
        obj_str: &str,
        conditions: Conditions,
    ) -> Result<PutResult, RPCErrors> {
        store
            .put_object(obj_id, obj_str)
            .await
            .map_err(|e| RPCErrors::ReasonError(format!("store {obj_id}: {e}")))?;
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), conditions, ..Default::default() };
        self.put(bucket_id, &req).await
    }

    pub async fn snapshot(&self, bucket_id: &BucketId, label: Option<&str>) -> Result<SnapshotInfo, RPCErrors> {
        self.call_as("bucket.snapshot", json!({"bucket_id": bucket_id, "label": label})).await
    }

    pub async fn create_access_token(&self, bucket_id: &BucketId, req: &TokenRequest) -> Result<IssuedToken, RPCErrors> {
        let mut params = serde_json::to_value(req).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        params["bucket_id"] = json!(bucket_id);
        self.call_as("bucket.create_access_token", params).await
    }

    pub async fn check_access(&self, bucket_id: &BucketId, action: Action, key: Option<&str>, prefix: Option<&str>) -> Result<AccessCheck, RPCErrors> {
        self.call_as("bucket.check_access", json!({"bucket_id": bucket_id, "action": action, "key": key, "prefix": prefix})).await
    }

    /// Disposes of the buckets of an uninstalled app instance by data policy (§4.5.1):
    /// documents / collected (and app_data when `keep_app_data`) become orphans, the rest are deleted.
    pub async fn release_app(&self, appid: &str, owner: &str, keep_app_data: bool) -> Result<AppRelease, RPCErrors> {
        let release: AppRelease =
            self.call_as("bucket.release_app", json!({"appid": appid, "owner": owner, "keep_app_data": keep_app_data})).await?;
        for b in &release.buckets {
            log::info!("bucket release {} ({}) of {appid}@{owner}: {:?}", b.bucket_id, b.data_policy, b.action);
        }
        Ok(release)
    }
}

/// The wire error code of a failed `bucket.*` call (`BucketError::code`).
pub fn bucket_error_code(e: &RPCErrors) -> Option<&str> {
    e.detail().map(|d| d.code.as_str())
}

pub fn is_no_such_key(e: &RPCErrors) -> bool {
    bucket_error_code(e) == Some("no_such_key")
}

pub fn is_precondition_failed(e: &RPCErrors) -> bool {
    bucket_error_code(e) == Some("precondition_failed")
}

/// `obj://bucket/<bucket_id>/<key>`: how Agents and CLIs address a binding (§4.2).
pub fn bucket_obj_url(bucket_id: &BucketId, key: &str) -> String {
    format!("obj://bucket/{bucket_id}/{key}")
}

/// Splits `obj://bucket/<bucket_id>/<key>`.
pub fn parse_bucket_obj_url(url: &str) -> Option<(BucketId, String)> {
    let rest = url.strip_prefix("obj://bucket/")?;
    let (id, key) = BucketId::split_path(rest).ok()?;
    (!key.is_empty()).then(|| (id, key.to_string()))
}

fn key_file_name(key: &str) -> String {
    key.rsplit('/').next().unwrap_or(key).to_string()
}

async fn store_reader<R: tokio::io::AsyncRead + Unpin + Send>(
    store: &dyn NamedObjectStore,
    name: &str,
    mut reader: R,
) -> ndn_lib::NdnResult<ObjId> {
    use tokio::io::AsyncReadExt;
    let chunk_size = ndn_lib::CHUNK_DEFAULT_SIZE as usize;
    let mut chunks = Vec::new();
    let mut total = 0u64;
    loop {
        let mut buf = Vec::with_capacity(chunk_size.min(1 << 20));
        (&mut reader).take(chunk_size as u64).read_to_end(&mut buf).await?;
        if buf.is_empty() && !chunks.is_empty() {
            break;
        }
        let hasher = ndn_lib::ChunkHasher::new(None)?;
        let method = hasher.hash_method;
        let hash = hasher.calc_from_bytes(&buf);
        let chunk_id = ndn_lib::ChunkId::from_mix_hash_result_by_hash_method(buf.len() as u64, &hash, method)?;
        store.put_chunk(&chunk_id, &buf).await?;
        total += buf.len() as u64;
        chunks.push(chunk_id);
        if buf.len() < chunk_size {
            break;
        }
    }
    let mut file = ndn_lib::FileObject::default();
    file.size = total;
    file.content_obj.name = name.to_string();
    file.content = if chunks.len() == 1 {
        chunks[0].to_string()
    } else {
        let (list_id, list_str) = ndn_lib::ChunkList::from_chunk_list(chunks)?.gen_obj_id();
        store.put_object(&list_id, &list_str).await?;
        list_id.to_string()
    };
    let (obj_id, obj_str) = file.gen_obj_id();
    store.put_object(&obj_id, &obj_str).await?;
    Ok(obj_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_bucket_declarations() {
        let buckets: HashMap<String, BucketDecl> = serde_json::from_value(json!({
            "library": {"kind": "data", "data_policy": "documents", "scope": "user", "versioning": true},
            "index": {"kind": "data", "data_policy": "app_data", "quota_mib": 2048},
            "scratch": {"kind": "data", "data_policy": "temp", "default_ttl_days": 7}
        }))
        .unwrap();
        assert_eq!(validate_app_buckets(&buckets, Some(4096)), Vec::<String>::new());
        assert_eq!(validate_app_buckets(&buckets, None).len(), 1);
        let bad: HashMap<String, BucketDecl> =
            serde_json::from_value(json!({"d": {"kind": "data", "data_policy": "documents", "quota_mib": 1}})).unwrap();
        assert_eq!(validate_app_buckets(&bad, None).len(), 1);
        let id = BucketId::user("photos", "alice", "library").unwrap();
        assert_eq!(bucket_event_id(&id), "/bucket/user/photos/alice/library");
        let url = bucket_obj_url(&id, "2026/a.png");
        assert_eq!(url, "obj://bucket/user/photos/alice/library/2026/a.png");
        assert_eq!(parse_bucket_obj_url(&url), Some((id, "2026/a.png".to_string())));
        assert_eq!(parse_bucket_obj_url("obj://bucket/user/photos/alice/library"), None);
        assert_eq!(key_file_name("a/b/c.txt"), "c.txt");
    }

    #[tokio::test]
    async fn reader_round_trips_through_the_named_store() {
        use named_store::{NamedDataMgr, NamedLocalStore, StoreLayout, StoreTarget};
        use tokio::io::AsyncReadExt;
        let dir = std::env::temp_dir().join(format!("bucket-client-{}", uuid::Uuid::new_v4()));
        let local = NamedLocalStore::get_named_store_by_path(dir.clone()).await.unwrap();
        let store_id = local.store_id().to_string();
        let mgr = NamedDataMgr::new();
        mgr.register_store(std::sync::Arc::new(tokio::sync::Mutex::new(local))).await;
        mgr.add_layout(StoreLayout::new(
            1,
            vec![StoreTarget { store_id, device_did: String::new(), capacity: None, used: None, readonly: false, enabled: true, weight: 1 }],
            0,
            0,
        ))
        .await;
        let store = crate::RuntimeNamedStore::local(mgr);
        let bytes = b"hello bucket".repeat(1000);
        let obj_id = store_reader(&store, "hello.txt", &bytes[..]).await.unwrap();
        let file: ndn_lib::FileObject = ndn_lib::load_named_obj(&store.get_object(&obj_id).await.unwrap()).unwrap();
        assert_eq!((file.size, file.content_obj.name.as_str()), (bytes.len() as u64, "hello.txt"));
        let (mut reader, _) = store.open_reader(&obj_id, None).await.unwrap();
        let mut back = Vec::new();
        reader.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, bytes);
        let empty = store_reader(&store, "empty", &b""[..]).await.unwrap();
        assert_ne!(empty, obj_id);
        let _ = std::fs::remove_dir_all(dir);
    }
}
