use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value as Json};

pub use ::agent_tool::{
    normalize_abs_path, parse_read_file_bash_args, rewrite_read_file_path_with_shell_cwd,
    EditFileTool, FileToolConfig, FileWriteAuditBackend, FileWriteAuditRecord, NoopFileWriteAudit,
    ReadFileTool, WriteFileTool, TOOL_EDIT_FILE, TOOL_READ_FILE, TOOL_WRITE_FILE,
};

use crate::agent_tool::{AgentToolError, SessionRuntimeContext};
use crate::runtime_utils::{optional_u64_arg, resolve_path_from_root};
use crate::worklog::{WorklogService, WorklogToolConfig};

pub const TOOL_EXEC_BASH: &str = "exec";

#[derive(Clone, Debug)]
pub struct WorkshopWriteAudit {
    worklog_cfg: WorklogToolConfig,
}

impl WorkshopWriteAudit {
    pub fn new(worklog_cfg: WorklogToolConfig) -> Self {
        Self { worklog_cfg }
    }
}

#[async_trait]
impl FileWriteAuditBackend for WorkshopWriteAudit {
    async fn record_file_write(
        &self,
        ctx: &SessionRuntimeContext,
        args: &Json,
        record: &FileWriteAuditRecord,
    ) -> Result<(), AgentToolError> {
        let worklog_service = WorklogService::new(self.worklog_cfg.clone())?;
        let owner_session_id = optional_string(args, "session_id")?;
        let step_id = optional_string(args, "step_id")?
            .unwrap_or_else(|| format!("{}#{}", ctx.behavior, ctx.step_idx));
        let task_id = optional_string(args, "task_id")?;
        let action_id = optional_string(args, "action_id")?;
        let run_id = optional_string(args, "run_id")?.unwrap_or_else(|| ctx.trace_id.clone());
        let mut tags = vec![
            "workspace".to_string(),
            "local_workspace".to_string(),
            "write".to_string(),
        ];
        if record.diff_truncated {
            tags.push("diff_truncated".to_string());
        }

        let _ = worklog_service
            .execute_action(
                ctx,
                json!({
                    "action": "append",
                    "type": "ActionRecord",
                    "status": "success",
                    "agent_id": ctx.agent_name,
                    "owner_session_id": owner_session_id,
                    "run_id": run_id,
                    "step_id": step_id,
                    "task_id": task_id,
                    "summary": format!("edit_file {} {}", record.operation, record.file_path),
                    "tags": tags,
                    "payload": {
                        "action_type": "workspace_file_write",
                        "cmd_digest": format!("{} {}", record.operation, record.file_path),
                        "exit_code": 0,
                        "path": record.file_path,
                        "operation": record.operation,
                        "created": record.created,
                        "changed": record.changed,
                        "bytes_before": record.bytes_before,
                        "bytes_after": record.bytes_after,
                        "diff": record.diff,
                        "diff_truncated": record.diff_truncated,
                        "action_id": action_id
                    }
                }),
            )
            .await?;
        Ok(())
    }
}

pub(crate) fn require_string(args: &Json, key: &str) -> Result<String, AgentToolError> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("missing or invalid `{key}`")))?;
    if value.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "`{key}` cannot be empty"
        )));
    }
    Ok(value)
}

fn optional_string(args: &Json, key: &str) -> Result<Option<String>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(raw.to_string()))
}

pub(crate) fn optional_u64(args: &Json, key: &str) -> Result<Option<u64>, AgentToolError> {
    optional_u64_arg(args, key)
}

pub(crate) fn read_u64_from_map(
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<u64>, AgentToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be an integer")))
}

pub(crate) fn read_bool_from_map(
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<bool>, AgentToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}

pub(crate) fn parse_workspace_relative_roots(
    value: Option<&Json>,
    agent_env_root: &Path,
) -> Result<Option<Vec<PathBuf>>, AgentToolError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let arr = value.as_array().ok_or_else(|| {
        AgentToolError::InvalidArgs("tool params path roots must be string array".to_string())
    })?;
    let mut roots = Vec::with_capacity(arr.len());
    for item in arr {
        let raw = item.as_str().ok_or_else(|| {
            AgentToolError::InvalidArgs("tool params path roots must be string array".to_string())
        })?;
        let resolved = resolve_path_in_agent_env(agent_env_root, raw)?;
        if !resolved.starts_with(agent_env_root) {
            return Err(AgentToolError::InvalidArgs(format!(
                "path out of workspace scope: {raw}"
            )));
        }
        roots.push(resolved);
    }
    Ok(Some(roots))
}

fn resolve_path_in_agent_env(
    agent_env_root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AgentToolError> {
    resolve_path_from_root(agent_env_root, raw_path)
}

pub(crate) fn is_path_under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

pub(crate) fn truncate_bytes(input: &[u8], max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (String::from_utf8_lossy(input).to_string(), false);
    }
    (
        String::from_utf8_lossy(&input[..max_bytes]).to_string(),
        true,
    )
}
