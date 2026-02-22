use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::Mutex;

use crate::agent_tool::ToolError;

const DEFAULT_LOCK_TTL_MS: u64 = 120_000;
const MAX_WORKSPACE_NAME_LEN: usize = 96;
const MAX_POLICY_PROFILE_ID_LEN: usize = 128;
const WORKSHOP_INDEX_FILE_NAME: &str = "index.json";
const SESSION_BINDINGS_REL_PATH: &str = "sessions/local_workspace_bindings.json";
const LOCAL_WORKSPACES_REL_PATH: &str = "workspaces/local";

static LOCAL_WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct LocalWorkspaceManagerConfig {
    pub workshop_root: PathBuf,
    pub lock_ttl_ms: u64,
}

impl LocalWorkspaceManagerConfig {
    pub fn new(workshop_root: impl Into<PathBuf>) -> Self {
        Self {
            workshop_root: workshop_root.into(),
            lock_ttl_ms: DEFAULT_LOCK_TTL_MS,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceType {
    Local,
    Remote,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceOwner {
    AgentCreated,
    UserProvided,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
    Ready,
    Syncing,
    Archived,
    Error,
    Conflict,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceErrorSummary {
    pub code: String,
    pub summary: String,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalWorkspaceLock {
    pub session_id: String,
    pub acquired_at_ms: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalWorkspaceSessionBinding {
    pub session_id: String,
    pub bound_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkshopWorkspaceRecord {
    pub workspace_id: String,
    pub workspace_type: WorkspaceType,
    pub owner: WorkspaceOwner,
    pub name: String,
    pub relative_path: Option<String>,
    pub created_by_session: Option<String>,
    pub policy_profile_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_sync_at_ms: Option<u64>,
    pub status: WorkspaceStatus,
    pub conflict_or_error: Option<WorkspaceErrorSummary>,
    pub lock: Option<LocalWorkspaceLock>,
    pub bound_sessions: Vec<LocalWorkspaceSessionBinding>,
}

impl Default for WorkshopWorkspaceRecord {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            workspace_type: WorkspaceType::Local,
            owner: WorkspaceOwner::AgentCreated,
            name: String::new(),
            relative_path: None,
            created_by_session: None,
            policy_profile_id: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            last_sync_at_ms: None,
            status: WorkspaceStatus::Ready,
            conflict_or_error: None,
            lock: None,
            bound_sessions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkshopIndex {
    pub agent_did: String,
    pub workspaces: Vec<WorkshopWorkspaceRecord>,
    pub updated_at_ms: u64,
}

impl Default for WorkshopIndex {
    fn default() -> Self {
        Self {
            agent_did: String::new(),
            workspaces: Vec::new(),
            updated_at_ms: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SessionWorkspaceBinding {
    pub session_id: String,
    pub local_workspace_id: String,
    pub workspace_path: String,
    pub bound_at_ms: u64,
}

impl Default for SessionWorkspaceBinding {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            local_workspace_id: String::new(),
            workspace_path: String::new(),
            bound_at_ms: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct SessionBindingsFile {
    bindings: Vec<SessionWorkspaceBinding>,
}

impl Default for SessionBindingsFile {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreateLocalWorkspaceRequest {
    pub name: String,
    pub template: Option<String>,
    pub owner: WorkspaceOwner,
    pub created_by_session: Option<String>,
    pub policy_profile_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LocalWorkspaceSnapshot {
    pub workspace_id: String,
    pub path: String,
    pub file_count: u64,
    pub dir_count: u64,
    pub total_size_bytes: u64,
    pub last_modified_at_ms: Option<u64>,
    pub status: WorkspaceStatus,
    pub lock: Option<LocalWorkspaceLock>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalWorkspaceLockResult {
    pub workspace_id: String,
    pub session_id: String,
    pub acquired: bool,
    pub reentrant: bool,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalWorkspaceCleanupResult {
    pub released_expired_locks: usize,
    pub removed_stale_bindings: usize,
}

#[derive(Clone, Debug)]
pub struct LocalWorkspaceManager {
    cfg: LocalWorkspaceManagerConfig,
    state: std::sync::Arc<Mutex<LocalWorkspaceState>>,
}

#[derive(Debug)]
struct LocalWorkspaceState {
    index: WorkshopIndex,
    session_bindings: HashMap<String, SessionWorkspaceBinding>,
}

impl LocalWorkspaceManager {
    pub async fn create_workshop(
        agent_did: impl Into<String>,
        mut cfg: LocalWorkspaceManagerConfig,
    ) -> Result<Self, ToolError> {
        let agent_did = validate_agent_did(agent_did.into())?;
        cfg.workshop_root = normalize_workshop_root(&cfg.workshop_root)?;
        if cfg.lock_ttl_ms == 0 {
            cfg.lock_ttl_ms = DEFAULT_LOCK_TTL_MS;
        }

        ensure_workshop_layout(&cfg.workshop_root).await?;
        let index_path = cfg.workshop_root.join(WORKSHOP_INDEX_FILE_NAME);
        let mut index = if fs::try_exists(&index_path)
            .await
            .map_err(|err| io_error("check workshop index", &index_path, err))?
        {
            read_json_file::<WorkshopIndex>(&index_path).await?
        } else {
            WorkshopIndex {
                agent_did: agent_did.clone(),
                workspaces: Vec::new(),
                updated_at_ms: now_ms(),
            }
        };

        if index.agent_did.trim().is_empty() {
            index.agent_did = agent_did;
            index.updated_at_ms = now_ms();
            write_json_file(&index_path, &index).await?;
        }

        let session_bindings = load_session_bindings(&cfg.workshop_root).await?;
        Ok(Self {
            cfg,
            state: std::sync::Arc::new(Mutex::new(LocalWorkspaceState {
                index,
                session_bindings,
            })),
        })
    }

    pub async fn load_workshop(
        agent_did: impl Into<String>,
        mut cfg: LocalWorkspaceManagerConfig,
    ) -> Result<Self, ToolError> {
        let agent_did = validate_agent_did(agent_did.into())?;
        cfg.workshop_root = normalize_workshop_root(&cfg.workshop_root)?;
        if cfg.lock_ttl_ms == 0 {
            cfg.lock_ttl_ms = DEFAULT_LOCK_TTL_MS;
        }

        ensure_workshop_layout(&cfg.workshop_root).await?;
        let index_path = cfg.workshop_root.join(WORKSHOP_INDEX_FILE_NAME);
        if !fs::try_exists(&index_path)
            .await
            .map_err(|err| io_error("check workshop index", &index_path, err))?
        {
            return Err(ToolError::InvalidArgs(format!(
                "workshop index not found: {}",
                index_path.display()
            )));
        }

        let mut index = read_json_file::<WorkshopIndex>(&index_path).await?;
        if index.agent_did.trim().is_empty() {
            index.agent_did = agent_did;
            index.updated_at_ms = now_ms();
            write_json_file(&index_path, &index).await?;
        }

        let session_bindings = load_session_bindings(&cfg.workshop_root).await?;
        Ok(Self {
            cfg,
            state: std::sync::Arc::new(Mutex::new(LocalWorkspaceState {
                index,
                session_bindings,
            })),
        })
    }

    pub fn workshop_root(&self) -> &Path {
        &self.cfg.workshop_root
    }

    pub fn workspaces_root(&self) -> PathBuf {
        self.cfg.workshop_root.join(LOCAL_WORKSPACES_REL_PATH)
    }

    pub async fn workshop_index(&self) -> WorkshopIndex {
        let guard = self.state.lock().await;
        guard.index.clone()
    }

    pub async fn list_workspaces(&self) -> Vec<WorkshopWorkspaceRecord> {
        let guard = self.state.lock().await;
        guard.index.workspaces.clone()
    }

    pub async fn create_local_workspace(
        &self,
        req: CreateLocalWorkspaceRequest,
    ) -> Result<WorkshopWorkspaceRecord, ToolError> {
        let name = validate_workspace_name(&req.name)?;
        if let Some(policy_profile_id) = req.policy_profile_id.as_ref() {
            if policy_profile_id.len() > MAX_POLICY_PROFILE_ID_LEN {
                return Err(ToolError::InvalidArgs(format!(
                    "policy_profile_id too long: {}",
                    policy_profile_id.len()
                )));
            }
        }

        let created_at_ms = now_ms();
        let workspace_id = generate_workspace_id(&name, created_at_ms);
        let relative_path = Path::new(LOCAL_WORKSPACES_REL_PATH)
            .join(&workspace_id)
            .to_string_lossy()
            .to_string();
        let abs_path = self.cfg.workshop_root.join(&relative_path);

        fs::create_dir_all(&abs_path)
            .await
            .map_err(|err| io_error("create local workspace", &abs_path, err))?;

        if let Some(template) = req.template.as_ref() {
            let template_marker = abs_path.join(".template");
            fs::write(&template_marker, template.as_bytes())
                .await
                .map_err(|err| {
                    io_error("write workspace template marker", &template_marker, err)
                })?;
        }

        let mut guard = self.state.lock().await;
        if guard
            .index
            .workspaces
            .iter()
            .any(|item| item.workspace_id == workspace_id)
        {
            return Err(ToolError::ExecFailed(format!(
                "workspace id collision: {workspace_id}"
            )));
        }

        let record = WorkshopWorkspaceRecord {
            workspace_id: workspace_id.clone(),
            workspace_type: WorkspaceType::Local,
            owner: req.owner,
            name,
            relative_path: Some(relative_path),
            created_by_session: req.created_by_session,
            policy_profile_id: req.policy_profile_id,
            created_at_ms,
            updated_at_ms: created_at_ms,
            last_sync_at_ms: None,
            status: WorkspaceStatus::Ready,
            conflict_or_error: None,
            lock: None,
            bound_sessions: Vec::new(),
        };
        guard.index.workspaces.push(record.clone());
        guard.index.updated_at_ms = now_ms();

        self.persist_state_locked(&guard).await?;
        Ok(record)
    }

    pub async fn bind_local_workspace(
        &self,
        session_id: &str,
        local_workspace_id: &str,
    ) -> Result<SessionWorkspaceBinding, ToolError> {
        let session_id = validate_session_id(session_id)?;
        let now = now_ms();

        let mut guard = self.state.lock().await;
        let target_path = {
            let item = guard
                .index
                .workspaces
                .iter_mut()
                .find(|item| item.workspace_id == local_workspace_id)
                .ok_or_else(|| {
                    ToolError::InvalidArgs(format!("workspace not found: {local_workspace_id}"))
                })?;

            if item.workspace_type != WorkspaceType::Local {
                return Err(ToolError::InvalidArgs(format!(
                    "workspace `{local_workspace_id}` is not a local workspace"
                )));
            }

            if item.status == WorkspaceStatus::Archived {
                return Err(ToolError::InvalidArgs(format!(
                    "workspace `{local_workspace_id}` is archived"
                )));
            }

            item.bound_sessions
                .retain(|binding| binding.session_id != session_id);
            item.bound_sessions.push(LocalWorkspaceSessionBinding {
                session_id: session_id.to_string(),
                bound_at_ms: now,
            });
            item.updated_at_ms = now;

            item.relative_path.clone().ok_or_else(|| {
                ToolError::ExecFailed(format!(
                    "local workspace `{local_workspace_id}` missing relative_path"
                ))
            })?
        };

        for item in &mut guard.index.workspaces {
            if item.workspace_id == local_workspace_id {
                continue;
            }
            item.bound_sessions
                .retain(|binding| binding.session_id != session_id);
        }

        let binding = SessionWorkspaceBinding {
            session_id: session_id.to_string(),
            local_workspace_id: local_workspace_id.to_string(),
            workspace_path: self
                .cfg
                .workshop_root
                .join(&target_path)
                .to_string_lossy()
                .to_string(),
            bound_at_ms: now,
        };

        guard
            .session_bindings
            .insert(session_id.to_string(), binding.clone());
        guard.index.updated_at_ms = now;

        self.persist_state_locked(&guard).await?;
        Ok(binding)
    }

    pub async fn get_bound_local_workspace(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionWorkspaceBinding>, ToolError> {
        let session_id = validate_session_id(session_id)?;
        let guard = self.state.lock().await;
        Ok(guard.session_bindings.get(session_id).cloned())
    }

    pub async fn get_local_workspace_path(
        &self,
        local_workspace_id: &str,
    ) -> Result<PathBuf, ToolError> {
        let guard = self.state.lock().await;
        let item = guard
            .index
            .workspaces
            .iter()
            .find(|item| item.workspace_id == local_workspace_id)
            .ok_or_else(|| {
                ToolError::InvalidArgs(format!("workspace not found: {local_workspace_id}"))
            })?;
        if item.workspace_type != WorkspaceType::Local {
            return Err(ToolError::InvalidArgs(format!(
                "workspace `{local_workspace_id}` is not a local workspace"
            )));
        }
        let rel_path = item.relative_path.clone().ok_or_else(|| {
            ToolError::ExecFailed(format!(
                "local workspace `{local_workspace_id}` missing relative_path"
            ))
        })?;
        Ok(self.cfg.workshop_root.join(rel_path))
    }

    pub async fn snapshot_metadata(
        &self,
        local_workspace_id: &str,
    ) -> Result<LocalWorkspaceSnapshot, ToolError> {
        let (path, status, lock) = {
            let guard = self.state.lock().await;
            let item = guard
                .index
                .workspaces
                .iter()
                .find(|item| item.workspace_id == local_workspace_id)
                .ok_or_else(|| {
                    ToolError::InvalidArgs(format!("workspace not found: {local_workspace_id}"))
                })?;

            if item.workspace_type != WorkspaceType::Local {
                return Err(ToolError::InvalidArgs(format!(
                    "workspace `{local_workspace_id}` is not a local workspace"
                )));
            }

            let rel_path = item.relative_path.clone().ok_or_else(|| {
                ToolError::ExecFailed(format!(
                    "local workspace `{local_workspace_id}` missing relative_path"
                ))
            })?;
            (
                self.cfg.workshop_root.join(rel_path),
                item.status.clone(),
                item.lock.clone(),
            )
        };

        let path_for_scan = path.clone();
        let stats = tokio::task::spawn_blocking(move || scan_directory_metadata(&path_for_scan))
            .await
            .map_err(|err| ToolError::ExecFailed(format!("scan metadata join error: {err}")))??;

        Ok(LocalWorkspaceSnapshot {
            workspace_id: local_workspace_id.to_string(),
            path: path.to_string_lossy().to_string(),
            file_count: stats.file_count,
            dir_count: stats.dir_count,
            total_size_bytes: stats.total_size_bytes,
            last_modified_at_ms: stats.last_modified_at_ms,
            status,
            lock,
        })
    }

    pub async fn acquire(
        &self,
        local_workspace_id: &str,
        session_id: &str,
    ) -> Result<LocalWorkspaceLockResult, ToolError> {
        let session_id = validate_session_id(session_id)?;
        let now = now_ms();

        let mut guard = self.state.lock().await;
        let item = guard
            .index
            .workspaces
            .iter_mut()
            .find(|item| item.workspace_id == local_workspace_id)
            .ok_or_else(|| {
                ToolError::InvalidArgs(format!("workspace not found: {local_workspace_id}"))
            })?;

        if item.workspace_type != WorkspaceType::Local {
            return Err(ToolError::InvalidArgs(format!(
                "workspace `{local_workspace_id}` is not a local workspace"
            )));
        }

        if item.status == WorkspaceStatus::Archived {
            return Err(ToolError::InvalidArgs(format!(
                "workspace `{local_workspace_id}` is archived"
            )));
        }

        let mut reentrant = false;
        if let Some(lock) = item.lock.as_ref() {
            let expired = lock.lease_expires_at_ms <= now;
            if !expired && lock.session_id != session_id {
                return Err(ToolError::InvalidArgs(format!(
                    "workspace `{local_workspace_id}` is locked by session `{}`",
                    lock.session_id
                )));
            }
            if !expired && lock.session_id == session_id {
                reentrant = true;
            }
        }

        let lease_expires_at_ms = now.saturating_add(self.cfg.lock_ttl_ms);
        item.lock = Some(LocalWorkspaceLock {
            session_id: session_id.to_string(),
            acquired_at_ms: now,
            lease_expires_at_ms,
        });
        item.updated_at_ms = now;
        guard.index.updated_at_ms = now;

        self.persist_state_locked(&guard).await?;

        Ok(LocalWorkspaceLockResult {
            workspace_id: local_workspace_id.to_string(),
            session_id: session_id.to_string(),
            acquired: true,
            reentrant,
            lease_expires_at_ms,
        })
    }

    pub async fn release(
        &self,
        local_workspace_id: &str,
        session_id: &str,
    ) -> Result<bool, ToolError> {
        let session_id = validate_session_id(session_id)?;
        let now = now_ms();

        let mut guard = self.state.lock().await;
        let item = guard
            .index
            .workspaces
            .iter_mut()
            .find(|item| item.workspace_id == local_workspace_id)
            .ok_or_else(|| {
                ToolError::InvalidArgs(format!("workspace not found: {local_workspace_id}"))
            })?;

        let Some(lock) = item.lock.as_ref() else {
            return Ok(false);
        };

        if lock.session_id != session_id {
            let expired = lock.lease_expires_at_ms <= now;
            if !expired {
                return Err(ToolError::InvalidArgs(format!(
                    "workspace `{local_workspace_id}` lock owned by `{}`",
                    lock.session_id
                )));
            }
        }

        item.lock = None;
        item.updated_at_ms = now;
        guard.index.updated_at_ms = now;

        self.persist_state_locked(&guard).await?;
        Ok(true)
    }

    pub async fn archive_workspace(
        &self,
        workspace_id: &str,
        reason: Option<String>,
    ) -> Result<WorkshopWorkspaceRecord, ToolError> {
        let now = now_ms();
        let mut guard = self.state.lock().await;

        let workspace_idx = guard
            .index
            .workspaces
            .iter()
            .position(|item| item.workspace_id == workspace_id)
            .ok_or_else(|| {
                ToolError::InvalidArgs(format!("workspace not found: {workspace_id}"))
            })?;

        let removed_sessions = {
            let item = guard
                .index
                .workspaces
                .get_mut(workspace_idx)
                .expect("workspace index should be valid");
            item.status = WorkspaceStatus::Archived;
            item.lock = None;
            if let Some(reason) = reason {
                item.conflict_or_error = Some(WorkspaceErrorSummary {
                    code: "archived".to_string(),
                    summary: reason,
                    timestamp_ms: now,
                });
            }
            item.updated_at_ms = now;
            let sessions = item
                .bound_sessions
                .iter()
                .map(|binding| binding.session_id.clone())
                .collect::<Vec<_>>();
            item.bound_sessions.clear();
            sessions
        };

        for session_id in removed_sessions {
            guard.session_bindings.remove(&session_id);
        }

        guard.index.updated_at_ms = now;
        let archived = guard
            .index
            .workspaces
            .iter()
            .find(|item| item.workspace_id == workspace_id)
            .cloned()
            .ok_or_else(|| ToolError::ExecFailed("archived workspace lost".to_string()))?;

        self.persist_state_locked(&guard).await?;
        Ok(archived)
    }

    pub async fn cleanup(&self) -> Result<LocalWorkspaceCleanupResult, ToolError> {
        let now = now_ms();
        let mut guard = self.state.lock().await;

        let mut released_expired_locks = 0usize;
        for item in &mut guard.index.workspaces {
            if let Some(lock) = item.lock.as_ref() {
                if lock.lease_expires_at_ms <= now {
                    item.lock = None;
                    item.updated_at_ms = now;
                    released_expired_locks += 1;
                }
            }
        }

        let known_local_ids: std::collections::HashSet<&str> = guard
            .index
            .workspaces
            .iter()
            .filter(|item| item.workspace_type == WorkspaceType::Local)
            .map(|item| item.workspace_id.as_str())
            .collect();

        let stale_sessions: Vec<String> = guard
            .session_bindings
            .iter()
            .filter(|(_, binding)| !known_local_ids.contains(binding.local_workspace_id.as_str()))
            .map(|(session_id, _)| session_id.clone())
            .collect();

        for session_id in &stale_sessions {
            guard.session_bindings.remove(session_id);
        }

        if released_expired_locks > 0 || !stale_sessions.is_empty() {
            guard.index.updated_at_ms = now;
            self.persist_state_locked(&guard).await?;
        }

        Ok(LocalWorkspaceCleanupResult {
            released_expired_locks,
            removed_stale_bindings: stale_sessions.len(),
        })
    }

    async fn persist_state_locked(&self, state: &LocalWorkspaceState) -> Result<(), ToolError> {
        let index_path = self.cfg.workshop_root.join(WORKSHOP_INDEX_FILE_NAME);
        write_json_file(&index_path, &state.index).await?;

        let bindings = SessionBindingsFile {
            bindings: state.session_bindings.values().cloned().collect(),
        };
        let binding_path = self.cfg.workshop_root.join(SESSION_BINDINGS_REL_PATH);
        if let Some(parent) = binding_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|err| io_error("create bindings dir", parent, err))?;
        }
        write_json_file(&binding_path, &bindings).await?;

        Ok(())
    }
}

fn validate_agent_did(input: String) -> Result<String, ToolError> {
    let did = input.trim();
    if did.is_empty() {
        return Err(ToolError::InvalidArgs(
            "agent_did cannot be empty".to_string(),
        ));
    }
    Ok(did.to_string())
}

fn validate_workspace_name(input: &str) -> Result<String, ToolError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidArgs(
            "workspace name cannot be empty".to_string(),
        ));
    }
    if trimmed.len() > MAX_WORKSPACE_NAME_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "workspace name too long: {}",
            trimmed.len()
        )));
    }
    Ok(trimmed.to_string())
}

fn validate_session_id(input: &str) -> Result<&str, ToolError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidArgs(
            "session_id cannot be empty".to_string(),
        ));
    }
    Ok(trimmed)
}

fn generate_workspace_id(name: &str, timestamp_ms: u64) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch == '.') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    if slug.is_empty() {
        slug.push_str("workspace");
    }

    let counter = LOCAL_WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("local-{slug}-{timestamp_ms}-{counter}")
}

async fn ensure_workshop_layout(workshop_root: &Path) -> Result<(), ToolError> {
    let dirs = [
        workshop_root.to_path_buf(),
        workshop_root.join("tools"),
        workshop_root.join("skills"),
        workshop_root.join("sessions"),
        workshop_root.join("workspaces"),
        workshop_root.join("workspaces/local"),
        workshop_root.join("workspaces/remote"),
        workshop_root.join("worklog"),
        workshop_root.join("todo"),
        workshop_root.join("artifacts"),
    ];

    for dir in dirs {
        fs::create_dir_all(&dir)
            .await
            .map_err(|err| io_error("create workshop layout", &dir, err))?;
    }
    Ok(())
}

async fn load_session_bindings(
    workshop_root: &Path,
) -> Result<HashMap<String, SessionWorkspaceBinding>, ToolError> {
    let path = workshop_root.join(SESSION_BINDINGS_REL_PATH);
    if !fs::try_exists(&path)
        .await
        .map_err(|err| io_error("check session bindings", &path, err))?
    {
        return Ok(HashMap::new());
    }

    let file = read_json_file::<SessionBindingsFile>(&path).await?;
    let mut out = HashMap::with_capacity(file.bindings.len());
    for binding in file.bindings {
        if binding.session_id.trim().is_empty() || binding.local_workspace_id.trim().is_empty() {
            continue;
        }
        out.insert(binding.session_id.clone(), binding);
    }
    Ok(out)
}

async fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ToolError> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|err| io_error("read json file", path, err))?;
    serde_json::from_str::<T>(&content).map_err(|err| {
        ToolError::ExecFailed(format!("parse json `{}` failed: {err}", path.display()))
    })
}

async fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|err| io_error("create json parent dir", parent, err))?;
    }

    let payload = serde_json::to_vec_pretty(value)
        .map_err(|err| ToolError::ExecFailed(format!("serialize json failed: {err}")))?;

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, payload)
        .await
        .map_err(|err| io_error("write json temp file", &tmp_path, err))?;
    fs::rename(&tmp_path, path)
        .await
        .map_err(|err| io_error("replace json file", path, err))?;
    Ok(())
}

fn io_error(action: &str, path: &Path, source: std::io::Error) -> ToolError {
    ToolError::ExecFailed(format!("{action} `{}` failed: {source}", path.display()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn normalize_workshop_root(root: &Path) -> Result<PathBuf, ToolError> {
    let abs = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| ToolError::ExecFailed(format!("read current_dir failed: {err}")))?
            .join(root)
    };
    Ok(normalize_abs_path(&abs))
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

#[derive(Clone, Debug, Default)]
struct DirStats {
    file_count: u64,
    dir_count: u64,
    total_size_bytes: u64,
    last_modified_at_ms: Option<u64>,
}

fn scan_directory_metadata(path: &Path) -> Result<DirStats, ToolError> {
    if !path.exists() {
        return Ok(DirStats::default());
    }

    let mut stats = DirStats::default();
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        let iter = std::fs::read_dir(&current)
            .map_err(|err| io_error("read workspace directory", &current, err))?;

        for entry in iter {
            let entry = entry.map_err(|err| io_error("read directory entry", &current, err))?;
            let entry_path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|err| io_error("read entry metadata", &entry_path, err))?;

            if metadata.is_dir() {
                stats.dir_count = stats.dir_count.saturating_add(1);
                stack.push(entry_path);
                continue;
            }

            if metadata.is_file() {
                stats.file_count = stats.file_count.saturating_add(1);
                stats.total_size_bytes = stats.total_size_bytes.saturating_add(metadata.len());
                if let Ok(modified) = metadata.modified() {
                    let modified_ms = modified
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let prev = stats.last_modified_at_ms.unwrap_or(0);
                    if modified_ms > prev {
                        stats.last_modified_at_ms = Some(modified_ms);
                    }
                }
            }
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root(test_name: &str) -> PathBuf {
        let ts = now_ms();
        std::env::temp_dir().join(format!("opendan-local-ws-{test_name}-{ts}"))
    }

    #[tokio::test]
    async fn create_bind_lock_snapshot_archive_cleanup() {
        let root = unique_root("lifecycle");
        let mut cfg = LocalWorkspaceManagerConfig::new(&root);
        cfg.lock_ttl_ms = 30;

        let manager = LocalWorkspaceManager::create_workshop("did:example:agent", cfg)
            .await
            .expect("create manager");

        let created = manager
            .create_local_workspace(CreateLocalWorkspaceRequest {
                name: "devbox".to_string(),
                template: Some("rust".to_string()),
                owner: WorkspaceOwner::AgentCreated,
                created_by_session: Some("session-a".to_string()),
                policy_profile_id: Some("default".to_string()),
            })
            .await
            .expect("create local workspace");

        let binding = manager
            .bind_local_workspace("session-a", &created.workspace_id)
            .await
            .expect("bind local workspace");
        assert_eq!(binding.local_workspace_id, created.workspace_id);

        let lock = manager
            .acquire(&created.workspace_id, "session-a")
            .await
            .expect("acquire lock");
        assert!(lock.acquired);

        let snapshot = manager
            .snapshot_metadata(&created.workspace_id)
            .await
            .expect("snapshot metadata");
        assert!(snapshot.path.ends_with(&created.workspace_id));

        let released = manager
            .release(&created.workspace_id, "session-a")
            .await
            .expect("release lock");
        assert!(released);

        let archived = manager
            .archive_workspace(&created.workspace_id, Some("done".to_string()))
            .await
            .expect("archive");
        assert_eq!(archived.status, WorkspaceStatus::Archived);

        let cleaned = manager.cleanup().await.expect("cleanup");
        assert_eq!(cleaned.removed_stale_bindings, 0);

        let _ = fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn lock_is_mutually_exclusive_and_reentrant() {
        let root = unique_root("lock");
        let mut cfg = LocalWorkspaceManagerConfig::new(&root);
        cfg.lock_ttl_ms = 120_000;

        let manager = LocalWorkspaceManager::create_workshop("did:example:agent", cfg)
            .await
            .expect("create manager");

        let created = manager
            .create_local_workspace(CreateLocalWorkspaceRequest {
                name: "code".to_string(),
                template: None,
                owner: WorkspaceOwner::AgentCreated,
                created_by_session: None,
                policy_profile_id: None,
            })
            .await
            .expect("create local workspace");

        let first = manager
            .acquire(&created.workspace_id, "session-a")
            .await
            .expect("acquire lock by session a");
        assert!(!first.reentrant);

        let reentrant = manager
            .acquire(&created.workspace_id, "session-a")
            .await
            .expect("reentrant acquire should pass");
        assert!(reentrant.reentrant);

        let denied = manager.acquire(&created.workspace_id, "session-b").await;
        assert!(matches!(denied, Err(ToolError::InvalidArgs(_))));

        let _ = fs::remove_dir_all(&root).await;
    }
}
