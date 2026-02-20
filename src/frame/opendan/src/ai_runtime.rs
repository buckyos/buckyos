use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use ::kRPC::{RPCContext, RPCErrors, Result as KRPCResult};
use async_trait::async_trait;
use buckyos_api::{
    OpenDanAgentInfo, OpenDanAgentListResult, OpenDanAgentSessionListResult,
    OpenDanAgentSessionRecord, OpenDanHandler, OpenDanListAgentSessionsReq, OpenDanListAgentsReq,
    OpenDanListWorkshopSubAgentsReq, OpenDanListWorkshopTodosReq, OpenDanListWorkshopWorklogsReq,
    OpenDanServerHandler, OpenDanSubAgentInfo, OpenDanTodoItem, OpenDanWorklogItem,
    OpenDanWorkspaceInfo, OpenDanWorkspaceSubAgentsResult, OpenDanWorkspaceTodosResult,
    OpenDanWorkspaceWorklogsResult,
};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::{fs, task};

use crate::agent_tool::{AgentTool, ToolError, ToolManager, ToolSpec};
use crate::behavior::TraceCtx;

pub const TOOL_CREATE_SUB_AGENT: &str = "create_sub_agent";
pub const TOOL_BIND_EXTERNAL_WORKSPACE: &str = "bind_external_workspace";
pub const TOOL_LIST_EXTERNAL_WORKSPACES: &str = "list_external_workspaces";

const AGENT_DOC_CANDIDATES: [&str; 2] = ["agent.json.doc", "Agent.json.doc"];
const DEFAULT_SUB_AGENTS_DIR: &str = "sub-agents";
const DEFAULT_ENVIRONMENT_DIR: &str = "environment";
const DEFAULT_EXTERNAL_WORKSPACES_DIR: &str = "workspaces";
const DEFAULT_BEHAVIORS_DIR: &str = "behaviors";
const DEFAULT_MEMORY_DIR: &str = "memory";
const DEFAULT_ROLE_FILE: &str = "role.md";
const DEFAULT_SELF_FILE: &str = "self.md";
const DEFAULT_WORKSPACE_BINDINGS_FILE: &str = "bindings.json";
const DEFAULT_AGENT_SESSIONS_DIR: &str = "session";
const DEFAULT_AGENT_SESSION_FILE_NAME: &str = "session.json";
const DEFAULT_SUB_AGENT_ROLE: &str = "# Role\nYou are a specialized sub-agent.\n";
const DEFAULT_SUB_AGENT_SELF: &str = "# Self\n- Follow parent constraints\n- Keep output concise\n";
const DEFAULT_KRPC_LIST_LIMIT: usize = 64;
const MAX_KRPC_LIST_LIMIT: usize = 512;
const MAX_SESSION_ID_LEN: usize = 180;
const ACTIVE_WINDOW_MS: u64 = 120_000;

#[derive(thiserror::Error, Debug)]
pub enum AiRuntimeError {
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("io error on `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse json `{path}`: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl From<AiRuntimeError> for ToolError {
    fn from(value: AiRuntimeError) -> Self {
        match value {
            AiRuntimeError::InvalidArgs(msg) => ToolError::InvalidArgs(msg),
            AiRuntimeError::AgentNotFound(msg) => {
                ToolError::InvalidArgs(format!("agent not registered in runtime: {msg}"))
            }
            AiRuntimeError::AlreadyExists(msg) => ToolError::InvalidArgs(msg),
            AiRuntimeError::Io { path, source } => {
                ToolError::ExecFailed(format!("io error on `{path}`: {source}"))
            }
            AiRuntimeError::Json { path, source } => {
                ToolError::ExecFailed(format!("json error on `{path}`: {source}"))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct AiRuntimeConfig {
    pub agents_root: PathBuf,
    pub sub_agents_dir_name: String,
    pub environment_dir_name: String,
    pub external_workspaces_dir_name: String,
    pub role_file_name: String,
    pub self_file_name: String,
    pub workspace_bindings_file_name: String,
}

impl AiRuntimeConfig {
    pub fn new(agents_root: impl Into<PathBuf>) -> Self {
        Self {
            agents_root: agents_root.into(),
            sub_agents_dir_name: DEFAULT_SUB_AGENTS_DIR.to_string(),
            environment_dir_name: DEFAULT_ENVIRONMENT_DIR.to_string(),
            external_workspaces_dir_name: DEFAULT_EXTERNAL_WORKSPACES_DIR.to_string(),
            role_file_name: DEFAULT_ROLE_FILE.to_string(),
            self_file_name: DEFAULT_SELF_FILE.to_string(),
            workspace_bindings_file_name: DEFAULT_WORKSPACE_BINDINGS_FILE.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeAgentInfo {
    pub did: String,
    pub name: String,
    pub root: String,
    pub parent_did: Option<String>,
    pub is_sub_agent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSubAgentRequest {
    pub name: String,
    pub did: Option<String>,
    pub role_md: Option<String>,
    pub self_md: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSubAgentResult {
    pub did: String,
    pub parent_did: String,
    pub root: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalWorkspaceBinding {
    pub name: String,
    pub source: String,
    pub mount: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BindExternalWorkspaceRequest {
    pub name: String,
    pub workspace_path: String,
}

#[derive(Clone)]
pub struct AiRuntime {
    cfg: AiRuntimeConfig,
    agents_by_did: Arc<RwLock<HashMap<String, PathBuf>>>,
}

impl AiRuntime {
    pub async fn new(mut cfg: AiRuntimeConfig) -> Result<Self, AiRuntimeError> {
        validate_runtime_config(&cfg)?;
        let agents_root = normalize_abs_path(&to_abs_path(&cfg.agents_root)?);
        fs::create_dir_all(&agents_root)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: agents_root.display().to_string(),
                source,
            })?;
        cfg.agents_root = agents_root;

        let runtime = Self {
            cfg,
            agents_by_did: Arc::new(RwLock::new(HashMap::new())),
        };
        let _ = runtime.scan_agents().await?;
        Ok(runtime)
    }

    pub fn config(&self) -> &AiRuntimeConfig {
        &self.cfg
    }

    pub async fn register_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        tool_mgr.register_tool(RuntimeCreateSubAgentTool {
            runtime: Arc::new(self.clone()),
        })?;
        tool_mgr.register_tool(RuntimeBindExternalWorkspaceTool {
            runtime: Arc::new(self.clone()),
        })?;
        tool_mgr.register_tool(RuntimeListExternalWorkspacesTool {
            runtime: Arc::new(self.clone()),
        })?;
        Ok(())
    }

    pub async fn register_agent(
        &self,
        agent_did: impl AsRef<str>,
        agent_root: impl Into<PathBuf>,
    ) -> Result<(), AiRuntimeError> {
        let did = agent_did.as_ref().trim();
        if did.is_empty() {
            return Err(AiRuntimeError::InvalidArgs(
                "agent did cannot be empty".to_string(),
            ));
        }

        let root = normalize_abs_path(&to_abs_path(&agent_root.into())?);
        fs::create_dir_all(&root)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: root.display().to_string(),
                source,
            })?;

        let mut guard = self
            .agents_by_did
            .write()
            .map_err(|_| AiRuntimeError::InvalidArgs("runtime lock poisoned".to_string()))?;
        guard.insert(did.to_string(), root);
        Ok(())
    }

    pub async fn scan_agents(&self) -> Result<Vec<RuntimeAgentInfo>, AiRuntimeError> {
        let mut stack = vec![(self.cfg.agents_root.clone(), None::<String>)];
        let mut out = Vec::<RuntimeAgentInfo>::new();
        let mut discovered = HashMap::<String, PathBuf>::new();

        while let Some((dir, parent_did)) = stack.pop() {
            if !fs::try_exists(&dir)
                .await
                .map_err(|source| AiRuntimeError::Io {
                    path: dir.display().to_string(),
                    source,
                })?
            {
                continue;
            }

            let maybe_doc = find_agent_doc_path(&dir).await?;
            if let Some(doc_path) = maybe_doc {
                let did_doc = load_json_file(&doc_path).await?;
                let did = extract_agent_did(&did_doc, &dir);
                let name = dir
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("agent")
                    .to_string();

                out.push(RuntimeAgentInfo {
                    did: did.clone(),
                    name,
                    root: dir.to_string_lossy().to_string(),
                    parent_did: parent_did.clone(),
                    is_sub_agent: parent_did.is_some(),
                });
                discovered.insert(did.clone(), dir.clone());

                let sub_root = dir.join(&self.cfg.sub_agents_dir_name);
                if fs::try_exists(&sub_root)
                    .await
                    .map_err(|source| AiRuntimeError::Io {
                        path: sub_root.display().to_string(),
                        source,
                    })?
                {
                    stack.push((sub_root, Some(did)));
                }
                continue;
            }

            let mut read_dir = fs::read_dir(&dir)
                .await
                .map_err(|source| AiRuntimeError::Io {
                    path: dir.display().to_string(),
                    source,
                })?;
            while let Some(entry) =
                read_dir
                    .next_entry()
                    .await
                    .map_err(|source| AiRuntimeError::Io {
                        path: dir.display().to_string(),
                        source,
                    })?
            {
                let entry_path = entry.path();
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|source| AiRuntimeError::Io {
                        path: entry_path.display().to_string(),
                        source,
                    })?;
                if file_type.is_dir() {
                    stack.push((entry_path, parent_did.clone()));
                }
            }
        }

        {
            let mut guard = self
                .agents_by_did
                .write()
                .map_err(|_| AiRuntimeError::InvalidArgs("runtime lock poisoned".to_string()))?;
            for (did, root) in discovered {
                guard.insert(did, root);
            }
        }

        out.sort_by(|a, b| a.did.cmp(&b.did));
        Ok(out)
    }

    pub async fn create_sub_agent(
        &self,
        parent_agent_did: &str,
        req: CreateSubAgentRequest,
    ) -> Result<CreateSubAgentResult, AiRuntimeError> {
        let parent_did = parent_agent_did.trim();
        if parent_did.is_empty() {
            return Err(AiRuntimeError::InvalidArgs(
                "parent_agent_did cannot be empty".to_string(),
            ));
        }
        validate_agent_name(&req.name)?;

        let parent_root = self.lookup_agent_root(parent_did)?;
        let sub_root = parent_root
            .join(&self.cfg.sub_agents_dir_name)
            .join(req.name.trim());
        if fs::try_exists(&sub_root)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: sub_root.display().to_string(),
                source,
            })?
        {
            return Err(AiRuntimeError::AlreadyExists(format!(
                "sub-agent `{}` already exists",
                req.name
            )));
        }

        create_minimal_agent_layout(&sub_root, &self.cfg).await?;

        let did = req
            .did
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| format!("{parent_did}:{}", req.name.trim()));
        let created_at_ms = now_ms();

        let agent_doc = json!({
            "id": did,
            "name": req.name.trim(),
            "kind": "sub-agent",
            "parent_did": parent_did,
            "created_at_ms": created_at_ms
        });
        write_json_file(sub_root.join("agent.json.doc"), &agent_doc).await?;

        fs::write(
            sub_root.join(&self.cfg.role_file_name),
            req.role_md
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_SUB_AGENT_ROLE.to_string()),
        )
        .await
        .map_err(|source| AiRuntimeError::Io {
            path: sub_root
                .join(&self.cfg.role_file_name)
                .display()
                .to_string(),
            source,
        })?;

        fs::write(
            sub_root.join(&self.cfg.self_file_name),
            req.self_md
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_SUB_AGENT_SELF.to_string()),
        )
        .await
        .map_err(|source| AiRuntimeError::Io {
            path: sub_root
                .join(&self.cfg.self_file_name)
                .display()
                .to_string(),
            source,
        })?;

        let mut guard = self
            .agents_by_did
            .write()
            .map_err(|_| AiRuntimeError::InvalidArgs("runtime lock poisoned".to_string()))?;
        guard.insert(did.clone(), sub_root.clone());

        Ok(CreateSubAgentResult {
            did,
            parent_did: parent_did.to_string(),
            root: sub_root.to_string_lossy().to_string(),
            created_at_ms,
        })
    }

    pub async fn bind_external_workspace(
        &self,
        agent_did: &str,
        req: BindExternalWorkspaceRequest,
    ) -> Result<ExternalWorkspaceBinding, AiRuntimeError> {
        let did = agent_did.trim();
        if did.is_empty() {
            return Err(AiRuntimeError::InvalidArgs(
                "agent_did cannot be empty".to_string(),
            ));
        }
        validate_binding_name(&req.name)?;

        let agent_root = self.lookup_agent_root(did)?;
        let source_path = normalize_abs_path(&to_abs_path(Path::new(req.workspace_path.trim()))?);
        let metadata = fs::metadata(&source_path)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: source_path.display().to_string(),
                source,
            })?;
        if !metadata.is_dir() {
            return Err(AiRuntimeError::InvalidArgs(format!(
                "workspace_path is not a directory: {}",
                source_path.display()
            )));
        }

        let mount_base = agent_root
            .join(&self.cfg.environment_dir_name)
            .join(&self.cfg.external_workspaces_dir_name);
        fs::create_dir_all(&mount_base)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: mount_base.display().to_string(),
                source,
            })?;

        let mount_path = mount_base.join(req.name.trim());
        ensure_mount_link(&mount_path, &source_path).await?;

        let binding = ExternalWorkspaceBinding {
            name: req.name.trim().to_string(),
            source: source_path.to_string_lossy().to_string(),
            mount: mount_path.to_string_lossy().to_string(),
        };

        let mut bindings = self.read_workspace_bindings(&agent_root).await?;
        upsert_binding(&mut bindings, binding.clone());
        self.write_workspace_bindings(&agent_root, &bindings)
            .await?;

        Ok(binding)
    }

    pub async fn list_external_workspaces(
        &self,
        agent_did: &str,
    ) -> Result<Vec<ExternalWorkspaceBinding>, AiRuntimeError> {
        let did = agent_did.trim();
        if did.is_empty() {
            return Err(AiRuntimeError::InvalidArgs(
                "agent_did cannot be empty".to_string(),
            ));
        }
        let agent_root = self.lookup_agent_root(did)?;
        let mut bindings = self.read_workspace_bindings(&agent_root).await?;
        bindings.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(bindings)
    }

    fn lookup_agent_root(&self, did: &str) -> Result<PathBuf, AiRuntimeError> {
        let guard = self
            .agents_by_did
            .read()
            .map_err(|_| AiRuntimeError::InvalidArgs("runtime lock poisoned".to_string()))?;
        guard
            .get(did)
            .cloned()
            .ok_or_else(|| AiRuntimeError::AgentNotFound(did.to_string()))
    }

    async fn read_workspace_bindings(
        &self,
        agent_root: &Path,
    ) -> Result<Vec<ExternalWorkspaceBinding>, AiRuntimeError> {
        let path = self.workspace_bindings_path(agent_root);
        if !fs::try_exists(&path)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: path.display().to_string(),
                source,
            })?
        {
            return Ok(vec![]);
        }
        let content = fs::read_to_string(&path)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: path.display().to_string(),
                source,
            })?;
        let parsed: Json =
            serde_json::from_str(&content).map_err(|source| AiRuntimeError::Json {
                path: path.display().to_string(),
                source,
            })?;

        let Some(arr) = parsed.get("bindings").and_then(|v| v.as_array()) else {
            return Ok(vec![]);
        };

        let mut out = Vec::<ExternalWorkspaceBinding>::with_capacity(arr.len());
        for item in arr {
            let parsed = serde_json::from_value::<ExternalWorkspaceBinding>(item.clone()).map_err(
                |source| AiRuntimeError::Json {
                    path: path.display().to_string(),
                    source,
                },
            )?;
            out.push(parsed);
        }
        Ok(out)
    }

    async fn write_workspace_bindings(
        &self,
        agent_root: &Path,
        bindings: &[ExternalWorkspaceBinding],
    ) -> Result<(), AiRuntimeError> {
        let path = self.workspace_bindings_path(agent_root);
        let payload = json!({ "bindings": bindings });
        write_json_file(path, &payload).await
    }

    fn workspace_bindings_path(&self, agent_root: &Path) -> PathBuf {
        agent_root
            .join(&self.cfg.environment_dir_name)
            .join(&self.cfg.external_workspaces_dir_name)
            .join(&self.cfg.workspace_bindings_file_name)
    }
}

#[derive(Clone)]
pub struct OpenDanRuntimeKrpcHandler {
    runtime: Arc<AiRuntime>,
}

impl OpenDanRuntimeKrpcHandler {
    pub fn new(runtime: Arc<AiRuntime>) -> Self {
        Self { runtime }
    }

    pub fn into_server_handler(self) -> OpenDanServerHandler<Self> {
        OpenDanServerHandler::new(self)
    }

    async fn find_agent(&self, agent_id: &str) -> KRPCResult<RuntimeAgentInfo> {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err(RPCErrors::ReasonError(
                "agent_id cannot be empty".to_string(),
            ));
        }

        let agents = self
            .runtime
            .scan_agents()
            .await
            .map_err(runtime_error_to_rpc)?;
        agents
            .into_iter()
            .find(|agent| agent.did == agent_id)
            .ok_or_else(|| RPCErrors::ReasonError(format!("agent not found: {agent_id}")))
    }

    async fn to_agent_info(&self, agent: &RuntimeAgentInfo) -> OpenDanAgentInfo {
        let agent_root = PathBuf::from(&agent.root);
        let workspace_root = workspace_root_from_agent_root(&self.runtime.cfg, &agent_root);
        let todo_db = todo_db_path(&workspace_root);
        let worklog_db = worklog_db_path(&workspace_root);
        let updated_at = latest_modified_ms(&[
            workspace_root.join("worklog").join("agent-loop.jsonl"),
            worklog_db,
            todo_db,
        ])
        .await;
        let status = derive_agent_status(updated_at);

        OpenDanAgentInfo {
            agent_id: agent.did.clone(),
            agent_name: Some(agent.name.clone()),
            agent_type: Some(if agent.is_sub_agent {
                "sub".to_string()
            } else {
                "main".to_string()
            }),
            status: Some(status),
            parent_agent_id: agent.parent_did.clone(),
            current_run_id: None,
            workspace_id: Some(format!("workspace:{}", agent.did)),
            workspace_path: Some(workspace_root.to_string_lossy().to_string()),
            last_active_at: updated_at.map(|ts| ts.to_string()),
            updated_at,
            extra: Some(json!({
                "agent_root": agent.root,
            })),
        }
    }
}

#[async_trait]
impl OpenDanHandler for OpenDanRuntimeKrpcHandler {
    async fn handle_list_agents(
        &self,
        request: OpenDanListAgentsReq,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanAgentListResult> {
        let include_sub_agents = request.include_sub_agents.unwrap_or(true);
        let status_filter = request.status.as_ref().map(|value| normalize_filter(value));
        let limit = normalize_limit(request.limit);
        let offset = parse_cursor(request.cursor.as_deref())?;

        let agents = self
            .runtime
            .scan_agents()
            .await
            .map_err(runtime_error_to_rpc)?;

        let mut mapped = Vec::<OpenDanAgentInfo>::new();
        for agent in agents {
            if !include_sub_agents && agent.is_sub_agent {
                continue;
            }
            let info = self.to_agent_info(&agent).await;
            if let Some(filter) = status_filter.as_deref() {
                let status = info.status.as_deref().unwrap_or("idle");
                if normalize_filter(status) != filter {
                    continue;
                }
            }
            mapped.push(info);
        }
        mapped.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

        let total = mapped.len() as u64;
        let items = mapped
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let next_cursor = build_next_cursor(offset, items.len(), total);

        Ok(OpenDanAgentListResult {
            items,
            next_cursor,
            total: Some(total),
        })
    }

    async fn handle_get_agent(
        &self,
        agent_id: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanAgentInfo> {
        let agent = self.find_agent(agent_id).await?;
        Ok(self.to_agent_info(&agent).await)
    }

    async fn handle_get_workshop(
        &self,
        agent_id: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanWorkspaceInfo> {
        let agent = self.find_agent(agent_id).await?;
        let workspace_root =
            workspace_root_from_agent_root(&self.runtime.cfg, Path::new(&agent.root));
        let todo_db = todo_db_path(&workspace_root);
        let worklog_db = worklog_db_path(&workspace_root);

        let sub_agent_total = self
            .runtime
            .scan_agents()
            .await
            .map_err(runtime_error_to_rpc)?
            .into_iter()
            .filter(|item| item.parent_did.as_deref() == Some(agent.did.as_str()))
            .count() as u64;

        let todo_db_for_count = todo_db.clone();
        let worklog_db_for_count = worklog_db.clone();
        let agent_id_owned = agent.did.clone();
        let (todo_total, worklog_total) =
            task::spawn_blocking(move || -> KRPCResult<(u64, u64)> {
                let (_, todo_total) = query_workshop_todos_sync(
                    &todo_db_for_count,
                    &agent_id_owned,
                    None,
                    true,
                    None,
                    1,
                    0,
                )?;
                let (_, worklog_total) = query_workshop_worklogs_sync(
                    &worklog_db_for_count,
                    &agent_id_owned,
                    None,
                    None,
                    None,
                    None,
                    None,
                    1,
                    0,
                )?;
                Ok((todo_total, worklog_total))
            })
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("query workspace summary failed: {error}"))
            })??;

        Ok(OpenDanWorkspaceInfo {
            workspace_id: format!("workspace:{}", agent.did),
            agent_id: agent.did.clone(),
            workspace_path: Some(workspace_root.to_string_lossy().to_string()),
            todo_db_path: Some(todo_db.to_string_lossy().to_string()),
            worklog_db_path: Some(worklog_db.to_string_lossy().to_string()),
            summary: Some(json!({
                "todo_total": todo_total,
                "worklog_total": worklog_total,
                "sub_agent_total": sub_agent_total,
            })),
            extra: None,
        })
    }

    async fn handle_list_workshop_worklogs(
        &self,
        request: OpenDanListWorkshopWorklogsReq,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanWorkspaceWorklogsResult> {
        let agent = self.find_agent(&request.agent_id).await?;
        let workspace_root =
            workspace_root_from_agent_root(&self.runtime.cfg, Path::new(&agent.root));
        let db_path = worklog_db_path(&workspace_root);

        let limit = normalize_limit(request.limit);
        let offset = parse_cursor(request.cursor.as_deref())?;

        let agent_id = request.agent_id.clone();
        let owner_session_id = normalize_owner_session_id(request.owner_session_id.as_str())?;
        let log_type = request.log_type.clone();
        let status = request.status.clone();
        let step_id = request.step_id.clone();
        let keyword = request.keyword.clone();
        let (items, total) = task::spawn_blocking(move || {
            query_workshop_worklogs_sync(
                &db_path,
                &agent_id,
                Some(owner_session_id.as_str()),
                log_type.as_deref(),
                status.as_deref(),
                step_id.as_deref(),
                keyword.as_deref(),
                limit,
                offset,
            )
        })
        .await
        .map_err(|error| {
            RPCErrors::ReasonError(format!("list workshop worklogs join failed: {error}"))
        })??;

        Ok(OpenDanWorkspaceWorklogsResult {
            next_cursor: build_next_cursor(offset, items.len(), total),
            items,
            total: Some(total),
        })
    }

    async fn handle_list_workshop_todos(
        &self,
        request: OpenDanListWorkshopTodosReq,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanWorkspaceTodosResult> {
        let agent = self.find_agent(&request.agent_id).await?;
        let workspace_root =
            workspace_root_from_agent_root(&self.runtime.cfg, Path::new(&agent.root));
        let db_path = todo_db_path(&workspace_root);

        let limit = normalize_limit(request.limit);
        let offset = parse_cursor(request.cursor.as_deref())?;
        let include_closed = request.include_closed.unwrap_or(true);
        let agent_id = request.agent_id.clone();
        let owner_session_id = normalize_owner_session_id(request.owner_session_id.as_str())?;
        let status = request.status.clone();
        let (items, total) = task::spawn_blocking(move || {
            query_workshop_todos_sync(
                &db_path,
                &agent_id,
                status.as_deref(),
                include_closed,
                Some(owner_session_id.as_str()),
                limit,
                offset,
            )
        })
        .await
        .map_err(|error| {
            RPCErrors::ReasonError(format!("list workshop todos join failed: {error}"))
        })??;

        Ok(OpenDanWorkspaceTodosResult {
            next_cursor: build_next_cursor(offset, items.len(), total),
            items,
            total: Some(total),
        })
    }

    async fn handle_list_workshop_sub_agents(
        &self,
        request: OpenDanListWorkshopSubAgentsReq,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanWorkspaceSubAgentsResult> {
        let _ = self.find_agent(&request.agent_id).await?;
        let include_disabled = request.include_disabled.unwrap_or(true);
        let limit = normalize_limit(request.limit);
        let offset = parse_cursor(request.cursor.as_deref())?;

        let mut sub_agents = Vec::<OpenDanSubAgentInfo>::new();
        for item in self
            .runtime
            .scan_agents()
            .await
            .map_err(runtime_error_to_rpc)?
        {
            if item.parent_did.as_deref() != Some(request.agent_id.as_str()) {
                continue;
            }
            let info = self.to_agent_info(&item).await;
            if !include_disabled && info.status.as_deref() == Some("disabled") {
                continue;
            }
            sub_agents.push(OpenDanSubAgentInfo {
                agent_id: info.agent_id,
                agent_name: info.agent_name,
                status: info.status,
                current_run_id: info.current_run_id,
                last_active_at: info.last_active_at,
                workspace_id: info.workspace_id,
                workspace_path: info.workspace_path,
                extra: info.extra,
            });
        }
        sub_agents.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        let total = sub_agents.len() as u64;
        let items = sub_agents
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();

        Ok(OpenDanWorkspaceSubAgentsResult {
            next_cursor: build_next_cursor(offset, items.len(), total),
            items,
            total: Some(total),
        })
    }

    async fn handle_list_agent_sessions(
        &self,
        request: OpenDanListAgentSessionsReq,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanAgentSessionListResult> {
        let agent = self.find_agent(&request.agent_id).await?;
        let workspace_root =
            workspace_root_from_agent_root(&self.runtime.cfg, Path::new(&agent.root));
        let sessions_dir = workspace_root.join(DEFAULT_AGENT_SESSIONS_DIR);
        let limit = normalize_limit(request.limit);
        let offset = parse_cursor(request.cursor.as_deref())?;

        let (items, total) =
            task::spawn_blocking(move || list_agent_session_ids_sync(&sessions_dir, limit, offset))
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("list agent sessions join failed: {error}"))
                })??;

        Ok(OpenDanAgentSessionListResult {
            next_cursor: build_next_cursor(offset, items.len(), total),
            items,
            total: Some(total),
        })
    }

    async fn handle_get_session_record(
        &self,
        session_id: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<OpenDanAgentSessionRecord> {
        let session_id = sanitize_session_id_for_path(session_id)?;
        let session_id_for_search = session_id.clone();
        let runtime_cfg = self.runtime.cfg.clone();
        let agents = self
            .runtime
            .scan_agents()
            .await
            .map_err(runtime_error_to_rpc)?;
        let matched_path = task::spawn_blocking(move || {
            find_session_record_path_sync(
                &runtime_cfg,
                agents.as_slice(),
                session_id_for_search.as_str(),
            )
        })
        .await
        .map_err(|error| {
            RPCErrors::ReasonError(format!("find session record path join failed: {error}"))
        })??;

        task::spawn_blocking(move || {
            load_agent_session_record_sync(matched_path.as_path(), session_id.as_str())
        })
        .await
        .map_err(|error| {
            RPCErrors::ReasonError(format!("get session record join failed: {error}"))
        })?
    }
}

fn runtime_error_to_rpc(err: AiRuntimeError) -> RPCErrors {
    RPCErrors::ReasonError(err.to_string())
}

fn normalize_limit(limit: Option<u32>) -> usize {
    let value = limit.map(|v| v as usize).unwrap_or(DEFAULT_KRPC_LIST_LIMIT);
    value.clamp(1, MAX_KRPC_LIST_LIMIT)
}

fn parse_cursor(cursor: Option<&str>) -> KRPCResult<usize> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let cursor = cursor.trim();
    if cursor.is_empty() {
        return Ok(0);
    }
    let parsed = cursor.parse::<u64>().map_err(|_| {
        RPCErrors::ReasonError(format!(
            "invalid cursor `{cursor}`, expected numeric offset"
        ))
    })?;
    usize::try_from(parsed)
        .map_err(|_| RPCErrors::ReasonError(format!("cursor too large: `{cursor}`")))
}

fn build_next_cursor(offset: usize, page_len: usize, total: u64) -> Option<String> {
    let next = offset.saturating_add(page_len);
    if (next as u64) < total {
        Some(next.to_string())
    } else {
        None
    }
}

fn normalize_filter(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .replace([' ', '-'], "_")
        .to_string()
}

fn normalize_owner_session_id(value: &str) -> KRPCResult<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(RPCErrors::ReasonError(
            "owner_session_id cannot be empty".to_string(),
        ));
    }
    Ok(normalized.to_string())
}

fn workspace_root_from_agent_root(cfg: &AiRuntimeConfig, agent_root: &Path) -> PathBuf {
    agent_root.join(&cfg.environment_dir_name)
}

fn todo_db_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("todo").join("todo.db")
}

fn worklog_db_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("worklog").join("worklog.db")
}

async fn latest_modified_ms(paths: &[PathBuf]) -> Option<u64> {
    let mut latest = None::<u64>;
    for path in paths {
        let Ok(meta) = fs::metadata(path).await else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
            continue;
        };
        let ts = duration.as_millis() as u64;
        latest = Some(match latest {
            Some(current) => current.max(ts),
            None => ts,
        });
    }
    latest
}

fn derive_agent_status(updated_at: Option<u64>) -> String {
    let Some(updated_at) = updated_at else {
        return "idle".to_string();
    };
    if now_ms().saturating_sub(updated_at) <= ACTIVE_WINDOW_MS {
        "running".to_string()
    } else {
        "idle".to_string()
    }
}

fn has_table(conn: &Connection, table_name: &str) -> KRPCResult<bool> {
    let mut stmt = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1")
        .map_err(|error| {
            RPCErrors::ReasonError(format!(
                "prepare sqlite table check failed for `{table_name}`: {error}"
            ))
        })?;
    let mut rows = stmt.query(params![table_name]).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "query sqlite table check failed for `{table_name}`: {error}"
        ))
    })?;
    rows.next().map(|row| row.is_some()).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "read sqlite table check failed for `{table_name}`: {error}"
        ))
    })
}

fn query_workshop_todos_sync(
    db_path: &Path,
    agent_id: &str,
    status: Option<&str>,
    include_closed: bool,
    owner_session_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> KRPCResult<(Vec<OpenDanTodoItem>, u64)> {
    if !db_path.exists() {
        return Ok((vec![], 0));
    }
    let conn = Connection::open(db_path).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "open todo db `{}` failed: {error}",
            db_path.display()
        ))
    })?;
    if !has_table(&conn, "todos")? {
        return Ok((vec![], 0));
    }

    let mut where_sql = String::from(" WHERE 1=1");
    let mut where_params = Vec::<SqlValue>::new();

    if let Some(status_filter) = status.map(normalize_filter) {
        match status_filter.as_str() {
            "open" => where_sql.push_str(" AND status NOT IN ('done','cancelled')"),
            "done" => where_sql.push_str(" AND status IN ('done','cancelled')"),
            raw => {
                where_sql.push_str(" AND status = ?");
                where_params.push(SqlValue::Text(raw.to_string()));
            }
        }
    }
    if !include_closed {
        where_sql.push_str(" AND status NOT IN ('done','cancelled')");
    }
    if let Some(v) = owner_session_id.map(str::trim).filter(|v| !v.is_empty()) {
        where_sql.push_str(" AND owner_session_id = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }

    let count_sql = format!("SELECT COUNT(1) FROM todos{}", where_sql);
    let mut count_stmt = conn.prepare(&count_sql).map_err(|error| {
        RPCErrors::ReasonError(format!("prepare todo count query failed: {error}"))
    })?;
    let total = count_stmt
        .query_row(params_from_iter(where_params.clone()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| RPCErrors::ReasonError(format!("query todo count failed: {error}")))?;
    let total = total.max(0) as u64;

    let mut list_sql = format!(
        "SELECT id, title, description, status, priority, tags_json, task_id, task_status, created_at, updated_at
        FROM todos{}",
        where_sql
    );
    list_sql.push_str(" ORDER BY updated_at DESC, created_at DESC LIMIT ? OFFSET ?");

    let mut list_params = where_params;
    list_params.push(SqlValue::Integer(limit as i64));
    list_params.push(SqlValue::Integer(offset as i64));

    let mut stmt = conn.prepare(&list_sql).map_err(|error| {
        RPCErrors::ReasonError(format!("prepare todo list query failed: {error}"))
    })?;
    let mut rows = stmt
        .query(params_from_iter(list_params))
        .map_err(|error| RPCErrors::ReasonError(format!("query todo list failed: {error}")))?;

    let mut items = Vec::<OpenDanTodoItem>::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| RPCErrors::ReasonError(format!("read todo row failed: {error}")))?
    {
        let todo_id: String = row.get(0).unwrap_or_default();
        let title: String = row.get(1).unwrap_or_default();
        let description: String = row.get(2).unwrap_or_default();
        let raw_status: String = row.get(3).unwrap_or_else(|_| "todo".to_string());
        let priority: String = row.get(4).unwrap_or_else(|_| "normal".to_string());
        let tags_json: String = row.get(5).unwrap_or_else(|_| "[]".to_string());
        let tags = serde_json::from_str::<Json>(&tags_json).unwrap_or_else(|_| json!([]));
        let task_id: Option<i64> = row.get(6).unwrap_or(None);
        let task_status: Option<String> = row.get(7).unwrap_or(None);
        let created_at = row.get::<_, i64>(8).unwrap_or(0).max(0) as u64;
        let updated_at = row.get::<_, i64>(9).unwrap_or(0).max(0) as u64;

        let status = if raw_status == "done" || raw_status == "cancelled" {
            "done".to_string()
        } else {
            "open".to_string()
        };

        items.push(OpenDanTodoItem {
            todo_id,
            title,
            status: status.clone(),
            agent_id: Some(agent_id.to_string()),
            description: if description.is_empty() {
                None
            } else {
                Some(description)
            },
            created_at: Some(created_at),
            completed_at: if status == "done" {
                Some(updated_at)
            } else {
                None
            },
            created_in_step_id: None,
            completed_in_step_id: None,
            extra: Some(json!({
                "raw_status": raw_status,
                "priority": priority,
                "tags": tags,
                "task_id": task_id,
                "task_status": task_status,
            })),
        });
    }

    Ok((items, total))
}

fn query_workshop_worklogs_sync(
    db_path: &Path,
    agent_id_hint: &str,
    owner_session_id: Option<&str>,
    log_type: Option<&str>,
    status: Option<&str>,
    step_id: Option<&str>,
    keyword: Option<&str>,
    limit: usize,
    offset: usize,
) -> KRPCResult<(Vec<OpenDanWorklogItem>, u64)> {
    if !db_path.exists() {
        return Ok((vec![], 0));
    }
    let conn = Connection::open(db_path).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "open worklog db `{}` failed: {error}",
            db_path.display()
        ))
    })?;
    if !has_table(&conn, "worklogs")? {
        return Ok((vec![], 0));
    }

    let mut where_sql = String::from(" WHERE 1=1");
    let mut where_params = Vec::<SqlValue>::new();

    if let Some(v) = log_type.map(normalize_filter) {
        where_sql.push_str(" AND log_type = ?");
        where_params.push(SqlValue::Text(v));
    }
    if let Some(v) = status.map(normalize_filter) {
        where_sql.push_str(" AND status = ?");
        where_params.push(SqlValue::Text(v));
    }
    if let Some(v) = step_id.map(str::trim).filter(|v| !v.is_empty()) {
        where_sql.push_str(" AND step_id = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }
    if let Some(v) = keyword.map(str::trim).filter(|v| !v.is_empty()) {
        let pattern = format!("%{v}%");
        where_sql.push_str(" AND (summary LIKE ? OR payload_json LIKE ?)");
        where_params.push(SqlValue::Text(pattern.clone()));
        where_params.push(SqlValue::Text(pattern));
    }
    if let Some(v) = owner_session_id.map(str::trim).filter(|v| !v.is_empty()) {
        where_sql.push_str(" AND owner_session_id = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }

    let count_sql = format!("SELECT COUNT(1) FROM worklogs{}", where_sql);
    let mut count_stmt = conn.prepare(&count_sql).map_err(|error| {
        RPCErrors::ReasonError(format!("prepare worklog count query failed: {error}"))
    })?;
    let total = count_stmt
        .query_row(params_from_iter(where_params.clone()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| RPCErrors::ReasonError(format!("query worklog count failed: {error}")))?;
    let total = total.max(0) as u64;

    let mut list_sql = format!(
        "SELECT log_id, log_type, status, timestamp, agent_id, related_agent_id, step_id, summary, payload_json
        FROM worklogs{}",
        where_sql
    );
    list_sql.push_str(" ORDER BY timestamp DESC, created_at DESC LIMIT ? OFFSET ?");

    let mut list_params = where_params;
    list_params.push(SqlValue::Integer(limit as i64));
    list_params.push(SqlValue::Integer(offset as i64));

    let mut stmt = conn.prepare(&list_sql).map_err(|error| {
        RPCErrors::ReasonError(format!("prepare worklog list query failed: {error}"))
    })?;
    let mut rows = stmt
        .query(params_from_iter(list_params))
        .map_err(|error| RPCErrors::ReasonError(format!("query worklog list failed: {error}")))?;

    let mut items = Vec::<OpenDanWorklogItem>::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| RPCErrors::ReasonError(format!("read worklog row failed: {error}")))?
    {
        let log_id: String = row.get(0).unwrap_or_default();
        let row_log_type: String = row.get(1).unwrap_or_default();
        let row_status: String = row.get(2).unwrap_or_else(|_| "info".to_string());
        let timestamp = row.get::<_, i64>(3).unwrap_or(0).max(0) as u64;
        let row_agent_id: Option<String> = row.get(4).unwrap_or(None);
        let related_agent_id: Option<String> = row.get(5).unwrap_or(None);
        let step_id: Option<String> = row.get(6).unwrap_or(None);
        let summary: Option<String> = row.get(7).unwrap_or(None);
        let payload_json: String = row.get(8).unwrap_or_else(|_| "{}".to_string());
        let payload = serde_json::from_str::<Json>(&payload_json).ok();

        items.push(OpenDanWorklogItem {
            log_id,
            log_type: row_log_type,
            status: row_status,
            timestamp,
            agent_id: row_agent_id.or_else(|| Some(agent_id_hint.to_string())),
            related_agent_id,
            step_id,
            summary,
            payload,
        });
    }

    Ok((items, total))
}

fn list_agent_session_ids_sync(
    sessions_dir: &Path,
    limit: usize,
    offset: usize,
) -> KRPCResult<(Vec<String>, u64)> {
    if !sessions_dir.exists() {
        return Ok((vec![], 0));
    }

    let mut ids = Vec::<String>::new();
    let read_dir = std::fs::read_dir(sessions_dir).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "read sessions dir `{}` failed: {error}",
            sessions_dir.display()
        ))
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "iterate sessions dir `{}` failed: {error}",
                sessions_dir.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            RPCErrors::ReasonError(format!(
                "read session entry type `{}` failed: {error}",
                entry.path().display()
            ))
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let Some(raw_session_id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if sanitize_session_id_for_path(raw_session_id.as_str()).is_err() {
            continue;
        }

        let session_file_path = entry.path().join(DEFAULT_AGENT_SESSION_FILE_NAME);
        if !session_file_path.is_file() {
            continue;
        }
        ids.push(raw_session_id);
    }

    ids.sort();
    let total = ids.len() as u64;
    let items = ids.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
    Ok((items, total))
}

fn find_session_record_path_sync(
    runtime_cfg: &AiRuntimeConfig,
    agents: &[RuntimeAgentInfo],
    session_id: &str,
) -> KRPCResult<PathBuf> {
    let mut matched = Vec::<PathBuf>::new();
    for agent in agents {
        let workspace_root =
            workspace_root_from_agent_root(runtime_cfg, Path::new(agent.root.as_str()));
        let candidate = workspace_root
            .join(DEFAULT_AGENT_SESSIONS_DIR)
            .join(session_id)
            .join(DEFAULT_AGENT_SESSION_FILE_NAME);
        if candidate.is_file() {
            matched.push(candidate);
            if matched.len() > 1 {
                break;
            }
        }
    }

    match matched.len() {
        0 => Err(RPCErrors::ReasonError(format!(
            "session not found: {session_id}"
        ))),
        1 => Ok(matched.remove(0)),
        _ => Err(RPCErrors::ReasonError(format!(
            "duplicate session_id detected (expected globally unique): {session_id}"
        ))),
    }
}

fn load_agent_session_record_sync(
    session_file_path: &Path,
    session_id: &str,
) -> KRPCResult<OpenDanAgentSessionRecord> {
    if !session_file_path.is_file() {
        return Err(RPCErrors::ReasonError(format!(
            "session not found: {session_id}"
        )));
    }

    let raw = std::fs::read_to_string(session_file_path).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "read session `{}` failed: {error}",
            session_file_path.display()
        ))
    })?;
    let mut record = serde_json::from_str::<OpenDanAgentSessionRecord>(&raw).map_err(|error| {
        RPCErrors::ReasonError(format!(
            "parse session `{}` failed: {error}",
            session_file_path.display()
        ))
    })?;
    record.session_id = session_id.to_string();
    if !record.meta.is_object() {
        record.meta = json!({});
    }
    Ok(record)
}

fn sanitize_session_id_for_path(session_id: &str) -> KRPCResult<String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(RPCErrors::ReasonError(
            "session_id cannot be empty".to_string(),
        ));
    }
    if session_id.len() > MAX_SESSION_ID_LEN {
        return Err(RPCErrors::ReasonError(format!(
            "session_id too long (>{MAX_SESSION_ID_LEN})"
        )));
    }
    if session_id == "." || session_id == ".." {
        return Err(RPCErrors::ReasonError(
            "session_id cannot be `.` or `..`".to_string(),
        ));
    }
    if session_id.contains('/') || session_id.contains('\\') {
        return Err(RPCErrors::ReasonError(
            "session_id cannot contain path separators".to_string(),
        ));
    }
    if session_id.chars().any(|ch| ch.is_control()) {
        return Err(RPCErrors::ReasonError(
            "session_id cannot contain control characters".to_string(),
        ));
    }
    Ok(session_id.to_string())
}

#[derive(Clone)]
struct RuntimeCreateSubAgentTool {
    runtime: Arc<AiRuntime>,
}

#[async_trait]
impl AgentTool for RuntimeCreateSubAgentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CREATE_SUB_AGENT.to_string(),
            description: "Create a sub-agent under current agent runtime root.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Sub-agent local name." },
                    "did": { "type": "string", "description": "Optional sub-agent DID." },
                    "parent_did": { "type": "string", "description": "Optional parent DID. Defaults to current agent DID." },
                    "role_md": { "type": "string" },
                    "self_md": { "type": "string" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "sub_agent": { "type": "object" }
                }
            }),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let parent_did = optional_string(&args, "parent_did")?.unwrap_or(ctx.agent_did.clone());
        let req = CreateSubAgentRequest {
            name: require_string(&args, "name")?,
            did: optional_string(&args, "did")?,
            role_md: optional_string(&args, "role_md")?,
            self_md: optional_string(&args, "self_md")?,
        };

        let result = self.runtime.create_sub_agent(&parent_did, req).await?;
        Ok(json!({
            "ok": true,
            "sub_agent": result
        }))
    }
}

#[derive(Clone)]
struct RuntimeBindExternalWorkspaceTool {
    runtime: Arc<AiRuntime>,
}

#[async_trait]
impl AgentTool for RuntimeBindExternalWorkspaceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BIND_EXTERNAL_WORKSPACE.to_string(),
            description:
                "Bind an external workspace directory so this agent can access it from runtime."
                    .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Local mount name." },
                    "workspace_path": { "type": "string", "description": "Absolute or relative source workspace path." },
                    "agent_did": { "type": "string", "description": "Optional target agent DID. Defaults to current agent DID." }
                },
                "required": ["name", "workspace_path"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "binding": { "type": "object" }
                }
            }),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let agent_did = optional_string(&args, "agent_did")?.unwrap_or(ctx.agent_did.clone());
        let req = BindExternalWorkspaceRequest {
            name: require_string(&args, "name")?,
            workspace_path: require_string(&args, "workspace_path")?,
        };

        let binding = self
            .runtime
            .bind_external_workspace(&agent_did, req)
            .await?;
        Ok(json!({
            "ok": true,
            "binding": binding
        }))
    }
}

#[derive(Clone)]
struct RuntimeListExternalWorkspacesTool {
    runtime: Arc<AiRuntime>,
}

#[async_trait]
impl AgentTool for RuntimeListExternalWorkspacesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LIST_EXTERNAL_WORKSPACES.to_string(),
            description: "List bound external workspaces visible to current agent.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "agent_did": { "type": "string", "description": "Optional agent DID. Defaults to current agent DID." }
                },
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "workspaces": { "type": "array", "items": { "type": "object" } }
                }
            }),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let agent_did = optional_string(&args, "agent_did")?.unwrap_or(ctx.agent_did.clone());
        let workspaces = self.runtime.list_external_workspaces(&agent_did).await?;
        Ok(json!({
            "ok": true,
            "workspaces": workspaces
        }))
    }
}

fn validate_runtime_config(cfg: &AiRuntimeConfig) -> Result<(), AiRuntimeError> {
    if cfg.sub_agents_dir_name.trim().is_empty()
        || cfg.environment_dir_name.trim().is_empty()
        || cfg.external_workspaces_dir_name.trim().is_empty()
        || cfg.role_file_name.trim().is_empty()
        || cfg.self_file_name.trim().is_empty()
        || cfg.workspace_bindings_file_name.trim().is_empty()
    {
        return Err(AiRuntimeError::InvalidArgs(
            "runtime config cannot contain empty path segment".to_string(),
        ));
    }
    Ok(())
}

fn validate_agent_name(name: &str) -> Result<(), AiRuntimeError> {
    validate_simple_name(name, "sub-agent name")
}

fn validate_binding_name(name: &str) -> Result<(), AiRuntimeError> {
    validate_simple_name(name, "workspace binding name")
}

fn validate_simple_name(value: &str, field: &str) -> Result<(), AiRuntimeError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AiRuntimeError::InvalidArgs(format!(
            "{field} cannot be empty"
        )));
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains('/') || trimmed.contains('\\') {
        return Err(AiRuntimeError::InvalidArgs(format!(
            "{field} must be a plain directory name"
        )));
    }
    Ok(())
}

fn to_abs_path(path: &Path) -> Result<PathBuf, AiRuntimeError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|source| AiRuntimeError::Io {
        path: ".".to_string(),
        source,
    })?;
    Ok(cwd.join(path))
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn create_minimal_agent_layout(
    agent_root: &Path,
    cfg: &AiRuntimeConfig,
) -> Result<(), AiRuntimeError> {
    let dirs = [
        agent_root.to_path_buf(),
        agent_root.join(DEFAULT_BEHAVIORS_DIR),
        agent_root.join(DEFAULT_MEMORY_DIR),
        agent_root.join(&cfg.sub_agents_dir_name),
        agent_root.join(&cfg.environment_dir_name),
        agent_root.join(&cfg.environment_dir_name).join("todo"),
        agent_root.join(&cfg.environment_dir_name).join("tools"),
        agent_root.join(&cfg.environment_dir_name).join("artifacts"),
        agent_root.join(&cfg.environment_dir_name).join("worklog"),
        agent_root
            .join(&cfg.environment_dir_name)
            .join(&cfg.external_workspaces_dir_name),
    ];

    for dir in dirs {
        fs::create_dir_all(&dir)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: dir.display().to_string(),
                source,
            })?;
    }

    Ok(())
}

async fn write_json_file(path: PathBuf, value: &Json) -> Result<(), AiRuntimeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: parent.display().to_string(),
                source,
            })?;
    }
    let payload = serde_json::to_string_pretty(value).map_err(|source| AiRuntimeError::Json {
        path: path.display().to_string(),
        source,
    })?;
    fs::write(&path, payload)
        .await
        .map_err(|source| AiRuntimeError::Io {
            path: path.display().to_string(),
            source,
        })
}

async fn load_json_file(path: &Path) -> Result<Json, AiRuntimeError> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|source| AiRuntimeError::Io {
            path: path.display().to_string(),
            source,
        })?;
    serde_json::from_str::<Json>(&content).map_err(|source| AiRuntimeError::Json {
        path: path.display().to_string(),
        source,
    })
}

async fn find_agent_doc_path(root: &Path) -> Result<Option<PathBuf>, AiRuntimeError> {
    for name in AGENT_DOC_CANDIDATES {
        let path = root.join(name);
        if fs::try_exists(&path)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: path.display().to_string(),
                source,
            })?
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn extract_agent_did(did_document: &Json, agent_root: &Path) -> String {
    did_document
        .get("id")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            did_document
                .get("did")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .unwrap_or_else(|| {
            let dir_name = agent_root
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("agent");
            format!("did:opendan:{dir_name}")
        })
}

async fn ensure_mount_link(mount_path: &Path, source_path: &Path) -> Result<(), AiRuntimeError> {
    if fs::try_exists(mount_path)
        .await
        .map_err(|source| AiRuntimeError::Io {
            path: mount_path.display().to_string(),
            source,
        })?
    {
        let meta = fs::symlink_metadata(mount_path)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: mount_path.display().to_string(),
                source,
            })?;

        if meta.file_type().is_symlink() {
            let current_target =
                fs::read_link(mount_path)
                    .await
                    .map_err(|source| AiRuntimeError::Io {
                        path: mount_path.display().to_string(),
                        source,
                    })?;
            if normalize_abs_path(&to_abs_path(&current_target)?) == normalize_abs_path(source_path)
            {
                return Ok(());
            }
            return Err(AiRuntimeError::AlreadyExists(format!(
                "mount `{}` already points to `{}`",
                mount_path.display(),
                current_target.display()
            )));
        }

        return Err(AiRuntimeError::AlreadyExists(format!(
            "mount path already exists and is not a symlink: {}",
            mount_path.display()
        )));
    }

    if let Some(parent) = mount_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| AiRuntimeError::Io {
                path: parent.display().to_string(),
                source,
            })?;
    }

    create_symlink(source_path, mount_path)
        .await
        .map_err(|source| AiRuntimeError::Io {
            path: mount_path.display().to_string(),
            source,
        })
}

async fn create_symlink(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    let source = source.to_path_buf();
    let target = target.to_path_buf();
    tokio::task::spawn_blocking(move || symlink_dir_impl(&source, &target))
        .await
        .map_err(|err| std::io::Error::other(format!("join symlink task failed: {err}")))?
}

#[cfg(unix)]
fn symlink_dir_impl(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn symlink_dir_impl(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    std::os::windows::fs::symlink_dir(source, target)
}

fn upsert_binding(bindings: &mut Vec<ExternalWorkspaceBinding>, binding: ExternalWorkspaceBinding) {
    if let Some(existing) = bindings.iter_mut().find(|item| item.name == binding.name) {
        *existing = binding;
        return;
    }
    bindings.push(binding);
}

fn require_string(args: &Json, key: &str) -> Result<String, ToolError> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` is required")))?;
    Ok(value.to_string())
}

fn optional_string(args: &Json, key: &str) -> Result<Option<String>, ToolError> {
    match args.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(ToolError::InvalidArgs(format!("`{key}` must be a string"))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::tempdir;

    use super::*;

    async fn write_agent_doc(path: &Path, did: &str) {
        fs::create_dir_all(path).await.expect("create agent dir");
        fs::write(
            path.join("agent.json.doc"),
            json!({
                "id": did,
                "name": did
            })
            .to_string(),
        )
        .await
        .expect("write agent doc");
    }

    async fn write_session_record(
        agent_root: &Path,
        session_id: &str,
        owner_agent: &str,
        title: &str,
        last_activity_ms: u64,
    ) {
        let session_path = agent_root
            .join(DEFAULT_ENVIRONMENT_DIR)
            .join(DEFAULT_AGENT_SESSIONS_DIR)
            .join(session_id);
        fs::create_dir_all(&session_path)
            .await
            .expect("create session dir");
        fs::write(
            session_path.join(DEFAULT_AGENT_SESSION_FILE_NAME),
            json!({
                "session_id": session_id,
                "owner_agent": owner_agent,
                "title": title,
                "summary": "",
                "status": "active",
                "created_at_ms": 1,
                "updated_at_ms": 2,
                "last_activity_ms": last_activity_ms,
                "links": [],
                "tags": [],
                "meta": {}
            })
            .to_string(),
        )
        .await
        .expect("write session record");
    }

    fn call_ctx(agent_did: &str) -> TraceCtx {
        TraceCtx {
            trace_id: "trace-1".to_string(),
            agent_did: agent_did.to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
        }
    }

    #[tokio::test]
    async fn scan_agents_detects_root_and_nested_sub_agents() {
        let tmp = tempdir().expect("create tempdir");
        let root = tmp.path().join("agents");

        let root_agent = root.join("jarvis");
        let sub_agent = root_agent.join("sub-agents").join("web-agent");
        let nested_sub_agent = sub_agent.join("sub-agents").join("crawler-agent");

        write_agent_doc(&root_agent, "did:test:jarvis").await;
        write_agent_doc(&sub_agent, "did:test:web-agent").await;
        write_agent_doc(&nested_sub_agent, "did:test:crawler-agent").await;

        let runtime = AiRuntime::new(AiRuntimeConfig::new(&root))
            .await
            .expect("create runtime");

        let agents = runtime.scan_agents().await.expect("scan agents");
        assert_eq!(agents.len(), 3);

        let root_info = agents
            .iter()
            .find(|item| item.did == "did:test:jarvis")
            .expect("find root agent");
        assert!(!root_info.is_sub_agent);
        assert_eq!(root_info.parent_did, None);

        let web_info = agents
            .iter()
            .find(|item| item.did == "did:test:web-agent")
            .expect("find first sub agent");
        assert!(web_info.is_sub_agent);
        assert_eq!(web_info.parent_did.as_deref(), Some("did:test:jarvis"));

        let crawler_info = agents
            .iter()
            .find(|item| item.did == "did:test:crawler-agent")
            .expect("find nested sub agent");
        assert!(crawler_info.is_sub_agent);
        assert_eq!(
            crawler_info.parent_did.as_deref(),
            Some("did:test:web-agent")
        );
    }

    #[tokio::test]
    async fn runtime_tools_create_sub_agent_and_bind_external_workspace() {
        let tmp = tempdir().expect("create tempdir");
        let agents_root = tmp.path().join("agents");
        let parent_root = agents_root.join("jarvis");
        let external_workspace = tmp.path().join("external-workspace");

        write_agent_doc(&parent_root, "did:test:jarvis").await;
        fs::create_dir_all(&external_workspace)
            .await
            .expect("create external workspace");

        let runtime = AiRuntime::new(AiRuntimeConfig::new(&agents_root))
            .await
            .expect("create runtime");
        runtime
            .register_agent("did:test:jarvis", &parent_root)
            .await
            .expect("register root agent");

        let tool_mgr = ToolManager::new();
        runtime
            .register_tools(&tool_mgr)
            .await
            .expect("register runtime tools");

        let create_result = tool_mgr
            .call_tool(
                &call_ctx("did:test:jarvis"),
                crate::agent_tool::ToolCall {
                    name: TOOL_CREATE_SUB_AGENT.to_string(),
                    args: json!({
                        "name": "web-agent",
                        "role_md": "# Role\nWeb specialist\n",
                        "self_md": "# Self\n- browser only\n"
                    }),
                    call_id: "call-create-sub-agent".to_string(),
                },
            )
            .await
            .expect("create sub agent via tool");

        let sub_did = create_result["sub_agent"]["did"]
            .as_str()
            .expect("read sub did");
        assert_eq!(sub_did, "did:test:jarvis:web-agent");

        let sub_root = parent_root.join("sub-agents").join("web-agent");
        assert!(fs::try_exists(sub_root.join("agent.json.doc"))
            .await
            .expect("check sub agent doc"));

        let bind_result = tool_mgr
            .call_tool(
                &call_ctx("did:test:jarvis"),
                crate::agent_tool::ToolCall {
                    name: TOOL_BIND_EXTERNAL_WORKSPACE.to_string(),
                    args: json!({
                        "name": "shared-repo",
                        "workspace_path": external_workspace.to_string_lossy().to_string()
                    }),
                    call_id: "call-bind-workspace".to_string(),
                },
            )
            .await
            .expect("bind workspace via tool");

        let mount_path = bind_result["binding"]["mount"]
            .as_str()
            .expect("read mount path");
        assert!(fs::try_exists(mount_path)
            .await
            .expect("check mount path exists"));

        let list_result = tool_mgr
            .call_tool(
                &call_ctx("did:test:jarvis"),
                crate::agent_tool::ToolCall {
                    name: TOOL_LIST_EXTERNAL_WORKSPACES.to_string(),
                    args: json!({}),
                    call_id: "call-list-workspaces".to_string(),
                },
            )
            .await
            .expect("list bound workspaces via tool");

        let workspaces = list_result["workspaces"]
            .as_array()
            .expect("workspaces array");
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0]["name"], "shared-repo");
        assert_eq!(
            workspaces[0]["source"],
            external_workspace.to_string_lossy().to_string()
        );
    }

    #[tokio::test]
    async fn opendan_handler_lists_agent_sessions_with_pagination() {
        let tmp = tempdir().expect("create tempdir");
        let agents_root = tmp.path().join("agents");
        let agent_root = agents_root.join("jarvis");
        let did = "did:test:jarvis";

        write_agent_doc(&agent_root, did).await;
        write_session_record(&agent_root, "session-c", did, "Session C", 30).await;
        write_session_record(&agent_root, "session-a", did, "Session A", 10).await;
        write_session_record(&agent_root, "session-b", did, "Session B", 20).await;
        fs::create_dir_all(
            agent_root
                .join(DEFAULT_ENVIRONMENT_DIR)
                .join(DEFAULT_AGENT_SESSIONS_DIR)
                .join("session-without-file"),
        )
        .await
        .expect("create ignored session dir");

        let runtime = Arc::new(
            AiRuntime::new(AiRuntimeConfig::new(&agents_root))
                .await
                .expect("create runtime"),
        );
        let handler = OpenDanRuntimeKrpcHandler::new(runtime);

        let first_page = handler
            .handle_list_agent_sessions(
                OpenDanListAgentSessionsReq::new(did.to_string(), Some(2), None),
                RPCContext::default(),
            )
            .await
            .expect("list first page");
        assert_eq!(first_page.items, vec!["session-a", "session-b"]);
        assert_eq!(first_page.total, Some(3));
        assert_eq!(first_page.next_cursor.as_deref(), Some("2"));

        let second_page = handler
            .handle_list_agent_sessions(
                OpenDanListAgentSessionsReq::new(
                    did.to_string(),
                    Some(2),
                    first_page.next_cursor.clone(),
                ),
                RPCContext::default(),
            )
            .await
            .expect("list second page");
        assert_eq!(second_page.items, vec!["session-c"]);
        assert_eq!(second_page.total, Some(3));
        assert!(second_page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn opendan_handler_can_get_session_record() {
        let tmp = tempdir().expect("create tempdir");
        let agents_root = tmp.path().join("agents");
        let agent_root = agents_root.join("jarvis");
        let did = "did:test:jarvis";

        write_agent_doc(&agent_root, did).await;
        write_session_record(&agent_root, "session-001", did, "Primary Session", 88).await;

        let runtime = Arc::new(
            AiRuntime::new(AiRuntimeConfig::new(&agents_root))
                .await
                .expect("create runtime"),
        );
        let handler = OpenDanRuntimeKrpcHandler::new(runtime);

        let session = handler
            .handle_get_session_record("session-001", RPCContext::default())
            .await
            .expect("get session");
        assert_eq!(session.session_id, "session-001");
        assert_eq!(session.owner_agent, did);
        assert_eq!(session.title, "Primary Session");
        assert_eq!(session.last_activity_ms, 88);

        let missing = handler
            .handle_get_session_record("session-404", RPCContext::default())
            .await;
        assert!(missing.is_err());

        let invalid = handler
            .handle_get_session_record("../bad", RPCContext::default())
            .await;
        assert!(invalid.is_err());
    }

    #[tokio::test]
    async fn opendan_handler_rejects_duplicate_global_session_id() {
        let tmp = tempdir().expect("create tempdir");
        let agents_root = tmp.path().join("agents");
        let agent_root_a = agents_root.join("jarvis");
        let agent_root_b = agents_root.join("vision");
        let did_a = "did:test:jarvis";
        let did_b = "did:test:vision";

        write_agent_doc(&agent_root_a, did_a).await;
        write_agent_doc(&agent_root_b, did_b).await;
        write_session_record(&agent_root_a, "session-dup-001", did_a, "Session A", 11).await;
        write_session_record(&agent_root_b, "session-dup-001", did_b, "Session B", 22).await;

        let runtime = Arc::new(
            AiRuntime::new(AiRuntimeConfig::new(&agents_root))
                .await
                .expect("create runtime"),
        );
        let handler = OpenDanRuntimeKrpcHandler::new(runtime);

        let result = handler
            .handle_get_session_record("session-dup-001", RPCContext::default())
            .await;
        assert!(result.is_err());
    }
}
