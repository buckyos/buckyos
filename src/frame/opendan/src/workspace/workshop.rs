use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;

use super::local_workspace::{
    CreateLocalWorkspaceRequest, LocalWorkspaceCleanupResult, LocalWorkspaceLockResult,
    LocalWorkspaceManager, LocalWorkspaceManagerConfig, LocalWorkspaceSnapshot,
    SessionWorkspaceBinding, WorkshopIndex, WorkshopWorkspaceRecord, WorkspaceOwner,
};
use super::todo::{TodoTool, TodoToolConfig};
use crate::agent_session::AgentSessionMgr;
use crate::agent_tool::{
    AgentSkillRecord, AgentSkillSpec, AgentTool, AgentToolError, AgentToolManager, MCPToolConfig,
    ToolSpec, TOOL_BIND_LOCAL_WORKSPACE, TOOL_CREATE_LOCAL_WORKSPACE, TOOL_EDIT_FILE,
    TOOL_EXEC_BASH, TOOL_READ_FILE, TOOL_TODO_MANAGE, TOOL_WORKLOG_MANAGE, TOOL_WRITE_FILE,
};
use crate::behavior::SessionRuntimeContext;
use crate::buildin_tool::{
    EditFileTool as BuiltinEditFileTool, ExecBashTool as BuiltinExecBashTool,
    ReadFileTool as BuiltinReadFileTool, WorkshopWriteAudit as BuiltinWorkshopWriteAudit,
    WriteFileTool as BuiltinWriteFileTool,
};
use crate::worklog::WorklogToolConfig;

const DEFAULT_BASH_PATH: &str = "/bin/bash";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_DIFF_LINES: usize = 200;
const DEFAULT_MAX_FILE_WRITE_BYTES: usize = 256 * 1024;
const DEFAULT_TOOLS_JSON_REL_PATH: &str = "tools/tools.json";
const DEFAULT_TODO_DB_REL_PATH: &str = "todo/todo.db";
const DEFAULT_WORKLOG_DB_REL_PATH: &str = "worklog/worklog.db";
const DEFAULT_TODO_LIST_LIMIT: usize = 32;
const DEFAULT_TODO_MAX_LIST_LIMIT: usize = 128;
const DEFAULT_WORKLOG_LIST_LIMIT: usize = 64;
const DEFAULT_WORKLOG_MAX_LIST_LIMIT: usize = 256;
const DEFAULT_AGENT_DID: &str = "did:opendan:unknown";
const DEFAULT_LOCAL_WORKSPACE_LOCK_TTL_MS: u64 = 120_000;

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
                WorkshopToolConfig::enabled(TOOL_WRITE_FILE),
                WorkshopToolConfig::enabled(TOOL_READ_FILE),
                WorkshopToolConfig::enabled(TOOL_TODO_MANAGE),
                WorkshopToolConfig::enabled(TOOL_CREATE_LOCAL_WORKSPACE),
                WorkshopToolConfig::enabled(TOOL_BIND_LOCAL_WORKSPACE),
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
        let write_audit = BuiltinWorkshopWriteAudit::new(self.resolve_write_audit_config()?);
        for tool in self
            .tools_cfg
            .enabled_tools
            .iter()
            .filter(|tool| tool.enabled)
        {
            match tool.kind.as_str() {
                "builtin" => match tool.name.as_str() {
                    TOOL_EXEC_BASH => {
                        tool_mgr.register_tool(BuiltinExecBashTool::from_tool_config(
                            &self.cfg,
                            tool,
                            session_store.clone(),
                            tool_mgr.clone(),
                        )?)?;
                    }
                    TOOL_EDIT_FILE => {
                        tool_mgr.register_tool(BuiltinEditFileTool::from_tool_config(
                            &self.cfg,
                            tool,
                            write_audit.clone(),
                        )?)?;
                    }
                    TOOL_WRITE_FILE => {
                        tool_mgr.register_tool(BuiltinWriteFileTool::from_tool_config(
                            &self.cfg,
                            tool,
                            write_audit.clone(),
                        )?)?;
                    }
                    TOOL_READ_FILE => {
                        tool_mgr.register_tool(BuiltinReadFileTool::from_tool_config(
                            &self.cfg, tool,
                        )?)?;
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
                        // worklog_manage is no longer exposed as a runtime tool.
                        // Keep params parsing for backward-compatible tools.json configs
                        // so write-audit settings can still be reused.
                        let _ = WorklogToolPolicy::from_tool_config(&self.cfg, tool)?;
                    }
                    TOOL_CREATE_LOCAL_WORKSPACE => {
                        tool_mgr.register_tool(CreateLocalWorkspaceTool::new(
                            self.local_workspace_mgr.clone(),
                            session_store.clone(),
                        ))?;
                    }
                    TOOL_BIND_LOCAL_WORKSPACE => {
                        tool_mgr.register_tool(BindLocalWorkspaceTool::new(
                            self.local_workspace_mgr.clone(),
                            session_store.clone(),
                        ))?;
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

#[derive(Clone)]
struct CreateLocalWorkspaceTool {
    local_workspace_mgr: LocalWorkspaceManager,
    session_store: Arc<AgentSessionMgr>,
}

impl CreateLocalWorkspaceTool {
    fn new(
        local_workspace_mgr: LocalWorkspaceManager,
        session_store: Arc<AgentSessionMgr>,
    ) -> Self {
        Self {
            local_workspace_mgr,
            session_store,
        }
    }
}

#[async_trait]
impl AgentTool for CreateLocalWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CREATE_LOCAL_WORKSPACE.to_string(),
            description: "Create a local workspace and optionally bind it to current session."
                .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "template": { "type": "string" },
                    "owner": {
                        "type": "string",
                        "enum": ["agent_created", "user_provided"]
                    },
                    "policy_profile_id": { "type": "string" },
                    "created_by_session": { "type": "string" },
                    "session_id": { "type": "string" },
                    "bind_session": { "type": "boolean" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "workspace": { "type": "object" },
                    "binding": { "type": "object" },
                    "session_id": { "type": "string" },
                    "session_updated": { "type": "boolean" }
                }
            }),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }
    fn support_action(&self) -> bool {
        false
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(&self, ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError> {
        let name = require_string_arg(&args, "name")?;
        let template = optional_string_arg(&args, "template")?;
        let owner = match optional_string_arg(&args, "owner")?.as_deref() {
            Some("agent_created") | None => WorkspaceOwner::AgentCreated,
            Some("user_provided") => WorkspaceOwner::UserProvided,
            Some(other) => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "invalid owner `{other}`, expected `agent_created` or `user_provided`"
                )));
            }
        };
        let bind_session = optional_bool_arg(&args, "bind_session")?.unwrap_or(true);
        let session_id =
            optional_string_arg(&args, "session_id")?.or_else(|| Some(ctx.session_id.clone()));
        let created_by_session =
            optional_string_arg(&args, "created_by_session")?.or_else(|| session_id.clone());
        let policy_profile_id = optional_string_arg(&args, "policy_profile_id")?;
        let result = async {
            let workspace = self
                .local_workspace_mgr
                .create_local_workspace(CreateLocalWorkspaceRequest {
                    name,
                    template,
                    owner,
                    created_by_session,
                    policy_profile_id,
                })
                .await?;

            let mut binding: Option<SessionWorkspaceBinding> = None;
            let mut session_updated = false;
            if bind_session {
                let session_id = session_id.clone().ok_or_else(|| {
                    AgentToolError::InvalidArgs(
                        "session_id is required when `bind_session` is true".to_string(),
                    )
                })?;
                let bind_result = self
                    .local_workspace_mgr
                    .bind_local_workspace(&session_id, &workspace.workspace_id)
                    .await?;
                binding = Some(bind_result.clone());
                session_updated = persist_session_workspace_binding(
                    &self.session_store,
                    &session_id,
                    &workspace.workspace_id,
                    Some(workspace.name.as_str()),
                    &bind_result,
                )
                .await?;
            }

            Ok::<Json, AgentToolError>(json!({
                "ok": true,
                "workspace": workspace,
                "binding": binding,
                "session_id": session_id,
                "session_updated": session_updated
            }))
        }
        .await;

        match &result {
            Ok(_) => {
                info!(
                    "opendan.tool_call: tool={} status=success trace_id={} session_id={} bind_session={}",
                    TOOL_CREATE_LOCAL_WORKSPACE,
                    ctx.trace_id,
                    session_id.as_deref().unwrap_or_default(),
                    bind_session
                );
            }
            Err(err) => {
                warn!(
                    "opendan.tool_call: tool={} status=failed trace_id={} session_id={} bind_session={} err={}",
                    TOOL_CREATE_LOCAL_WORKSPACE,
                    ctx.trace_id,
                    session_id.as_deref().unwrap_or_default(),
                    bind_session,
                    err
                );
            }
        }

        result
    }
}

#[derive(Clone)]
struct BindLocalWorkspaceTool {
    local_workspace_mgr: LocalWorkspaceManager,
    session_store: Arc<AgentSessionMgr>,
}

impl BindLocalWorkspaceTool {
    fn new(
        local_workspace_mgr: LocalWorkspaceManager,
        session_store: Arc<AgentSessionMgr>,
    ) -> Self {
        Self {
            local_workspace_mgr,
            session_store,
        }
    }
}

#[async_trait]
impl AgentTool for BindLocalWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BIND_LOCAL_WORKSPACE.to_string(),
            description:
                "Bind an existing local workspace to current session. Rebinding is forbidden."
                    .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "local_workspace_id": { "type": "string" },
                    "session_id": { "type": "string" }
                },
                "required": ["local_workspace_id"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "binding": { "type": "object" },
                    "session_id": { "type": "string" },
                    "session_updated": { "type": "boolean" }
                }
            }),
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }
    fn support_action(&self) -> bool {
        false
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }
    

    async fn call(&self, ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError> {
        let local_workspace_id = require_string_arg(&args, "local_workspace_id")?;
        let session_id = optional_string_arg(&args, "session_id")?
            .or_else(|| Some(ctx.session_id.clone()))
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(
                    "session_id is required for bind_local_workspace".to_string(),
                )
            })?;

        let result = async {
            let session = self
                .session_store
                .get_session(&session_id)
                .await
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs(format!("session not found: {session_id}"))
                })?;

            {
                let guard = session.lock().await;
                if let Some(bound_workspace_id) = guard
                    .local_workspace_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "session `{session_id}` already bound local workspace `{bound_workspace_id}`"
                    )));
                }
            }

            if let Some(existing_binding) = self
                .local_workspace_mgr
                .get_bound_local_workspace(&session_id)
                .await?
            {
                return Err(AgentToolError::InvalidArgs(format!(
                    "session `{session_id}` already bound local workspace `{}`",
                    existing_binding.local_workspace_id
                )));
            }

            let bind_result = self
                .local_workspace_mgr
                .bind_local_workspace(&session_id, &local_workspace_id)
                .await?;
            let workspace_name = self
                .local_workspace_mgr
                .list_workspaces()
                .await
                .into_iter()
                .find(|item| item.workspace_id == local_workspace_id)
                .map(|item| item.name);
            let session_updated = persist_session_workspace_binding(
                &self.session_store,
                &session_id,
                &local_workspace_id,
                workspace_name.as_deref(),
                &bind_result,
            )
            .await?;

            Ok::<Json, AgentToolError>(json!({
                "ok": true,
                "binding": bind_result,
                "session_id": session_id,
                "session_updated": session_updated
            }))
        }
        .await;

        match &result {
            Ok(_) => {
                info!(
                    "opendan.tool_call: tool={} status=success trace_id={} session_id={} local_workspace_id={}",
                    TOOL_BIND_LOCAL_WORKSPACE,
                    ctx.trace_id,
                    session_id,
                    local_workspace_id
                );
            }
            Err(err) => {
                warn!(
                    "opendan.tool_call: tool={} status=failed trace_id={} session_id={} local_workspace_id={} err={}",
                    TOOL_BIND_LOCAL_WORKSPACE,
                    ctx.trace_id,
                    session_id,
                    local_workspace_id,
                    err
                );
            }
        }

        result
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

fn require_string_arg(args: &Json, key: &str) -> Result<String, AgentToolError> {
    optional_string_arg(args, key)?
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("missing `{key}`")))
}

fn optional_string_arg(args: &Json, key: &str) -> Result<Option<String>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

fn optional_bool_arg(args: &Json, key: &str) -> Result<Option<bool>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}

async fn persist_session_workspace_binding(
    session_store: &AgentSessionMgr,
    session_id: &str,
    workspace_id: &str,
    workspace_name: Option<&str>,
    binding: &SessionWorkspaceBinding,
) -> Result<bool, AgentToolError> {
    let Some(session) = session_store.get_session(session_id).await else {
        return Ok(false);
    };

    let mut guard = session.lock().await;
    guard.local_workspace_id = Some(workspace_id.to_string());

    let mut workspace_info = guard.workspace_info.take().unwrap_or_else(|| json!({}));
    if !workspace_info.is_object() {
        workspace_info = json!({});
    }
    workspace_info["workspace_id"] = Json::String(workspace_id.to_string());
    workspace_info["local_workspace_id"] = Json::String(workspace_id.to_string());
    if let Some(name) = workspace_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workspace_info["workspace_name"] = Json::String(name.to_string());
    }
    workspace_info["workspace_type"] = Json::String("local".to_string());
    workspace_info["binding"] = json!(binding);
    guard.workspace_info = Some(workspace_info);
    drop(guard);
    session_store.save_session(session_id).await?;
    Ok(true)
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
        if !fs::try_exists(&dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("check dir `{}` failed: {err}", dir.display()))
        })? {
            info!(
                "opendan.persist_entity_prepare: kind=workshop_root_dir path={}",
                dir.display()
            );
        }
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
    Ok(AgentWorkshopToolsConfig::default())
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

fn u64_to_usize(v: u64) -> Result<usize, AgentToolError> {
    usize::try_from(v).map_err(|_| AgentToolError::InvalidArgs(format!("value too large: {v}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_session::AgentSessionMgr;
    use crate::behavior::SessionRuntimeContext;
    use buckyos_api::{value_to_object_map, AiToolCall};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::process::Command;

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
                "on_wakeup".to_string(),
            )
            .await
            .expect("create session store"),
        );
        let session = store
            .ensure_session("session-test", Some("Session Test".to_string()), None, None)
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
                &SessionRuntimeContext {
                    trace_id: "trace-test".to_string(),
                    agent_name: "did:example:agent".to_string(),
                    behavior: "on_wakeup".to_string(),
                    step_idx: 0,
                    wakeup_id: "wakeup-test".to_string(),
                    session_id: "session-test".to_string(),
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
    async fn exec_bash_tool_can_forward_line_to_registered_tool() {
        let root = unique_workspace_root("exec-bash-forward-tool");
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
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/forward.txt",
                "content": "L1\nL2\n",
                "mode": "new"
            }),
        )
        .await
        .expect("seed file");

        let result = call(
            &tool_mgr,
            TOOL_EXEC_BASH,
            json!({
                "command": "read_file notes/forward.txt 1:1",
            }),
        )
        .await
        .expect("exec should forward to tool");

        assert_eq!(result["ok"], true);
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["engine"], "tool");
        assert_eq!(result["line_results"][0]["mode"], "tool");

        let stdout = result["stdout"].as_str().unwrap_or_default();
        let first_line = stdout.lines().next().unwrap_or_default();
        let forwarded: Json = serde_json::from_str(first_line).expect("stdout json line");
        assert_eq!(forwarded["ok"], true);
        assert!(forwarded["path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("/notes/forward.txt"));
        assert_eq!(forwarded["content"], "L1");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn exec_bash_tool_forwarded_read_file_uses_tmux_pane_cwd_for_relative_path() {
        if !tmux_ready().await {
            return;
        }
        let root = unique_workspace_root("exec-bash-pane-cwd");
        fs::create_dir_all(root.join("subdir"))
            .await
            .expect("create subdir");
        fs::write(root.join("subdir/1.txt"), "pane-line\n")
            .await
            .expect("write file in subdir");

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let session = session_store
            .get_session("session-test")
            .await
            .expect("session should exist");
        {
            let mut guard = session.lock().await;
            guard.cwd = root.join("subdir");
        }
        session_store
            .save_session("session-test")
            .await
            .expect("save session");

        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        let result = call(
            &tool_mgr,
            TOOL_EXEC_BASH,
            json!({
                "command": "cat 1.txt\nread_file 1.txt 1:1",
            }),
        )
        .await
        .expect("exec should succeed");

        assert_eq!(result["ok"], true);
        assert_eq!(result["engine"], "tmux+tool");
        let stdout = result["stdout"].as_str().unwrap_or_default();
        assert!(stdout.lines().any(|line| line.trim() == "pane-line"));

        let json_line = stdout
            .lines()
            .rev()
            .find(|line| line.trim_start().starts_with('{'))
            .expect("stdout should contain tool json line");
        let forwarded: Json = serde_json::from_str(json_line).expect("parse tool json line");
        assert_eq!(forwarded["ok"], true);
        assert_eq!(forwarded["content"], "pane-line");

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
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/todo.md",
                "content": "line1\nline2\n",
                "mode": "new"
            }),
        )
        .await
        .expect("create file should succeed");

        let result = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/todo.md",
                "pos_chunk": "line2",
                "new_content": "lineX",
                "mode": "replace"
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
    async fn write_file_tool_supports_new_append_write_modes() {
        let root = unique_workspace_root("write-file-modes");
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
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/mode.txt",
                "content": "A",
                "mode": "new"
            }),
        )
        .await
        .expect("new should create file");

        call(
            &tool_mgr,
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/mode.txt",
                "content": "B",
                "mode": "append"
            }),
        )
        .await
        .expect("append should succeed");

        call(
            &tool_mgr,
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/mode.txt",
                "content": "C",
                "mode": "write"
            }),
        )
        .await
        .expect("write should overwrite");

        let content = fs::read_to_string(root.join("notes/mode.txt"))
            .await
            .expect("read file");
        assert_eq!(content, "C");

        let err = call(
            &tool_mgr,
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/mode.txt",
                "content": "D",
                "mode": "new"
            }),
        )
        .await
        .expect_err("new on existing file should fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn edit_file_tool_supports_pos_chunk_modes_and_noop_on_miss() {
        let root = unique_workspace_root("edit-pos-chunk");
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
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/anchor.txt",
                "content": "A\nB\nC\n",
                "mode": "new"
            }),
        )
        .await
        .expect("seed file");

        let before_result = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/anchor.txt",
                "pos_chunk": "B",
                "new_content": "X\n",
                "mode": "before"
            }),
        )
        .await
        .expect("before edit should succeed");
        assert_eq!(before_result["matched"], true);
        assert_eq!(before_result["changed"], true);

        let miss_result = call(
            &tool_mgr,
            TOOL_EDIT_FILE,
            json!({
                "path": "notes/anchor.txt",
                "pos_chunk": "NOT_FOUND",
                "new_content": "Y",
                "mode": "replace"
            }),
        )
        .await
        .expect("miss should not fail");
        assert_eq!(miss_result["matched"], false);
        assert_eq!(miss_result["changed"], false);

        let content = fs::read_to_string(root.join("notes/anchor.txt"))
            .await
            .expect("read file");
        assert!(content.contains("X\nB"));
        assert!(!content.contains("NOT_FOUND"));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn read_file_tool_supports_first_chunk_and_range() {
        let root = unique_workspace_root("read-range");
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
            TOOL_WRITE_FILE,
            json!({
                "path": "notes/read.txt",
                "content": "L1\nL2\nL3\nL4\nL5\n",
                "mode": "new"
            }),
        )
        .await
        .expect("seed file");

        let result = call(
            &tool_mgr,
            TOOL_READ_FILE,
            json!({
                "path": "notes/read.txt",
                "first_chunk": "L3",
                "range": "1-2"
            }),
        )
        .await
        .expect("read should succeed");

        assert_eq!(result["matched"], true);
        assert_eq!(result["line_range"], "1-2");
        let content = result["content"].as_str().unwrap_or_default();
        assert_eq!(content, "L3\nL4");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn create_local_workspace_tool_creates_and_binds_for_session() {
        let root = unique_workspace_root("create-local-workspace");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store.clone())
            .expect("register workshop tools");

        let result = call(
            &tool_mgr,
            TOOL_CREATE_LOCAL_WORKSPACE,
            json!({
                "name": "demo-project"
            }),
        )
        .await
        .expect("create local workspace should succeed");

        assert_eq!(result["ok"], true);
        let workspace_id = result["workspace"]["workspace_id"]
            .as_str()
            .expect("workspace id");
        assert!(!workspace_id.trim().is_empty());
        let workspace_path = result["binding"]["workspace_path"]
            .as_str()
            .expect("workspace path");
        assert!(fs::try_exists(Path::new(workspace_path))
            .await
            .expect("workspace path should exist"));

        let session = session_store
            .get_session("session-test")
            .await
            .expect("session should exist");
        let guard = session.lock().await;
        assert_eq!(guard.local_workspace_id.as_deref(), Some(workspace_id));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn bind_local_workspace_tool_fails_when_session_already_bound() {
        let root = unique_workspace_root("bind-local-workspace-fail-rebind");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;

        let workspace_a = workshop
            .create_local_workspace(CreateLocalWorkspaceRequest {
                name: "repo-a".to_string(),
                template: None,
                owner: WorkspaceOwner::AgentCreated,
                created_by_session: None,
                policy_profile_id: None,
            })
            .await
            .expect("create workspace a");
        let workspace_b = workshop
            .create_local_workspace(CreateLocalWorkspaceRequest {
                name: "repo-b".to_string(),
                template: None,
                owner: WorkspaceOwner::AgentCreated,
                created_by_session: None,
                policy_profile_id: None,
            })
            .await
            .expect("create workspace b");

        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store.clone())
            .expect("register workshop tools");

        call(
            &tool_mgr,
            TOOL_BIND_LOCAL_WORKSPACE,
            json!({
                "local_workspace_id": workspace_a.workspace_id
            }),
        )
        .await
        .expect("first bind should succeed");

        let err = call(
            &tool_mgr,
            TOOL_BIND_LOCAL_WORKSPACE,
            json!({
                "local_workspace_id": workspace_b.workspace_id
            }),
        )
        .await
        .expect_err("rebind should fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
        assert!(
            err.to_string().contains("already bound local workspace"),
            "unexpected error: {err}"
        );

        let session = session_store
            .get_session("session-test")
            .await
            .expect("session should exist");
        let guard = session.lock().await;
        assert_eq!(
            guard.local_workspace_id.as_deref(),
            Some(workspace_a.workspace_id.as_str())
        );

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
    async fn tools_markdown_is_ignored_without_tools_json() {
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
        assert!(tool_mgr.has_tool(TOOL_EDIT_FILE));
        assert!(tool_mgr.has_tool(TOOL_TODO_MANAGE));
        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));
        assert!(tool_mgr.has_tool(TOOL_CREATE_LOCAL_WORKSPACE));
        assert!(tool_mgr.has_tool(TOOL_BIND_LOCAL_WORKSPACE));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn worklog_manage_tool_is_not_registered_even_if_enabled() {
        let root = unique_workspace_root("worklog-manage-removed");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "worklog_manage",
                        "enabled": true,
                        "params": {
                            "db_path": "worklog/custom.db",
                            "default_list_limit": 16,
                            "max_list_limit": 128
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

        assert!(!tool_mgr.has_tool(TOOL_WORKLOG_MANAGE));
        let err = call(
            &tool_mgr,
            TOOL_WORKLOG_MANAGE,
            json!({
                "action": "list"
            }),
        )
        .await
        .expect_err("worklog_manage should not be callable");
        assert!(matches!(err, AgentToolError::NotFound(_)));

        let _ = fs::remove_dir_all(root).await;
    }
}
