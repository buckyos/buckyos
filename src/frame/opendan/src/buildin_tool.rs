use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value as Json};
use tokio::fs;

use crate::agent_tool::{
    tokenize_bash_command_line, AgentTool, AgentToolError, AgentToolResult, ToolSpec,
};
use crate::behavior::SessionRuntimeContext;
use crate::worklog::{WorklogService, WorklogToolConfig};
use crate::workspace::workshop::{AgentWorkshopConfig, WorkshopToolConfig};

pub const TOOL_EDIT_FILE: &str = "edit_file";
pub const TOOL_EXEC_BASH: &str = "exec";
pub const TOOL_WRITE_FILE: &str = "write_file";
pub const TOOL_READ_FILE: &str = "read_file";

const DEFAULT_MAX_FILE_READ_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
struct EditFilePolicy {
    allow_create: bool,
    allow_replace: bool,
    max_write_bytes: usize,
    max_diff_lines: usize,
    allowed_write_roots: Vec<PathBuf>,
}

impl EditFilePolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, AgentToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let allow_create = read_bool_from_map(params, "allow_create")?.unwrap_or(true);
        let allow_replace = read_bool_from_map(params, "allow_replace")?.unwrap_or(true);

        let max_write_bytes = read_u64_from_map(params, "max_write_bytes")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(workshop_cfg.default_max_file_write_bytes);
        if max_write_bytes == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` max_write_bytes must be > 0",
                tool_cfg.name
            )));
        }

        let max_diff_lines = read_u64_from_map(params, "max_diff_lines")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(workshop_cfg.default_max_diff_lines);
        if max_diff_lines == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` max_diff_lines must be > 0",
                tool_cfg.name
            )));
        }

        let allowed_write_roots = parse_workspace_relative_roots(
            params.get("allowed_write_roots"),
            &workshop_cfg.workspace_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.workspace_root.clone()]);

        Ok(Self {
            allow_create,
            allow_replace,
            max_write_bytes,
            max_diff_lines,
            allowed_write_roots,
        })
    }
}

#[derive(Clone, Debug)]
struct WriteFilePolicy {
    allow_create: bool,
    max_write_bytes: usize,
    max_diff_lines: usize,
    allowed_write_roots: Vec<PathBuf>,
}

impl WriteFilePolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, AgentToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let allow_create = read_bool_from_map(params, "allow_create")?.unwrap_or(true);
        let max_write_bytes = read_u64_from_map(params, "max_write_bytes")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(workshop_cfg.default_max_file_write_bytes);
        if max_write_bytes == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` max_write_bytes must be > 0",
                tool_cfg.name
            )));
        }

        let max_diff_lines = read_u64_from_map(params, "max_diff_lines")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(workshop_cfg.default_max_diff_lines);
        if max_diff_lines == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` max_diff_lines must be > 0",
                tool_cfg.name
            )));
        }

        let allowed_write_roots = parse_workspace_relative_roots(
            params.get("allowed_write_roots"),
            &workshop_cfg.workspace_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.workspace_root.clone()]);

        Ok(Self {
            allow_create,
            max_write_bytes,
            max_diff_lines,
            allowed_write_roots,
        })
    }
}

#[derive(Clone, Debug)]
struct ReadFilePolicy {
    max_read_bytes: usize,
    allowed_read_roots: Vec<PathBuf>,
}

impl ReadFilePolicy {
    fn from_tool_config(
        workshop_cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, AgentToolError> {
        let params = tool_cfg.params.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs(format!(
                "tool `{}` params must be a json object",
                tool_cfg.name
            ))
        })?;

        let max_read_bytes = read_u64_from_map(params, "max_read_bytes")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(
                workshop_cfg
                    .max_output_bytes
                    .max(DEFAULT_MAX_FILE_READ_BYTES),
            );
        if max_read_bytes == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` max_read_bytes must be > 0",
                tool_cfg.name
            )));
        }

        let allowed_read_roots = parse_workspace_relative_roots(
            params.get("allowed_read_roots"),
            &workshop_cfg.workspace_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.workspace_root.clone()]);

        Ok(Self {
            max_read_bytes,
            allowed_read_roots,
        })
    }
}

#[derive(Clone, Debug)]
pub struct EditFileTool {
    cfg: AgentWorkshopConfig,
    policy: EditFilePolicy,
    write_audit: WorkshopWriteAudit,
}

impl EditFileTool {
    pub fn from_tool_config(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
        write_audit: WorkshopWriteAudit,
    ) -> Result<Self, AgentToolError> {
        Ok(Self {
            cfg: cfg.clone(),
            policy: EditFilePolicy::from_tool_config(cfg, tool_cfg)?,
            write_audit,
        })
    }
}

#[async_trait]
impl AgentTool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EDIT_FILE.to_string(),
            description: "Edit file.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "pos_chunk": { "type": "string" },
                    "new_content": { "type": "string" },
                    "mode": { "type": "string", "enum": ["replace", "after", "before"] }
                },
                "required": ["path", "pos_chunk", "new_content"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
            usage: None,
        }
    }
    fn support_bash(&self) -> bool {
        false
    }
    fn support_action(&self) -> bool {
        true
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let file_path = require_string(&args, "path")?;
        let abs_path = resolve_path_in_workspace(&self.cfg.workspace_root, &file_path)?;
        if !is_path_under_any(&abs_path, &self.policy.allowed_write_roots) {
            return Err(AgentToolError::InvalidArgs(format!(
                "path `{file_path}` is not writable by workshop tool policy"
            )));
        }

        let exists = fs::metadata(&abs_path).await.is_ok();
        if !exists && !self.policy.allow_create {
            return Err(AgentToolError::InvalidArgs(format!(
                "file does not exist or create disabled by policy: {file_path}"
            )));
        }

        let original_content = if exists {
            read_text_file_lossy(&abs_path).await?
        } else {
            String::new()
        };

        let pos_chunk = require_string(&args, "pos_chunk")?;
        let new_content = require_string(&args, "new_content")?;
        let mode = parse_edit_mode(&args)?;
        if mode == "replace" && !self.policy.allow_replace {
            return Err(AgentToolError::InvalidArgs(
                "replace mode disabled by workshop tool policy".to_string(),
            ));
        }
        let (operation, updated_content, matched) =
            if let Some(anchor_pos) = original_content.find(&pos_chunk) {
                let updated = match mode {
                    "replace" => {
                        let mut out = original_content.clone();
                        let end = anchor_pos + pos_chunk.len();
                        out.replace_range(anchor_pos..end, &new_content);
                        out
                    }
                    "after" => {
                        let mut out = original_content.clone();
                        let insert_at = anchor_pos + pos_chunk.len();
                        out.insert_str(insert_at, &new_content);
                        out
                    }
                    "before" => {
                        let mut out = original_content.clone();
                        out.insert_str(anchor_pos, &new_content);
                        out
                    }
                    _ => unreachable!("validated by parse_edit_mode"),
                };
                (mode.to_string(), updated, true)
            } else {
                (mode.to_string(), original_content.clone(), false)
            };

        if updated_content.len() > self.policy.max_write_bytes {
            return Err(AgentToolError::InvalidArgs(format!(
                "file content too large: {} > {} bytes",
                updated_content.len(),
                self.policy.max_write_bytes
            )));
        }

        let changed = original_content != updated_content;
        let created = changed && !exists;
        if changed {
            if let Some(parent) = abs_path.parent() {
                fs::create_dir_all(parent).await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("create parent dir failed: {err}"))
                })?;
            }
            fs::write(&abs_path, updated_content.as_bytes())
                .await
                .map_err(|err| AgentToolError::ExecFailed(format!("write file failed: {err}")))?;
        }

        let (diff, diff_truncated) = build_simple_diff(
            &file_path,
            &original_content,
            &updated_content,
            self.policy.max_diff_lines,
        );
        if changed {
            self.write_audit
                .record_file_write(
                    ctx,
                    &args,
                    &FileWriteAuditRecord {
                        file_path: file_path.clone(),
                        operation: operation.clone(),
                        created,
                        changed,
                        bytes_before: original_content.len(),
                        bytes_after: updated_content.len(),
                        diff: diff.clone(),
                        diff_truncated,
                    },
                )
                .await?;
        }

        let details = json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "operation": operation,
            "matched": matched,
            "created": created,
            "changed": changed,
            "bytes_before": original_content.len(),
            "bytes_after": updated_content.len(),
            "diff": diff,
            "diff_truncated": diff_truncated
        });
        let summary = if changed {
            format!(
                "{} {} -> {} bytes",
                operation,
                original_content.len(),
                updated_content.len()
            )
        } else if !matched {
            "anchor not found, no change".to_string()
        } else {
            "no change".to_string()
        };
        Ok(AgentToolResult::from_details(details)
            .with_cmd_line(format!("{} {}", TOOL_EDIT_FILE, abs_path.to_string_lossy()))
            .with_result(summary))
    }
}

#[derive(Clone, Debug)]
pub struct WriteFileTool {
    cfg: AgentWorkshopConfig,
    policy: WriteFilePolicy,
    write_audit: WorkshopWriteAudit,
}

impl WriteFileTool {
    pub fn from_tool_config(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
        write_audit: WorkshopWriteAudit,
    ) -> Result<Self, AgentToolError> {
        Ok(Self {
            cfg: cfg.clone(),
            policy: WriteFilePolicy::from_tool_config(cfg, tool_cfg)?,
            write_audit,
        })
    }
}

#[async_trait]
impl AgentTool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WRITE_FILE.to_string(),
            description: "Write file.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "mode": { "type": "string", "enum": ["new", "append", "write"] }
                },
                "required": ["path", "content"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
            usage: None,
        }
    }
    fn support_bash(&self) -> bool {
        false
    }
    fn support_action(&self) -> bool {
        true
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }
    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let file_path = require_string(&args, "path")?;
        let content = require_string(&args, "content")?;
        let abs_path = resolve_path_in_workspace(&self.cfg.workspace_root, &file_path)?;
        if !is_path_under_any(&abs_path, &self.policy.allowed_write_roots) {
            return Err(AgentToolError::InvalidArgs(format!(
                "path `{file_path}` is not writable by workshop tool policy"
            )));
        }

        let mode = parse_write_mode(&args)?;
        let exists = fs::metadata(&abs_path).await.is_ok();

        if !exists && !self.policy.allow_create {
            return Err(AgentToolError::InvalidArgs(format!(
                "file creation disabled by policy: {file_path}"
            )));
        }
        if mode == "new" && exists {
            return Err(AgentToolError::InvalidArgs(format!(
                "write mode `new` requires target file not exist: {file_path}"
            )));
        }

        let original_content = if exists {
            read_text_file_lossy(&abs_path).await?
        } else {
            String::new()
        };
        let operation = if mode == "append" {
            "append"
        } else if mode == "new" {
            "new"
        } else if exists {
            "write"
        } else {
            "create"
        }
        .to_string();
        let updated_content = if mode == "append" {
            format!("{original_content}{content}")
        } else {
            content
        };

        if updated_content.len() > self.policy.max_write_bytes {
            return Err(AgentToolError::InvalidArgs(format!(
                "file content too large: {} > {} bytes",
                updated_content.len(),
                self.policy.max_write_bytes
            )));
        }

        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("create parent dir failed: {err}"))
            })?;
        }
        fs::write(&abs_path, updated_content.as_bytes())
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("write file failed: {err}")))?;

        let changed = original_content != updated_content;
        let (diff, diff_truncated) = build_simple_diff(
            &file_path,
            &original_content,
            &updated_content,
            self.policy.max_diff_lines,
        );
        self.write_audit
            .record_file_write(
                ctx,
                &args,
                &FileWriteAuditRecord {
                    file_path: file_path.clone(),
                    operation: operation.clone(),
                    created: !exists,
                    changed,
                    bytes_before: original_content.len(),
                    bytes_after: updated_content.len(),
                    diff: diff.clone(),
                    diff_truncated,
                },
            )
            .await?;

        let details = json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "operation": operation,
            "mode": mode,
            "created": !exists,
            "changed": changed,
            "bytes_before": original_content.len(),
            "bytes_after": updated_content.len(),
            "diff": diff,
            "diff_truncated": diff_truncated
        });
        let summary = format!(
            "{} {} -> {} bytes",
            operation,
            original_content.len(),
            updated_content.len()
        );
        Ok(AgentToolResult::from_details(details)
            .with_cmd_line(format!(
                "{} {}",
                TOOL_WRITE_FILE,
                abs_path.to_string_lossy()
            ))
            .with_result(summary))
    }
}

#[derive(Clone, Debug)]
pub struct ReadFileTool {
    cfg: AgentWorkshopConfig,
    policy: ReadFilePolicy,
}

impl ReadFileTool {
    pub fn from_tool_config(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
    ) -> Result<Self, AgentToolError> {
        Ok(Self {
            cfg: cfg.clone(),
            policy: ReadFilePolicy::from_tool_config(cfg, tool_cfg)?,
        })
    }
}

#[async_trait]
impl AgentTool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_READ_FILE.to_string(),
            description: "Read file.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "range": {},
                    "first_chunk": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
            usage: Some(
                "read_file <path> [range] [first_chunk]\n\trange: 1-based; supports negative/$/+N, and applies within first_chunk slice"
                    .to_string(),
            ),
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        true
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let file_path = require_string(&args, "path")?;
        let abs_path = resolve_path_in_workspace(&self.cfg.workspace_root, &file_path)?;
        if !is_path_under_any(&abs_path, &self.policy.allowed_read_roots) {
            return Err(AgentToolError::InvalidArgs(format!(
                "path `{file_path}` is not readable by workshop tool policy"
            )));
        }

        let full_content = read_text_file_lossy(&abs_path).await?;
        let (selected_content, matched) =
            if let Some(first_chunk) = optional_string(&args, "first_chunk")? {
                if let Some(pos) = full_content.find(&first_chunk) {
                    (full_content[pos..].to_string(), true)
                } else {
                    (String::new(), false)
                }
            } else {
                (full_content.clone(), true)
            };

        let (selected_content, line_range_label) = {
            let lines = selected_content.lines().collect::<Vec<_>>();
            if let Some((start, end)) = parse_line_range(&args, lines.len())? {
                let start_idx = start.saturating_sub(1).min(lines.len());
                let end_idx = end.min(lines.len());
                let slice = if start_idx < end_idx {
                    lines[start_idx..end_idx].join("\n")
                } else {
                    String::new()
                };
                (slice, format!("{start}-{end}"))
            } else {
                (selected_content, String::new())
            }
        };

        let (content, truncated) =
            truncate_bytes(selected_content.as_bytes(), self.policy.max_read_bytes);

        let details = json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "content": content,
            "matched": matched,
            "line_range": line_range_label,
            "bytes": full_content.len(),
            "truncated": truncated,
            "pwd": self.cfg.workspace_root.to_string_lossy().to_string(),
        });
        let summary = format!(
            "read {} bytes{}",
            full_content.len(),
            if truncated { " (truncated)" } else { "" }
        );
        let stdout_payload = (!content.trim().is_empty()).then_some(content.clone());
        Ok(AgentToolResult::from_details(details)
            .with_cmd_line(format!("{} {}", TOOL_READ_FILE, file_path))
            .with_result(summary)
            .with_stdout(stdout_payload))
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }

        let mut args = parse_read_file_bash_args(&tokens[1..])?;
        let shell_cwd_for_details = shell_cwd.map(|cwd| cwd.to_path_buf());
        if let Some(shell_cwd) = shell_cwd {
            rewrite_read_file_path_with_shell_cwd(&mut args, shell_cwd);
        }
        let mut result = self.call(ctx, args).await?;
        result.cmd_line = line.trim().to_string();
        if let Some(cwd) = shell_cwd_for_details {
            if let Some(map) = result.details.as_object_mut() {
                map.insert(
                    "pwd".to_string(),
                    Json::String(cwd.to_string_lossy().to_string()),
                );
            }
        }
        Ok(result)
    }
}

fn parse_read_file_bash_args(tokens: &[String]) -> Result<Json, AgentToolError> {
    let mut out = serde_json::Map::<String, Json>::new();
    if tokens.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "missing required arg `path`".to_string(),
        ));
    }

    let has_key_value = tokens.iter().any(|token| token.contains('='));
    if has_key_value {
        if tokens.iter().any(|token| !token.contains('=')) {
            return Err(AgentToolError::InvalidArgs(
                "read_file bash args cannot mix positional args with key=value args".to_string(),
            ));
        }
        for token in tokens {
            let (raw_key, raw_value) = token.split_once('=').ok_or_else(|| {
                AgentToolError::InvalidArgs("invalid key=value token".to_string())
            })?;
            let key = raw_key.trim();
            if key.is_empty() {
                return Err(AgentToolError::InvalidArgs(
                    "arg key cannot be empty".to_string(),
                ));
            }
            if key != "path" && key != "range" && key != "first_chunk" {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported read_file arg `{key}`"
                )));
            }
            out.insert(key.to_string(), Json::String(raw_value.trim().to_string()));
        }
    } else {
        if tokens.len() > 3 {
            return Err(AgentToolError::InvalidArgs(format!(
                "too many positional args for tool `{}`: got {}, max 3",
                TOOL_READ_FILE,
                tokens.len()
            )));
        }
        if let Some(path) = tokens.first() {
            out.insert("path".to_string(), Json::String(path.trim().to_string()));
        }
        if let Some(range) = tokens.get(1) {
            out.insert("range".to_string(), Json::String(range.trim().to_string()));
        }
        if let Some(first_chunk) = tokens.get(2) {
            out.insert(
                "first_chunk".to_string(),
                Json::String(first_chunk.trim().to_string()),
            );
        }
    }

    if out
        .get("path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err(AgentToolError::InvalidArgs(
            "missing required arg `path`".to_string(),
        ));
    }

    Ok(Json::Object(out))
}

fn rewrite_read_file_path_with_shell_cwd(args: &mut Json, shell_cwd: &Path) {
    let Some(map) = args.as_object_mut() else {
        return;
    };
    let Some(raw_path) = map.get("path").and_then(|value| value.as_str()) else {
        return;
    };

    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return;
    }
    let parsed = Path::new(trimmed);
    if parsed.is_absolute() {
        return;
    }

    let joined = shell_cwd.join(parsed);
    map.insert(
        "path".to_string(),
        Json::String(joined.to_string_lossy().to_string()),
    );
}

#[derive(Clone, Debug)]
pub struct WorkshopWriteAudit {
    worklog_cfg: WorklogToolConfig,
}

impl WorkshopWriteAudit {
    pub fn new(worklog_cfg: WorklogToolConfig) -> Self {
        Self { worklog_cfg }
    }

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
                    "type": "workspace_file_write",
                    "status": "success",
                    "agent_id": ctx.agent_name,
                    "owner_session_id": owner_session_id,
                    "run_id": run_id,
                    "step_id": step_id,
                    "task_id": task_id,
                    "summary": format!("edit_file {} {}", record.operation, record.file_path),
                    "tags": tags,
                    "payload": {
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

#[derive(Clone, Debug)]
struct FileWriteAuditRecord {
    file_path: String,
    operation: String,
    created: bool,
    changed: bool,
    bytes_before: usize,
    bytes_after: usize,
    diff: String,
    diff_truncated: bool,
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
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a positive integer")))
}

fn parse_edit_mode(args: &Json) -> Result<&'static str, AgentToolError> {
    let raw = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("replace");
    normalize_edit_mode(raw)
}

fn normalize_edit_mode(raw: &str) -> Result<&'static str, AgentToolError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "replace" => Ok("replace"),
        "after" => Ok("after"),
        "before" => Ok("before"),
        other => Err(AgentToolError::InvalidArgs(format!(
            "invalid edit mode `{other}`, expected replace/after/before"
        ))),
    }
}

fn parse_write_mode(args: &Json) -> Result<&'static str, AgentToolError> {
    let raw = args.get("mode").and_then(|v| v.as_str()).unwrap_or("write");
    normalize_write_mode(raw)
}

fn normalize_write_mode(raw: &str) -> Result<&'static str, AgentToolError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "new" | "create" => Ok("new"),
        "append" => Ok("append"),
        "" | "write" | "overwrite" => Ok("write"),
        other => Err(AgentToolError::InvalidArgs(format!(
            "invalid write mode `{other}`, expected new/append/write"
        ))),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineMarker {
    Index(i64),
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RangeDefaultEnd {
    SameAsStart,
    End,
    TailIfStartNegative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LineRangeSpec {
    start: Option<LineMarker>,
    end: Option<LineMarker>,
    count: Option<usize>,
    default_end: RangeDefaultEnd,
}

fn parse_line_range(
    args: &Json,
    total_lines: usize,
) -> Result<Option<(usize, usize)>, AgentToolError> {
    let Some(spec) = parse_line_range_spec(args)? else {
        return Ok(None);
    };
    let (start, end) = resolve_line_range(spec, total_lines)?;
    Ok(Some((start, end)))
}

fn parse_line_range_spec(args: &Json) -> Result<Option<LineRangeSpec>, AgentToolError> {
    let Some(raw) = args.get("range") else {
        return Ok(None);
    };

    if let Some(obj) = raw.as_object() {
        let start = obj
            .get("start")
            .map(|v| parse_line_marker_json(v, "range.start"))
            .transpose()?;
        let end = obj
            .get("end")
            .map(|v| parse_line_marker_json(v, "range.end"))
            .transpose()?;
        let count = obj
            .get("count")
            .map(|v| parse_line_count_json(v, "range.count"))
            .transpose()?;

        if count.is_some() && end.is_some() {
            return Err(AgentToolError::InvalidArgs(
                "range cannot set both `end` and `count`".to_string(),
            ));
        }
        if count.is_some() && start.is_none() {
            return Err(AgentToolError::InvalidArgs(
                "range.count requires range.start".to_string(),
            ));
        }
        if start.is_none() && end.is_none() && count.is_none() {
            return Err(AgentToolError::InvalidArgs(
                "range object must contain at least one of start/end/count".to_string(),
            ));
        }

        return Ok(Some(LineRangeSpec {
            start,
            end,
            count,
            default_end: RangeDefaultEnd::TailIfStartNegative,
        }));
    } else if let Some(arr) = raw.as_array() {
        if arr.is_empty() || arr.len() > 2 {
            return Err(AgentToolError::InvalidArgs(
                "range array must be [start] or [start,end]".to_string(),
            ));
        }
        let start = parse_line_marker_json(&arr[0], "range[0]")?;
        let end = if arr.len() == 2 {
            Some(parse_line_marker_json(&arr[1], "range[1]")?)
        } else {
            None
        };
        return Ok(Some(LineRangeSpec {
            start: Some(start),
            end,
            count: None,
            default_end: RangeDefaultEnd::SameAsStart,
        }));
    } else if raw.is_i64() || raw.is_u64() {
        let marker = parse_line_marker_json(raw, "range")?;
        return Ok(Some(LineRangeSpec {
            start: Some(marker),
            end: None,
            count: None,
            default_end: RangeDefaultEnd::SameAsStart,
        }));
    } else if let Some(text) = raw.as_str() {
        return parse_line_range_text(text);
    } else {
        return Err(AgentToolError::InvalidArgs(
            "range must be string/number/array/object".to_string(),
        ));
    }
}

fn parse_line_range_text(raw: &str) -> Result<Option<LineRangeSpec>, AgentToolError> {
    let text = raw.trim();
    if text.is_empty() || text == "-" {
        return Ok(None);
    }

    if text.contains(',') || text.contains(':') {
        let has_comma = text.contains(',');
        let has_colon = text.contains(':');
        if has_comma && has_colon {
            return Err(AgentToolError::InvalidArgs(format!(
                "invalid range text `{raw}`"
            )));
        }
        let delim = if has_comma { ',' } else { ':' };
        let parts = text.split(delim).collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(AgentToolError::InvalidArgs(format!(
                "invalid range text `{raw}`"
            )));
        }

        let start = if parts[0].trim().is_empty() {
            None
        } else {
            Some(parse_line_marker_token(parts[0].trim(), raw)?)
        };

        let end_text = parts[1].trim();
        if end_text.starts_with('+') {
            if start.is_none() {
                return Err(AgentToolError::InvalidArgs(format!(
                    "range `{raw}` with `+count` requires start"
                )));
            }
            let count = parse_line_count_text(end_text, raw)?;
            return Ok(Some(LineRangeSpec {
                start,
                end: None,
                count: Some(count),
                default_end: RangeDefaultEnd::SameAsStart,
            }));
        }
        let end = if end_text.is_empty() {
            None
        } else {
            Some(parse_line_marker_token(end_text, raw)?)
        };
        return Ok(Some(LineRangeSpec {
            start,
            end,
            count: None,
            default_end: RangeDefaultEnd::End,
        }));
    }

    if let Some((left, right)) = text.split_once('-') {
        let left_trim = left.trim();
        let right_trim = right.trim();
        if !left_trim.is_empty()
            && !right_trim.is_empty()
            && left_trim.chars().all(|ch| ch.is_ascii_digit())
            && right_trim.chars().all(|ch| ch.is_ascii_digit())
        {
            let start = parse_line_marker_token(left_trim, raw)?;
            let end = parse_line_marker_token(right_trim, raw)?;
            return Ok(Some(LineRangeSpec {
                start: Some(start),
                end: Some(end),
                count: None,
                default_end: RangeDefaultEnd::SameAsStart,
            }));
        }
    }

    let marker = parse_line_marker_token(text, raw)?;
    Ok(Some(LineRangeSpec {
        start: Some(marker),
        end: None,
        count: None,
        default_end: RangeDefaultEnd::SameAsStart,
    }))
}

fn parse_line_marker_token(raw: &str, range_text: &str) -> Result<LineMarker, AgentToolError> {
    let token = raw.trim();
    if token == "$" {
        return Ok(LineMarker::End);
    }
    if token.starts_with('+') {
        return Err(AgentToolError::InvalidArgs(format!(
            "invalid range text `{range_text}`"
        )));
    }
    let value = token
        .parse::<i64>()
        .map_err(|_| AgentToolError::InvalidArgs(format!("invalid range text `{range_text}`")))?;
    if value == 0 {
        return Err(AgentToolError::InvalidArgs(
            "range must be 1-based, zero is invalid".to_string(),
        ));
    }
    Ok(LineMarker::Index(value))
}

fn parse_line_count_text(raw: &str, range_text: &str) -> Result<usize, AgentToolError> {
    let token = raw.trim();
    let Some(count_text) = token.strip_prefix('+') else {
        return Err(AgentToolError::InvalidArgs(format!(
            "invalid range text `{range_text}`"
        )));
    };
    let count_u64 = count_text
        .trim()
        .parse::<u64>()
        .map_err(|_| AgentToolError::InvalidArgs(format!("invalid range text `{range_text}`")))?;
    if count_u64 == 0 {
        return Err(AgentToolError::InvalidArgs(
            "range count must be positive integer".to_string(),
        ));
    }
    u64_to_usize(count_u64)
}

fn parse_line_marker_json(value: &Json, name: &str) -> Result<LineMarker, AgentToolError> {
    if let Some(text) = value.as_str() {
        return parse_line_marker_token(text, name);
    }
    if let Some(v) = value.as_i64() {
        if v == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "{name} must be non-zero integer"
            )));
        }
        return Ok(LineMarker::Index(v));
    }
    if let Some(v) = value.as_u64() {
        let as_i64 = i64::try_from(v)
            .map_err(|_| AgentToolError::InvalidArgs(format!("{name} is too large")))?;
        return Ok(LineMarker::Index(as_i64));
    }
    Err(AgentToolError::InvalidArgs(format!(
        "{name} must be integer or `$`"
    )))
}

fn parse_line_count_json(value: &Json, name: &str) -> Result<usize, AgentToolError> {
    let count_u64 = value
        .as_u64()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("{name} must be positive integer")))?;
    if count_u64 == 0 {
        return Err(AgentToolError::InvalidArgs(format!(
            "{name} must be positive integer"
        )));
    }
    u64_to_usize(count_u64)
}

fn resolve_line_range(
    spec: LineRangeSpec,
    total_lines: usize,
) -> Result<(usize, usize), AgentToolError> {
    let total_i64 = i64::try_from(total_lines)
        .map_err(|_| AgentToolError::InvalidArgs("file is too large".to_string()))?;

    let start_marker = spec.start.unwrap_or(LineMarker::Index(1));
    let start_raw = resolve_line_marker(start_marker, total_i64);

    let end_raw = if let Some(count) = spec.count {
        let count_i64 = i64::try_from(count)
            .map_err(|_| AgentToolError::InvalidArgs("range count is too large".to_string()))?;
        start_raw
            .checked_add(count_i64 - 1)
            .ok_or_else(|| AgentToolError::InvalidArgs("range end overflow".to_string()))?
    } else if let Some(end_marker) = spec.end {
        resolve_line_marker(end_marker, total_i64)
    } else {
        match spec.default_end {
            RangeDefaultEnd::SameAsStart => start_raw,
            RangeDefaultEnd::End => total_i64,
            RangeDefaultEnd::TailIfStartNegative => {
                if matches!(start_marker, LineMarker::Index(v) if v < 0) {
                    total_i64
                } else {
                    start_raw
                }
            }
        }
    };

    let start_clamped = start_raw.clamp(1, total_i64.saturating_add(1));
    let end_clamped = end_raw.clamp(0, total_i64);
    let start = usize::try_from(start_clamped)
        .map_err(|_| AgentToolError::InvalidArgs("range start out of bounds".to_string()))?;
    let end = usize::try_from(end_clamped)
        .map_err(|_| AgentToolError::InvalidArgs("range end out of bounds".to_string()))?;
    Ok((start, end))
}

fn resolve_line_marker(marker: LineMarker, total_lines: i64) -> i64 {
    match marker {
        LineMarker::End => total_lines,
        LineMarker::Index(v) if v > 0 => v,
        LineMarker::Index(v) => total_lines.saturating_add(v).saturating_add(1),
    }
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
    workspace_root: &Path,
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
        roots.push(resolve_path_in_workspace(workspace_root, raw)?);
    }
    Ok(Some(roots))
}

fn resolve_path_in_workspace(
    workspace_root: &Path,
    raw_path: &str,
) -> Result<PathBuf, AgentToolError> {
    if raw_path.trim().is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "path cannot be empty".to_string(),
        ));
    }
    let user_path = Path::new(raw_path);
    let candidate = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        workspace_root.join(user_path)
    };
    let normalized = normalize_abs_path(&candidate);
    if !normalized.starts_with(workspace_root) {
        return Err(AgentToolError::InvalidArgs(format!(
            "path out of workspace scope: {raw_path}"
        )));
    }
    Ok(normalized)
}

pub(crate) fn normalize_abs_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

pub(crate) fn is_path_under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

async fn read_text_file_lossy(path: &Path) -> Result<String, AgentToolError> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("read file failed: {err}")))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
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

fn build_simple_diff(
    display_path: &str,
    before: &str,
    after: &str,
    max_body_lines: usize,
) -> (String, bool) {
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();

    let mut body = Vec::new();
    let mut truncated = false;
    let max_len = before_lines.len().max(after_lines.len());
    for idx in 0..max_len {
        if body.len() >= max_body_lines {
            truncated = true;
            break;
        }
        let old = before_lines.get(idx).copied();
        let new = after_lines.get(idx).copied();
        match (old, new) {
            (Some(a), Some(b)) if a == b => body.push(format!(" {a}")),
            (Some(a), Some(b)) => {
                body.push(format!("-{a}"));
                if body.len() >= max_body_lines {
                    truncated = true;
                    break;
                }
                body.push(format!("+{b}"));
            }
            (Some(a), None) => body.push(format!("-{a}")),
            (None, Some(b)) => body.push(format!("+{b}")),
            (None, None) => {}
        }
    }

    let mut diff = Vec::new();
    diff.push(format!("--- a/{display_path}"));
    diff.push(format!("+++ b/{display_path}"));
    diff.push(format!(
        "@@ -1,{} +1,{} @@",
        before_lines.len(),
        after_lines.len()
    ));
    diff.extend(body);
    if truncated {
        diff.push("... [DIFF_TRUNCATED]".to_string());
    }

    (diff.join("\n"), truncated)
}

fn u64_to_usize(v: u64) -> Result<usize, AgentToolError> {
    usize::try_from(v).map_err(|_| AgentToolError::InvalidArgs(format!("value too large: {v}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_range_text_formats() {
        assert_eq!(
            parse_line_range(&json!({"range": "10,20"}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10:20"}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10"}), 100).expect("parse"),
            Some((10, 10))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10,+5"}), 100).expect("parse"),
            Some((10, 14))
        );
        assert_eq!(
            parse_line_range(&json!({"range": ",30"}), 100).expect("parse"),
            Some((1, 30))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-30,"}), 100).expect("parse"),
            Some((71, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-30,$"}), 100).expect("parse"),
            Some((71, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-1"}), 100).expect("parse"),
            Some((100, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-5,-1"}), 100).expect("parse"),
            Some((96, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10,$"}), 100).expect("parse"),
            Some((10, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "$"}), 100).expect("parse"),
            Some((100, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "1-2"}), 100).expect("parse"),
            Some((1, 2))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-"}), 100).expect("parse"),
            None
        );
    }

    #[test]
    fn parse_line_range_json_formats() {
        assert_eq!(
            parse_line_range(&json!({"range": {"start": 10, "end": 20}}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": {"start": 10}}), 100).expect("parse"),
            Some((10, 10))
        );
        assert_eq!(
            parse_line_range(&json!({"range": {"start": -5}}), 100).expect("parse"),
            Some((96, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": {"start": 10, "count": 5}}), 100).expect("parse"),
            Some((10, 14))
        );
        assert_eq!(
            parse_line_range(&json!({"range": [10, 20]}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": [10]}), 100).expect("parse"),
            Some((10, 10))
        );
        assert_eq!(
            parse_line_range(&json!({"range": 10}), 100).expect("parse"),
            Some((10, 10))
        );
    }

    #[test]
    fn parse_line_range_errors_on_invalid_forms() {
        assert!(parse_line_range(&json!({"range": "0"}), 10).is_err());
        assert!(parse_line_range(&json!({"range": "10,+0"}), 10).is_err());
        assert!(
            parse_line_range(&json!({"range": {"start": 1, "end": 2, "count": 3}}), 10).is_err()
        );
        assert!(parse_line_range(&json!({"range": {"count": 3}}), 10).is_err());
        assert!(parse_line_range(&json!({"range": [1, 2, 3]}), 10).is_err());
    }
}
