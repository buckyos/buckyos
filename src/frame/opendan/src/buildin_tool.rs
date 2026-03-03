use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use log::warn;
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};

use crate::agent_session::AgentSessionMgr;
use crate::agent_tool::{AgentTool, AgentToolError, ToolSpec};
use crate::behavior::TraceCtx;
use crate::worklog::{WorklogTool, WorklogToolConfig};
use crate::workspace::workshop::{AgentWorkshopConfig, WorkshopToolConfig};

pub const TOOL_EDIT_FILE: &str = "edit";
pub const TOOL_EXEC_BASH: &str = "exec";
pub const TOOL_WRITE_FILE: &str = "write";
pub const TOOL_READ_FILE: &str = "read";

const DEFAULT_BASH_PATH: &str = "/bin/bash";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_DIFF_LINES: usize = 200;
const DEFAULT_MAX_FILE_WRITE_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_FILE_READ_BYTES: usize = 256 * 1024;
const EXEC_BASH_SUCCESS_DETAIL_LINES: usize = 16;
const EXEC_BASH_TMUX_POLL_MS: u64 = 120;
const EXEC_BASH_CAPTURE_SCROLLBACK_LINES: &str = "-6000";
const EXEC_BASH_TMUX_SESSION_PREFIX: &str = "od_";
const EXEC_BASH_TMUX_GC_TRIGGER_COUNT: usize = 16;
const EXEC_BASH_TMUX_GC_IDLE_SECS: u64 = 24 * 60 * 60;
const EXEC_BASH_TMUX_LIST_FORMAT: &str = "#{session_name}\t#{session_activity}";

pub fn builtin_tool_summary(action_name: &str) -> Option<&'static str> {
    match action_name {
        TOOL_EXEC_BASH => Some("Run shell command."),
        TOOL_EDIT_FILE => Some("Edit file by anchor."),
        TOOL_WRITE_FILE => Some("Write file."),
        TOOL_READ_FILE => Some("Read file."),
        _ => None,
    }
}

pub fn builtin_tool_args_schema(action_name: &str) -> Option<Json> {
    match action_name {
        TOOL_EXEC_BASH => Some(json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout_ms": { "type": "integer", "minimum": 1 },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["command"]
        })),
        TOOL_EDIT_FILE => Some(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "pos_chunk": { "type": "string" },
                "new_content": { "type": "string" },
                "mode": { "type": "string", "enum": ["replace", "after", "before"] }
            },
            "required": ["path", "pos_chunk", "new_content", "mode"]
        })),
        TOOL_WRITE_FILE => Some(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "mode": { "type": "string", "enum": ["new", "append", "write"] }
            },
            "required": ["path", "content", "mode"]
        })),
        TOOL_READ_FILE => Some(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "range": {},
                "first_chunk": { "type": "string" }
            },
            "required": ["path"]
        })),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct ExecBashPolicy {
    default_timeout_ms: u64,
    max_timeout_ms: u64,
    allow_env: bool,
    allowed_cwd_roots: Vec<PathBuf>,
}

impl ExecBashPolicy {
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

        let default_timeout_raw = read_u64_from_map(params, "default_timeout_ms")?;
        let max_timeout_raw = read_u64_from_map(params, "max_timeout_ms")?;
        let max_timeout_ms = max_timeout_raw
            .unwrap_or(default_timeout_raw.unwrap_or(workshop_cfg.default_timeout_ms));
        let default_timeout_ms =
            default_timeout_raw.unwrap_or(workshop_cfg.default_timeout_ms.min(max_timeout_ms));
        if default_timeout_ms == 0 || max_timeout_ms == 0 || default_timeout_ms > max_timeout_ms {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` has invalid timeout bounds",
                tool_cfg.name
            )));
        }

        let allow_env = read_bool_from_map(params, "allow_env")?.unwrap_or(true);
        let allowed_cwd_roots = parse_workspace_relative_roots(
            params.get("allowed_cwd_roots"),
            &workshop_cfg.workspace_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.workspace_root.clone()]);

        Ok(Self {
            default_timeout_ms,
            max_timeout_ms,
            allow_env,
            allowed_cwd_roots,
        })
    }
}

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
pub struct ExecBashTool {
    cfg: AgentWorkshopConfig,
    policy: ExecBashPolicy,
    session_store: Arc<AgentSessionMgr>,
}

impl ExecBashTool {
    pub fn from_tool_config(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
        session_store: Arc<AgentSessionMgr>,
    ) -> Result<Self, AgentToolError> {
        Ok(Self {
            cfg: cfg.clone(),
            policy: ExecBashPolicy::from_tool_config(cfg, tool_cfg)?,
            session_store,
        })
    }
}

#[async_trait]
impl AgentTool for ExecBashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EXEC_BASH.to_string(),
            description: "Run shell command.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout_ms": { "type": "integer", "minimum": 1 },
                    "env": {
                        "type": "object",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["command"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, AgentToolError> {
        let command = require_string(&args, "command")?;
        let session_id = ctx
            .session_id
            .as_deref()
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(
                    "missing session context for exec_bash; runtime should bind TraceCtx.session_id"
                        .to_string(),
                )
            })?
            .to_string();
        let session_id = sanitize_exec_session_id(&session_id)?;
        let cwd = self.resolve_session_cwd(&session_id).await?;

        let timeout_ms =
            optional_u64(&args, "timeout_ms")?.unwrap_or(self.policy.default_timeout_ms);
        if timeout_ms == 0 || timeout_ms > self.policy.max_timeout_ms {
            return Err(AgentToolError::InvalidArgs(format!(
                "timeout_ms out of range: {} (max: {})",
                timeout_ms, self.policy.max_timeout_ms
            )));
        }

        let env_vars = parse_exec_env_args(&args, self.policy.allow_env)?;
        let run_result = self
            .run_tmux_bash(ctx, &session_id, &command, &cwd, timeout_ms, &env_vars)
            .await?;
        let (stdout, stdout_truncated) = truncate_bytes(
            &run_result.stdout,
            self.cfg.max_output_bytes.max(DEFAULT_MAX_OUTPUT_BYTES),
        );
        let (stderr, stderr_truncated) = truncate_bytes(
            &run_result.stderr,
            self.cfg.max_output_bytes.max(DEFAULT_MAX_OUTPUT_BYTES),
        );
        let ok = run_result.exit_code == 0;
        let details = if ok {
            tail_lines_limited(&stdout, EXEC_BASH_SUCCESS_DETAIL_LINES)
        } else {
            stderr.clone()
        };

        Ok(json!({
            "ok": ok,
            "exit_code": run_result.exit_code,
            "stdout": stdout,
            "stderr": stderr,
            "details": details,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "duration_ms": run_result.duration_ms,
            "command": command,
            "cwd": cwd.to_string_lossy().to_string(),
            "session_id": session_id,
            "tmux_session": run_result.tmux_session,
            "engine": "tmux"
        }))
    }
}

struct ExecBashTmuxRunResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    duration_ms: u64,
    tmux_session: String,
}

impl ExecBashTool {
    async fn resolve_session_cwd(&self, session_id: &str) -> Result<PathBuf, AgentToolError> {
        let Some(session) = self.session_store.get_session(session_id).await else {
            return Err(AgentToolError::InvalidArgs(format!(
                "session not found for exec_bash: {session_id}"
            )));
        };
        let raw_cwd = {
            let guard = session.lock().await;
            guard.cwd.clone()
        };
        let cwd = if raw_cwd.as_os_str().is_empty() {
            self.cfg.workspace_root.clone()
        } else if raw_cwd.is_absolute() {
            normalize_abs_path(&raw_cwd)
        } else {
            normalize_abs_path(&self.cfg.workspace_root.join(raw_cwd))
        };

        if !cwd.starts_with(&self.cfg.workspace_root) {
            return Err(AgentToolError::InvalidArgs(format!(
                "session cwd out of workspace scope: {}",
                cwd.display()
            )));
        }
        if !is_path_under_any(&cwd, &self.policy.allowed_cwd_roots) {
            return Err(AgentToolError::InvalidArgs(format!(
                "session cwd `{}` not allowed by workshop tool policy",
                cwd.display()
            )));
        }

        Ok(cwd)
    }

    async fn run_tmux_bash(
        &self,
        ctx: &TraceCtx,
        session_id: &str,
        command: &str,
        cwd: &Path,
        timeout_ms: u64,
        env_vars: &[(String, String)],
    ) -> Result<ExecBashTmuxRunResult, AgentToolError> {
        let tmux_session = build_tmux_session_name(session_id);
        let tmux_target = format!("{tmux_session}:0.0");
        ensure_tmux_session(&tmux_session, cwd).await?;

        let runtime_dir = self
            .cfg
            .workspace_root
            .join(".runtime")
            .join("exec_bash")
            .join(sanitize_token_for_id(session_id));
        fs::create_dir_all(&runtime_dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create exec runtime dir `{}` failed: {err}",
                runtime_dir.display()
            ))
        })?;

        let run_id = format!(
            "{}-{}-{}-{}",
            now_ms(),
            sanitize_token_for_id(&ctx.trace_id),
            sanitize_token_for_id(&ctx.behavior),
            ctx.step_idx
        );
        let stdout_path = runtime_dir.join(format!("{run_id}.stdout.log"));
        let stderr_path = runtime_dir.join(format!("{run_id}.stderr.log"));
        let script_path = runtime_dir.join(format!("{run_id}.exec.sh"));
        let script = build_tmux_exec_script(
            &run_id,
            &stdout_path,
            &stderr_path,
            cwd,
            command,
            env_vars,
            &self.cfg.bash_path,
        );
        fs::write(&script_path, script).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "write exec script `{}` failed: {err}",
                script_path.display()
            ))
        })?;

        clear_tmux_history(&tmux_target).await?;

        let invoke = format!(
            "{} {}",
            shell_single_quote(self.cfg.bash_path.to_string_lossy().as_ref()),
            shell_single_quote(script_path.to_string_lossy().as_ref())
        );
        send_tmux_command(&tmux_target, &invoke).await?;

        let started = Instant::now();
        let exit_code = match wait_tmux_exit_code(&tmux_target, &run_id, timeout_ms).await {
            Ok(code) => code,
            Err(err) => {
                let _ = interrupt_tmux_target(&tmux_target).await;
                let _ = fs::remove_file(&script_path).await;
                let _ = fs::remove_file(&stdout_path).await;
                let _ = fs::remove_file(&stderr_path).await;
                return Err(err);
            }
        };

        sleep(Duration::from_millis(30)).await;

        let stdout = fs::read(&stdout_path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read stdout log `{}` failed: {err}",
                stdout_path.display()
            ))
        })?;
        let stderr = fs::read(&stderr_path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read stderr log `{}` failed: {err}",
                stderr_path.display()
            ))
        })?;

        let _ = fs::remove_file(&script_path).await;
        let _ = fs::remove_file(&stdout_path).await;
        let _ = fs::remove_file(&stderr_path).await;

        Ok(ExecBashTmuxRunResult {
            exit_code,
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis() as u64,
            tmux_session,
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
                "required": ["path", "pos_chunk", "new_content", "mode"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, AgentToolError> {
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

        Ok(json!({
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
        }))
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
                "required": ["path", "content", "mode"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, AgentToolError> {
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

        Ok(json!({
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
        }))
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
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, AgentToolError> {
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

        let (selected_content, line_range_label) =
            if let Some((start, end)) = parse_line_range(&args)? {
                let lines = selected_content.lines().collect::<Vec<_>>();
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
            };

        let (content, truncated) =
            truncate_bytes(selected_content.as_bytes(), self.policy.max_read_bytes);

        Ok(json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "content": content,
            "matched": matched,
            "line_range": line_range_label,
            "bytes": full_content.len(),
            "truncated": truncated,
        }))
    }
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
        ctx: &TraceCtx,
        args: &Json,
        record: &FileWriteAuditRecord,
    ) -> Result<(), AgentToolError> {
        let worklog_tool = WorklogTool::new(self.worklog_cfg.clone())?;
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

        let _ = worklog_tool
            .call(
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

fn require_string(args: &Json, key: &str) -> Result<String, AgentToolError> {
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

fn optional_u64(args: &Json, key: &str) -> Result<Option<u64>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a positive integer")))
}

fn parse_edit_mode(args: &Json) -> Result<&'static str, AgentToolError> {
    let raw = require_string(args, "mode")?;
    normalize_edit_mode(raw.as_str())
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
    let raw = require_string(args, "mode")?;
    normalize_write_mode(raw.as_str())
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

fn parse_line_range(args: &Json) -> Result<Option<(usize, usize)>, AgentToolError> {
    let Some(raw) = args.get("range") else {
        return Ok(None);
    };
    let (start_u64, end_u64) = if let Some(obj) = raw.as_object() {
        let start = obj.get("start").and_then(|v| v.as_u64()).ok_or_else(|| {
            AgentToolError::InvalidArgs("range.start must be positive integer".to_string())
        })?;
        let end = obj.get("end").and_then(|v| v.as_u64()).unwrap_or(start);
        (start, end)
    } else if let Some(arr) = raw.as_array() {
        if arr.is_empty() || arr.len() > 2 {
            return Err(AgentToolError::InvalidArgs(
                "range array must be [start] or [start,end]".to_string(),
            ));
        }
        let start = arr[0].as_u64().ok_or_else(|| {
            AgentToolError::InvalidArgs("range[0] must be positive integer".to_string())
        })?;
        let end = if arr.len() == 2 {
            arr[1].as_u64().ok_or_else(|| {
                AgentToolError::InvalidArgs("range[1] must be positive integer".to_string())
            })?
        } else {
            start
        };
        (start, end)
    } else if let Some(num) = raw.as_u64() {
        (num, num)
    } else if let Some(text) = raw.as_str() {
        parse_line_range_text(text)?
    } else {
        return Err(AgentToolError::InvalidArgs(
            "range must be string/number/array/object".to_string(),
        ));
    };

    if start_u64 == 0 || end_u64 == 0 {
        return Err(AgentToolError::InvalidArgs(
            "range must be 1-based positive integers".to_string(),
        ));
    }
    if end_u64 < start_u64 {
        return Err(AgentToolError::InvalidArgs(format!(
            "invalid range: end({end_u64}) < start({start_u64})"
        )));
    }
    let start = u64_to_usize(start_u64)?;
    let end = u64_to_usize(end_u64)?;
    Ok(Some((start, end)))
}

fn parse_line_range_text(raw: &str) -> Result<(u64, u64), AgentToolError> {
    let parts = raw
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 2 {
        return Err(AgentToolError::InvalidArgs(format!(
            "invalid range text `{raw}`"
        )));
    }
    let start = parts[0]
        .parse::<u64>()
        .map_err(|_| AgentToolError::InvalidArgs(format!("invalid range start in `{raw}`")))?;
    let end = if parts.len() == 2 {
        parts[1]
            .parse::<u64>()
            .map_err(|_| AgentToolError::InvalidArgs(format!("invalid range end in `{raw}`")))?
    } else {
        start
    };
    Ok((start, end))
}

fn read_u64_from_map(
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

fn read_bool_from_map(
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

fn parse_workspace_relative_roots(
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

fn normalize_abs_path(path: &Path) -> PathBuf {
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

fn is_path_under_any(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

async fn read_text_file_lossy(path: &Path) -> Result<String, AgentToolError> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("read file failed: {err}")))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn truncate_bytes(input: &[u8], max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (String::from_utf8_lossy(input).to_string(), false);
    }
    (
        String::from_utf8_lossy(&input[..max_bytes]).to_string(),
        true,
    )
}

fn parse_exec_env_args(
    args: &Json,
    allow_env: bool,
) -> Result<Vec<(String, String)>, AgentToolError> {
    let Some(env) = args.get("env") else {
        return Ok(Vec::new());
    };
    if !allow_env {
        return Err(AgentToolError::InvalidArgs(
            "env injection is disabled by workshop tool policy".to_string(),
        ));
    }
    let env_obj = env.as_object().ok_or_else(|| {
        AgentToolError::InvalidArgs("env must be an object of string values".to_string())
    })?;
    let mut vars = Vec::with_capacity(env_obj.len());
    for (key, value) in env_obj {
        if key.trim().is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "env key cannot be empty".to_string(),
            ));
        }
        if !is_valid_shell_env_key(key) {
            return Err(AgentToolError::InvalidArgs(format!(
                "env key `{key}` is invalid, expected [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
        let value = value
            .as_str()
            .ok_or_else(|| AgentToolError::InvalidArgs(format!("env.{key} must be a string")))?;
        vars.push((key.clone(), value.to_string()));
    }
    Ok(vars)
}

fn sanitize_exec_session_id(raw: &str) -> Result<String, AgentToolError> {
    let session_id = raw.trim();
    if session_id.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot be empty".to_string(),
        ));
    }
    if session_id.len() > 180 {
        return Err(AgentToolError::InvalidArgs(
            "session_id too long (>180)".to_string(),
        ));
    }
    if session_id == "." || session_id == ".." {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot be `.` or `..`".to_string(),
        ));
    }
    if session_id.contains('/') || session_id.contains('\\') {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot contain path separators".to_string(),
        ));
    }
    if session_id.chars().any(|ch| ch.is_control()) {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot contain control characters".to_string(),
        ));
    }
    Ok(session_id.to_string())
}

fn sanitize_token_for_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(64));
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
        if out.len() >= 64 {
            break;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_tmux_session_name(session_id: &str) -> String {
    format!(
        "{}{}",
        EXEC_BASH_TMUX_SESSION_PREFIX,
        sanitize_token_for_id(session_id)
    )
}

fn shell_single_quote(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn build_tmux_exec_script(
    run_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
    cwd: &Path,
    command: &str,
    env_vars: &[(String, String)],
    bash_path: &Path,
) -> String {
    let mut lines = Vec::new();
    lines.push("#!/usr/bin/env bash".to_string());
    lines.push(format!("__od_run_id={}", shell_single_quote(run_id)));
    lines.push(format!(
        "__od_stdout={}",
        shell_single_quote(stdout_path.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_stderr={}",
        shell_single_quote(stderr_path.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_cwd={}",
        shell_single_quote(cwd.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_bash={}",
        shell_single_quote(bash_path.to_string_lossy().as_ref())
    ));
    lines.push("printf \"__OD_BEGIN__%s\\n\" \"$__od_run_id\"".to_string());
    lines.push("mkdir -p \"$(dirname \"$__od_stdout\")\"".to_string());
    lines.push(": > \"$__od_stdout\"".to_string());
    lines.push(": > \"$__od_stderr\"".to_string());
    lines.push("{".to_string());
    lines.push("  cd \"$__od_cwd\" || exit 97".to_string());
    for (key, value) in env_vars {
        lines.push(format!(
            "  export {}={}",
            key,
            shell_single_quote(value.as_str())
        ));
    }
    lines.push("  \"$__od_bash\" <<'__OD_COMMAND__'".to_string());
    lines.push(command.to_string());
    lines.push("__OD_COMMAND__".to_string());
    lines.push("} > >(tee \"$__od_stdout\") 2> >(tee \"$__od_stderr\" >&2)".to_string());
    lines.push("__od_ec=$?".to_string());
    lines.push("printf \"__OD_EXIT__%s:%s\\n\" \"$__od_run_id\" \"$__od_ec\"".to_string());
    lines.push("exit 0".to_string());
    lines.join("\n")
}

async fn tmux_version_check() -> Result<(), AgentToolError> {
    let output = Command::new("tmux")
        .arg("-V")
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux unavailable: {err}")))?;
    if !output.status.success() {
        return Err(AgentToolError::ExecFailed(
            "tmux command exists but version probe failed".to_string(),
        ));
    }
    Ok(())
}

async fn ensure_tmux_session(session_name: &str, cwd: &Path) -> Result<(), AgentToolError> {
    tmux_version_check().await?;
    let has_session = Command::new("tmux")
        .args(["has-session", "-t", session_name])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux has-session failed: {err}")))?;
    if has_session.status.success() {
        return Ok(());
    }
    maybe_gc_stale_tmux_sessions().await;
    let output = Command::new("tmux")
        .args(["new-session", "-d", "-s", session_name, "-c"])
        .arg(cwd)
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux new-session failed: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "create tmux session `{session_name}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TmuxSessionInfo {
    name: String,
    last_activity_secs: Option<u64>,
}

async fn maybe_gc_stale_tmux_sessions() {
    let sessions = match list_tmux_sessions().await {
        Ok(list) => list,
        Err(err) => {
            warn!("tmux list-sessions failed before create; skip GC: {err}");
            return;
        }
    };
    if sessions.len() < EXEC_BASH_TMUX_GC_TRIGGER_COUNT {
        return;
    }
    let now_secs = now_unix_secs();
    let mut collected = 0usize;
    for session in sessions
        .iter()
        .filter(|item| should_gc_tmux_session(item, now_secs))
    {
        match kill_tmux_session(session.name.as_str()).await {
            Ok(()) => {
                collected = collected.saturating_add(1);
            }
            Err(err) => {
                warn!(
                    "tmux gc kill-session failed: session={} err={}",
                    session.name, err
                );
            }
        }
    }
    if collected > 0 {
        warn!("tmux gc reclaimed {} stale sessions", collected);
    }
}

async fn list_tmux_sessions() -> Result<Vec<TmuxSessionInfo>, AgentToolError> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", EXEC_BASH_TMUX_LIST_FORMAT])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux list-sessions failed: {err}")))?;
    if output.status.success() {
        return Ok(parse_tmux_session_list_output(
            String::from_utf8_lossy(&output.stdout).as_ref(),
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_ascii_lowercase().contains("no server running") {
        return Ok(Vec::new());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux list-sessions failed: {}",
        stderr
    )))
}

fn parse_tmux_session_list_output(raw: &str) -> Vec<TmuxSessionInfo> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let name = parts.next().unwrap_or_default().trim();
        if name.is_empty() {
            continue;
        }
        let activity = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<u64>().ok());
        out.push(TmuxSessionInfo {
            name: name.to_string(),
            last_activity_secs: activity,
        });
    }
    out
}

fn should_gc_tmux_session(session: &TmuxSessionInfo, now_secs: u64) -> bool {
    if !session.name.starts_with(EXEC_BASH_TMUX_SESSION_PREFIX) {
        return false;
    }
    let Some(last_activity) = session.last_activity_secs else {
        return false;
    };
    now_secs.saturating_sub(last_activity) >= EXEC_BASH_TMUX_GC_IDLE_SECS
}

async fn kill_tmux_session(session_name: &str) -> Result<(), AgentToolError> {
    let output = Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux kill-session failed: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_ascii_lowercase().contains("can't find session") {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux kill-session `{session_name}` failed: {}",
        stderr
    )))
}

async fn clear_tmux_history(target: &str) -> Result<(), AgentToolError> {
    let output = Command::new("tmux")
        .args(["clear-history", "-t", target])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux clear-history failed: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux clear-history `{target}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )))
}

async fn send_tmux_command(target: &str, command: &str) -> Result<(), AgentToolError> {
    let output = Command::new("tmux")
        .args(["send-keys", "-t", target, "--", command, "C-m"])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux send-keys failed: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux send-keys `{target}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )))
}

async fn wait_tmux_exit_code(
    target: &str,
    run_id: &str,
    timeout_ms: u64,
) -> Result<i32, AgentToolError> {
    let started = Instant::now();
    let timeout_dur = Duration::from_millis(timeout_ms);
    let marker = format!("__OD_EXIT__{run_id}:");
    loop {
        if started.elapsed() >= timeout_dur {
            return Err(AgentToolError::Timeout);
        }
        let output = Command::new("tmux")
            .args([
                "capture-pane",
                "-p",
                "-J",
                "-S",
                EXEC_BASH_CAPTURE_SCROLLBACK_LINES,
                "-t",
                target,
            ])
            .output()
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("tmux capture-pane failed: {err}"))
            })?;
        if !output.status.success() {
            return Err(AgentToolError::ExecFailed(format!(
                "tmux capture-pane `{target}` failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let pane = String::from_utf8_lossy(&output.stdout);
        if let Some(code) = parse_tmux_exit_code(pane.as_ref(), marker.as_str())? {
            return Ok(code);
        }
        sleep(Duration::from_millis(EXEC_BASH_TMUX_POLL_MS)).await;
    }
}

fn parse_tmux_exit_code(pane: &str, marker: &str) -> Result<Option<i32>, AgentToolError> {
    for line in pane.lines().rev() {
        let Some(pos) = line.find(marker) else {
            continue;
        };
        let raw = &line[pos + marker.len()..];
        let mut code_buf = String::new();
        for ch in raw.chars() {
            if ch == '-' && code_buf.is_empty() {
                code_buf.push(ch);
                continue;
            }
            if ch.is_ascii_digit() {
                code_buf.push(ch);
                continue;
            }
            if !code_buf.is_empty() {
                break;
            }
        }
        if code_buf.is_empty() || code_buf == "-" {
            return Err(AgentToolError::ExecFailed(format!(
                "invalid tmux exit code marker payload `{raw}`"
            )));
        }
        let code = code_buf.parse::<i32>().map_err(|err| {
            AgentToolError::ExecFailed(format!("invalid tmux exit code marker `{raw}`: {err}"))
        })?;
        return Ok(Some(code));
    }
    Ok(None)
}

async fn interrupt_tmux_target(target: &str) -> Result<(), AgentToolError> {
    let output = Command::new("tmux")
        .args(["send-keys", "-t", target, "C-c"])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux interrupt failed: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux interrupt `{target}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    )))
}

fn tail_lines_limited(text: &str, max_lines: usize) -> String {
    if max_lines == 0 {
        return String::new();
    }
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    lines[lines.len() - max_lines..].join("\n")
}

fn is_valid_shell_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

#[allow(dead_code)]
fn _defaults_for_docs() -> (PathBuf, u64, usize, usize) {
    (
        PathBuf::from(DEFAULT_BASH_PATH),
        DEFAULT_TIMEOUT_MS,
        DEFAULT_MAX_OUTPUT_BYTES,
        DEFAULT_MAX_DIFF_LINES.max(DEFAULT_MAX_FILE_WRITE_BYTES),
    )
}
