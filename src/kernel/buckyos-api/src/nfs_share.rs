use crate::*;
use crate::bucket_client::{BucketDecl, BucketId, BucketKind, BucketScope, DataPolicy};
use serde_json::json;
use std::collections::HashMap;

/// Zone bucket retaining share snapshots: `zone/nfs-server/_/shares`, key
/// `<share_id>`; stopping or expiring a share deletes its key.
pub const NFS_SHARE_BUCKET: &str = "shares";
/// Zone bucket of Preview Pipeline results: `zone/nfs-server/_/preview-cache`,
/// key `<work_key>`. Pinned (`app_data`): nfs-server expires results unread for
/// 30 days and keeps the total under `preview_cache_mib` itself.
pub const NFS_PREVIEW_CACHE_BUCKET: &str = "preview-cache";
/// Zone bucket of what this Zone publishes for `/ndn/` (bucket design §4.6.5): key
/// `<provider>/<entry>/<root_obj_id>`, bound by `bucket.provide_public`, unbound
/// when the entry is withdrawn or its audience narrows.
pub const NFS_PUBLIC_OBJECTS_BUCKET: &str = "public-objects";
/// Zone bucket of content anonymously readable elsewhere, re-served on `/ndn/`:
/// key `<appid>/<root_obj_id>`, bound by `bucket.cache_public` after a verified
/// download. Not pinned; may be evicted at any time.
pub const NFS_PUBLIC_CACHE_BUCKET: &str = "public-cache";
/// Soft limit of the whole public cache; each app has its own smaller limit.
pub const NFS_PUBLIC_CACHE_QUOTA_MIB: u64 = 64 * 1024;
/// App pool limit of the nfs-server buckets (D17); one share is at most 64 GiB.
pub const NFS_SERVER_STORAGE_QUOTA_MIB: u64 = 1024 * 1024;

/// What the scheduler declares in `spec_config.buckets` of nfs-server (and what
/// a standalone nfs-server declares for itself).
pub fn nfs_server_bucket_decls() -> HashMap<String, BucketDecl> {
    let zone_data = || BucketDecl {
        kind: BucketKind::Data,
        data_policy: DataPolicy::AppData,
        scope: BucketScope::Zone,
        versioning: None,
        quota_mib: None,
        max_objects: None,
        default_ttl_days: None,
        lifecycle: Vec::new(),
    };
    let public_cache = BucketDecl { data_policy: DataPolicy::Cache, quota_mib: Some(NFS_PUBLIC_CACHE_QUOTA_MIB), ..zone_data() };
    HashMap::from([
        (NFS_SHARE_BUCKET.to_string(), zone_data()),
        (NFS_PREVIEW_CACHE_BUCKET.to_string(), zone_data()),
        (NFS_PUBLIC_OBJECTS_BUCKET.to_string(), zone_data()),
        (NFS_PUBLIC_CACHE_BUCKET.to_string(), public_cache),
    ])
}

pub fn nfs_share_bucket() -> BucketId {
    BucketId::zone(NFS_SERVER_SERVICE_NAME, NFS_SHARE_BUCKET).expect("valid nfs-server bucket id")
}

pub fn nfs_public_objects_bucket() -> BucketId {
    BucketId::zone(NFS_SERVER_SERVICE_NAME, NFS_PUBLIC_OBJECTS_BUCKET).expect("valid nfs-server bucket id")
}

pub fn nfs_public_cache_bucket() -> BucketId {
    BucketId::zone(NFS_SERVER_SERVICE_NAME, NFS_PUBLIC_CACHE_BUCKET).expect("valid nfs-server bucket id")
}

pub fn nfs_preview_cache_bucket() -> BucketId {
    BucketId::zone(NFS_SERVER_SERVICE_NAME, NFS_PREVIEW_CACHE_BUCKET).expect("valid nfs-server bucket id")
}

/// Background snapshot of a share source larger than the in-request threshold
/// (nfs-server share module). The business state lives in nfs-server's
/// `share_operations`; this task mirrors it for task UIs.
pub const NFS_SHARE_SNAPSHOT_SCHEMA_ID: &str = "nfs.share_snapshot/v1";

pub fn nfs_share_snapshot_task_schema() -> TaskSchemaDefinition {
    TaskSchemaDefinition {
        schema_id: NFS_SHARE_SNAPSHOT_SCHEMA_ID.into(),
        schema_version: 1,
        input_schema: json!({
            "type": "object", "additionalProperties": false,
            "required": ["operation_id", "intent", "source_path", "total_size"],
            "properties": {
                "operation_id": {"type": "string"},
                "intent": {"enum": ["create_link", "app_handoff"]},
                "source_path": {"type": "string"},
                "total_size": {"type": "integer", "minimum": 0},
                "item_count": {"type": "integer", "minimum": 0}
            }
        }),
        output_schema: json!({"type": "object", "required": ["obj_id"], "properties": {"obj_id": {"type": "string"}, "share_id": {"type": "string"}, "total_size": {"type": "integer"}, "item_count": {"type": "integer"}}}),
        presentation_schema: None,
        allowed_executor_kinds: vec![TaskExecutorKind::App],
        user_creatable: false,
        default_storage_domain: StorageDomain::System,
        publisher_app_id: NFS_SERVER_SERVICE_NAME.into(),
        enabled: true,
        created_at: 0,
    }
}
