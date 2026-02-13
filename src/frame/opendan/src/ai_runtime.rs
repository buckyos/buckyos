use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;

use crate::agent_tool::{AgentTool, ToolCallContext, ToolError, ToolManager, ToolSpec};

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
const DEFAULT_SUB_AGENT_ROLE: &str = "# Role\nYou are a specialized sub-agent.\n";
const DEFAULT_SUB_AGENT_SELF: &str = "# Self\n- Follow parent constraints\n- Keep output concise\n";

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

    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
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

    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
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

    async fn call(&self, ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
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

    fn call_ctx(agent_did: &str) -> ToolCallContext {
        ToolCallContext {
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
}
