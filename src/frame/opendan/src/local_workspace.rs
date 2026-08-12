//! §9.7 of NewOpenDANRuntime — local workspace records.
//!
//! Workspaces are the agent-owned private working areas where filesystem
//! tools (exec_bash / read / edit_file / ...) operate. The new
//! runtime's invariants:
//!
//! 1. Each workspace lives at `<agent_root>/workspace/<workspace_id>/` and
//!    owns a small `.workspace.json` describing it plus an optional
//!    `readme.md` consumed as environment context for prompts.
//! 2. Session binding is owned by `AgentSession.meta.workspace_id`, NOT a
//!    global in-memory map. The workspace record stores the *currently
//!    bound* session so future tooling can detect conflicts, but the
//!    session-side meta is the source of truth.
//! 3. This module owns *records*, not policies — locking / lease /
//!    conflict resolution lands when worksession §8 lands. Until then the
//!    only "lock" is `current_session`, used for warn-on-overlap.
//!
//! Compared to the beta2.1 `LocalWorkspaceManager` we deleted:
//!   - No global `session_bindings` HashMap (it duplicated session state).
//!   - No `LocalWorkspaceLock` lease machinery (worksession §8 will).
//!   - The record schema is much smaller — we don't need `WorkspaceType`
//!     (only local for now), `policy_profile_id`, or `bound_sessions: Vec`
//!     until those features actually plug in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;

/// On-disk descriptor for a workspace. Persisted to
/// `<workspace_dir>/.workspace.json` (tmp + rename for crash-consistency).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub name: String,
    /// Session that created the workspace. Informational — once a workspace
    /// exists it can be re-bound to any session.
    pub created_by_session: Option<String>,
    /// Session currently bound to this workspace. `None` ⇒ workspace is
    /// idle and any session can claim it.
    pub current_session: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub status: WorkspaceStatus,
}

impl Default for WorkspaceRecord {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            name: String::new(),
            created_by_session: None,
            current_session: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            status: WorkspaceStatus::Ready,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
    Ready,
    Archived,
    Error,
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("io: {path}: {err}")]
    Io {
        path: String,
        #[source]
        err: std::io::Error,
    },
    #[error("decode {path}: {err}")]
    Decode {
        path: String,
        #[source]
        err: serde_json::Error,
    },
    #[error("encode workspace record: {err}")]
    Encode {
        #[source]
        err: serde_json::Error,
    },
    #[error("workspace {id} not found")]
    NotFound { id: String },
    #[error("workspace id `{0}` is invalid")]
    InvalidId(String),
}

/// Manager — owns the `workspaces_root` directory but no in-memory state.
/// Cloning is cheap (just the path); cloning instead of `Arc`-ing avoids
/// accidental shared mutation. All state is read fresh from disk on each
/// operation.
#[derive(Clone, Debug)]
pub struct LocalWorkspaceManager {
    workspaces_root: PathBuf,
}

impl LocalWorkspaceManager {
    pub fn new(workspaces_root: impl Into<PathBuf>) -> Self {
        Self {
            workspaces_root: workspaces_root.into(),
        }
    }

    pub fn workspaces_root(&self) -> &Path {
        &self.workspaces_root
    }

    pub fn workspace_dir(&self, workspace_id: &str) -> PathBuf {
        self.workspaces_root.join(workspace_id)
    }

    pub fn record_path(&self, workspace_id: &str) -> PathBuf {
        self.workspace_dir(workspace_id).join(".workspace.json")
    }

    /// Create a new workspace directory + record, or return the existing
    /// record if the id is already present. The caller chooses the id;
    /// worksession creation should pass a human-meaningful directory name
    /// and only add a suffix when needed to avoid collisions.
    pub async fn create_or_open(
        &self,
        workspace_id: &str,
        name: &str,
        created_by_session: Option<&str>,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        validate_workspace_id(workspace_id)?;
        fs::create_dir_all(self.workspace_dir(workspace_id))
            .await
            .map_err(|err| WorkspaceError::Io {
                path: self.workspace_dir(workspace_id).display().to_string(),
                err,
            })?;
        match self.load_record(workspace_id).await {
            Ok(rec) => Ok(rec),
            Err(WorkspaceError::NotFound { .. }) => {
                let now = now_ms();
                let record = WorkspaceRecord {
                    workspace_id: workspace_id.to_string(),
                    name: if name.is_empty() {
                        workspace_id.to_string()
                    } else {
                        name.to_string()
                    },
                    created_by_session: created_by_session.map(str::to_string),
                    current_session: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                    status: WorkspaceStatus::Ready,
                };
                self.save_record(&record).await?;
                Ok(record)
            }
            Err(other) => Err(other),
        }
    }

    /// Read a workspace record from disk. Returns `NotFound` when the
    /// directory exists but has no `.workspace.json` (the legacy MVP
    /// auto-create path created bare directories — callers should treat
    /// that as "needs initialization").
    pub async fn load_record(&self, workspace_id: &str) -> Result<WorkspaceRecord, WorkspaceError> {
        validate_workspace_id(workspace_id)?;
        let path = self.record_path(workspace_id);
        match fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|err| WorkspaceError::Decode {
                path: path.display().to_string(),
                err,
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(WorkspaceError::NotFound {
                    id: workspace_id.to_string(),
                })
            }
            Err(err) => Err(WorkspaceError::Io {
                path: path.display().to_string(),
                err,
            }),
        }
    }

    /// Atomic write of the record using tmp + rename. Updates
    /// `updated_at_ms` before serializing.
    pub async fn save_record(&self, record: &WorkspaceRecord) -> Result<(), WorkspaceError> {
        validate_workspace_id(&record.workspace_id)?;
        let dir = self.workspace_dir(&record.workspace_id);
        fs::create_dir_all(&dir)
            .await
            .map_err(|err| WorkspaceError::Io {
                path: dir.display().to_string(),
                err,
            })?;
        let mut to_save = record.clone();
        to_save.updated_at_ms = now_ms();
        let bytes =
            serde_json::to_vec_pretty(&to_save).map_err(|err| WorkspaceError::Encode { err })?;
        let path = self.record_path(&record.workspace_id);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)
            .await
            .map_err(|err| WorkspaceError::Io {
                path: tmp.display().to_string(),
                err,
            })?;
        fs::rename(&tmp, &path)
            .await
            .map_err(|err| WorkspaceError::Io {
                path: path.display().to_string(),
                err,
            })?;
        Ok(())
    }

    /// Set `current_session` on the workspace record and persist. Pass
    /// `None` to unbind. Callers should pair this with updating the
    /// session's `workspace_id` field so both sides agree.
    pub async fn set_current_session(
        &self,
        workspace_id: &str,
        session_id: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        let mut record = match self.load_record(workspace_id).await {
            Ok(r) => r,
            Err(WorkspaceError::NotFound { .. }) => {
                // First-binding case: synthesize a minimal record so we
                // don't drop the binding on the floor when the workspace
                // dir exists without an explicit `create_or_open` call.
                self.create_or_open(workspace_id, workspace_id, session_id)
                    .await?
            }
            Err(other) => return Err(other),
        };
        record.current_session = session_id.map(str::to_string);
        self.save_record(&record).await
    }

    /// List all workspace records under `workspaces_root`. Unreadable /
    /// missing `.workspace.json` entries are silently skipped (legacy MVP
    /// bare dirs from the §9.6 era look like that).
    pub async fn list(&self) -> Result<Vec<WorkspaceRecord>, WorkspaceError> {
        let mut out = Vec::new();
        let mut read = match fs::read_dir(&self.workspaces_root).await {
            Ok(rd) => rd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(err) => {
                return Err(WorkspaceError::Io {
                    path: self.workspaces_root.display().to_string(),
                    err,
                })
            }
        };
        while let Ok(Some(entry)) = read.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Ok(rec) = self.load_record(id).await {
                out.push(rec);
            }
        }
        Ok(out)
    }

    /// Mark a workspace as archived. Does not move files — that's
    /// worksession §8's job; here we just flip status so list/restore
    /// can filter.
    pub async fn archive(&self, workspace_id: &str) -> Result<(), WorkspaceError> {
        let mut record = self.load_record(workspace_id).await?;
        record.status = WorkspaceStatus::Archived;
        record.current_session = None;
        self.save_record(&record).await
    }
}

fn validate_workspace_id(id: &str) -> Result<(), WorkspaceError> {
    if id.is_empty() {
        return Err(WorkspaceError::InvalidId(id.to_string()));
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(WorkspaceError::InvalidId(id.to_string()));
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn create_then_load_roundtrips() {
        let tmp = tempdir().unwrap();
        let mgr = LocalWorkspaceManager::new(tmp.path().to_path_buf());
        let rec = mgr
            .create_or_open("ws-1", "main", Some("ui-alice"))
            .await
            .unwrap();
        assert_eq!(rec.workspace_id, "ws-1");
        assert_eq!(rec.created_by_session.as_deref(), Some("ui-alice"));
        let loaded = mgr.load_record("ws-1").await.unwrap();
        assert_eq!(loaded.workspace_id, rec.workspace_id);
        assert_eq!(loaded.created_by_session.as_deref(), Some("ui-alice"));
    }

    #[tokio::test]
    async fn create_or_open_is_idempotent() {
        let tmp = tempdir().unwrap();
        let mgr = LocalWorkspaceManager::new(tmp.path().to_path_buf());
        let a = mgr.create_or_open("ws-2", "x", Some("ui-1")).await.unwrap();
        let b = mgr
            .create_or_open("ws-2", "different name", Some("other"))
            .await
            .unwrap();
        // Second call must return the original record (no clobber).
        assert_eq!(a.name, b.name);
        assert_eq!(a.created_by_session, b.created_by_session);
    }

    #[tokio::test]
    async fn set_current_session_persists() {
        let tmp = tempdir().unwrap();
        let mgr = LocalWorkspaceManager::new(tmp.path().to_path_buf());
        mgr.create_or_open("ws-3", "x", Some("ui-1")).await.unwrap();
        mgr.set_current_session("ws-3", Some("ui-2")).await.unwrap();
        let rec = mgr.load_record("ws-3").await.unwrap();
        assert_eq!(rec.current_session.as_deref(), Some("ui-2"));
        mgr.set_current_session("ws-3", None).await.unwrap();
        let rec = mgr.load_record("ws-3").await.unwrap();
        assert!(rec.current_session.is_none());
    }

    #[tokio::test]
    async fn set_current_session_synthesizes_record_when_missing() {
        // Older MVP code created bare workspace dirs without a record.
        // set_current_session must initialize one on demand.
        let tmp = tempdir().unwrap();
        let mgr = LocalWorkspaceManager::new(tmp.path().to_path_buf());
        std::fs::create_dir_all(mgr.workspace_dir("bare")).unwrap();
        mgr.set_current_session("bare", Some("ui-1")).await.unwrap();
        let rec = mgr.load_record("bare").await.unwrap();
        assert_eq!(rec.current_session.as_deref(), Some("ui-1"));
    }

    #[tokio::test]
    async fn list_skips_unmanaged_dirs() {
        let tmp = tempdir().unwrap();
        let mgr = LocalWorkspaceManager::new(tmp.path().to_path_buf());
        std::fs::create_dir_all(tmp.path().join("bare_no_record")).unwrap();
        mgr.create_or_open("good", "x", None).await.unwrap();
        let all = mgr.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].workspace_id, "good");
    }

    #[tokio::test]
    async fn archive_flips_status_and_clears_binding() {
        let tmp = tempdir().unwrap();
        let mgr = LocalWorkspaceManager::new(tmp.path().to_path_buf());
        mgr.create_or_open("ws-arch", "x", None).await.unwrap();
        mgr.set_current_session("ws-arch", Some("ui-1"))
            .await
            .unwrap();
        mgr.archive("ws-arch").await.unwrap();
        let rec = mgr.load_record("ws-arch").await.unwrap();
        assert!(matches!(rec.status, WorkspaceStatus::Archived));
        assert!(rec.current_session.is_none());
    }

    #[test]
    fn validate_rejects_dangerous_ids() {
        assert!(validate_workspace_id("").is_err());
        assert!(validate_workspace_id("../escape").is_err());
        assert!(validate_workspace_id("a/b").is_err());
        assert!(validate_workspace_id("good_id-1").is_ok());
    }
}
