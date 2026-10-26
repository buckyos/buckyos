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
        self.call_as("bucket.put", params).await
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
        self.call_as("bucket.delete", json!({"bucket_id": bucket_id, "key": key, "version": version})).await
    }

    pub async fn list_objects(&self, bucket_id: &BucketId, req: &ListRequest) -> Result<ListResult, RPCErrors> {
        let mut params = serde_json::to_value(req).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        params["bucket_id"] = json!(bucket_id);
        self.call_as("bucket.list_objects", params).await
    }

    pub async fn copy(&self, req: &CopyRequest) -> Result<PutResult, RPCErrors> {
        self.call_as("bucket.copy", serde_json::to_value(req).map_err(|e| RPCErrors::ReasonError(e.to_string()))?).await
    }

    pub async fn batch(&self, bucket_id: &BucketId, ops: &[BatchOp]) -> Result<Vec<BatchResult>, RPCErrors> {
        self.call_as("bucket.batch", json!({"bucket_id": bucket_id, "ops": ops})).await
    }

    /// Stores a local file in the named store this runtime may use and binds it
    /// under `key`; success means the bucket retains it (design §12.4).
    pub async fn put_file(
        &self,
        runtime: &crate::BuckyOSRuntime,
        bucket_id: &BucketId,
        key: &str,
        path: &std::path::Path,
        content_type: Option<String>,
    ) -> Result<PutResult, RPCErrors> {
        let store = runtime.get_named_store().await?;
        let mut template = ndn_lib::FileObject::default();
        if let Some(name) = std::path::Path::new(key).file_name().and_then(|n| n.to_str()) {
            template.content_obj.name = name.to_string();
        }
        let (_, obj_id, _) = ndn_toolkit::cacl_file_object(
            Some(&store),
            path,
            &template,
            true,
            &ndn_toolkit::CheckMode::ByFullHash,
            ndn_lib::StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .map_err(|e| RPCErrors::ReasonError(format!("store {}: {e}", path.display())))?;
        let req = PutRequest { key: key.to_string(), obj_id: obj_id.to_string(), content_type, ..Default::default() };
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
    }
}
