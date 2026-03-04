use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use log::warn;
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};

use crate::agent_session::AgentSessionMgr;
use crate::agent_tool::{AgentTool, AgentToolError, AgentToolManager, AgentToolResult, ToolSpec};
use crate::behavior::SessionRuntimeContext;
use crate::buildin_tool::{
    is_path_under_any, normalize_abs_path, optional_u64, parse_workspace_relative_roots,
    read_bool_from_map, read_u64_from_map, require_string, truncate_bytes, TOOL_EXEC_BASH,
};
use crate::workspace::workshop::{AgentWorkshopConfig, WorkshopToolConfig};

const DEFAULT_MAX_OUTPUT_BYTES: usize = 32 * 1024;
const EXEC_BASH_SUCCESS_DETAIL_LINES: usize = 16;
const EXEC_BASH_TMUX_POLL_MS: u64 = 120;
const EXEC_BASH_CAPTURE_SCROLLBACK_LINES: &str = "-6000";
const EXEC_BASH_TMUX_SESSION_PREFIX: &str = "od_";
const EXEC_BASH_TMUX_GC_TRIGGER_COUNT: usize = 16;
const EXEC_BASH_TMUX_GC_IDLE_SECS: u64 = 24 * 60 * 60;
const EXEC_BASH_TMUX_LIST_FORMAT: &str = "#{session_name}\t#{session_activity}";

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

#[derive(Clone)]
pub struct ExecBashTool {
    cfg: AgentWorkshopConfig,
    policy: ExecBashPolicy,
    session_store: Arc<AgentSessionMgr>,
    tool_mgr: AgentToolManager,
}

impl ExecBashTool {
    pub fn from_tool_config(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
        session_store: Arc<AgentSessionMgr>,
        tool_mgr: AgentToolManager,
    ) -> Result<Self, AgentToolError> {
        Ok(Self {
            cfg: cfg.clone(),
            policy: ExecBashPolicy::from_tool_config(cfg, tool_cfg)?,
            session_store,
            tool_mgr,
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
            usage: Some(
                "exec command=\"<shell command>\" [timeout_ms=<ms>] [env=<json object>]"
                    .to_string(),
            ),
        }
    }

    fn support_bash(&self) -> bool {
        false
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        true
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let command = require_string(&args, "command")?;
        let session_id = sanitize_exec_session_id(ctx.session_id.as_str())?;
        let pwd = self.resolve_session_pwd(&session_id).await?;

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
            .run_command_lines(ctx, &session_id, &command, &pwd, timeout_ms, &env_vars)
            .await?;
        self.persist_session_pwd(&session_id, &run_result.pwd).await;
        let pwd = run_result.pwd.clone();
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

        let details_json = json!({
            "ok": ok,
            "exit_code": run_result.exit_code,
            "stdout": stdout,
            "stderr": stderr,
            "details": details,
            "stdout_truncated": stdout_truncated,
            "stderr_truncated": stderr_truncated,
            "duration_ms": run_result.duration_ms,
            "command": command,
            "cwd": pwd.to_string_lossy().to_string(),
            "pwd": pwd.to_string_lossy().to_string(),
            "session_id": session_id,
            "tmux_session": run_result.tmux_session,
            "engine": run_result.engine,
            "line_results": run_result.line_results,
        });
        let summary = if ok {
            format!("OK (exit=0, {}ms)", run_result.duration_ms)
        } else {
            format!("FAILED (exit={})", run_result.exit_code)
        };
        let stdout_prompt = (!stdout.trim().is_empty()).then_some(stdout.clone());
        let stderr_prompt = (!stderr.trim().is_empty()).then_some(stderr.clone());
        Ok(AgentToolResult::from_details(details_json)
            .with_cmd_line(command)
            .with_result(summary)
            .with_stdout(stdout_prompt)
            .with_stderr(stderr_prompt))
    }
}

struct ExecBashRunResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    duration_ms: u64,
    pwd: PathBuf,
    tmux_session: String,
    engine: &'static str,
    line_results: Vec<Json>,
}

struct ExecBashTmuxRunResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    duration_ms: u64,
    tmux_session: String,
}

impl ExecBashTool {
    async fn run_command_lines(
        &self,
        ctx: &SessionRuntimeContext,
        session_id: &str,
        command: &str,
        pwd: &Path,
        timeout_ms: u64,
        env_vars: &[(String, String)],
    ) -> Result<ExecBashRunResult, AgentToolError> {
        let mut aggregate_stdout = Vec::<u8>::new();
        let mut aggregate_stderr = Vec::<u8>::new();
        let mut aggregate_duration_ms: u64 = 0;
        let mut aggregate_exit_code: i32 = 0;
        let mut tmux_session = String::new();
        let mut used_tmux = false;
        let mut used_tool = false;
        let mut line_results = Vec::<Json>::new();
        let mut has_non_empty_line = false;
        let mut current_pwd = self.resolve_tmux_pane_cwd(session_id, pwd).await;

        for (idx, raw_line) in command.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            has_non_empty_line = true;

            current_pwd = self.resolve_tmux_pane_cwd(session_id, current_pwd.as_path()).await;

            let command_name = AgentToolManager::parse_bash_command_name(line);
            if let Some(tool_name) = self.tool_mgr.resolve_bash_registered_tool_name(line) {
                if tool_name != TOOL_EXEC_BASH {
                    let pane_pwd = current_pwd.clone();
                    let tool_result = self
                        .tool_mgr
                        .call_tool_from_bash_line_with_cwd(ctx, line, Some(pane_pwd.as_path()))
                        .await;
                    match tool_result {
                        Ok(Some(result)) => {
                            used_tool = true;
                            let json_line = serde_json::to_string(&result.details)
                                .unwrap_or_else(|_| "{\"ok\":false}".to_string());
                            aggregate_stdout.extend_from_slice(json_line.as_bytes());
                            aggregate_stdout.push(b'\n');
                            let rendered = result.render_prompt();
                            if !rendered.trim().is_empty() {
                                aggregate_stdout.extend_from_slice(rendered.as_bytes());
                                aggregate_stdout.push(b'\n');
                            }
                            line_results.push(json!({
                                "line": idx + 1,
                                "mode": "tool",
                                "command": line,
                                "command_name": command_name.clone().unwrap_or_default(),
                                "tool_name": tool_name,
                                "cwd": pane_pwd.to_string_lossy().to_string(),
                                "ok": true,
                            }));
                            continue;
                        }
                        Ok(None) => {}
                        Err(err) => {
                            aggregate_exit_code = 1;
                            let err_text = err.to_string();
                            aggregate_stderr.extend_from_slice(err_text.as_bytes());
                            if !err_text.ends_with('\n') {
                                aggregate_stderr.push(b'\n');
                            }
                            line_results.push(json!({
                                "line": idx + 1,
                                "mode": "tool",
                                "command": line,
                                "command_name": command_name.unwrap_or_default(),
                                "tool_name": tool_name,
                                "cwd": pane_pwd.to_string_lossy().to_string(),
                                "ok": false,
                                "error": err_text,
                            }));
                            return Ok(ExecBashRunResult {
                                exit_code: aggregate_exit_code,
                                stdout: aggregate_stdout,
                                stderr: aggregate_stderr,
                                duration_ms: aggregate_duration_ms,
                                pwd: current_pwd,
                                tmux_session,
                                engine: if used_tmux { "tmux+tool" } else { "tool" },
                                line_results,
                            });
                        }
                    }
                }
            }

            let run_result = self
                .run_tmux_bash(ctx, session_id, line, current_pwd.as_path(), timeout_ms, env_vars)
                .await?;
            used_tmux = true;
            tmux_session = run_result.tmux_session.clone();
            aggregate_duration_ms = aggregate_duration_ms.saturating_add(run_result.duration_ms);
            aggregate_stdout.extend_from_slice(&run_result.stdout);
            aggregate_stderr.extend_from_slice(&run_result.stderr);
            current_pwd = self.resolve_tmux_pane_cwd(session_id, current_pwd.as_path()).await;
            line_results.push(json!({
                "line": idx + 1,
                "mode": "bash",
                "command": line,
                "command_name": command_name.unwrap_or_default(),
                "exit_code": run_result.exit_code,
                "cwd": current_pwd.to_string_lossy().to_string(),
            }));
            if run_result.exit_code != 0 {
                aggregate_exit_code = run_result.exit_code;
                return Ok(ExecBashRunResult {
                    exit_code: aggregate_exit_code,
                    stdout: aggregate_stdout,
                    stderr: aggregate_stderr,
                    duration_ms: aggregate_duration_ms,
                    pwd: current_pwd,
                    tmux_session,
                    engine: if used_tool { "tmux+tool" } else { "tmux" },
                    line_results,
                });
            }
        }

        if !has_non_empty_line {
            let run_result = self
                .run_tmux_bash(ctx, session_id, command, current_pwd.as_path(), timeout_ms, env_vars)
                .await?;
            used_tmux = true;
            tmux_session = run_result.tmux_session.clone();
            aggregate_duration_ms = aggregate_duration_ms.saturating_add(run_result.duration_ms);
            aggregate_stdout.extend_from_slice(&run_result.stdout);
            aggregate_stderr.extend_from_slice(&run_result.stderr);
            aggregate_exit_code = run_result.exit_code;
            current_pwd = self.resolve_tmux_pane_cwd(session_id, current_pwd.as_path()).await;
        }

        Ok(ExecBashRunResult {
            exit_code: aggregate_exit_code,
            stdout: aggregate_stdout,
            stderr: aggregate_stderr,
            duration_ms: aggregate_duration_ms,
            pwd: current_pwd,
            tmux_session,
            engine: if used_tmux && used_tool {
                "tmux+tool"
            } else if used_tool {
                "tool"
            } else {
                "tmux"
            },
            line_results,
        })
    }

    async fn resolve_tmux_pane_cwd(&self, session_id: &str, fallback_cwd: &Path) -> PathBuf {
        let tmux_session = build_tmux_session_name(session_id);
        if let Err(err) = ensure_tmux_session(&tmux_session, fallback_cwd).await {
            warn!(
                "resolve tmux pane cwd fallback to session cwd: session={} err={}",
                session_id, err
            );
            return fallback_cwd.to_path_buf();
        }
        let tmux_target = format!("{tmux_session}:0.0");
        match read_tmux_pane_current_path(&tmux_target).await {
            Ok(Some(path)) => self
                .align_tmux_path_to_workspace(path.as_path())
                .await
                .unwrap_or_else(|| fallback_cwd.to_path_buf()),
            Ok(None) => fallback_cwd.to_path_buf(),
            Err(err) => {
                warn!(
                    "read tmux pane current_path failed, fallback to session cwd: session={} err={}",
                    session_id, err
                );
                fallback_cwd.to_path_buf()
            }
        }
    }

    async fn align_tmux_path_to_workspace(&self, tmux_path: &Path) -> Option<PathBuf> {
        let normalized = normalize_abs_path(tmux_path);
        if normalized.starts_with(&self.cfg.workspace_root) {
            return Some(normalized);
        }

        // tmux may report a different absolute alias (e.g. /private/var vs /var on macOS).
        // Canonicalize and remap back under workspace_root so downstream policy checks remain stable.
        let canonical_tmux = fs::canonicalize(tmux_path).await.ok()?;
        let canonical_root = fs::canonicalize(&self.cfg.workspace_root).await.ok()?;
        let relative = canonical_tmux.strip_prefix(&canonical_root).ok()?;
        Some(normalize_abs_path(&self.cfg.workspace_root.join(relative)))
    }

    async fn resolve_session_pwd(&self, session_id: &str) -> Result<PathBuf, AgentToolError> {
        let Some(session) = self.session_store.get_session(session_id).await else {
            return Err(AgentToolError::InvalidArgs(format!(
                "session not found for exec_bash: {session_id}"
            )));
        };
        let raw_pwd = {
            let guard = session.lock().await;
            guard.pwd.clone()
        };
        let pwd = if raw_pwd.as_os_str().is_empty() {
            self.cfg.workspace_root.clone()
        } else if raw_pwd.is_absolute() {
            normalize_abs_path(&raw_pwd)
        } else {
            normalize_abs_path(&self.cfg.workspace_root.join(raw_pwd))
        };

        if !pwd.starts_with(&self.cfg.workspace_root) {
            return Err(AgentToolError::InvalidArgs(format!(
                "session pwd out of workspace scope: {}",
                pwd.display()
            )));
        }
        if !is_path_under_any(&pwd, &self.policy.allowed_cwd_roots) {
            return Err(AgentToolError::InvalidArgs(format!(
                "session pwd `{}` not allowed by workshop tool policy",
                pwd.display()
            )));
        }

        Ok(pwd)
    }

    async fn persist_session_pwd(&self, session_id: &str, pwd: &Path) {
        if !pwd.starts_with(&self.cfg.workspace_root)
            || !is_path_under_any(pwd, &self.policy.allowed_cwd_roots)
        {
            return;
        }
        let Some(session) = self.session_store.get_session(session_id).await else {
            return;
        };
        {
            let mut guard = session.lock().await;
            guard.pwd = pwd.to_path_buf();
        }
        if let Err(err) = self.session_store.save_session(session_id).await {
            warn!(
                "persist exec_bash session pwd failed: session={} pwd={} err={}",
                session_id,
                pwd.display(),
                err
            );
        }
    }

    async fn run_tmux_bash(
        &self,
        ctx: &SessionRuntimeContext,
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
        );
        fs::write(&script_path, script).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "write exec script `{}` failed: {err}",
                script_path.display()
            ))
        })?;

        clear_tmux_history(&tmux_target).await?;

        let invoke = format!(". {}", shell_single_quote(script_path.to_string_lossy().as_ref()));
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
) -> String {
    let mut lines = Vec::new();
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
    lines.push("printf \"__OD_BEGIN__%s\\n\" \"$__od_run_id\"".to_string());
    lines.push("mkdir -p \"$(dirname \"$__od_stdout\")\"".to_string());
    lines.push(": > \"$__od_stdout\"".to_string());
    lines.push(": > \"$__od_stderr\"".to_string());
    lines.push("{".to_string());
    lines.push(
        "  cd \"$__od_cwd\" || { echo \"cd failed: $__od_cwd\" >&2; false; }".to_string(),
    );
    for (key, value) in env_vars {
        lines.push(format!(
            "  export {}={}",
            key,
            shell_single_quote(value.as_str())
        ));
    }
    lines.push(command.to_string());
    lines.push("} > >(tee \"$__od_stdout\") 2> >(tee \"$__od_stderr\" >&2)".to_string());
    lines.push("__od_ec=$?".to_string());
    lines.push("printf \"__OD_EXIT__%s:%s\\n\" \"$__od_run_id\" \"$__od_ec\"".to_string());
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

async fn read_tmux_pane_current_path(target: &str) -> Result<Option<PathBuf>, AgentToolError> {
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            target,
            "#{pane_current_path}",
        ])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux display-message failed: {err}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lower = stderr.to_ascii_lowercase();
        if lower.contains("can't find pane") || lower.contains("can't find session") {
            return Ok(None);
        }
        return Err(AgentToolError::ExecFailed(format!(
            "tmux display-message `{target}` failed: {}",
            stderr
        )));
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(value)))
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
