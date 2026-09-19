use std::path::{Path, PathBuf};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use buckyos_api::{get_buckyos_api_runtime, ResourceRef};
use ndn_lib::FileObject;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use crate::{
    resolve_path_under_root, AgentTool, AgentToolError, AgentToolResult, AgentToolStatus,
    CallingConventions, SessionRuntimeContext, ToolSpec, AGENT_TOOL_PROTOCOL_VERSION,
};

pub const TOOL_MATERIALIZE_RESOURCE: &str = "materialize_resource";

pub struct MaterializeResourceTool {
    workspace_root: PathBuf,
}

impl MaterializeResourceTool {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

#[async_trait]
impl AgentTool for MaterializeResourceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_MATERIALIZE_RESOURCE.to_string(),
            description: "Materialize the specified ResourceRef in the current session workspace and return its local path. Use this before a local command processes a named_object/cyfile, URL, or base64 resource. Preserve resource identity; do not substitute another workspace file."
                .to_string(),
            args_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["resource", "path"],
                "properties": {
                    "resource": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind"],
                        "properties": {
                            "kind": {
                                "type": "string",
                                "enum": ["named_object", "url", "base64"]
                            },
                            "obj_id": {
                                "type": "string",
                                "description": "Required when kind is named_object. Copy the complete typed ID, for example cyfile:abc…."
                            },
                            "url": {
                                "type": "string",
                                "description": "Required when kind is url."
                            },
                            "mime_hint": {"type": "string"},
                            "mime": {
                                "type": "string",
                                "description": "Required when kind is base64."
                            },
                            "data_base64": {
                                "type": "string",
                                "description": "Required when kind is base64."
                            }
                        },
                        "description": "The ResourceRef to materialize. For a cyfile resource use {kind:\"named_object\",obj_id:\"cyfile:…\"}."
                    },
                    "path": {
                        "type": "string",
                        "description": "Destination path inside the current session workspace. Preserve an appropriate file extension when known."
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["path", "bytes", "source_kind"],
                "properties": {
                    "path": {"type": "string"},
                    "bytes": {"type": "integer"},
                    "mime": {"type": ["string", "null"]},
                    "source_kind": {"type": "string"}
                }
            }),
            usage: Some("materialize_resource resource={\"kind\":\"named_object\",\"obj_id\":\"cyfile:…\"} path=\"inputs/resource.bin\"".to_string()),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let map = args.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("materialize_resource args must be object".to_string())
        })?;
        let resource = map
            .get("resource")
            .cloned()
            .ok_or_else(|| AgentToolError::InvalidArgs("missing `resource`".to_string()))?;
        let resource: ResourceRef = serde_json::from_value(resource).map_err(|err| {
            AgentToolError::InvalidArgs(format!("invalid `resource` ResourceRef: {err}"))
        })?;
        let raw_path = map
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("missing non-empty `path`".to_string()))?;
        let path = resolve_path_under_root(&self.workspace_root, raw_path)?;
        ensure_safe_parent(&self.workspace_root, &path).await?;
        if tokio::fs::try_exists(&path)
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("check destination failed: {err}")))?
        {
            return Err(AgentToolError::InvalidArgs(format!(
                "destination already exists: {}",
                path.display()
            )));
        }

        let (bytes, mime, source_kind) = write_resource(&resource, &path).await?;
        let path_text = path.to_string_lossy().to_string();
        Ok(AgentToolResult {
            agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
            tool: Some(TOOL_MATERIALIZE_RESOURCE.to_string()),
            cmd_name: Some(TOOL_MATERIALIZE_RESOURCE.to_string()),
            status: AgentToolStatus::Success,
            task_id: None,
            pending_reason: None,
            check_after: None,
            estimated_wait: None,
            title: format!("materialized resource to {path_text}"),
            summary: format!("saved {bytes} bytes to {path_text}"),
            details: json!({
                "path": path_text,
                "bytes": bytes,
                "mime": mime,
                "source_kind": source_kind,
            }),
            cmd_args: None,
            return_code: Some(0),
            partial_output: None,
            output: Some(path_text),
        })
    }
}

async fn ensure_safe_parent(root: &Path, path: &Path) -> Result<(), AgentToolError> {
    let parent = path.parent().ok_or_else(|| {
        AgentToolError::InvalidArgs(format!("destination has no parent: {}", path.display()))
    })?;
    let canonical_root = tokio::fs::canonicalize(root).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("canonicalize workspace root failed: {err}"))
    })?;
    let mut existing_ancestor = parent;
    while !tokio::fs::try_exists(existing_ancestor)
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("check destination path failed: {err}"))
        })?
    {
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            AgentToolError::InvalidArgs(format!(
                "destination has no existing ancestor: {}",
                path.display()
            ))
        })?;
    }
    let canonical_ancestor = tokio::fs::canonicalize(existing_ancestor)
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("canonicalize destination ancestor failed: {err}"))
        })?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(AgentToolError::InvalidArgs(format!(
            "destination resolves outside the session workspace: {}",
            path.display()
        )));
    }
    tokio::fs::create_dir_all(parent).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("create destination directory failed: {err}"))
    })?;
    let canonical_parent = tokio::fs::canonicalize(parent).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("canonicalize destination directory failed: {err}"))
    })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(AgentToolError::InvalidArgs(format!(
            "destination resolves outside the session workspace: {}",
            path.display()
        )));
    }
    Ok(())
}

async fn write_resource(
    resource: &ResourceRef,
    path: &Path,
) -> Result<(u64, Option<String>, &'static str), AgentToolError> {
    let result = async {
        match resource {
            ResourceRef::NamedObject { obj_id } => {
                let runtime = get_buckyos_api_runtime().map_err(|err| {
                    AgentToolError::ExecFailed(format!("get buckyos runtime failed: {err}"))
                })?;
                let named_store = runtime.get_named_store().await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("get named_store failed: {err}"))
                })?;
                let mime = named_store
                    .get_object(obj_id)
                    .await
                    .ok()
                    .and_then(|json| serde_json::from_str::<FileObject>(&json).ok())
                    .and_then(|file| {
                        file.meta
                            .get("mime_type")
                            .or_else(|| file.meta.get("mime"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                let (mut reader, _) =
                    named_store.open_reader(obj_id, None).await.map_err(|err| {
                        AgentToolError::ExecFailed(format!(
                            "open named object reader failed: {err}"
                        ))
                    })?;
                let mut file = create_destination(path).await?;
                let bytes = tokio::io::copy(&mut reader, &mut file)
                    .await
                    .map_err(|err| {
                        AgentToolError::ExecFailed(format!("copy named object failed: {err}"))
                    })?;
                file.flush().await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("flush destination failed: {err}"))
                })?;
                Ok((bytes, mime, "named_object"))
            }
            ResourceRef::Url { url, mime_hint } => {
                let mut response = reqwest::Client::new()
                    .get(url)
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                    .map_err(|err| {
                        AgentToolError::ExecFailed(format!("download resource failed: {err}"))
                    })?;
                let mime = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .or_else(|| mime_hint.clone());
                let mut file = create_destination(path).await?;
                let mut bytes = 0u64;
                while let Some(chunk) = response.chunk().await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("read downloaded resource failed: {err}"))
                })? {
                    file.write_all(&chunk).await.map_err(|err| {
                        AgentToolError::ExecFailed(format!(
                            "write downloaded resource failed: {err}"
                        ))
                    })?;
                    bytes = bytes.saturating_add(chunk.len() as u64);
                }
                file.flush().await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("flush destination failed: {err}"))
                })?;
                Ok((bytes, mime, "url"))
            }
            ResourceRef::Base64 { mime, data_base64 } => {
                let data = general_purpose::STANDARD
                    .decode(data_base64)
                    .map_err(|err| {
                        AgentToolError::InvalidArgs(format!(
                            "resource contains invalid base64: {err}"
                        ))
                    })?;
                let mut file = create_destination(path).await?;
                file.write_all(&data).await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("write decoded resource failed: {err}"))
                })?;
                file.flush().await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("flush destination failed: {err}"))
                })?;
                Ok((data.len() as u64, Some(mime.clone()), "base64"))
            }
        }
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(path).await;
    }
    result
}

async fn create_destination(path: &Path) -> Result<tokio::fs::File, AgentToolError> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("create destination failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace".to_string(),
            agent_name: "agent".to_string(),
            behavior: "chat_route".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup".to_string(),
            session_id: "session".to_string(),
            read_token_limit: 1024,
        }
    }

    #[test]
    fn spec_uses_resource_domain_language() {
        let root = tempfile::tempdir().unwrap();
        let spec = MaterializeResourceTool::new(root.path()).spec();
        assert!(spec.description.contains("ResourceRef"));
        let rendered = serde_json::to_string(&spec).unwrap();
        assert!(!rendered.contains("attachment"));
        assert!(!rendered.contains("user message"));
    }

    #[tokio::test]
    async fn base64_resource_is_saved_inside_workspace() {
        let root = tempfile::tempdir().unwrap();
        let tool = MaterializeResourceTool::new(root.path());
        let result = tool
            .call(
                &context(),
                json!({
                    "resource": {
                        "kind": "base64",
                        "mime": "text/plain",
                        "data_base64": "aGVsbG8="
                    },
                    "path": "inputs/sample.txt"
                }),
            )
            .await
            .unwrap();

        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(
            tokio::fs::read(root.path().join("inputs/sample.txt"))
                .await
                .unwrap(),
            b"hello"
        );
        assert_eq!(result.details["source_kind"], "base64");
        assert_eq!(result.details["bytes"], 5);
    }

    #[tokio::test]
    async fn destination_must_stay_inside_workspace_and_not_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let tool = MaterializeResourceTool::new(root.path());
        let resource = json!({
            "kind": "base64",
            "mime": "text/plain",
            "data_base64": "aGVsbG8="
        });
        assert!(tool
            .call(
                &context(),
                json!({"resource": resource.clone(), "path": "../escape.txt"}),
            )
            .await
            .is_err());

        tokio::fs::write(root.path().join("existing.txt"), b"keep")
            .await
            .unwrap();
        assert!(tool
            .call(
                &context(),
                json!({"resource": resource, "path": "existing.txt"}),
            )
            .await
            .is_err());
        assert_eq!(
            tokio::fs::read(root.path().join("existing.txt"))
                .await
                .unwrap(),
            b"keep"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn destination_rejects_workspace_symlink_escape() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
        let tool = MaterializeResourceTool::new(root.path());
        let result = tool
            .call(
                &context(),
                json!({
                    "resource": {
                        "kind": "base64",
                        "mime": "text/plain",
                        "data_base64": "aGVsbG8="
                    },
                    "path": "linked/nested/sample.txt"
                }),
            )
            .await;

        assert!(result.is_err());
        assert!(!outside.path().join("nested").exists());
    }
}
