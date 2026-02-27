use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};

use super::local_workspace::{
    CreateLocalWorkspaceRequest, LocalWorkspaceCleanupResult, LocalWorkspaceLockResult,
    LocalWorkspaceManager, LocalWorkspaceManagerConfig, LocalWorkspaceSnapshot,
    SessionWorkspaceBinding, WorkshopIndex, WorkshopWorkspaceRecord,
};
use super::todo::{TodoTool, TodoToolConfig};
use crate::agent_session::AgentSessionMgr;
use crate::agent_tool::{
    AgentSkillRecord, AgentSkillSpec, AgentTool, AgentToolError, AgentToolManager, MCPToolConfig,
    ToolSpec, TOOL_EDIT_FILE, TOOL_EXEC_BASH, TOOL_TODO_MANAGE, TOOL_WORKLOG_MANAGE,
};
use crate::behavior::TraceCtx;
use crate::worklog::{WorklogTool, WorklogToolConfig};

const DEFAULT_BASH_PATH: &str = "/bin/bash";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_DIFF_LINES: usize = 200;
const DEFAULT_MAX_FILE_WRITE_BYTES: usize = 256 * 1024;
const DEFAULT_TOOLS_JSON_REL_PATH: &str = "tools/tools.json";
const DEFAULT_TOOLS_MD_REL_PATH: &str = "tools/tools.md";
const DEFAULT_TODO_DB_REL_PATH: &str = "todo/todo.db";
const DEFAULT_WORKLOG_DB_REL_PATH: &str = "worklog/worklog.db";
const DEFAULT_TODO_LIST_LIMIT: usize = 32;
const DEFAULT_TODO_MAX_LIST_LIMIT: usize = 128;
const DEFAULT_WORKLOG_LIST_LIMIT: usize = 64;
const DEFAULT_WORKLOG_MAX_LIST_LIMIT: usize = 256;
const DEFAULT_AGENT_DID: &str = "did:opendan:unknown";
const DEFAULT_LOCAL_WORKSPACE_LOCK_TTL_MS: u64 = 120_000;
const EXEC_BASH_SUCCESS_DETAIL_LINES: usize = 16;
const EXEC_BASH_TMUX_POLL_MS: u64 = 120;
const EXEC_BASH_CAPTURE_SCROLLBACK_LINES: &str = "-6000";

#[derive(Clone, Debug)]
pub struct AgentWorkshopConfig {
    pub workspace_root: PathBuf,
    pub agent_did: String,
    pub bash_path: PathBuf,
    pub default_timeout_ms: u64,
    pub max_output_bytes: usize,
    pub default_max_diff_lines: usize,
    pub default_max_file_write_bytes: usize,
    pub tools_json_rel_path: PathBuf,
    pub tools_markdown_rel_path: PathBuf,
    pub local_workspace_lock_ttl_ms: u64,
}

impl AgentWorkshopConfig {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent_did: DEFAULT_AGENT_DID.to_string(),
            bash_path: PathBuf::from(DEFAULT_BASH_PATH),
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            default_max_diff_lines: DEFAULT_MAX_DIFF_LINES,
            default_max_file_write_bytes: DEFAULT_MAX_FILE_WRITE_BYTES,
            tools_json_rel_path: PathBuf::from(DEFAULT_TOOLS_JSON_REL_PATH),
            tools_markdown_rel_path: PathBuf::from(DEFAULT_TOOLS_MD_REL_PATH),
            local_workspace_lock_ttl_ms: DEFAULT_LOCAL_WORKSPACE_LOCK_TTL_MS,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentWorkshopToolsConfig {
    pub enabled_tools: Vec<WorkshopToolConfig>,
}

impl Default for AgentWorkshopToolsConfig {
    fn default() -> Self {
        Self {
            enabled_tools: vec![
                WorkshopToolConfig::enabled(TOOL_EXEC_BASH),
                WorkshopToolConfig::enabled(TOOL_EDIT_FILE),
                WorkshopToolConfig::enabled(TOOL_TODO_MANAGE),
                WorkshopToolConfig::enabled(TOOL_WORKLOG_MANAGE),
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkshopToolConfig {
    pub name: String,
    #[serde(default = "default_tool_kind")]
    pub kind: String,
    pub enabled: bool,
    pub params: Json,
}

impl Default for WorkshopToolConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: default_tool_kind(),
            enabled: true,
            params: Json::Object(serde_json::Map::new()),
        }
    }
}

impl WorkshopToolConfig {
    pub fn enabled(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: default_tool_kind(),
            enabled: true,
            params: Json::Object(serde_json::Map::new()),
        }
    }
}

fn default_tool_kind() -> String {
    "builtin".to_string()
}

#[derive(Clone, Debug)]
pub struct AgentWorkshop {
    cfg: AgentWorkshopConfig,
    tools_cfg: AgentWorkshopToolsConfig,
    local_workspace_mgr: LocalWorkspaceManager,
}

impl AgentWorkshop {
    pub async fn new(mut cfg: AgentWorkshopConfig) -> Result<Self, AgentToolError> {
        if cfg.agent_did.trim().is_empty() {
            cfg.agent_did = DEFAULT_AGENT_DID.to_string();
        }
        Self::initialize(cfg, true).await
    }

    pub async fn create_workshop(
        agent_did: impl Into<String>,
        mut cfg: AgentWorkshopConfig,
    ) -> Result<Self, AgentToolError> {
        cfg.agent_did = agent_did.into();
        Self::initialize(cfg, true).await
    }

    pub async fn load_workshop(
        agent_did: impl Into<String>,
        mut cfg: AgentWorkshopConfig,
    ) -> Result<Self, AgentToolError> {
        cfg.agent_did = agent_did.into();
        Self::initialize(cfg, false).await
    }

    async fn initialize(
        mut cfg: AgentWorkshopConfig,
        create_if_missing: bool,
    ) -> Result<Self, AgentToolError> {
        let workspace_root = normalize_workspace_root(&cfg.workspace_root)?;
        create_minimal_workspace_dirs(&workspace_root).await?;
        cfg.workspace_root = workspace_root.clone();

        let local_cfg = LocalWorkspaceManagerConfig {
            workshop_root: workspace_root.clone(),
            lock_ttl_ms: cfg.local_workspace_lock_ttl_ms,
        };
        let local_workspace_mgr = if create_if_missing {
            LocalWorkspaceManager::create_workshop(cfg.agent_did.clone(), local_cfg).await?
        } else {
            LocalWorkspaceManager::load_workshop(cfg.agent_did.clone(), local_cfg).await?
        };

        let tools_cfg = load_tools_config(&workspace_root, &cfg).await?;
        validate_tools_config(&tools_cfg)?;

        Ok(Self {
            cfg,
            tools_cfg,
            local_workspace_mgr,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.cfg.workspace_root
    }

    pub fn tools_config(&self) -> &AgentWorkshopToolsConfig {
        &self.tools_cfg
    }

    pub fn local_workspace_manager(&self) -> &LocalWorkspaceManager {
        &self.local_workspace_mgr
    }

    pub async fn workshop_index(&self) -> WorkshopIndex {
        self.local_workspace_mgr.workshop_index().await
    }

    pub async fn list_workspaces(&self) -> Vec<WorkshopWorkspaceRecord> {
        self.local_workspace_mgr.list_workspaces().await
    }

    pub async fn create_local_workspace(
        &self,
        req: CreateLocalWorkspaceRequest,
    ) -> Result<WorkshopWorkspaceRecord, AgentToolError> {
        self.local_workspace_mgr.create_local_workspace(req).await
    }

    pub async fn bind_local_workspace(
        &self,
        session_id: &str,
        local_workspace_id: &str,
    ) -> Result<SessionWorkspaceBinding, AgentToolError> {
        self.local_workspace_mgr
            .bind_local_workspace(session_id, local_workspace_id)
            .await
    }

    pub async fn get_local_workspace_path(
        &self,
        local_workspace_id: &str,
    ) -> Result<PathBuf, AgentToolError> {
        self.local_workspace_mgr
            .get_local_workspace_path(local_workspace_id)
            .await
    }

    pub async fn snapshot_metadata(
        &self,
        local_workspace_id: &str,
    ) -> Result<LocalWorkspaceSnapshot, AgentToolError> {
        self.local_workspace_mgr
            .snapshot_metadata(local_workspace_id)
            .await
    }

    pub async fn acquire_local_workspace_lock(
        &self,
        local_workspace_id: &str,
        session_id: &str,
    ) -> Result<LocalWorkspaceLockResult, AgentToolError> {
        self.local_workspace_mgr
            .acquire(local_workspace_id, session_id)
            .await
    }

    pub async fn release_local_workspace_lock(
        &self,
        local_workspace_id: &str,
        session_id: &str,
    ) -> Result<bool, AgentToolError> {
        self.local_workspace_mgr
            .release(local_workspace_id, session_id)
            .await
    }

    pub async fn archive_workspace(
        &self,
        workspace_id: &str,
        reason: Option<String>,
    ) -> Result<WorkshopWorkspaceRecord, AgentToolError> {
        self.local_workspace_mgr
            .archive_workspace(workspace_id, reason)
            .await
    }

    pub async fn cleanup(&self) -> Result<LocalWorkspaceCleanupResult, AgentToolError> {
        self.local_workspace_mgr.cleanup().await
    }

    pub async fn list_skills(
        &self,
        local_workspace_id: &str,
    ) -> Result<Vec<AgentSkillRecord>, AgentToolError> {
        self.local_workspace_mgr
            .list_skills(local_workspace_id)
            .await
    }

    pub async fn load_skill(&self, skill_name: &str) -> Result<AgentSkillSpec, AgentToolError> {
        self.local_workspace_mgr.load_skill(skill_name).await
    }

    pub fn register_tools(
        &self,
        tool_mgr: &AgentToolManager,
        session_store: Arc<AgentSessionMgr>,
    ) -> Result<(), AgentToolError> {
        let write_audit = WorkshopWriteAudit::new(self.resolve_write_audit_config()?);
        for tool in self
            .tools_cfg
            .enabled_tools
            .iter()
            .filter(|tool| tool.enabled)
        {
            match tool.kind.as_str() {
                "builtin" => match tool.name.as_str() {
                    TOOL_EXEC_BASH => {
                        tool_mgr.register_tool(ExecBashTool {
                            cfg: self.cfg.clone(),
                            policy: ExecBashPolicy::from_tool_config(&self.cfg, tool)?,
                            session_store: session_store.clone(),
                        })?;
                    }
                    TOOL_EDIT_FILE => {
                        tool_mgr.register_tool(EditFileTool {
                            cfg: self.cfg.clone(),
                            policy: EditFilePolicy::from_tool_config(&self.cfg, tool)?,
                            write_audit: write_audit.clone(),
                        })?;
                    }
                    TOOL_TODO_MANAGE => {
                        let policy = TodoToolPolicy::from_tool_config(&self.cfg, tool)?;
                        tool_mgr.register_tool(TodoTool::new(TodoToolConfig {
                            db_path: policy.db_path,
                            default_list_limit: policy.default_list_limit,
                            max_list_limit: policy.max_list_limit,
                        })?)?;
                    }
                    TOOL_WORKLOG_MANAGE => {
                        let policy = WorklogToolPolicy::from_tool_config(&self.cfg, tool)?;
                        tool_mgr.register_tool(WorklogTool::new(WorklogToolConfig {
                            db_path: policy.db_path,
                            default_list_limit: policy.default_list_limit,
                            max_list_limit: policy.max_list_limit,
                        })?)?;
                    }
                    unsupported => {
                        return Err(AgentToolError::InvalidArgs(format!(
                            "builtin tool `{unsupported}` is not supported by current runtime"
                        )));
                    }
                },
                "mcp" => {
                    tool_mgr.register_mcp_tool(build_mcp_tool_config(tool)?)?;
                }
                unsupported_kind => {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "tool `{}` has unsupported kind `{unsupported_kind}`",
                        tool.name
                    )));
                }
            }
        }
        Ok(())
    }

    fn resolve_write_audit_config(&self) -> Result<WorklogToolConfig, AgentToolError> {
        for tool in self
            .tools_cfg
            .enabled_tools
            .iter()
            .filter(|tool| tool.enabled)
        {
            if tool.name == TOOL_WORKLOG_MANAGE {
                let policy = WorklogToolPolicy::from_tool_config(&self.cfg, tool)?;
                return Ok(WorklogToolConfig {
                    db_path: policy.db_path,
                    default_list_limit: policy.default_list_limit,
                    max_list_limit: policy.max_list_limit,
                });
            }
        }

        Ok(WorklogToolConfig::with_db_path(resolve_path_in_workspace(
            &self.cfg.workspace_root,
            DEFAULT_WORKLOG_DB_REL_PATH,
        )?))
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
struct TodoToolPolicy {
    db_path: PathBuf,
    default_list_limit: usize,
    max_list_limit: usize,
}

impl TodoToolPolicy {
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

        let db_path = if let Some(raw_db_path) = read_string_from_map(params, "db_path")? {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, &raw_db_path)?
        } else {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, DEFAULT_TODO_DB_REL_PATH)?
        };

        let default_list_limit = read_u64_from_map(params, "default_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_TODO_LIST_LIMIT);
        let max_list_limit = read_u64_from_map(params, "max_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_TODO_MAX_LIST_LIMIT.max(default_list_limit));

        if default_list_limit == 0 || max_list_limit == 0 || default_list_limit > max_list_limit {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` has invalid list limit bounds",
                tool_cfg.name
            )));
        }

        Ok(Self {
            db_path,
            default_list_limit,
            max_list_limit,
        })
    }
}

#[derive(Clone, Debug)]
struct WorklogToolPolicy {
    db_path: PathBuf,
    default_list_limit: usize,
    max_list_limit: usize,
}

impl WorklogToolPolicy {
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

        let db_path = if let Some(raw_db_path) = read_string_from_map(params, "db_path")? {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, &raw_db_path)?
        } else {
            resolve_path_in_workspace(&workshop_cfg.workspace_root, DEFAULT_WORKLOG_DB_REL_PATH)?
        };

        let default_list_limit = read_u64_from_map(params, "default_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_WORKLOG_LIST_LIMIT);
        let max_list_limit = read_u64_from_map(params, "max_list_limit")?
            .map(u64_to_usize)
            .transpose()?
            .unwrap_or(DEFAULT_WORKLOG_MAX_LIST_LIMIT.max(default_list_limit));

        if default_list_limit == 0 || max_list_limit == 0 || default_list_limit > max_list_limit {
            return Err(AgentToolError::InvalidArgs(format!(
                "tool `{}` has invalid list limit bounds",
                tool_cfg.name
            )));
        }

        Ok(Self {
            db_path,
            default_list_limit,
            max_list_limit,
        })
    }
}

#[derive(Clone, Debug)]
struct ExecBashTool {
    cfg: AgentWorkshopConfig,
    policy: ExecBashPolicy,
    session_store: Arc<AgentSessionMgr>,
}

#[async_trait]
impl AgentTool for ExecBashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EXEC_BASH.to_string(),
            description: "Run a Linux bash command in a tmux-backed workshop session.".to_string(),
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
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "exit_code": { "type": ["integer", "null"] },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "details": { "type": "string" },
                    "stdout_truncated": { "type": "boolean" },
                    "stderr_truncated": { "type": "boolean" },
                    "duration_ms": { "type": "integer" },
                    "cwd": { "type": "string" },
                    "session_id": { "type": "string" },
                    "tmux_session": { "type": "string" },
                    "engine": { "type": "string" }
                }
            }),
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
        let (stdout, stdout_truncated) =
            truncate_bytes(&run_result.stdout, self.cfg.max_output_bytes);
        let (stderr, stderr_truncated) =
            truncate_bytes(&run_result.stderr, self.cfg.max_output_bytes);
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

        // flush potential trailing buffered bytes produced by tee.
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
struct EditFileTool {
    cfg: AgentWorkshopConfig,
    policy: EditFilePolicy,
    write_audit: WorkshopWriteAudit,
}

#[async_trait]
impl AgentTool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EDIT_FILE.to_string(),
            description: "Edit file content under workshop scope and return a diff.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string", "description": "Used by overwrite/append mode." },
                    "append": { "type": "boolean", "default": false },
                    "old_text": { "type": "string", "description": "If present, run replace mode." },
                    "new_text": { "type": "string" },
                    "replace_all": { "type": "boolean", "default": false },
                    "create_if_missing": { "type": "boolean", "default": true }
                },
                "required": ["path"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "path": { "type": "string" },
                    "operation": { "type": "string" },
                    "created": { "type": "boolean" },
                    "changed": { "type": "boolean" },
                    "bytes_before": { "type": "integer" },
                    "bytes_after": { "type": "integer" },
                    "diff": { "type": "string" },
                    "diff_truncated": { "type": "boolean" }
                }
            }),
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

        let create_if_missing = optional_bool(&args, "create_if_missing")?.unwrap_or(true);
        let exists = fs::metadata(&abs_path).await.is_ok();
        if !exists && (!create_if_missing || !self.policy.allow_create) {
            return Err(AgentToolError::InvalidArgs(format!(
                "file does not exist or create disabled by policy: {file_path}"
            )));
        }

        let original_content = if exists {
            read_text_file_lossy(&abs_path).await?
        } else {
            String::new()
        };

        let has_replace_mode = args.get("old_text").is_some();
        let (operation, updated_content) = if has_replace_mode {
            if !self.policy.allow_replace {
                return Err(AgentToolError::InvalidArgs(
                    "replace mode disabled by workshop tool policy".to_string(),
                ));
            }
            let old_text = require_string(&args, "old_text")?;
            let new_text = require_string(&args, "new_text")?;
            let replace_all = optional_bool(&args, "replace_all")?.unwrap_or(false);
            if !original_content.contains(&old_text) {
                return Err(AgentToolError::InvalidArgs(format!(
                    "old_text not found in file: {file_path}"
                )));
            }
            let replaced = if replace_all {
                original_content.replace(&old_text, &new_text)
            } else {
                original_content.replacen(&old_text, &new_text, 1)
            };
            ("replace".to_string(), replaced)
        } else {
            let content = require_string(&args, "content")?;
            let append = optional_bool(&args, "append")?.unwrap_or(false);
            if append {
                ("append".to_string(), format!("{original_content}{content}"))
            } else if exists {
                ("overwrite".to_string(), content)
            } else {
                ("create".to_string(), content)
            }
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
struct WorkshopWriteAudit {
    worklog_cfg: WorklogToolConfig,
}

impl WorkshopWriteAudit {
    fn new(worklog_cfg: WorklogToolConfig) -> Self {
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
                    "agent_id": ctx.agent_did,
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

fn optional_bool(args: &Json, key: &str) -> Result<Option<bool>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a boolean")))
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

fn read_string_from_map(
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<String>, AgentToolError> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(value.to_string()))
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

fn build_mcp_tool_config(tool_cfg: &WorkshopToolConfig) -> Result<MCPToolConfig, AgentToolError> {
    let params = tool_cfg.params.as_object().ok_or_else(|| {
        AgentToolError::InvalidArgs(format!(
            "mcp tool `{}` params must be a json object",
            tool_cfg.name
        ))
    })?;

    let endpoint = read_string_from_map(params, "endpoint")?.ok_or_else(|| {
        AgentToolError::InvalidArgs(format!(
            "mcp tool `{}` requires params.endpoint",
            tool_cfg.name
        ))
    })?;

    let mcp_tool_name = read_string_from_map(params, "mcp_tool_name")?;
    let description = read_string_from_map(params, "description")?;
    let timeout_ms = read_u64_from_map(params, "timeout_ms")?.unwrap_or(30_000);

    let headers = match params.get("headers") {
        None => HashMap::new(),
        Some(value) => {
            let obj = value.as_object().ok_or_else(|| {
                AgentToolError::InvalidArgs(format!(
                    "mcp tool `{}` params.headers must be an object",
                    tool_cfg.name
                ))
            })?;
            let mut headers = HashMap::with_capacity(obj.len());
            for (key, value) in obj {
                let val = value.as_str().ok_or_else(|| {
                    AgentToolError::InvalidArgs(format!(
                        "mcp tool `{}` params.headers.{key} must be string",
                        tool_cfg.name
                    ))
                })?;
                headers.insert(key.to_string(), val.to_string());
            }
            headers
        }
    };

    let args_schema = params
        .get("args_schema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object"}));
    let output_schema = params
        .get("output_schema")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object"}));

    Ok(MCPToolConfig {
        name: tool_cfg.name.clone(),
        endpoint,
        mcp_tool_name,
        description,
        args_schema,
        output_schema,
        headers,
        timeout_ms,
    })
}

fn normalize_workspace_root(root: &Path) -> Result<PathBuf, AgentToolError> {
    let root_abs = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| AgentToolError::ExecFailed(format!("read current_dir failed: {err}")))?
            .join(root)
    };
    Ok(normalize_abs_path(&root_abs))
}

async fn create_minimal_workspace_dirs(workspace_root: &Path) -> Result<(), AgentToolError> {
    let roots = [
        workspace_root.to_path_buf(),
        workspace_root.join("skills"),
        workspace_root.join("sessions"),
        workspace_root.join("workspaces"),
        workspace_root.join("workspaces/local"),
        workspace_root.join("workspaces/remote"),
        workspace_root.join("worklog"),
        workspace_root.join("todo"),
        workspace_root.join("tools"),
        workspace_root.join("artifacts"),
    ];
    for dir in roots {
        fs::create_dir_all(&dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("create dir `{}` failed: {err}", dir.display()))
        })?;
    }
    Ok(())
}

async fn load_tools_config(
    workspace_root: &Path,
    workshop_cfg: &AgentWorkshopConfig,
) -> Result<AgentWorkshopToolsConfig, AgentToolError> {
    let tools_json_path = workspace_root.join(&workshop_cfg.tools_json_rel_path);
    match fs::read_to_string(&tools_json_path).await {
        Ok(content) => {
            let cfg =
                serde_json::from_str::<AgentWorkshopToolsConfig>(&content).map_err(|err| {
                    AgentToolError::InvalidArgs(format!(
                        "invalid tools config json `{}`: {err}",
                        tools_json_path.display()
                    ))
                })?;
            return Ok(cfg);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(AgentToolError::ExecFailed(format!(
                "read tools config json `{}` failed: {err}",
                tools_json_path.display()
            )));
        }
    }

    let tools_md_path = workspace_root.join(&workshop_cfg.tools_markdown_rel_path);
    match fs::read_to_string(&tools_md_path).await {
        Ok(content) => parse_tools_markdown_config(&tools_md_path, &content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(AgentWorkshopToolsConfig::default())
        }
        Err(err) => Err(AgentToolError::ExecFailed(format!(
            "read tools config markdown `{}` failed: {err}",
            tools_md_path.display()
        ))),
    }
}

fn parse_tools_markdown_config(
    source_path: &Path,
    content: &str,
) -> Result<AgentWorkshopToolsConfig, AgentToolError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(AgentWorkshopToolsConfig::default());
    }

    if let Ok(cfg) = serde_json::from_str::<AgentWorkshopToolsConfig>(trimmed) {
        return Ok(cfg);
    }

    let Some(json_block) = try_extract_json_block(trimmed) else {
        return Err(AgentToolError::InvalidArgs(format!(
            "tools markdown `{}` does not contain valid json config block",
            source_path.display()
        )));
    };

    serde_json::from_str::<AgentWorkshopToolsConfig>(&json_block).map_err(|err| {
        AgentToolError::InvalidArgs(format!(
            "invalid tools markdown config `{}`: {err}",
            source_path.display()
        ))
    })
}

fn validate_tools_config(cfg: &AgentWorkshopToolsConfig) -> Result<(), AgentToolError> {
    let mut seen = HashSet::new();
    for tool in &cfg.enabled_tools {
        if tool.name.trim().is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "tool config contains empty tool name".to_string(),
            ));
        }
        if !seen.insert(tool.name.clone()) {
            return Err(AgentToolError::InvalidArgs(format!(
                "duplicate tool config entry: {}",
                tool.name
            )));
        }
    }
    Ok(())
}

fn try_extract_json_block(content: &str) -> Option<String> {
    let fence_parts: Vec<&str> = content.split("```").collect();
    if fence_parts.len() >= 3 {
        for segment in fence_parts.iter().skip(1).step_by(2) {
            let trimmed = segment.trim();
            let payload = if let Some(rest) = trimmed.strip_prefix("json") {
                rest.trim()
            } else {
                trimmed
            };
            if serde_json::from_str::<Json>(payload).is_ok() {
                return Some(payload.to_string());
            }
        }
    }

    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    let candidate = &content[start..=end];
    if serde_json::from_str::<Json>(candidate).is_ok() {
        return Some(candidate.to_string());
    }
    None
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
    format!("od_{}", sanitize_token_for_id(session_id))
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
    use crate::agent_session::AgentSessionMgr;
    use crate::behavior::TraceCtx;
    use buckyos_api::{value_to_object_map, AiToolCall};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_workspace_root(test_name: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("opendan-{test_name}-{ts}"))
    }

    async fn create_session_store(root: &Path) -> Arc<AgentSessionMgr> {
        let store = Arc::new(
            AgentSessionMgr::new(
                "did:example:agent".to_string(),
                root.join("session"),
                Some("on_wakeup".to_string()),
            )
            .await
            .expect("create session store"),
        );
        let session = store
            .ensure_session("session-test", Some("Session Test".to_string()))
            .await
            .expect("ensure session");
        {
            let mut guard = session.lock().await;
            guard.cwd = root.to_path_buf();
        }
        store
            .save_session("session-test")
            .await
            .expect("save session");
        store
    }

    async fn call(
        tool_mgr: &AgentToolManager,
        name: &str,
        args: Json,
    ) -> Result<Json, AgentToolError> {
        tool_mgr
            .call_tool(
                &TraceCtx {
                    trace_id: "trace-test".to_string(),
                    agent_did: "did:example:agent".to_string(),
                    behavior: "on_wakeup".to_string(),
                    step_idx: 0,
                    wakeup_id: "wakeup-test".to_string(),
                    session_id: Some("session-test".to_string()),
                },
                AiToolCall {
                    name: name.to_string(),
                    args: value_to_object_map(args),
                    call_id: "call-test".to_string(),
                },
            )
            .await
    }

    async fn write_tools_json(root: &Path, payload: Json) {
        let path = root.join("tools/tools.json");
        fs::create_dir_all(path.parent().expect("tools parent"))
            .await
            .expect("create tools dir");
        fs::write(
            path,
            serde_json::to_string_pretty(&payload).expect("serialize tools config"),
        )
        .await
        .expect("write tools config");
    }

    async fn tmux_ready() -> bool {
        match Command::new("tmux").arg("-V").output().await {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn exec_bash_tool_runs_linux_command() {
        if !tmux_ready().await {
            return;
        }
        let root = unique_workspace_root("exec-bash");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        let result = call(
            &tool_mgr,
            TOOL_EXEC_BASH,
            json!({
                "command": "printf 'hello-linux'",
            }),
        )
        .await
        .expect("exec bash should succeed");

        assert_eq!(result["ok"], true);
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["stdout"], "hello-linux");
        assert_eq!(result["details"], "hello-linux");
        assert_eq!(result["engine"], "tmux");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn edit_file_tool_writes_file_and_returns_diff() {
        let root = unique_workspace_root("edit-file");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/todo.md",
                "content": "line1\nline2\n"
            }),
        )
        .await
        .expect("create file should succeed");

        let result = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/todo.md",
                "old_text": "line2",
                "new_text": "lineX"
            }),
        )
        .await
        .expect("replace should succeed");

        let content = fs::read_to_string(root.join("notes/todo.md"))
            .await
            .expect("read file");
        assert!(content.contains("lineX"));
        let diff = result["diff"].as_str().unwrap_or_default();
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+lineX"));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_config_can_enable_subset_of_runtime_tools() {
        let root = unique_workspace_root("tool-subset");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    { "name": "edit_file", "enabled": true, "params": {} }
                ]
            }),
        )
        .await;

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(!tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(!tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let err = call(
            &tool_mgr,
            TOOL_EXEC_BASH,
            json!({"command":"echo should_not_run"}),
        )
        .await
        .expect_err("tool should not be registered");
        assert!(matches!(err, AgentToolError::NotFound(_)));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tool_params_apply_workshop_boundary_controls() {
        let root = unique_workspace_root("tool-policy");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "edit_file",
                        "enabled": true,
                        "params": {
                            "allowed_write_roots": ["todo"],
                            "allow_create": true,
                            "max_write_bytes": 128,
                            "max_diff_lines": 40
                        }
                    }
                ]
            }),
        )
        .await;

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "todo/ok.md",
                "content": "ok"
            }),
        )
        .await
        .expect("path under policy root should be writable");

        let err = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "artifacts/out.md",
                "content": "blocked"
            }),
        )
        .await
        .expect_err("path outside policy root should be denied");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_config_can_register_mcp_tool() {
        let root = unique_workspace_root("tool-mcp");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "mcp.weather",
                        "kind": "mcp",
                        "enabled": true,
                        "params": {
                            "endpoint": "http://127.0.0.1:9",
                            "mcp_tool_name": "weather.query",
                            "timeout_ms": 3000
                        }
                    }
                ]
            }),
        )
        .await;

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool("weather"));
        assert!(!tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(!tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(!tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_markdown_json_block_is_loaded() {
        let root = unique_workspace_root("tool-md");
        let md_path = root.join("tools/tools.md");
        fs::create_dir_all(md_path.parent().expect("tools parent"))
            .await
            .expect("create tools dir");
        fs::write(
            &md_path,
            r#"
# Tools

```json
{
  "enabled_tools": [
    { "name": "exec_bash", "enabled": true, "params": { "max_timeout_ms": 30 } }
  ]
}
```
"#,
        )
        .await
        .expect("write tools.md");

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(!tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(!tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn worklog_manage_tool_supports_append_and_query_fields() {
        let root = unique_workspace_root("worklog-manage");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        assert!(tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));

        let appended = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "append",
                "type": "function_call",
                "status": "success",
                "step_id": "step-1",
                "owner_session_id": "session-alpha",
                "summary": "exec_bash finished",
                "tags": ["tool", "runtime"],
                "payload": { "tool": "exec_bash", "ok": true }
            }),
        )
        .await
        .expect("append worklog should succeed");
        let log_id = appended["log"]["log_id"]
            .as_str()
            .expect("log id should exist")
            .to_string();

        let listed = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "list",
                "owner_session_id": "session-alpha",
                "tag": "runtime"
            }),
        )
        .await
        .expect("list worklog should succeed");
        let listed_logs = listed["logs"].as_array().expect("logs should be an array");
        assert_eq!(listed_logs.len(), 1);
        assert_eq!(listed_logs[0]["log_id"], log_id);

        let got = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "get",
                "log_id": log_id
            }),
        )
        .await
        .expect("get worklog should succeed");
        println!("got log: {got}");
        assert_eq!(got["log"]["log_type"], "opendan.worklog.FunctionRecord.v1");
        assert_eq!(got["log"]["owner_session_id"], "session-alpha");
        assert!(fs::metadata(root.join("worklog/worklog.db")).await.is_ok());

        let _ = fs::remove_dir_all(root).await;
    }
}
