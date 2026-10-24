//! Cognition facade (§6.4) used by the runner's `recall` and `note`
//! commands. Cognitions are written only by the consolidation of the
//! memory_maintenance Goal ([`crate::memory::Consolidator`]); the Notebook
//! path is the legacy caller list of TD-09 (retired separately).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use agent_tool::agent_memory::{AgentMemory, AgentMemoryConfig, RecallOutcome, RecallQuery as GraphRecallQuery};
use agent_tool::agent_notebook::{AgentNotebook, AgentNotebookConfig, AppendNoteInput, WriteReason};

use crate::error::{OpenDanError, Result};

use super::fs_client::AgentLayout;
use super::Cognition;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecallQuery {
    pub tags: Vec<String>,
    #[serde(default)]
    pub max_hints: usize,
}

/// `time + sentence + id` shaped clue from past cognition (S-30 `<hints>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    pub id: String,
    pub time: String,
    pub sentence: String,
    #[serde(default)]
    pub kind: String,
}

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
}

#[async_trait]
impl Cognition for FsCognition {
    /// Tags only, no Session grants yet (S stage): no tag means "not
    /// triggered" (an empty list, never the whole Graph, TD-10); a store
    /// that cannot be read is an error, never an empty list (TD-01).
    async fn recall_hints(&self, q: &RecallQuery) -> Result<Vec<Hint>> {
        let dir = self.layout.memory_dir();
        let tags = q.tags.clone();
        let max = if q.max_hints == 0 { 5 } else { q.max_hints };
        tokio::task::spawn_blocking(move || -> Result<Vec<Hint>> {
            let mem = AgentMemory::open_read(AgentMemoryConfig::new(dir))
                .map_err(|e| OpenDanError::Other(format!("memory unavailable: {e}")))?;
            let outcome = mem
                .recall(&GraphRecallQuery {
                    tags,
                    max_records: max,
                    ..GraphRecallQuery::default()
                })
                .map_err(|e| OpenDanError::Other(format!("memory recall: {e}")))?;
            Ok(match outcome {
                RecallOutcome::NotTriggered => Vec::new(),
                RecallOutcome::Recalled(r) => r
                    .items
                    .into_iter()
                    .map(|i| Hint {
                        id: format!("{}@{}", i.item_id, i.revision),
                        time: i.noticed_at,
                        sentence: i.content,
                        kind: i.semantic_kind.unwrap_or_else(|| i.kind.to_string()),
                    })
                    .collect(),
            })
        })
        .await
        .map_err(|e| OpenDanError::Other(e.to_string()))?
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
