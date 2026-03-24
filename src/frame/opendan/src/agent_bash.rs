use std::collections::{BTreeMap, HashSet};
use std::fs as std_fs;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{CreateTaskOptions, TaskManagerClient, TaskStatus, OPENDAN_SERVICE_NAME};
use log::{error, warn};
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};

use crate::agent_session::AgentSessionMgr;
use crate::agent_tool::{
    AgentTool, AgentToolError, AgentToolManager, AgentToolResult, AgentToolStatus,
    CliResultEnvelope, ToolSpec, TOOL_BIND_WORKSPACE, TOOL_CREATE_WORKSPACE, TOOL_EDIT_FILE,
    TOOL_GET_SESSION, TOOL_READ_FILE, TOOL_WRITE_FILE,
};
use crate::behavior::SessionRuntimeContext;
use crate::buildin_tool::{
    is_path_under_any, normalize_abs_path, optional_u64, parse_workspace_relative_roots,
    read_bool_from_map, read_u64_from_map, require_string, truncate_bytes, TOOL_EXEC_BASH,
};
use crate::workspace::workshop::{AgentWorkshopConfig, WorkshopToolConfig};

const DEFAULT_MAX_OUTPUT_BYTES: usize = 32 * 1024;
const EXEC_BASH_TMUX_POLL_MS: u64 = 120;
const EXEC_BASH_CAPTURE_SCROLLBACK_LINES: &str = "-6000";
const EXEC_BASH_TMUX_SESSION_PREFIX: &str = "od_";
const EXEC_BASH_TMUX_GC_TRIGGER_COUNT: usize = 16;
const EXEC_BASH_TMUX_GC_IDLE_SECS: u64 = 24 * 60 * 60;
const EXEC_BASH_TMUX_LIST_FORMAT: &str = "#{session_name}\t#{session_activity}";
const EXEC_BASH_PENDING_PARTIAL_LINES: usize = 10;
const EXEC_BASH_LONG_RUNNING_CHECK_AFTER_SECS: u64 = 5;
const EXEC_BASH_LONG_RUNNING_TASK_TYPE: &str = "exec_bash";
const EXEC_BASH_SESSION_TOOL_DIR: &str = ".tool";
const EXEC_BASH_AGENT_TOOL_BIN: &str = "agent_tool";
const EXEC_BASH_COMMAND_NOT_FOUND_PROXY: &str = "__command_not_found__";
const EXEC_BASH_AGENT_CLI_TOOL_NAMES: [&str; 7] = [
    TOOL_READ_FILE,
    TOOL_WRITE_FILE,
    TOOL_EDIT_FILE,
    TOOL_GET_SESSION,
    "todo",
    TOOL_CREATE_WORKSPACE,
    TOOL_BIND_WORKSPACE,
];
const EXEC_BASH_ALWAYS_AVAILABLE_CLI_TOOL_NAMES: [&str; 2] = ["check_task", "cancel_task"];
static EXEC_BASH_TASK_NAME_SEQ: AtomicU64 = AtomicU64::new(1);
static EXEC_BASH_RUN_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
struct PreparedSessionToolEnv {
    tool_dir: PathBuf,
    agent_tool_path: PathBuf,
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
            &workshop_cfg.agent_env_root,
        )?
        .unwrap_or_else(|| vec![workshop_cfg.agent_env_root.clone()]);

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
    task_mgr: Option<Arc<TaskManagerClient>>,
}

impl ExecBashTool {
    pub fn from_tool_config(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
        session_store: Arc<AgentSessionMgr>,
        _tool_mgr: AgentToolManager,
    ) -> Result<Self, AgentToolError> {
        Self::from_tool_config_with_task_mgr(cfg, tool_cfg, session_store, None)
    }

    pub fn from_tool_config_with_task_mgr(
        cfg: &AgentWorkshopConfig,
        tool_cfg: &WorkshopToolConfig,
        session_store: Arc<AgentSessionMgr>,
        task_mgr: Option<Arc<TaskManagerClient>>,
    ) -> Result<Self, AgentToolError> {
        Ok(Self {
            cfg: cfg.clone(),
            policy: ExecBashPolicy::from_tool_config(cfg, tool_cfg)?,
            session_store,
            task_mgr,
        })
    }

    async fn prepare_session_tool_env(
        &self,
        session_id: &str,
    ) -> Result<PreparedSessionToolEnv, AgentToolError> {
        let agent_tool_path = resolve_agent_tool_path().ok_or_else(|| {
            AgentToolError::ExecFailed("resolve agent_tool binary failed".to_string())
        })?;

        let tool_dir = self
            .cfg
            .agent_env_root
            .join("sessions")
            .join(session_id)
            .join(EXEC_BASH_SESSION_TOOL_DIR);
        let tool_names = self.resolve_session_cli_tool_names(session_id).await;
        sync_session_tool_links(&tool_dir, &agent_tool_path, &tool_names)?;

        Ok(PreparedSessionToolEnv {
            tool_dir,
            agent_tool_path,
        })
    }

    async fn resolve_session_cli_tool_names(&self, session_id: &str) -> Vec<String> {
        let Some(session) = self.session_store.get_session(session_id).await else {
            return default_agent_cli_tool_names();
        };
        let guard = session.lock().await;
        filter_session_cli_tool_names(&guard.loaded_tools)
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

        let session_tool_env = self.prepare_session_tool_env(&session_id).await?;
        let env_vars = parse_exec_env_args(&args, self.policy.allow_env)?;
        let env_vars = build_exec_env_vars(
            ctx,
            &self.cfg.agent_env_root,
            &env_vars,
            Some(&session_tool_env),
        );
        let run_result = self
            .run_command_lines(ctx, &session_id, &command, &pwd, timeout_ms, &env_vars)
            .await?;
        match run_result {
            ExecBashRunOutcome::Completed(run_result) => {
                self.persist_session_pwd(&session_id, &run_result.pwd).await;
                if let Some(parsed) = decode_exec_bash_json_result(&run_result, &command) {
                    return Ok(parsed);
                }
                Ok(build_default_exec_bash_result(
                    &run_result,
                    &command,
                    &session_id,
                    self.cfg.max_output_bytes.max(DEFAULT_MAX_OUTPUT_BYTES),
                ))
            }
            ExecBashRunOutcome::Pending(pending_result) => {
                self.persist_session_pwd(&session_id, &pending_result.pwd)
                    .await;
                let stdout_prompt = (!pending_result.partial_output.trim().is_empty())
                    .then_some(pending_result.partial_output.clone());
                Ok(AgentToolResult::from_details(json!({}))
                    .with_status(::agent_tool::AgentToolStatus::Pending)
                    .with_cmd_line(&command)
                    .with_result(format!(
                        "PENDING (long_running, check_after={}s)",
                        EXEC_BASH_LONG_RUNNING_CHECK_AFTER_SECS
                    ))
                    .with_task_id(pending_result.task_id)
                    .with_pending_reason(::agent_tool::AgentToolPendingReason::LongRunning)
                    .with_check_after(EXEC_BASH_LONG_RUNNING_CHECK_AFTER_SECS)
                    .with_partial_output(pending_result.partial_output)
                    .with_return_code(0)
                    .with_command_metadata_from_line(&command)
                    .with_output(stdout_prompt.unwrap_or_default()))
            }
        }
    }
}

struct ExecBashRunResult {
    exit_code: i32,
    stdout: Vec<u8>,
    mixed_output: String,
    duration_ms: u64,
    pwd: PathBuf,
    tmux_session: String,
    engine: &'static str,
    line_results: Vec<Json>,
}

#[derive(Clone, Debug)]
struct ExecCommandLine {
    line_no: usize,
    text: String,
}

#[derive(Clone, Debug)]
struct ExecBashAggregateState {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    mixed_output: String,
    duration_ms: u64,
    tmux_session: String,
    used_tmux: bool,
    used_tool: bool,
    line_results: Vec<Json>,
    current_pwd: PathBuf,
}

impl ExecBashAggregateState {
    fn new(current_pwd: PathBuf) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            mixed_output: String::new(),
            duration_ms: 0,
            tmux_session: String::new(),
            used_tmux: false,
            used_tool: false,
            line_results: Vec::new(),
            current_pwd,
        }
    }

    fn engine(&self) -> &'static str {
        if self.used_tmux && self.used_tool {
            "tmux+tool"
        } else if self.used_tool {
            "tool"
        } else {
            "tmux"
        }
    }
}

enum ExecBashRunOutcome {
    Completed(ExecBashRunResult),
    Pending(ExecBashPendingResult),
}

struct ExecBashPendingResult {
    task_id: String,
    partial_output: String,
    pwd: PathBuf,
}

struct ExecBashTmuxRunHandle {
    run_id: String,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    exit_code_path: PathBuf,
    script_path: PathBuf,
    tmux_session: String,
    tmux_target: String,
    pane_pid: Option<u32>,
    started: Instant,
}

struct ExecBashTmuxRunResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    mixed_output: String,
    duration_ms: u64,
    tmux_session: String,
}

struct ExecBashTmuxPendingRun {
    handle: ExecBashTmuxRunHandle,
    partial_output: String,
    duration_ms: u64,
}

enum ExecBashTmuxRunState {
    Completed(ExecBashTmuxRunResult),
    Pending(ExecBashTmuxPendingRun),
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
    ) -> Result<ExecBashRunOutcome, AgentToolError> {
        let mut state =
            ExecBashAggregateState::new(self.resolve_tmux_pane_cwd(session_id, pwd).await);
        let command_lines = collect_exec_command_lines(command);

        for (index, line) in command_lines.iter().enumerate() {
            state.current_pwd = self
                .resolve_tmux_pane_cwd(session_id, state.current_pwd.as_path())
                .await;
            let command_name = AgentToolManager::parse_bash_command_name(&line.text);

            match self
                .run_tmux_bash(
                    ctx,
                    session_id,
                    &line.text,
                    state.current_pwd.as_path(),
                    Some(timeout_ms),
                    env_vars,
                )
                .await?
            {
                ExecBashTmuxRunState::Completed(run_result) => {
                    self.apply_tmux_run_result(
                        &mut state,
                        session_id,
                        line,
                        command_name.as_deref(),
                        run_result,
                    )
                    .await;
                    let exit_code = state
                        .line_results
                        .last()
                        .and_then(|item| item.get("exit_code"))
                        .and_then(Json::as_i64)
                        .unwrap_or(0) as i32;
                    if exit_code != 0 {
                        return Ok(ExecBashRunOutcome::Completed(
                            self.build_completed_result(state, exit_code),
                        ));
                    }
                }
                ExecBashTmuxRunState::Pending(pending_run) => {
                    let pending_result = self
                        .promote_pending_run_to_task(
                            ctx,
                            session_id,
                            command,
                            timeout_ms,
                            env_vars,
                            command_lines.clone(),
                            index,
                            state,
                            pending_run,
                        )
                        .await?;
                    return Ok(ExecBashRunOutcome::Pending(pending_result));
                }
            }
        }

        Ok(ExecBashRunOutcome::Completed(
            self.build_completed_result(state, 0),
        ))
    }

    fn build_completed_result(
        &self,
        state: ExecBashAggregateState,
        exit_code: i32,
    ) -> ExecBashRunResult {
        let engine = state.engine();
        ExecBashRunResult {
            exit_code,
            stdout: state.stdout,
            mixed_output: state.mixed_output,
            duration_ms: state.duration_ms,
            pwd: state.current_pwd,
            tmux_session: state.tmux_session,
            engine,
            line_results: state.line_results,
        }
    }
    async fn apply_tmux_run_result(
        &self,
        state: &mut ExecBashAggregateState,
        session_id: &str,
        line: &ExecCommandLine,
        command_name: Option<&str>,
        run_result: ExecBashTmuxRunResult,
    ) {
        state.used_tmux = true;
        state.tmux_session = run_result.tmux_session.clone();
        state.duration_ms = state.duration_ms.saturating_add(run_result.duration_ms);
        state.stdout.extend_from_slice(&run_result.stdout);
        state.stderr.extend_from_slice(&run_result.stderr);
        append_text_block(&mut state.mixed_output, run_result.mixed_output.as_str());
        state.current_pwd = self
            .resolve_tmux_pane_cwd(session_id, state.current_pwd.as_path())
            .await;
        state.line_results.push(json!({
            "line": line.line_no,
            "mode": "bash",
            "command": line.text,
            "command_name": command_name.unwrap_or_default(),
            "exit_code": run_result.exit_code,
            "cwd": state.current_pwd.to_string_lossy().to_string(),
        }));
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
        if normalized.starts_with(&self.cfg.agent_env_root) {
            return Some(normalized);
        }

        // tmux may report a different absolute alias (e.g. /private/var vs /var on macOS).
        // Canonicalize and remap back under agent_env_root so downstream policy checks remain stable.
        let canonical_tmux = fs::canonicalize(tmux_path).await.ok()?;
        let canonical_root = fs::canonicalize(&self.cfg.agent_env_root).await.ok()?;
        let relative = canonical_tmux.strip_prefix(&canonical_root).ok()?;
        Some(normalize_abs_path(&self.cfg.agent_env_root.join(relative)))
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
            self.cfg.agent_env_root.clone()
        } else if raw_pwd.is_absolute() {
            normalize_abs_path(&raw_pwd)
        } else {
            normalize_abs_path(&self.cfg.agent_env_root.join(raw_pwd))
        };

        if !pwd.starts_with(&self.cfg.agent_env_root) {
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
        if !pwd.starts_with(&self.cfg.agent_env_root)
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
        timeout_ms: Option<u64>,
        env_vars: &[(String, String)],
    ) -> Result<ExecBashTmuxRunState, AgentToolError> {
        let tmux_session = build_tmux_session_name(session_id);
        let tmux_target = format!("{tmux_session}:0.0");
        ensure_tmux_session(&tmux_session, cwd).await?;

        let runtime_dir = self
            .cfg
            .agent_env_root
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
            "{}-{}-{}-{}-{}",
            now_ms(),
            sanitize_token_for_id(&ctx.trace_id),
            sanitize_token_for_id(&ctx.behavior),
            ctx.step_idx,
            EXEC_BASH_RUN_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let stdout_path = runtime_dir.join(format!("{run_id}.stdout.log"));
        let stderr_path = runtime_dir.join(format!("{run_id}.stderr.log"));
        let exit_code_path = runtime_dir.join(format!("{run_id}.exit.code"));
        let script_path = runtime_dir.join(format!("{run_id}.exec.sh"));
        let script = build_tmux_exec_script(
            &run_id,
            &stdout_path,
            &stderr_path,
            &exit_code_path,
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

        if let Err(err) = clear_tmux_history(&tmux_target).await {
            warn!(
                "tmux clear-history failed before exec: session={} target={} err={}",
                session_id, tmux_target, err
            );
        }

        let invoke = format!(
            ". {}",
            shell_single_quote(script_path.to_string_lossy().as_ref())
        );
        send_tmux_command(&tmux_target, &invoke).await?;
        let pane_pid = read_tmux_pane_pid(&tmux_target).await.ok().flatten();

        let handle = ExecBashTmuxRunHandle {
            run_id,
            stdout_path,
            stderr_path,
            exit_code_path,
            script_path,
            tmux_session,
            tmux_target,
            pane_pid,
            started: Instant::now(),
        };
        self.await_tmux_run(handle, timeout_ms).await
    }

    async fn await_tmux_run(
        &self,
        handle: ExecBashTmuxRunHandle,
        timeout_ms: Option<u64>,
    ) -> Result<ExecBashTmuxRunState, AgentToolError> {
        match wait_tmux_exit_code(&handle, &handle.run_id, timeout_ms).await? {
            Some(exit_code) => {
                let result = read_tmux_run_result(&handle, exit_code).await?;
                cleanup_tmux_run_files(&handle).await;
                Ok(ExecBashTmuxRunState::Completed(result))
            }
            None => {
                let pane = capture_tmux_pane_output(&handle.tmux_target).await?;
                let partial_output = extract_tmux_run_output(pane.as_str(), handle.run_id.as_str());
                Ok(ExecBashTmuxRunState::Pending(ExecBashTmuxPendingRun {
                    duration_ms: handle.started.elapsed().as_millis() as u64,
                    partial_output: tail_lines_limited(
                        partial_output.trim_end(),
                        EXEC_BASH_PENDING_PARTIAL_LINES,
                    ),
                    handle,
                }))
            }
        }
    }

    async fn promote_pending_run_to_task(
        &self,
        ctx: &SessionRuntimeContext,
        session_id: &str,
        command: &str,
        timeout_ms: u64,
        env_vars: &[(String, String)],
        command_lines: Vec<ExecCommandLine>,
        current_index: usize,
        mut state: ExecBashAggregateState,
        pending_run: ExecBashTmuxPendingRun,
    ) -> Result<ExecBashPendingResult, AgentToolError> {
        let Some(task_mgr) = self.task_mgr.clone() else {
            let _ = interrupt_tmux_target(&pending_run.handle.tmux_target).await;
            cleanup_tmux_run_files(&pending_run.handle).await;
            return Err(AgentToolError::ExecFailed(
                "exec_bash exceeded timeout but task_mgr is unavailable".to_string(),
            ));
        };

        let current_line = command_lines.get(current_index).ok_or_else(|| {
            AgentToolError::ExecFailed("pending exec line is missing".to_string())
        })?;
        let pending_pwd = self
            .resolve_tmux_pane_cwd(session_id, state.current_pwd.as_path())
            .await;
        state.used_tmux = true;
        state.tmux_session = pending_run.handle.tmux_session.clone();
        state.current_pwd = pending_pwd.clone();
        state.line_results.push(json!({
            "line": current_line.line_no,
            "mode": "bash",
            "command": current_line.text,
            "command_name": AgentToolManager::parse_bash_command_name(&current_line.text).unwrap_or_default(),
            "cwd": pending_pwd.to_string_lossy().to_string(),
            "pending": true,
            "pending_reason": "long_running",
        }));

        let summary = format!(
            "exec_bash exceeded {}ms and continues in background",
            timeout_ms
        );
        let pending_partial_output = pending_run.partial_output.clone();
        let task = task_mgr
            .create_task(
                &format!(
                    "exec_bash:{}#{}",
                    sanitize_token_for_id(session_id),
                    next_exec_bash_task_name_suffix()
                ),
                EXEC_BASH_LONG_RUNNING_TASK_TYPE,
                Some(build_exec_bash_pending_task_data(
                    ctx,
                    session_id,
                    command,
                    timeout_ms,
                    &pending_pwd,
                    state.engine(),
                    &state.line_results,
                    &pending_run.partial_output,
                    pending_run.duration_ms,
                    &pending_run.handle,
                    &summary,
                )),
                ctx.agent_name.as_str(),
                OPENDAN_SERVICE_NAME,
                Some(CreateTaskOptions::with_root_id(
                    resolve_exec_bash_task_root_id(ctx.agent_name.as_str(), Some(session_id)),
                )),
            )
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("create exec_bash task failed: {err}"))
            })?;
        let _ = task_mgr
            .update_task(
                task.id,
                Some(TaskStatus::Running),
                Some(0.0),
                Some(summary.clone()),
                Some(build_exec_bash_pending_task_data(
                    ctx,
                    session_id,
                    command,
                    timeout_ms,
                    &pending_pwd,
                    state.engine(),
                    &state.line_results,
                    &pending_run.partial_output,
                    pending_run.duration_ms,
                    &pending_run.handle,
                    &summary,
                )),
            )
            .await;

        let runner = self.clone();
        let ctx = ctx.clone();
        let session_id = session_id.to_string();
        let command = command.to_string();
        let env_vars = env_vars.to_vec();
        tokio::spawn(async move {
            runner
                .finalize_long_running_task(
                    task_mgr,
                    task.id,
                    ctx,
                    session_id,
                    command,
                    timeout_ms,
                    env_vars,
                    command_lines,
                    current_index,
                    state,
                    pending_run,
                )
                .await;
        });

        Ok(ExecBashPendingResult {
            task_id: task.id.to_string(),
            partial_output: pending_partial_output,
            pwd: pending_pwd,
        })
    }

    async fn finalize_long_running_task(
        &self,
        task_mgr: Arc<TaskManagerClient>,
        task_id: i64,
        ctx: SessionRuntimeContext,
        session_id: String,
        command: String,
        timeout_ms: u64,
        env_vars: Vec<(String, String)>,
        command_lines: Vec<ExecCommandLine>,
        current_index: usize,
        mut state: ExecBashAggregateState,
        pending_run: ExecBashTmuxPendingRun,
    ) {
        let final_result = self
            .resume_pending_command_lines(
                &ctx,
                session_id.as_str(),
                env_vars.as_slice(),
                command_lines,
                current_index,
                &mut state,
                pending_run,
            )
            .await;

        match final_result {
            Ok(run_result) => {
                self.persist_session_pwd(session_id.as_str(), &run_result.pwd)
                    .await;
                let ok = run_result.exit_code == 0;
                let summary = if ok {
                    extract_single_tool_prompt_summary(&run_result)
                        .unwrap_or_else(|| format!("OK (exit=0, {}ms)", run_result.duration_ms))
                } else {
                    format!("FAILED (exit={})", run_result.exit_code)
                };
                let task_data = build_exec_bash_final_task_data(
                    session_id.as_str(),
                    &command,
                    timeout_ms,
                    &run_result,
                    self.cfg.max_output_bytes.max(DEFAULT_MAX_OUTPUT_BYTES),
                    &summary,
                );
                match task_mgr.get_task(task_id).await {
                    Ok(task) if task.status == TaskStatus::Canceled => {
                        warn!(
                            "skip finalizing canceled exec_bash task: task_id={} session={}",
                            task_id, session_id
                        );
                        return;
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!(
                            "reload exec_bash task before finalize failed: task_id={} session={} err={}",
                            task_id, session_id, err
                        );
                    }
                }
                let _ = task_mgr
                    .update_task(
                        task_id,
                        None,
                        Some(1.0),
                        Some(summary.clone()),
                        Some(task_data),
                    )
                    .await;
                if ok {
                    let _ = task_mgr.complete_task(task_id).await;
                } else {
                    let _ = task_mgr.mark_task_as_failed(task_id, &summary).await;
                }
            }
            Err(err) => {
                error!(
                    "exec_bash background task failed: task_id={} session={} err={}",
                    task_id, session_id, err
                );
                if let Ok(task) = task_mgr.get_task(task_id).await {
                    if task.status == TaskStatus::Canceled {
                        warn!(
                            "skip failing canceled exec_bash task: task_id={} session={}",
                            task_id, session_id
                        );
                        return;
                    }
                }
                let _ = task_mgr
                    .mark_task_as_failed(task_id, &err.to_string())
                    .await;
            }
        }
    }

    async fn resume_pending_command_lines(
        &self,
        ctx: &SessionRuntimeContext,
        session_id: &str,
        env_vars: &[(String, String)],
        command_lines: Vec<ExecCommandLine>,
        current_index: usize,
        state: &mut ExecBashAggregateState,
        pending_run: ExecBashTmuxPendingRun,
    ) -> Result<ExecBashRunResult, AgentToolError> {
        let current_line = command_lines
            .get(current_index)
            .ok_or_else(|| AgentToolError::ExecFailed("missing pending command line".to_string()))?
            .clone();
        let current_result = match self.await_tmux_run(pending_run.handle, None).await? {
            ExecBashTmuxRunState::Completed(result) => result,
            ExecBashTmuxRunState::Pending(_) => {
                return Err(AgentToolError::ExecFailed(
                    "background exec_bash unexpectedly stayed pending".to_string(),
                ))
            }
        };
        if !state.line_results.is_empty() {
            state.line_results.pop();
        }
        self.apply_tmux_run_result(
            state,
            session_id,
            &current_line,
            AgentToolManager::parse_bash_command_name(&current_line.text).as_deref(),
            current_result,
        )
        .await;
        let current_exit_code = state
            .line_results
            .last()
            .and_then(|item| item.get("exit_code"))
            .and_then(Json::as_i64)
            .unwrap_or(0) as i32;
        if current_exit_code != 0 {
            return Ok(self.build_completed_result(state.clone(), current_exit_code));
        }

        for line in command_lines.into_iter().skip(current_index + 1) {
            state.current_pwd = self
                .resolve_tmux_pane_cwd(session_id, state.current_pwd.as_path())
                .await;
            let bash_result = match self
                .run_tmux_bash(
                    ctx,
                    session_id,
                    &line.text,
                    state.current_pwd.as_path(),
                    None,
                    env_vars,
                )
                .await?
            {
                ExecBashTmuxRunState::Completed(result) => result,
                ExecBashTmuxRunState::Pending(_) => {
                    return Err(AgentToolError::ExecFailed(
                        "background exec_bash line unexpectedly entered pending".to_string(),
                    ))
                }
            };
            self.apply_tmux_run_result(
                state,
                session_id,
                &line,
                AgentToolManager::parse_bash_command_name(&line.text).as_deref(),
                bash_result,
            )
            .await;
            let exit_code = state
                .line_results
                .last()
                .and_then(|item| item.get("exit_code"))
                .and_then(Json::as_i64)
                .unwrap_or(0) as i32;
            if exit_code != 0 {
                return Ok(self.build_completed_result(state.clone(), exit_code));
            }
        }

        Ok(self.build_completed_result(state.clone(), 0))
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

fn build_exec_env_vars(
    ctx: &SessionRuntimeContext,
    agent_env_root: &Path,
    user_env: &[(String, String)],
    session_tool_env: Option<&PreparedSessionToolEnv>,
) -> Vec<(String, String)> {
    let mut merged = BTreeMap::<String, String>::new();
    for (key, value) in user_env {
        merged.insert(key.clone(), value.clone());
    }

    if let Some(tool_env) = session_tool_env {
        let base_path = merged
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        merged.insert(
            "PATH".to_string(),
            prepend_path_entry(tool_env.tool_dir.to_string_lossy().as_ref(), &base_path),
        );
        merged.insert(
            "OPENDAN_AGENT_BIN".to_string(),
            tool_env
                .agent_tool_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy()
                .to_string(),
        );
        merged.insert(
            "OPENDAN_AGENT_TOOL".to_string(),
            tool_env.agent_tool_path.to_string_lossy().to_string(),
        );
        merged.insert(
            "OPENDAN_SESSION_TOOL_PATH".to_string(),
            tool_env.tool_dir.to_string_lossy().to_string(),
        );
    }

    let agent_env = agent_env_root.to_string_lossy().to_string();
    merged.insert("OPENDAN_AGENT_ENV".to_string(), agent_env);
    merged.insert("OPENDAN_AGENT_ID".to_string(), ctx.agent_name.clone());
    merged.insert("OPENDAN_BEHAVIOR".to_string(), ctx.behavior.clone());
    merged.insert("OPENDAN_STEP_IDX".to_string(), ctx.step_idx.to_string());
    merged.insert("OPENDAN_WAKEUP_ID".to_string(), ctx.wakeup_id.clone());
    merged.insert("OPENDAN_SESSION_ID".to_string(), ctx.session_id.clone());
    merged.insert("OPENDAN_TRACE_ID".to_string(), ctx.trace_id.clone());

    merged.into_iter().collect()
}

fn resolve_agent_tool_path() -> Option<PathBuf> {
    if let Some(candidate) = std::env::var_os("CARGO_BIN_EXE_agent_tool").map(PathBuf::from) {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Some(root) = std::env::var_os("BUCKYOS_ROOT") {
        let candidate = PathBuf::from(root)
            .join("bin")
            .join("opendan")
            .join(EXEC_BASH_AGENT_TOOL_BIN);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let current_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))?;
    let preferred = current_dir.join(EXEC_BASH_AGENT_TOOL_BIN);
    if preferred.exists() {
        return Some(preferred);
    }
    if let Some(parent_dir) = current_dir.parent() {
        let parent_preferred = parent_dir.join(EXEC_BASH_AGENT_TOOL_BIN);
        if parent_preferred.exists() {
            return Some(parent_preferred);
        }
    }
    None
}

fn prepend_path_entry(entry: &str, base_path: &str) -> String {
    let entry = entry.trim();
    if entry.is_empty() {
        return base_path.to_string();
    }
    let already_present = base_path.split(':').any(|item| item == entry);
    if base_path.is_empty() {
        return entry.to_string();
    }
    if already_present {
        return base_path.to_string();
    }
    format!("{entry}:{base_path}")
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

fn default_agent_cli_tool_names() -> Vec<String> {
    EXEC_BASH_AGENT_CLI_TOOL_NAMES
        .iter()
        .chain(EXEC_BASH_ALWAYS_AVAILABLE_CLI_TOOL_NAMES.iter())
        .map(|name| (*name).to_string())
        .collect()
}

fn filter_session_cli_tool_names(raw_names: &[String]) -> Vec<String> {
    if raw_names.is_empty() {
        return default_agent_cli_tool_names();
    }

    let mut filtered = Vec::new();
    let mut seen = HashSet::<String>::new();
    for raw_name in raw_names {
        let tool_name = raw_name.trim();
        if tool_name.is_empty() {
            continue;
        }
        if EXEC_BASH_AGENT_CLI_TOOL_NAMES.contains(&tool_name) && seen.insert(tool_name.to_string())
        {
            filtered.push(tool_name.to_string());
        }
    }

    for tool_name in EXEC_BASH_ALWAYS_AVAILABLE_CLI_TOOL_NAMES {
        if seen.insert(tool_name.to_string()) {
            filtered.push(tool_name.to_string());
        }
    }
    filtered
}

fn sync_session_tool_links(
    tool_dir: &Path,
    agent_tool_path: &Path,
    tool_names: &[String],
) -> Result<(), AgentToolError> {
    #[cfg(not(unix))]
    {
        let _ = (tool_dir, agent_tool_path, tool_names);
        return Err(AgentToolError::ExecFailed(
            "session-scoped tool links require unix symlink support".to_string(),
        ));
    }

    #[cfg(unix)]
    {
        std_fs::create_dir_all(tool_dir).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create session tool dir `{}` failed: {err}",
                tool_dir.display()
            ))
        })?;

        let desired = tool_names
            .iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect::<HashSet<_>>();

        for entry in std_fs::read_dir(tool_dir).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read session tool dir `{}` failed: {err}",
                tool_dir.display()
            ))
        })? {
            let entry = entry.map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "read session tool dir entry `{}` failed: {err}",
                    tool_dir.display()
                ))
            })?;
            let entry_name = entry.file_name().to_string_lossy().to_string();
            if desired.contains(entry_name.as_str()) {
                continue;
            }
            remove_fs_entry(entry.path().as_path())?;
        }

        for tool_name in tool_names {
            let tool_name = tool_name.trim();
            if tool_name.is_empty() {
                continue;
            }
            let link_path = tool_dir.join(tool_name);
            let needs_refresh = match std_fs::read_link(&link_path) {
                Ok(target) => target != agent_tool_path,
                Err(_) => true,
            };
            if !needs_refresh {
                continue;
            }
            if link_path.exists() || std_fs::symlink_metadata(&link_path).is_ok() {
                remove_fs_entry(link_path.as_path())?;
            }
            unix_fs::symlink(agent_tool_path, &link_path).map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "link session tool `{}` -> `{}` failed: {err}",
                    link_path.display(),
                    agent_tool_path.display()
                ))
            })?;
        }

        Ok(())
    }
}

fn remove_fs_entry(path: &Path) -> Result<(), AgentToolError> {
    let metadata = std_fs::symlink_metadata(path).map_err(|err| {
        AgentToolError::ExecFailed(format!("stat `{}` failed: {err}", path.display()))
    })?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        std_fs::remove_dir_all(path).map_err(|err| {
            AgentToolError::ExecFailed(format!("remove dir `{}` failed: {err}", path.display()))
        })?;
    } else {
        std_fs::remove_file(path).map_err(|err| {
            AgentToolError::ExecFailed(format!("remove file `{}` failed: {err}", path.display()))
        })?;
    }
    Ok(())
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

fn append_text_block(target: &mut String, block: &str) {
    let block = block.trim();
    if block.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(block);
}

fn extract_tmux_run_output(pane: &str, run_id: &str) -> String {
    let begin_marker = format!("__OD_BEGIN__{run_id}");
    let end_marker = format!("__OD_EXIT__{run_id}:");
    let mut output = pane;

    if let Some(begin_pos) = output.find(begin_marker.as_str()) {
        output = &output[begin_pos + begin_marker.len()..];
        output = output
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or_default();
    }

    if let Some(end_pos) = output.rfind(end_marker.as_str()) {
        output = &output[..end_pos];
    }

    output.trim().to_string()
}

fn mixed_output_from_tmux_and_logs(pane_output: String, stdout: &[u8], stderr: &[u8]) -> String {
    let pane_output = pane_output.trim().to_string();
    if !pane_output.is_empty() {
        return pane_output;
    }

    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
        (true, true) => String::new(),
    }
}

fn build_tmux_exec_script(
    run_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
    exit_code_path: &Path,
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
        "__od_exit_code={}",
        shell_single_quote(exit_code_path.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_cwd={}",
        shell_single_quote(cwd.to_string_lossy().as_ref())
    ));
    lines.push("printf \"__OD_BEGIN__%s\\n\" \"$__od_run_id\"".to_string());
    lines.push("mkdir -p \"$(dirname \"$__od_stdout\")\"".to_string());
    lines.push(": > \"$__od_stdout\"".to_string());
    lines.push(": > \"$__od_stderr\"".to_string());
    lines.push("rm -f \"$__od_exit_code\"".to_string());
    lines.push("{".to_string());
    lines.push("  cd \"$__od_cwd\" || { echo \"cd failed: $__od_cwd\" >&2; false; }".to_string());
    for (key, value) in env_vars {
        lines.push(format!(
            "  export {}={}",
            key,
            shell_single_quote(value.as_str())
        ));
    }
    lines.push("  if [ -n \"${OPENDAN_AGENT_TOOL:-}\" ]; then".to_string());
    lines.push("    command_not_found_handle() {".to_string());
    lines.push("      local __od_tool=\"${OPENDAN_AGENT_TOOL:-}\"".to_string());
    lines.push(format!(
        "      \"$__od_tool\" {} \"$@\"",
        shell_single_quote(EXEC_BASH_COMMAND_NOT_FOUND_PROXY)
    ));
    lines.push("      local __od_ec=$?".to_string());
    lines.push("      if [ \"$__od_ec\" -eq 127 ]; then".to_string());
    lines.push("        printf 'bash: %s: command not found\\n' \"$1\" >&2".to_string());
    lines.push("      fi".to_string());
    lines.push("      return \"$__od_ec\"".to_string());
    lines.push("    }".to_string());
    lines.push("  fi".to_string());
    lines.push(command.to_string());
    lines.push("} > >(tee \"$__od_stdout\") 2> >(tee \"$__od_stderr\" >&2)".to_string());
    lines.push("__od_ec=$?".to_string());
    lines.push("printf \"%s\\n\" \"$__od_ec\" > \"$__od_exit_code\"".to_string());
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

async fn capture_tmux_pane_output(target: &str) -> Result<String, AgentToolError> {
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
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux capture-pane failed: {err}")))?;
    if !output.status.success() {
        return Err(AgentToolError::ExecFailed(format!(
            "tmux capture-pane `{target}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn read_tmux_pane_pid(target: &str) -> Result<Option<u32>, AgentToolError> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "-t", target, "#{pane_pid}"])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux display-message failed: {err}")))?;
    if !output.status.success() {
        return Err(AgentToolError::ExecFailed(format!(
            "tmux display-message `{target}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(raw.parse::<u32>().ok())
}

async fn read_tmux_run_result(
    handle: &ExecBashTmuxRunHandle,
    exit_code: i32,
) -> Result<ExecBashTmuxRunResult, AgentToolError> {
    sleep(Duration::from_millis(30)).await;
    let pane = capture_tmux_pane_output(&handle.tmux_target).await?;
    let stdout = fs::read(&handle.stdout_path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "read stdout log `{}` failed: {err}",
            handle.stdout_path.display()
        ))
    })?;
    let stderr = fs::read(&handle.stderr_path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "read stderr log `{}` failed: {err}",
            handle.stderr_path.display()
        ))
    })?;
    let mixed_output = mixed_output_from_tmux_and_logs(
        extract_tmux_run_output(pane.as_str(), handle.run_id.as_str()),
        stdout.as_slice(),
        stderr.as_slice(),
    );
    Ok(ExecBashTmuxRunResult {
        exit_code,
        stdout,
        stderr,
        mixed_output,
        duration_ms: handle.started.elapsed().as_millis() as u64,
        tmux_session: handle.tmux_session.clone(),
    })
}

async fn cleanup_tmux_run_files(handle: &ExecBashTmuxRunHandle) {
    let _ = fs::remove_file(&handle.script_path).await;
    let _ = fs::remove_file(&handle.stdout_path).await;
    let _ = fs::remove_file(&handle.stderr_path).await;
    let _ = fs::remove_file(&handle.exit_code_path).await;
}

async fn wait_tmux_exit_code(
    handle: &ExecBashTmuxRunHandle,
    run_id: &str,
    timeout_ms: Option<u64>,
) -> Result<Option<i32>, AgentToolError> {
    let started = Instant::now();
    let timeout_dur = timeout_ms.map(Duration::from_millis);
    let marker = format!("__OD_EXIT__{run_id}:");
    loop {
        if let Some(timeout_dur) = timeout_dur {
            if started.elapsed() >= timeout_dur {
                return Ok(None);
            }
        }
        if let Some(code) = read_tmux_exit_code_file(&handle.exit_code_path).await? {
            return Ok(Some(code));
        }
        let pane = capture_tmux_pane_output(&handle.tmux_target).await?;
        if let Some(code) = parse_tmux_exit_code(pane.as_ref(), marker.as_str())? {
            return Ok(Some(code));
        }
        sleep(Duration::from_millis(EXEC_BASH_TMUX_POLL_MS)).await;
    }
}

async fn read_tmux_exit_code_file(path: &Path) -> Result<Option<i32>, AgentToolError> {
    let raw = match fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(AgentToolError::ExecFailed(format!(
                "read exit code file `{}` failed: {err}",
                path.display()
            )));
        }
    };
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let exit_code = value.parse::<i32>().map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "invalid exit code file `{}` content `{value}`: {err}",
            path.display()
        ))
    })?;
    Ok(Some(exit_code))
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

fn extract_single_tool_prompt_summary(_run_result: &ExecBashRunResult) -> Option<String> {
    None
}

fn decode_exec_bash_json_result(
    run_result: &ExecBashRunResult,
    command: &str,
) -> Option<AgentToolResult> {
    if !is_internal_agent_tool_command(command) {
        return None;
    }

    let stdout = String::from_utf8_lossy(&run_result.stdout);
    let payload = stdout.trim();
    if payload.is_empty() {
        return None;
    }

    let result = if let Ok(envelope) = serde_json::from_str::<CliResultEnvelope>(payload) {
        envelope.into_tool_result()
    } else {
        serde_json::from_str::<AgentToolResult>(payload).ok()?
    };
    if !result.is_agent_tool {
        return None;
    }
    Some(
        result
            .with_cmd_line(command)
            .with_command_metadata_from_line(command)
            .with_return_code(run_result.exit_code),
    )
}

fn build_default_exec_bash_result(
    run_result: &ExecBashRunResult,
    command: &str,
    _session_id: &str,
    _max_output_bytes: usize,
) -> AgentToolResult {
    let ok = run_result.exit_code == 0;
    let summary = if ok {
        extract_single_tool_prompt_summary(run_result)
            .unwrap_or_else(|| format!("OK (exit=0, {}ms)", run_result.duration_ms))
    } else {
        format!("FAILED (exit={})", run_result.exit_code)
    };

    AgentToolResult::from_details(json!({}))
        .with_status(if ok {
            AgentToolStatus::Success
        } else {
            AgentToolStatus::Error
        })
        .with_cmd_line(command)
        .with_result(summary)
        .with_output(run_result.mixed_output.clone())
        .with_return_code(run_result.exit_code)
        .with_command_metadata_from_line(command)
}

fn is_internal_agent_tool_command(command: &str) -> bool {
    let tokens = match AgentToolManager::parse_bash_command_name(command) {
        Some(name) => vec![name],
        None => tokenize_simple_command(command),
    };
    let Some(cmd_name) = tokens.first().map(|value| value.trim()) else {
        return false;
    };
    if cmd_name.is_empty() {
        return false;
    }
    if cmd_name == EXEC_BASH_AGENT_TOOL_BIN {
        return true;
    }
    default_agent_cli_tool_names()
        .iter()
        .any(|tool_name| tool_name == cmd_name)
}

fn tokenize_simple_command(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
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

fn collect_exec_command_lines(command: &str) -> Vec<ExecCommandLine> {
    let mut lines = command
        .lines()
        .enumerate()
        .filter_map(|(index, raw)| {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then(|| ExecCommandLine {
                line_no: index + 1,
                text: trimmed.to_string(),
            })
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(ExecCommandLine {
            line_no: 1,
            text: command.to_string(),
        });
    }
    lines
}

fn resolve_exec_bash_task_root_id(agent_name: &str, session_id: Option<&str>) -> String {
    let session_id = session_id.unwrap_or_default().trim();
    if !session_id.is_empty() {
        return session_id.to_string();
    }

    agent_name
        .split([':', '/', '#'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .last()
        .map(|value| format!("{value}#default"))
        .unwrap_or_else(|| "agent#default".to_string())
}

fn next_exec_bash_task_name_suffix() -> u64 {
    now_ms().saturating_add(EXEC_BASH_TASK_NAME_SEQ.fetch_add(1, Ordering::Relaxed))
}

fn build_exec_bash_pending_task_data(
    ctx: &SessionRuntimeContext,
    session_id: &str,
    command: &str,
    timeout_ms: u64,
    pwd: &Path,
    engine: &str,
    line_results: &[Json],
    partial_output: &str,
    duration_ms: u64,
    handle: &ExecBashTmuxRunHandle,
    summary: &str,
) -> Json {
    json!({
        "is_agent_tool": false,
        "kind": "tool.exec_bash",
        "status": "pending",
        "pending_reason": "long_running",
        "summary": summary,
        "output": partial_output,
        "check_after": EXEC_BASH_LONG_RUNNING_CHECK_AFTER_SECS,
        "timeout_ms": timeout_ms,
        "duration_ms": duration_ms,
        "command": command,
        "partial_output": partial_output,
        "cwd": pwd.to_string_lossy().to_string(),
        "pwd": pwd.to_string_lossy().to_string(),
        "session_id": session_id,
        "trace_id": ctx.trace_id,
        "agent_id": ctx.agent_name,
        "behavior": ctx.behavior,
        "step_idx": ctx.step_idx,
        "wakeup_id": ctx.wakeup_id,
        "tmux_session": handle.tmux_session,
        "tmux_target": handle.tmux_target,
        "run_id": handle.run_id,
        "pane_pid": handle.pane_pid,
        "stdout_path": handle.stdout_path.to_string_lossy().to_string(),
        "stderr_path": handle.stderr_path.to_string_lossy().to_string(),
        "script_path": handle.script_path.to_string_lossy().to_string(),
        "engine": engine,
        "line_results": line_results,
    })
}

fn build_exec_bash_final_task_data(
    session_id: &str,
    command: &str,
    timeout_ms: u64,
    run_result: &ExecBashRunResult,
    max_output_bytes: usize,
    summary: &str,
) -> Json {
    let (output, output_truncated) =
        truncate_bytes(run_result.mixed_output.as_bytes(), max_output_bytes);
    json!({
        "is_agent_tool": false,
        "kind": "tool.exec_bash",
        "status": if run_result.exit_code == 0 { "success" } else { "error" },
        "summary": summary,
        "output": run_result.mixed_output,
        "timeout_ms": timeout_ms,
        "duration_ms": run_result.duration_ms,
        "command": command,
        "cwd": run_result.pwd.to_string_lossy().to_string(),
        "pwd": run_result.pwd.to_string_lossy().to_string(),
        "session_id": session_id,
        "tmux_session": run_result.tmux_session,
        "engine": run_result.engine,
        "return_code": run_result.exit_code,
        "exit_code": run_result.exit_code,
        "output_preview": output,
        "output_truncated": output_truncated,
        "line_results": run_result.line_results.clone(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn filter_session_cli_tool_names_defaults_to_all_supported_tools() {
        assert_eq!(
            filter_session_cli_tool_names(&[]),
            default_agent_cli_tool_names()
        );
    }

    #[test]
    fn sync_session_tool_links_rewrites_visible_tool_set() {
        let temp = tempdir().expect("create tempdir");
        let tool_dir = temp.path().join("session").join(".tool");
        let agent_tool_path = temp.path().join("agent_tool");
        std_fs::create_dir_all(&tool_dir).expect("create tool dir");
        std_fs::write(&agent_tool_path, "#!/bin/sh\n").expect("seed agent_tool");

        sync_session_tool_links(
            &tool_dir,
            &agent_tool_path,
            &[TOOL_READ_FILE.to_string(), TOOL_WRITE_FILE.to_string()],
        )
        .expect("seed tool links");

        assert_eq!(
            std_fs::read_link(tool_dir.join(TOOL_READ_FILE)).expect("read read_file link"),
            agent_tool_path
        );
        assert_eq!(
            std_fs::read_link(tool_dir.join(TOOL_WRITE_FILE)).expect("read write_file link"),
            agent_tool_path
        );

        sync_session_tool_links(&tool_dir, &agent_tool_path, &[TOOL_GET_SESSION.to_string()])
            .expect("rewrite tool links");

        assert!(tool_dir.join(TOOL_GET_SESSION).exists());
        assert!(!tool_dir.join(TOOL_READ_FILE).exists());
        assert!(!tool_dir.join(TOOL_WRITE_FILE).exists());
    }

    #[test]
    fn build_tmux_exec_script_installs_command_not_found_proxy() {
        let script = build_tmux_exec_script(
            "run-1",
            Path::new("/tmp/stdout"),
            Path::new("/tmp/stderr"),
            Path::new("/tmp/exit.code"),
            Path::new("/tmp"),
            "read_file demo.txt",
            &[(
                "OPENDAN_AGENT_TOOL".to_string(),
                "/opt/buckyos/bin/opendan/agent_tool".to_string(),
            )],
        );

        assert!(script.contains("command_not_found_handle"));
        assert!(script.contains(EXEC_BASH_COMMAND_NOT_FOUND_PROXY));
    }
}
