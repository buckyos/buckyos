//! Cognition facade (§6.3, §6.4) of the Agent State: a session's active
//! Memory query (`agent-session recall`) and reading a reference in full,
//! both as the session's host-bound identity
//! ([`crate::memory::session_caller`]: its grants decide what is visible,
//! never the caller's arguments). Cognitions are written only by the
//! consolidation of the memory_maintenance Goal
//! ([`crate::memory::Consolidator`]); the Notebook path is the legacy
//! caller list of TD-09 (retired separately).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use agent_tool::agent_notebook::{AgentNotebook, AgentNotebookConfig, AppendNoteInput, WriteReason};

use crate::error::{OpenDanError, Result};
use crate::memory::{Caller, Memory, MemoryConfig, MemoryQuery, MemoryRead, QueryResult};

use super::fs_client::AgentLayout;
use super::{Cognition, SessionRegistry};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotebookNote {
    pub notebook_id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub(super) struct FsCognition {
    layout: AgentLayout,
}

impl FsCognition {
    pub(super) fn new(layout: AgentLayout) -> Self {
        Self { layout }
    }

    /// The Memory identity of session `sid`, from its own configuration.
    async fn caller(&self, sid: &str) -> Result<Caller> {
        let registry = super::registry::FsRegistry::new(self.layout.clone(), None, None);
        let entry = registry
            .lookup(sid)
            .await?
            .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
        let cfg = crate::session::SessionDir::open(&entry.location)?.config()?;
        Ok(crate::memory::session_caller(&cfg, None))
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(Memory) -> crate::memory::MemoryResult<T> + Send + 'static,
    ) -> Result<T> {
        let root = self.layout.root.clone();
        tokio::task::spawn_blocking(move || f(Memory::open(root, MemoryConfig::default())))
            .await
            .map_err(|e| OpenDanError::Other(e.to_string()))?
            .map_err(OpenDanError::from)
    }
}

#[async_trait]
impl Cognition for FsCognition {
    /// A store that cannot be read is an error, never an empty list
    /// (TD-01).
    async fn query(&self, sid: &str, q: &MemoryQuery) -> Result<QueryResult> {
        let caller = self.caller(sid).await?;
        let q = q.clone();
        self.blocking(move |m| m.query(&caller, &q)).await
    }

    async fn read(&self, sid: &str, reference: &str) -> Result<MemoryRead> {
        let caller = self.caller(sid).await?;
        let reference = reference.to_string();
        self.blocking(move |m| m.read(&caller, &reference)).await
    }

    async fn notebook_append(&self, note: &NotebookNote, _who: &str) -> Result<()> {
        let dir = self.layout.notebook_dir();
        let note = note.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let nb = AgentNotebook::open(AgentNotebookConfig::new(dir))
                .map_err(|e| OpenDanError::Other(format!("notebook open: {e}")))?;
            nb.append_note(AppendNoteInput {
                session_id: note.session_id.clone(),
                notebook_id: note.notebook_id,
                title: note.title,
                content: note.content,
                source_excerpt: None,
                source_ref: None,
                source_session_id: note.session_id,
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: None,
                tags: note.tags,
                detect_conflicts: false,
            })
            .map_err(|e| OpenDanError::Other(format!("notebook append: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| OpenDanError::Other(e.to_string()))?
    }
}
