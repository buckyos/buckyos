use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::TaskManagerClient;
use log::info;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;

use super::agent_skill::{AgentSkillRecord, AgentSkillSpec};
use super::local_workspace::{
    CreateLocalWorkspaceRequest, LocalWorkspaceCleanupResult, LocalWorkspaceLockResult,
    LocalWorkspaceManager, LocalWorkspaceManagerConfig, LocalWorkspaceSnapshot,
    SessionWorkspaceBinding, WorkshopIndex, WorkshopWorkspaceRecord, WorkspaceOwner,
};
use crate::agent_bash::ExecBashTool as BuiltinExecBashTool;
use crate::agent_session::AgentSessionMgr;
use crate::agent_tool::{
    read_string_from_map, read_u64_from_map, AgentToolError, AgentToolManager,
    BindWorkspaceTool as SharedBindWorkspaceTool, CreateWorkspaceTool as SharedCreateWorkspaceTool,
    EditFileTool as SharedEditFileTool, FileToolConfig, MCPToolConfig, ManagedWorkspaceToolBackend,
    ReadFileTool as SharedReadFileTool, SessionWorkspaceBindingView, WorkspaceRecordView,
    TodoTool, TodoToolConfig, WorkspaceRuntimeBackend, WriteFileTool as SharedWriteFileTool,
    TOOL_BIND_WORKSPACE, TOOL_CREATE_WORKSPACE, TOOL_EDIT_FILE, TOOL_EXEC_BASH, TOOL_READ_FILE,
    TOOL_TODO_MANAGE, TOOL_WORKLOG_MANAGE, TOOL_WRITE_FILE,
};
use crate::buildin_tool::WorkshopWriteAudit as BuiltinWorkshopWriteAudit;
use crate::agent_tool::{normalize_root_path, u64_to_usize_arg};
use ::agent_tool::resolve_path_under_root as resolve_path_in_agent_env;
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
    pub agent_env_root: PathBuf,
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
    pub fn new(agent_env_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_env_root: agent_env_root.into(),
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
                WorkshopToolConfig::enabled(TOOL_CREATE_WORKSPACE),
                WorkshopToolConfig::enabled(TOOL_BIND_WORKSPACE),
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
        let agent_env_root = normalize_root_path(&cfg.agent_env_root)?;
        create_minimal_agent_env_dirs(&agent_env_root).await?;
        cfg.agent_env_root = agent_env_root.clone();

        let local_cfg = LocalWorkspaceManagerConfig {
            agent_env_root: agent_env_root.clone(),
            lock_ttl_ms: cfg.local_workspace_lock_ttl_ms,
        };
        let local_workspace_mgr = if create_if_missing {
            LocalWorkspaceManager::create_workshop(cfg.agent_did.clone(), local_cfg).await?
        } else {
            LocalWorkspaceManager::load_workshop(cfg.agent_did.clone(), local_cfg).await?
        };

        let tools_cfg = load_tools_config(&agent_env_root, &cfg).await?;
        validate_tools_config(&tools_cfg)?;

        Ok(Self {
            cfg,
            tools_cfg,
            local_workspace_mgr,
        })
    }

    pub fn agent_env_root(&self) -> &Path {
        &self.cfg.agent_env_root
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
        self.register_tools_with_task_mgr(tool_mgr, session_store, None)
    }

    pub fn register_tools_with_task_mgr(
        &self,
        tool_mgr: &AgentToolManager,
        session_store: Arc<AgentSessionMgr>,
        task_mgr: Option<Arc<TaskManagerClient>>,
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
                        tool_mgr.register_tool(
                            BuiltinExecBashTool::from_tool_config_with_task_mgr(
                                &self.cfg,
                                tool,
                                session_store.clone(),
                                task_mgr.clone(),
                            )?,
                        )?;
                    }
                    TOOL_EDIT_FILE => {
                        let _ = tool;
                        tool_mgr.register_tool(SharedEditFileTool::new(
                            FileToolConfig::new(self.cfg.agent_env_root.clone()),
                            Arc::new(write_audit.clone()),
                        ))?;
                    }
                    TOOL_WRITE_FILE => {
                        let _ = tool;
                        tool_mgr.register_tool(SharedWriteFileTool::new(
                            FileToolConfig::new(self.cfg.agent_env_root.clone()),
                            Arc::new(write_audit.clone()),
                        ))?;
                    }
                    TOOL_READ_FILE => {
                        let _ = tool;
                        tool_mgr.register_tool(SharedReadFileTool::new(FileToolConfig::new(
                            self.cfg.agent_env_root.clone(),
                        )))?;
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
                    TOOL_CREATE_WORKSPACE => {
                        let runtime = Arc::new(OpenDanWorkspaceRuntime::new(
                            self.local_workspace_mgr.clone(),
                            session_store.clone(),
                        ));
                        tool_mgr.register_tool(SharedCreateWorkspaceTool::new(Arc::new(
                            ManagedWorkspaceToolBackend::new(runtime),
                        )))?;
                    }
                    TOOL_BIND_WORKSPACE => {
                        let runtime = Arc::new(OpenDanWorkspaceRuntime::new(
                            self.local_workspace_mgr.clone(),
                            session_store.clone(),
                        ));
                        tool_mgr.register_tool(SharedBindWorkspaceTool::new(Arc::new(
                            ManagedWorkspaceToolBackend::new(runtime),
                        )))?;
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

        Ok(WorklogToolConfig::with_db_path(resolve_path_in_agent_env(
            &self.cfg.agent_env_root,
            DEFAULT_WORKLOG_DB_REL_PATH,
        )?))
    }
}

#[derive(Clone)]
struct OpenDanWorkspaceRuntime {
    local_workspace_mgr: LocalWorkspaceManager,
    session_store: Arc<AgentSessionMgr>,
}

impl OpenDanWorkspaceRuntime {
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
impl WorkspaceRuntimeBackend for OpenDanWorkspaceRuntime {
    async fn create_workspace_record(
        &self,
        session_id: &str,
        name: &str,
    ) -> Result<WorkspaceRecordView, AgentToolError> {
        let workspace = self
            .local_workspace_mgr
            .create_local_workspace(CreateLocalWorkspaceRequest {
                name: name.to_string(),
                template: None,
                owner: WorkspaceOwner::AgentCreated,
                created_by_session: Some(session_id.to_string()),
                policy_profile_id: None,
            })
            .await?;

        Ok(WorkspaceRecordView {
            workspace_id: workspace.workspace_id.clone(),
            name: workspace.name.clone(),
            payload: serde_json::to_value(&workspace).map_err(|err| {
                AgentToolError::ExecFailed(format!("serialize workspace record failed: {err}"))
            })?,
        })
    }

    async fn get_workspace_path(&self, workspace_id: &str) -> Result<PathBuf, AgentToolError> {
        self.local_workspace_mgr
            .get_local_workspace_path(workspace_id)
            .await
    }

    async fn bind_workspace_record(
        &self,
        session_id: &str,
        workspace_id: &str,
    ) -> Result<SessionWorkspaceBindingView, AgentToolError> {
        let binding = self
            .local_workspace_mgr
            .bind_local_workspace(session_id, workspace_id)
            .await?;
        Ok(SessionWorkspaceBindingView {
            session_id: binding.session_id,
            local_workspace_id: binding.local_workspace_id,
            workspace_path: binding.workspace_path,
            workspace_rel_path: binding.workspace_rel_path,
            agent_env_root: binding.agent_env_root,
            bound_at_ms: binding.bound_at_ms,
        })
    }

    async fn list_workspaces(&self) -> Result<Vec<WorkspaceRecordView>, AgentToolError> {
        self.local_workspace_mgr
            .list_workspaces()
            .await
            .into_iter()
            .map(|item| {
                Ok(WorkspaceRecordView {
                    workspace_id: item.workspace_id.clone(),
                    name: item.name.clone(),
                    payload: serde_json::to_value(&item).map_err(|err| {
                        AgentToolError::ExecFailed(format!(
                            "serialize workspace record failed: {err}"
                        ))
                    })?,
                })
            })
            .collect()
    }

    async fn session_bound_workspace_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, AgentToolError> {
        let session = self
            .session_store
            .get_session(session_id)
            .await
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!("session not found: {session_id}"))
            })?;
        let guard = session.lock().await;
        Ok(guard
            .local_workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string()))
    }

    async fn session_binding(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionWorkspaceBindingView>, AgentToolError> {
        Ok(self
            .local_workspace_mgr
            .get_bound_local_workspace(session_id)
            .await?
            .map(|binding| SessionWorkspaceBindingView {
                session_id: binding.session_id,
                local_workspace_id: binding.local_workspace_id,
                workspace_path: binding.workspace_path,
                workspace_rel_path: binding.workspace_rel_path,
                agent_env_root: binding.agent_env_root,
                bound_at_ms: binding.bound_at_ms,
            }))
    }

    async fn persist_session_workspace_binding(
        &self,
        session_id: &str,
        workspace_id: &str,
        workspace_name: Option<&str>,
        binding: &SessionWorkspaceBindingView,
    ) -> Result<bool, AgentToolError> {
        persist_session_workspace_binding(
            &self.session_store,
            session_id,
            workspace_id,
            workspace_name,
            &SessionWorkspaceBinding {
                session_id: binding.session_id.clone(),
                local_workspace_id: binding.local_workspace_id.clone(),
                workspace_path: binding.workspace_path.clone(),
                workspace_rel_path: binding.workspace_rel_path.clone(),
                agent_env_root: binding.agent_env_root.clone(),
                bound_at_ms: binding.bound_at_ms,
            },
        )
        .await
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
            resolve_path_in_agent_env(&workshop_cfg.agent_env_root, &raw_db_path)?
        } else {
            resolve_path_in_agent_env(&workshop_cfg.agent_env_root, DEFAULT_TODO_DB_REL_PATH)?
        };

        let default_list_limit = read_u64_from_map(params, "default_list_limit")?
            .map(|value| u64_to_usize_arg(value, "default_list_limit"))
            .transpose()?
            .unwrap_or(DEFAULT_TODO_LIST_LIMIT);
        let max_list_limit = read_u64_from_map(params, "max_list_limit")?
            .map(|value| u64_to_usize_arg(value, "max_list_limit"))
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
            resolve_path_in_agent_env(&workshop_cfg.agent_env_root, &raw_db_path)?
        } else {
            resolve_path_in_agent_env(&workshop_cfg.agent_env_root, DEFAULT_WORKLOG_DB_REL_PATH)?
        };

        let default_list_limit = read_u64_from_map(params, "default_list_limit")?
            .map(|value| u64_to_usize_arg(value, "default_list_limit"))
            .transpose()?
            .unwrap_or(DEFAULT_WORKLOG_LIST_LIMIT);
        let max_list_limit = read_u64_from_map(params, "max_list_limit")?
            .map(|value| u64_to_usize_arg(value, "max_list_limit"))
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
    let workspace_root = binding.workspace_path.trim();
    if !workspace_root.is_empty() {
        guard.pwd = PathBuf::from(workspace_root);
    }

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

async fn create_minimal_agent_env_dirs(agent_env_root: &Path) -> Result<(), AgentToolError> {
    let roots = [
        agent_env_root.to_path_buf(),
        agent_env_root.join("memory"),
        agent_env_root.join("skills"),
        agent_env_root.join("sessions"),
        agent_env_root.join("workspaces"),
        agent_env_root.join("worklog"),
        agent_env_root.join("todo"),
        agent_env_root.join("tools"),
        agent_env_root.join("artifacts"),
    ];
    for dir in roots {
        if !fs::try_exists(&dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("check dir `{}` failed: {err}", dir.display()))
        })? {
            info!(
                "opendan.persist_entity_prepare: kind=agent_env_root_dir path={}",
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
    agent_env_root: &Path,
    workshop_cfg: &AgentWorkshopConfig,
) -> Result<AgentWorkshopToolsConfig, AgentToolError> {
    let tools_json_path = agent_env_root.join(&workshop_cfg.tools_json_rel_path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_session::AgentSessionMgr;
    use crate::agent_tool::{AgentToolResult, DoAction, DoActions};
    use crate::behavior::SessionRuntimeContext;
    use crate::test_utils::MockTaskMgrHandler;
    use buckyos_api::{value_to_object_map, AiToolCall, TaskManagerClient, TaskStatus};
    use std::collections::HashMap;
    use std::fs as std_fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::process::Command;
    use tokio::time::Duration;

    fn unique_agent_env_root(test_name: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("opendan-{test_name}-{ts}"))
    }

    async fn create_session_store(root: &Path) -> Arc<AgentSessionMgr> {
        create_session_store_with_id(root, "session-test").await
    }

    async fn create_session_store_with_id(root: &Path, session_id: &str) -> Arc<AgentSessionMgr> {
        let store = Arc::new(
            AgentSessionMgr::new(
                "did:example:agent".to_string(),
                root.join("sessions"),
                "on_wakeup".to_string(),
            )
            .await
            .expect("create session store"),
        );
        let session = store
            .ensure_session(session_id, Some("Session Test".to_string()), None, None)
            .await
            .expect("ensure session");
        {
            let mut guard = session.lock().await;
            guard.pwd = root.to_path_buf();
        }
        store.save_session(session_id).await.expect("save session");
        store
    }

    async fn call(
        tool_mgr: &AgentToolManager,
        name: &str,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        call_with_session_id(tool_mgr, "session-test", name, args).await
    }

    async fn call_with_session_id(
        tool_mgr: &AgentToolManager,
        session_id: &str,
        name: &str,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        tool_mgr
            .call_tool(
                &SessionRuntimeContext {
                    trace_id: "trace-test".to_string(),
                    agent_name: "did:example:agent".to_string(),
                    behavior: "on_wakeup".to_string(),
                    step_idx: 0,
                    wakeup_id: "wakeup-test".to_string(),
                    session_id: session_id.to_string(),
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

    fn parse_cli_json_line(text: &str) -> Json {
        serde_json::from_str(text.trim()).expect("stdout should be cli json")
    }

    fn parse_last_cli_json_line(text: &str) -> Json {
        let line = text
            .lines()
            .rev()
            .find(|line| line.trim_start().starts_with('{'))
            .expect("stdout should contain cli json line");
        parse_cli_json_line(line)
    }

    fn ensure_test_agent_tool_bin() {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .expect("resolve current exe dir");
        let agent_tool_path = exe_dir.join("agent_tool");
        let script = r#"#!/bin/sh
cmd="$(basename "$0")"
if [ "$cmd" = "agent_tool" ]; then
  cmd="$1"
  shift
fi
case "$cmd" in
  read_file)
    path=""
    while [ $# -gt 0 ]; do
      case "$1" in
        path=*) path="${1#path=}" ;;
        *) if [ -z "$path" ]; then path="$1"; fi ;;
      esac
      shift
    done
    line="$(sed -n '1p' "$path" 2>/dev/null)"
    printf '{"status":"success","tool":"read_file","detail":{"content":"%s"}}\n' "$line"
    ;;
  *)
    printf '{"status":"error","tool":"%s"}\n' "$cmd"
    exit 1
    ;;
esac
"#;
        std_fs::write(&agent_tool_path, script).expect("write fake agent_tool");
        #[cfg(unix)]
        {
            let mut perms = std_fs::metadata(&agent_tool_path)
                .expect("stat fake agent_tool")
                .permissions();
            perms.set_mode(0o755);
            std_fs::set_permissions(&agent_tool_path, perms).expect("chmod fake agent_tool");
        }
    }

    #[tokio::test]
    async fn exec_bash_tool_runs_linux_command() {
        if !tmux_ready().await {
            return;
        }
        ensure_test_agent_tool_bin();
        let root = unique_agent_env_root("exec-bash");
        let session_id = "session-tmux-linux";
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store_with_id(&root, session_id).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        let result = call_with_session_id(
            &tool_mgr,
            session_id,
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
    async fn exec_bash_tool_timeout_returns_pending_and_completes_task() {
        if !tmux_ready().await {
            return;
        }
        ensure_test_agent_tool_bin();

        let root = unique_agent_env_root("exec-bash-pending-task");
        let session_id = "session-exec-bash-pending";
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store_with_id(&root, session_id).await;
        let tool_mgr = AgentToolManager::new();
        let task_mgr = Arc::new(TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: Arc::new(Mutex::new(HashMap::new())),
            },
        )));
        workshop
            .register_tools_with_task_mgr(&tool_mgr, session_store, Some(task_mgr.clone()))
            .expect("register workshop tools with task_mgr");

        let result = call_with_session_id(
            &tool_mgr,
            session_id,
            TOOL_EXEC_BASH,
            json!({
                "command": "sleep 0.2\nprintf 'done'",
                "timeout_ms": 30,
            }),
        )
        .await
        .expect("exec bash should return pending");

        assert_eq!(result["status"], "pending");
        assert_eq!(result["pending_reason"], "long_running");
        let task_id = result["task_id"]
            .as_str()
            .expect("pending task id should exist")
            .parse::<i64>()
            .expect("task id should parse");

        tokio::time::timeout(
            Duration::from_secs(5),
            task_mgr.wait_for_task_end_with_interval(task_id, Duration::from_millis(100)),
        )
        .await
        .expect("task should finish within timeout")
        .expect("wait for task end");
        let task = task_mgr.get_task(task_id).await.expect("get task");
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.data["status"], "success");
        assert_eq!(task.data["exit_code"], 0);
        assert_eq!(task.data["stdout"], "done");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn exec_bash_tool_can_forward_line_to_registered_tool() {
        ensure_test_agent_tool_bin();
        let root = unique_agent_env_root("exec-bash-forward-tool");
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
        .expect("exec should run via cli");

        assert_eq!(result["ok"], true);
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["engine"], "tmux");
        assert_eq!(result["line_results"][0]["mode"], "bash");

        let stdout = result["stdout"].as_str().unwrap_or_default();
        let payload = parse_cli_json_line(stdout);
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["tool"], TOOL_READ_FILE);
        assert_eq!(payload["detail"]["content"], "L1");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn exec_bash_tool_forwarded_read_file_uses_tmux_pane_cwd_for_relative_path() {
        if !tmux_ready().await {
            return;
        }
        ensure_test_agent_tool_bin();
        let root = unique_agent_env_root("exec-bash-pane-cwd");
        let session_id = "session-tmux-pane-cwd";
        fs::create_dir_all(root.join("subdir"))
            .await
            .expect("create subdir");
        fs::write(root.join("subdir/1.txt"), "pane-line\n")
            .await
            .expect("write file in subdir");

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store_with_id(&root, session_id).await;
        let session = session_store
            .get_session(session_id)
            .await
            .expect("session should exist");
        {
            let mut guard = session.lock().await;
            guard.pwd = root.join("subdir");
        }
        session_store
            .save_session(session_id)
            .await
            .expect("save session");

        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        let result = call_with_session_id(
            &tool_mgr,
            session_id,
            TOOL_EXEC_BASH,
            json!({
                "command": "cat 1.txt\nread_file 1.txt 1:1",
            }),
        )
        .await
        .expect("exec should succeed");

        assert_eq!(result["ok"], true);
        assert_eq!(result["engine"], "tmux");
        let stdout = result["stdout"].as_str().unwrap_or_default();
        let pane_lines = stdout
            .lines()
            .filter(|line| line.trim() == "pane-line")
            .count();
        assert_eq!(pane_lines, 1, "stdout={stdout}");
        let payload = parse_last_cli_json_line(stdout);
        assert_eq!(payload["tool"], TOOL_READ_FILE);
        assert_eq!(payload["detail"]["content"], "pane-line");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn exec_bash_tool_cd_updates_tmux_pane_pwd_for_forwarded_read_file() {
        if !tmux_ready().await {
            return;
        }
        ensure_test_agent_tool_bin();
        let root = unique_agent_env_root("exec-bash-cd-pane-pwd");
        let session_id = "session-tmux-cd-pane-pwd";
        fs::create_dir_all(root.join("subdir"))
            .await
            .expect("create subdir");
        fs::write(root.join("subdir/1.txt"), "cd-pane-line\n")
            .await
            .expect("write file in subdir");

        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store_with_id(&root, session_id).await;
        let session = session_store
            .get_session(session_id)
            .await
            .expect("session should exist");
        {
            let mut guard = session.lock().await;
            guard.pwd = root.clone();
        }
        session_store
            .save_session(session_id)
            .await
            .expect("save session");

        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        let result = call_with_session_id(
            &tool_mgr,
            session_id,
            TOOL_EXEC_BASH,
            json!({
                "command": "cd subdir\nread_file 1.txt 1:1",
            }),
        )
        .await
        .expect("exec should succeed");

        assert_eq!(result["ok"], true);
        assert_eq!(result["engine"], "tmux");
        assert_eq!(result["line_results"][0]["mode"], "bash");
        assert_eq!(result["line_results"][1]["mode"], "bash");
        assert_eq!(result["line_results"][0]["command"], "cd subdir");
        assert!(result["pwd"]
            .as_str()
            .unwrap_or_default()
            .ends_with("/subdir"));

        let stdout = result["stdout"].as_str().unwrap_or_default();
        let payload = parse_cli_json_line(stdout);
        assert_eq!(payload["tool"], TOOL_READ_FILE);
        assert_eq!(payload["detail"]["content"], "cd-pane-line");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn do_actions_mixed_cmds_and_calls_support_builtin_tools_in_bash_cmd() {
        let root = unique_agent_env_root("do-actions-mixed");
        let workshop = AgentWorkshop::new(AgentWorkshopConfig::new(&root))
            .await
            .expect("create workshop");
        let session_store = create_session_store(&root).await;
        let tool_mgr = AgentToolManager::new();
        workshop
            .register_tools(&tool_mgr, session_store)
            .expect("register workshop tools");

        let actions: DoActions = serde_json::from_value(json!({
            "mode": "all",
            "cmds": [
                ["write_file", {
                    "path": "notes/mixed.txt",
                    "content": "L1\nL2\n",
                    "mode": "new"
                }],
                "read_file notes/mixed.txt 1:1",
                {"write_file": {
                    "path": "notes/mixed.txt",
                    "content": "L1\nL2-updated\n",
                    "mode": "write"
                }},
                "read_file notes/mixed.txt 1:2",
                ["read_file", {
                    "path": "notes/mixed.txt",
                    "range": "1:2"
                }]
            ]
        }))
        .expect("raw json should deserialize as DoActions");

        assert_eq!(actions.mode, "all");
        assert_eq!(actions.cmds.len(), 5);
        let exec_count = actions
            .cmds
            .iter()
            .filter(|item| matches!(item, DoAction::Exec(_)))
            .count();
        let call_count = actions
            .cmds
            .iter()
            .filter(|item| matches!(item, DoAction::Call(_)))
            .count();
        assert_eq!(exec_count, 2);
        assert_eq!(call_count, 3);

        let mut results = Vec::with_capacity(actions.cmds.len());
        for (idx, action) in actions.cmds.iter().enumerate() {
            let result = match action {
                DoAction::Exec(command) => {
                    call(&tool_mgr, TOOL_EXEC_BASH, json!({ "command": command }))
                        .await
                        .expect("exec action should run")
                }
                DoAction::Call(call_action) => call(
                    &tool_mgr,
                    call_action.call_action_name.as_str(),
                    call_action.call_params.clone(),
                )
                .await
                .expect("call action should run"),
            };
            assert_eq!(
                result["ok"], true,
                "action #{idx} should succeed: {}",
                result
            );
            results.push(result);
        }

        let first_bash = &results[1];
        assert_eq!(first_bash["engine"], "tmux");
        let first_payload = parse_cli_json_line(first_bash["stdout"].as_str().unwrap_or_default());
        assert_eq!(first_payload["detail"]["content"], "L1");

        let second_bash = &results[3];
        assert_eq!(second_bash["engine"], "tmux");
        let second_payload =
            parse_cli_json_line(second_bash["stdout"].as_str().unwrap_or_default());
        assert_eq!(second_payload["detail"]["content"], "L1\nL2-updated");

        let final_call = &results[4];
        assert_eq!(final_call["content"], "L1\nL2-updated");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn edit_file_tool_writes_file_and_returns_diff() {
        let root = unique_agent_env_root("edit-file");
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
        let root = unique_agent_env_root("write-file-modes");
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
        let root = unique_agent_env_root("edit-pos-chunk");
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
        let root = unique_agent_env_root("read-range");
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

        let chunk_to_end = call(
            &tool_mgr,
            TOOL_READ_FILE,
            json!({
                "path": "notes/read.txt",
                "first_chunk": "L3",
                "range": "-"
            }),
        )
        .await
        .expect("read from chunk to end should succeed");
        assert_eq!(chunk_to_end["matched"], true);
        assert_eq!(chunk_to_end["line_range"], "");
        let chunk_content = chunk_to_end["content"].as_str().unwrap_or_default();
        assert_eq!(chunk_content, "L3\nL4\nL5\n");

        let last_line_in_chunk = call(
            &tool_mgr,
            TOOL_READ_FILE,
            json!({
                "path": "notes/read.txt",
                "first_chunk": "L3",
                "range": "-1"
            }),
        )
        .await
        .expect("read last line in chunk should succeed");
        assert_eq!(last_line_in_chunk["matched"], true);
        assert_eq!(last_line_in_chunk["content"], "L5");

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_config_can_enable_subset_of_runtime_tools() {
        let root = unique_agent_env_root("tool-subset");
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

        assert!(tool_mgr.get_action(TOOL_EDIT_FILE).is_some());
        assert!(!tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(tool_mgr.get_bash_cmd(TOOL_TODO_MANAGE).is_none());
        assert!(tool_mgr.get_bash_cmd(TOOL_WORKLOG_MANAGE).is_none());

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
        let root = unique_agent_env_root("tool-policy");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "write_file",
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
            TOOL_WRITE_FILE,
            json!({
                "path": "todo/ok.md",
                "content": "ok",
                "mode": "new"
            }),
        )
        .await
        .expect("path under policy root should be writable");

        let err = call(
            &tool_mgr,
            TOOL_WRITE_FILE,
            json!({
                "path": "artifacts/out.md",
                "content": "blocked",
                "mode": "new"
            }),
        )
        .await
        .expect_err("path outside policy root should be denied");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_config_can_register_mcp_tool() {
        let root = unique_agent_env_root("tool-mcp");
        write_tools_json(
            &root,
            json!({
                "enabled_tools": [
                    {
                        "name": "weather",
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

        assert!(tool_mgr.get_bash_cmd("weather").is_some());
        assert!(tool_mgr.get_action("weather").is_some());
        assert!(!tool_mgr.has_tool(TOOL_EXEC_BASH));
        assert!(tool_mgr.get_action(TOOL_EDIT_FILE).is_none());
        assert!(tool_mgr.get_bash_cmd(TOOL_TODO_MANAGE).is_none());
        assert!(tool_mgr.get_bash_cmd(TOOL_WORKLOG_MANAGE).is_none());

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn tools_markdown_is_ignored_without_tools_json() {
        let root = unique_agent_env_root("tool-md");
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
        assert!(tool_mgr.get_action(TOOL_EDIT_FILE).is_some());
        assert!(tool_mgr.get_bash_cmd("todo").is_some());
        assert!(tool_mgr.get_bash_cmd(TOOL_WORKLOG_MANAGE).is_none());
        assert!(tool_mgr.get_bash_cmd(TOOL_CREATE_WORKSPACE).is_some());
        assert!(tool_mgr.get_bash_cmd(TOOL_BIND_WORKSPACE).is_some());

        let _ = fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn worklog_manage_tool_is_not_registered_even_if_enabled() {
        let root = unique_agent_env_root("worklog-manage-removed");
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
