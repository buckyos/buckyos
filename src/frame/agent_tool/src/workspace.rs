use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;

use crate::{
    AgentToolError, ExternalWorkspaceBackend, SessionRuntimeContext, WorkspaceToolBackend,
    normalize_abs_path,
};

const DEFAULT_EXTERNAL_WORKSPACES_DIR: &str = "workspaces";
const DEFAULT_WORKSPACE_BINDINGS_FILE: &str = "bindings.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRecordView {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub payload: Json,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionWorkspaceBindingView {
    pub session_id: String,
    pub local_workspace_id: String,
    pub workspace_path: String,
    pub workspace_rel_path: String,
    pub agent_env_root: String,
    pub bound_at_ms: u64,
}

impl SessionWorkspaceBindingView {
    pub fn payload(&self) -> Json {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

#[async_trait]
pub trait WorkspaceRuntimeBackend: Send + Sync {
    async fn create_workspace_record(
        &self,
        session_id: &str,
        name: &str,
    ) -> Result<WorkspaceRecordView, AgentToolError>;

    async fn get_workspace_path(&self, workspace_id: &str) -> Result<PathBuf, AgentToolError>;

    async fn bind_workspace_record(
        &self,
        session_id: &str,
        workspace_id: &str,
    ) -> Result<SessionWorkspaceBindingView, AgentToolError>;

    async fn list_workspaces(&self) -> Result<Vec<WorkspaceRecordView>, AgentToolError>;

    async fn session_bound_workspace_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, AgentToolError>;

    async fn session_binding(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionWorkspaceBindingView>, AgentToolError>;

    async fn persist_session_workspace_binding(
        &self,
        session_id: &str,
        workspace_id: &str,
        workspace_name: Option<&str>,
        binding: &SessionWorkspaceBindingView,
    ) -> Result<bool, AgentToolError>;
}

#[derive(Clone)]
pub struct ManagedWorkspaceToolBackend {
    runtime: Arc<dyn WorkspaceRuntimeBackend>,
}

impl ManagedWorkspaceToolBackend {
    pub fn new(runtime: Arc<dyn WorkspaceRuntimeBackend>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl WorkspaceToolBackend for ManagedWorkspaceToolBackend {
    async fn create_workspace(
        &self,
        ctx: &SessionRuntimeContext,
        name: String,
        summary: String,
    ) -> Result<Json, AgentToolError> {
        let session_id = ctx.session_id.trim().to_string();
        if session_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }
        let summary = summary.trim().to_string();
        if summary.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace summary cannot be empty".to_string(),
            ));
        }

        let result = async {
            let workspace = self
                .runtime
                .create_workspace_record(&session_id, name.as_str())
                .await?;

            let workspace_path = self.runtime.get_workspace_path(&workspace.workspace_id).await?;
            let summary_path = workspace_path.join("SUMMARY.md");
            fs::write(&summary_path, format!("{summary}\n"))
                .await
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "write workspace summary failed: path={} err={}",
                        summary_path.display(),
                        err
                    ))
                })?;

            let binding = self
                .runtime
                .bind_workspace_record(&session_id, &workspace.workspace_id)
                .await?;
            let session_updated = self
                .runtime
                .persist_session_workspace_binding(
                    &session_id,
                    &workspace.workspace_id,
                    Some(workspace.name.as_str()),
                    &binding,
                )
                .await?;

            Ok::<Json, AgentToolError>(json!({
                "ok": true,
                "workspace": workspace.payload,
                "binding": binding.payload(),
                "summary_path": summary_path.to_string_lossy().to_string(),
                "session_id": session_id,
                "session_updated": session_updated
            }))
        }
        .await;

        match &result {
            Ok(_) => {
                info!(
                    "opendan.tool_call: tool=create_workspace status=success trace_id={} session_id={}",
                    ctx.trace_id, session_id
                );
            }
            Err(err) => {
                warn!(
                    "opendan.tool_call: tool=create_workspace status=failed trace_id={} session_id={} err={}",
                    ctx.trace_id, session_id, err
                );
            }
        }

        result
    }

    async fn resolve_workspace_id(
        &self,
        workspace_ref: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<String, AgentToolError> {
        let workspace_ref = workspace_ref.trim();
        if workspace_ref.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace argument cannot be empty".to_string(),
            ));
        }

        let workspaces = self.runtime.list_workspaces().await?;
        if let Some(found) = workspaces
            .iter()
            .find(|item| item.workspace_id == workspace_ref)
        {
            return Ok(found.workspace_id.clone());
        }

        let parsed = Path::new(workspace_ref);
        let candidate = if parsed.is_absolute() {
            parsed.to_path_buf()
        } else if let Some(cwd) = shell_cwd {
            cwd.join(parsed)
        } else {
            std::env::current_dir()
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("read current_dir failed: {err}"))
                })?
                .join(parsed)
        };
        let normalized_candidate = normalize_abs_path(&candidate);

        for item in workspaces {
            let Ok(path) = self.runtime.get_workspace_path(&item.workspace_id).await else {
                continue;
            };
            if normalize_abs_path(&path) == normalized_candidate {
                return Ok(item.workspace_id);
            }
        }

        Err(AgentToolError::InvalidArgs(format!(
            "workspace not found: `{workspace_ref}`; expected workspace_id or workspace_path"
        )))
    }

    async fn bind_workspace(
        &self,
        ctx: &SessionRuntimeContext,
        session_id: &str,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError> {
        let session_id = session_id.trim().to_string();
        let workspace_id = workspace_id.trim().to_string();
        if session_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }
        if workspace_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace_id is required".to_string(),
            ));
        }

        let result = async {
            if let Some(bound_workspace_id) =
                self.runtime.session_bound_workspace_id(&session_id).await?
            {
                return Err(AgentToolError::InvalidArgs(format!(
                    "session `{session_id}` already bound local workspace `{bound_workspace_id}`"
                )));
            }

            if let Some(existing_binding) = self.runtime.session_binding(&session_id).await? {
                return Err(AgentToolError::InvalidArgs(format!(
                    "session `{session_id}` already bound local workspace `{}`",
                    existing_binding.local_workspace_id
                )));
            }

            let binding = self
                .runtime
                .bind_workspace_record(&session_id, &workspace_id)
                .await?;
            let workspace_name = self
                .runtime
                .list_workspaces()
                .await?
                .into_iter()
                .find(|item| item.workspace_id == workspace_id)
                .map(|item| item.name);
            let session_updated = self
                .runtime
                .persist_session_workspace_binding(
                    &session_id,
                    &workspace_id,
                    workspace_name.as_deref(),
                    &binding,
                )
                .await?;

            Ok::<Json, AgentToolError>(json!({
                "ok": true,
                "binding": binding.payload(),
                "session_id": session_id,
                "session_updated": session_updated
            }))
        }
        .await;

        match &result {
            Ok(_) => {
                info!(
                    "opendan.tool_call: tool=bind_workspace status=success trace_id={} session_id={} workspace_id={}",
                    ctx.trace_id, session_id, workspace_id
                );
            }
            Err(err) => {
                warn!(
                    "opendan.tool_call: tool=bind_workspace status=failed trace_id={} session_id={} workspace_id={} err={}",
                    ctx.trace_id, session_id, workspace_id, err
                );
            }
        }

        result
    }
}

#[derive(Clone, Debug)]
pub struct ExternalWorkspaceServiceConfig {
    pub external_workspaces_dir_name: String,
    pub workspace_bindings_file_name: String,
}

impl Default for ExternalWorkspaceServiceConfig {
    fn default() -> Self {
        Self {
            external_workspaces_dir_name: DEFAULT_EXTERNAL_WORKSPACES_DIR.to_string(),
            workspace_bindings_file_name: DEFAULT_WORKSPACE_BINDINGS_FILE.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalWorkspaceBinding {
    pub name: String,
    pub source: String,
    pub mount: String,
}

#[async_trait]
pub trait ExternalWorkspaceRuntimeBackend: Send + Sync {
    async fn resolve_agent_root(&self, agent_did: &str) -> Result<PathBuf, AgentToolError>;
}

#[derive(Clone)]
pub struct ManagedExternalWorkspaceBackend {
    runtime: Arc<dyn ExternalWorkspaceRuntimeBackend>,
    cfg: ExternalWorkspaceServiceConfig,
}

impl ManagedExternalWorkspaceBackend {
    pub fn new(
        runtime: Arc<dyn ExternalWorkspaceRuntimeBackend>,
        cfg: ExternalWorkspaceServiceConfig,
    ) -> Self {
        Self { runtime, cfg }
    }

    fn workspace_bindings_path(&self, agent_root: &Path) -> PathBuf {
        agent_root
            .join(&self.cfg.external_workspaces_dir_name)
            .join(&self.cfg.workspace_bindings_file_name)
    }

    async fn read_workspace_bindings(
        &self,
        agent_root: &Path,
    ) -> Result<Vec<ExternalWorkspaceBinding>, AgentToolError> {
        let path = self.workspace_bindings_path(agent_root);
        if !fs::try_exists(&path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("check workspace bindings failed: path={} err={err}", path.display()))
        })? {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(&path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("read workspace bindings failed: path={} err={err}", path.display()))
        })?;
        let parsed: Json = serde_json::from_str(&content).map_err(|err| {
            AgentToolError::ExecFailed(format!("parse workspace bindings failed: path={} err={err}", path.display()))
        })?;
        let Some(arr) = parsed.get("bindings").and_then(|v| v.as_array()) else {
            return Ok(vec![]);
        };

        arr.iter()
            .map(|item| {
                serde_json::from_value::<ExternalWorkspaceBinding>(item.clone()).map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "parse workspace binding entry failed: path={} err={err}",
                        path.display()
                    ))
                })
            })
            .collect()
    }

    async fn write_workspace_bindings(
        &self,
        agent_root: &Path,
        bindings: &[ExternalWorkspaceBinding],
    ) -> Result<(), AgentToolError> {
        let path = self.workspace_bindings_path(agent_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "create workspace bindings dir failed: path={} err={err}",
                    parent.display()
                ))
            })?;
        }
        let payload = serde_json::to_string_pretty(&json!({ "bindings": bindings })).map_err(
            |err| AgentToolError::ExecFailed(format!("serialize workspace bindings failed: {err}")),
        )?;
        fs::write(&path, payload).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "write workspace bindings failed: path={} err={err}",
                path.display()
            ))
        })
    }
}

#[async_trait]
impl ExternalWorkspaceBackend for ManagedExternalWorkspaceBackend {
    async fn bind_external_workspace(
        &self,
        agent_did: &str,
        name: &str,
        workspace_path: &str,
    ) -> Result<Json, AgentToolError> {
        let did = agent_did.trim();
        if did.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "agent_did cannot be empty".to_string(),
            ));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "name is required".to_string(),
            ));
        }
        let raw_path = workspace_path.trim();
        if raw_path.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace_path is required".to_string(),
            ));
        }

        let agent_root = self.runtime.resolve_agent_root(did).await?;
        let source_path = to_abs_path(Path::new(raw_path))?;
        let metadata = fs::metadata(&source_path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read workspace metadata failed: path={} err={err}",
                source_path.display()
            ))
        })?;
        if !metadata.is_dir() {
            return Err(AgentToolError::InvalidArgs(format!(
                "workspace_path is not a directory: {}",
                source_path.display()
            )));
        }

        let mount_base = agent_root.join(&self.cfg.external_workspaces_dir_name);
        info!(
            "opendan.persist_entity_prepare: kind=external_workspace_mount_base agent_did={} path={}",
            did,
            mount_base.display()
        );
        fs::create_dir_all(&mount_base).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create external workspace mount base failed: path={} err={err}",
                mount_base.display()
            ))
        })?;

        let mount_path = mount_base.join(name);
        ensure_mount_link(&mount_path, &source_path).await?;

        let binding = ExternalWorkspaceBinding {
            name: name.to_string(),
            source: source_path.to_string_lossy().to_string(),
            mount: mount_path.to_string_lossy().to_string(),
        };
        let mut bindings = self.read_workspace_bindings(&agent_root).await?;
        upsert_binding(&mut bindings, binding.clone());
        self.write_workspace_bindings(&agent_root, &bindings).await?;

        serde_json::to_value(binding)
            .map_err(|err| AgentToolError::ExecFailed(format!("serialize binding failed: {err}")))
    }

    async fn list_external_workspaces(&self, agent_did: &str) -> Result<Json, AgentToolError> {
        let did = agent_did.trim();
        if did.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "agent_did cannot be empty".to_string(),
            ));
        }
        let agent_root = self.runtime.resolve_agent_root(did).await?;
        let mut bindings = self.read_workspace_bindings(&agent_root).await?;
        bindings.sort_by(|a, b| a.name.cmp(&b.name));
        serde_json::to_value(bindings).map_err(|err| {
            AgentToolError::ExecFailed(format!("serialize external workspaces failed: {err}"))
        })
    }
}

fn to_abs_path(path: &Path) -> Result<PathBuf, AgentToolError> {
    if path.is_absolute() {
        Ok(normalize_abs_path(path))
    } else {
        std::env::current_dir()
            .map(|cwd| normalize_abs_path(&cwd.join(path)))
            .map_err(|err| AgentToolError::ExecFailed(format!("read current_dir failed: {err}")))
    }
}

async fn ensure_mount_link(mount_path: &Path, source_path: &Path) -> Result<(), AgentToolError> {
    if fs::try_exists(mount_path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("check mount path failed: path={} err={err}", mount_path.display()))
    })? {
        let meta = fs::symlink_metadata(mount_path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read mount metadata failed: path={} err={err}",
                mount_path.display()
            ))
        })?;

        if meta.file_type().is_symlink() {
            let current_target = fs::read_link(mount_path).await.map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "read mount symlink failed: path={} err={err}",
                    mount_path.display()
                ))
            })?;
            if normalize_abs_path(&to_abs_path(&current_target)?) == normalize_abs_path(source_path)
            {
                return Ok(());
            }
            return Err(AgentToolError::AlreadyExists(format!(
                "mount `{}` already points to `{}`",
                mount_path.display(),
                current_target.display()
            )));
        }

        return Err(AgentToolError::AlreadyExists(format!(
            "mount path already exists and is not a symlink: {}",
            mount_path.display()
        )));
    }

    if let Some(parent) = mount_path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create mount parent dir failed: path={} err={err}",
                parent.display()
            ))
        })?;
    }

    info!(
        "opendan.persist_entity_prepare: kind=workspace_mount_symlink source={} target={}",
        source_path.display(),
        mount_path.display()
    );
    create_symlink(source_path, mount_path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "create workspace mount symlink failed: path={} err={err}",
            mount_path.display()
        ))
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use buckyos_api::{value_to_object_map, AiToolCall};
    use tempfile::tempdir;

    use crate::{
        AgentTool, AgentToolManager, BindExternalWorkspaceTool, BindWorkspaceTool,
        CreateWorkspaceTool, ListExternalWorkspacesTool, SessionRuntimeContext,
        TOOL_BIND_EXTERNAL_WORKSPACE, TOOL_BIND_WORKSPACE, TOOL_LIST_EXTERNAL_WORKSPACES,
    };

    #[derive(Default)]
    struct FakeWorkspaceState {
        seq: u64,
        workspaces: HashMap<String, WorkspaceRecordView>,
        workspace_paths: HashMap<String, PathBuf>,
        session_bindings: HashMap<String, SessionWorkspaceBindingView>,
        session_bound_workspace: HashMap<String, String>,
        session_payloads: HashMap<String, Json>,
    }

    #[derive(Clone)]
    struct FakeWorkspaceRuntime {
        root: PathBuf,
        state: Arc<tokio::sync::Mutex<FakeWorkspaceState>>,
    }

    #[async_trait]
    impl WorkspaceRuntimeBackend for FakeWorkspaceRuntime {
        async fn create_workspace_record(
            &self,
            session_id: &str,
            name: &str,
        ) -> Result<WorkspaceRecordView, AgentToolError> {
            let mut guard = self.state.lock().await;
            guard.seq += 1;
            let workspace_id = format!("ws-{}", guard.seq);
            let workspace_path = self.root.join("workspaces").join(&workspace_id);
            fs::create_dir_all(&workspace_path).await.map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "create fake workspace dir failed: path={} err={err}",
                    workspace_path.display()
                ))
            })?;
            let record = WorkspaceRecordView {
                workspace_id: workspace_id.clone(),
                name: name.to_string(),
                payload: json!({
                    "workspace_id": workspace_id,
                    "name": name,
                    "created_by_session": session_id
                }),
            };
            guard
                .workspace_paths
                .insert(record.workspace_id.clone(), workspace_path);
            guard
                .workspaces
                .insert(record.workspace_id.clone(), record.clone());
            Ok(record)
        }

        async fn get_workspace_path(&self, workspace_id: &str) -> Result<PathBuf, AgentToolError> {
            let guard = self.state.lock().await;
            guard
                .workspace_paths
                .get(workspace_id)
                .cloned()
                .ok_or_else(|| AgentToolError::InvalidArgs(format!("workspace not found: `{workspace_id}`")))
        }

        async fn bind_workspace_record(
            &self,
            session_id: &str,
            workspace_id: &str,
        ) -> Result<SessionWorkspaceBindingView, AgentToolError> {
            let mut guard = self.state.lock().await;
            let workspace_path = guard
                .workspace_paths
                .get(workspace_id)
                .cloned()
                .ok_or_else(|| AgentToolError::InvalidArgs(format!("workspace not found: `{workspace_id}`")))?;
            let binding = SessionWorkspaceBindingView {
                session_id: session_id.to_string(),
                local_workspace_id: workspace_id.to_string(),
                workspace_path: workspace_path.to_string_lossy().to_string(),
                workspace_rel_path: format!("workspaces/{workspace_id}"),
                agent_env_root: self.root.to_string_lossy().to_string(),
                bound_at_ms: 1,
            };
            guard
                .session_bindings
                .insert(session_id.to_string(), binding.clone());
            guard
                .session_bound_workspace
                .insert(session_id.to_string(), workspace_id.to_string());
            Ok(binding)
        }

        async fn list_workspaces(&self) -> Result<Vec<WorkspaceRecordView>, AgentToolError> {
            let guard = self.state.lock().await;
            Ok(guard.workspaces.values().cloned().collect())
        }

        async fn session_bound_workspace_id(
            &self,
            session_id: &str,
        ) -> Result<Option<String>, AgentToolError> {
            let guard = self.state.lock().await;
            Ok(guard.session_bound_workspace.get(session_id).cloned())
        }

        async fn session_binding(
            &self,
            session_id: &str,
        ) -> Result<Option<SessionWorkspaceBindingView>, AgentToolError> {
            let guard = self.state.lock().await;
            Ok(guard.session_bindings.get(session_id).cloned())
        }

        async fn persist_session_workspace_binding(
            &self,
            session_id: &str,
            workspace_id: &str,
            workspace_name: Option<&str>,
            binding: &SessionWorkspaceBindingView,
        ) -> Result<bool, AgentToolError> {
            let mut guard = self.state.lock().await;
            guard.session_payloads.insert(
                session_id.to_string(),
                json!({
                    "local_workspace_id": workspace_id,
                    "workspace_name": workspace_name,
                    "workspace_path": binding.workspace_path,
                }),
            );
            Ok(true)
        }
    }

    #[derive(Clone, Default)]
    struct FakeExternalRuntime {
        roots: Arc<Mutex<HashMap<String, PathBuf>>>,
    }

    #[async_trait]
    impl ExternalWorkspaceRuntimeBackend for FakeExternalRuntime {
        async fn resolve_agent_root(&self, agent_did: &str) -> Result<PathBuf, AgentToolError> {
            let guard = self.roots.lock().expect("lock roots");
            guard
                .get(agent_did)
                .cloned()
                .ok_or_else(|| AgentToolError::InvalidArgs(format!("agent not found: {agent_did}")))
        }
    }

    fn call_ctx(session_id: &str) -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "did:test:jarvis".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-1".to_string(),
            session_id: session_id.to_string(),
        }
    }

    #[tokio::test]
    async fn create_and_bind_workspace_tools_run_from_agent_tool_backend() {
        let temp = tempdir().expect("create tempdir");
        let runtime = Arc::new(FakeWorkspaceRuntime {
            root: temp.path().join("agent"),
            state: Arc::new(tokio::sync::Mutex::new(FakeWorkspaceState::default())),
        });
        let backend = Arc::new(ManagedWorkspaceToolBackend::new(runtime.clone()));
        let tool_mgr = AgentToolManager::new();
        tool_mgr
            .register_tool(CreateWorkspaceTool::new(backend.clone()))
            .expect("register create tool");
        tool_mgr
            .register_tool(BindWorkspaceTool::new(backend))
            .expect("register bind tool");

        let create_result = tool_mgr
            .call_tool_from_bash_line(
                &call_ctx("session-1"),
                "create_workspace demo-project \"Workspace structure: src/, docs/\"",
            )
            .await
            .expect("create workspace tool call")
            .expect("tool matched");

        assert_eq!(create_result["ok"], true);
        let workspace_id = create_result["workspace"]["workspace_id"]
            .as_str()
            .expect("workspace id");
        assert_eq!(workspace_id, "ws-1");

        let summary_path = PathBuf::from(
            create_result["summary_path"]
                .as_str()
                .expect("summary path"),
        );
        let summary = fs::read_to_string(summary_path)
            .await
            .expect("read summary");
        assert!(summary.contains("Workspace structure: src/, docs/"));

        let runtime = runtime.state.lock().await;
        assert_eq!(
            runtime.session_payloads["session-1"]["local_workspace_id"],
            "ws-1"
        );
        drop(runtime);

        let err = tool_mgr
            .call_tool(
                &call_ctx("session-1"),
                AiToolCall {
                    name: TOOL_BIND_WORKSPACE.to_string(),
                    args: value_to_object_map(json!({
                        "workspace": "ws-1"
                    })),
                    call_id: "call-bind".to_string(),
                },
            )
            .await
            .expect_err("non-bash bind should fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn bind_workspace_rejects_rebinding_existing_session() {
        let temp = tempdir().expect("create tempdir");
        let runtime = Arc::new(FakeWorkspaceRuntime {
            root: temp.path().join("agent"),
            state: Arc::new(tokio::sync::Mutex::new(FakeWorkspaceState::default())),
        });
        let backend = Arc::new(ManagedWorkspaceToolBackend::new(runtime.clone()));
        let tool = BindWorkspaceTool::new(backend);

        {
            let mut guard = runtime.state.lock().await;
            guard.workspaces.insert(
                "ws-demo".to_string(),
                WorkspaceRecordView {
                    workspace_id: "ws-demo".to_string(),
                    name: "demo".to_string(),
                    payload: json!({"workspace_id":"ws-demo","name":"demo"}),
                },
            );
            guard.workspace_paths.insert(
                "ws-demo".to_string(),
                temp.path().join("agent/workspaces/ws-demo"),
            );
            guard
                .session_bound_workspace
                .insert("session-1".to_string(), "ws-old".to_string());
        }

        let err = tool
            .exec(&call_ctx("session-1"), "bind_workspace ws-demo", None)
            .await
            .expect_err("rebind should fail");
        assert!(err
            .to_string()
            .contains("already bound local workspace `ws-old`"));
    }

    #[tokio::test]
    async fn external_workspace_tools_bind_and_list_from_agent_tool_backend() {
        let temp = tempdir().expect("create tempdir");
        let agent_root = temp.path().join("agents/jarvis");
        let external_workspace = temp.path().join("external-workspace");
        fs::create_dir_all(&agent_root)
            .await
            .expect("create agent root");
        fs::create_dir_all(&external_workspace)
            .await
            .expect("create external workspace");

        let runtime = FakeExternalRuntime::default();
        runtime
            .roots
            .lock()
            .expect("lock roots")
            .insert("did:test:jarvis".to_string(), agent_root.clone());
        let backend = Arc::new(ManagedExternalWorkspaceBackend::new(
            Arc::new(runtime),
            ExternalWorkspaceServiceConfig::default(),
        ));

        let tool_mgr = AgentToolManager::new();
        tool_mgr
            .register_tool(BindExternalWorkspaceTool::new(backend.clone()))
            .expect("register bind external tool");
        tool_mgr
            .register_tool(ListExternalWorkspacesTool::new(backend))
            .expect("register list external tool");

        let bind_result = tool_mgr
            .call_tool(
                &call_ctx("session-1"),
                AiToolCall {
                    name: TOOL_BIND_EXTERNAL_WORKSPACE.to_string(),
                    args: value_to_object_map(json!({
                        "name": "shared-repo",
                        "workspace_path": external_workspace.to_string_lossy().to_string()
                    })),
                    call_id: "call-bind-external".to_string(),
                },
            )
            .await
            .expect("bind external workspace");

        let mount_path = bind_result["binding"]["mount"]
            .as_str()
            .expect("mount path");
        assert!(fs::try_exists(mount_path)
            .await
            .expect("check mount exists"));

        let list_result = tool_mgr
            .call_tool(
                &call_ctx("session-1"),
                AiToolCall {
                    name: TOOL_LIST_EXTERNAL_WORKSPACES.to_string(),
                    args: value_to_object_map(json!({})),
                    call_id: "call-list-external".to_string(),
                },
            )
            .await
            .expect("list external workspaces");
        let workspaces = list_result["workspaces"]
            .as_array()
            .expect("workspaces array");
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0]["name"], "shared-repo");
    }
}
