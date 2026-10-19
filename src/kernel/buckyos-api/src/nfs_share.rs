use crate::*;
use serde_json::json;

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
