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

/// `time + sentence + id` shaped clue from past cognition (S-30 `<hints>`),
/// with what keeps it from being read as more than it is: the nature of
/// its evidence, its range and its end.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    pub id: String,
    pub time: String,
    pub sentence: String,
    #[serde(default)]
    pub kind: String,
    /// The object it is about, as a URL `read` follows (TODO A4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// What supports it: `user_statement | tool_observation |
    /// third_party_claim | inference`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    /// The user stated it as a lasting preference, requirement or boundary.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// `stable | contextual | volatile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    /// What it is limited to: the workspaces among its subjects and its
    /// scope objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applies_to: Vec<String>,
    /// Objects its scope explicitly leaves out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except: Vec<String>,
}

/// Evidence below this is said to be weak next to the clue.
const WEAK_CONFIDENCE: f64 = 0.5;
const QUALIFIER_ENTRIES: usize = 3;

impl Hint {
    /// What a reader must know to not take the clue for more than it is:
    /// the nature of its evidence, its range and its end. Empty when the
    /// cognition carries none of them.
    pub fn qualifier(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match (self.basis.as_deref(), self.explicit) {
            (Some("user_statement"), true) => parts.push("the user's explicit statement".into()),
            (Some("user_statement"), false) => parts.push("stated by the user".into()),
            (Some("tool_observation"), _) => parts.push("observed".into()),
            (Some("third_party_claim"), _) => parts.push("a third party's claim, not verified".into()),
            (Some("inference"), _) => parts.push("the agent's inference, not confirmed".into()),
            _ => {}
        }
        if self.confidence.is_some_and(|c| c < WEAK_CONFIDENCE) {
            parts.push("weak evidence".into());
        }
        let list = |v: &[String]| {
            let mut t = v
                .iter()
                .take(QUALIFIER_ENTRIES)
                .map(|s| s.chars().take(80).collect::<String>())
                .collect::<Vec<_>>()
                .join(", ");
            if v.len() > QUALIFIER_ENTRIES {
                t.push_str(", …");
            }
            t
        };
        let applies: Vec<String> = self
            .applies_to
            .iter()
            .filter(|a| Some(a.as_str()) != self.object.as_deref())
            .cloned()
            .collect();
        if !applies.is_empty() {
            parts.push(format!("applies to {}", list(&applies)));
        }
        if !self.except.is_empty() {
            parts.push(format!("not to {}", list(&self.except)));
        }
        match (&self.valid_until, self.stability.as_deref()) {
            (Some(t), _) => parts.push(format!("holds until {t}")),
            (None, Some("volatile")) => parts.push("short-lived".into()),
            _ => {}
        }
        parts.join("; ")
    }

    /// The parenthesis after the sentence of a hint line.
    pub fn note(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let q = self.qualifier();
        if !q.is_empty() {
            parts.push(q);
        }
        parts.push(format!("noticed {}", self.time));
        if let Some(o) = &self.object {
            parts.push(format!("about {o}"));
        }
        parts.join("; ")
    }
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
                    .map(|i| {
                        let (applies_to, except) = match &i.scope {
                            Some(s) => (
                                s.subjects
                                    .iter()
                                    .filter(|x| x.starts_with("workspace:"))
                                    .chain(s.objects.iter())
                                    .cloned()
                                    .collect(),
                                s.exceptions.clone(),
                            ),
                            None => Default::default(),
                        };
                        Hint {
                            id: format!("{}@{}", i.item_id, i.revision),
                            time: i.noticed_at,
                            object: crate::memory::scope_object_url(i.scope.as_ref()),
                            sentence: i.content,
                            kind: i.semantic_kind.unwrap_or_else(|| i.kind.to_string()),
                            basis: i.basis.map(|b| b.as_str().to_string()),
                            explicit: i.explicit,
                            confidence: Some(i.confidence),
                            stability: Some(i.stability.as_str().to_string()),
                            valid_until: i.valid_until,
                            applies_to,
                            except,
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(basis: &str) -> Hint {
        Hint {
            id: "item_a@1".into(),
            time: "2026-11-01T00:00:00Z".into(),
            sentence: "s".into(),
            basis: Some(basis.into()),
            confidence: Some(0.8),
            stability: Some("contextual".into()),
            ..Default::default()
        }
    }

    #[test]
    fn an_inference_is_never_shown_as_the_users_words() {
        assert_eq!(hint("inference").qualifier(), "the agent's inference, not confirmed");
        assert_eq!(hint("user_statement").qualifier(), "stated by the user");
        let mut h = hint("user_statement");
        h.explicit = true;
        assert_eq!(h.qualifier(), "the user's explicit statement");
        assert_eq!(hint("third_party_claim").qualifier(), "a third party's claim, not verified");
        let mut h = hint("tool_observation");
        h.confidence = Some(0.4);
        assert_eq!(h.qualifier(), "observed; weak evidence");
    }

    #[test]
    fn range_and_end_are_part_of_the_clue() {
        let mut h = hint("tool_observation");
        h.applies_to = vec!["workspace:proj-a".into(), "tool-x/v1".into()];
        h.except = vec!["tool-x/v2".into()];
        assert_eq!(h.qualifier(), "observed; applies to workspace:proj-a, tool-x/v1; not to tool-x/v2");
        // A short-lived clue says when it ends, or at least that it does.
        let mut h = hint("user_statement");
        h.stability = Some("volatile".into());
        assert_eq!(h.qualifier(), "stated by the user; short-lived");
        h.valid_until = Some("2026-11-13T23:59:59+08:00".into());
        assert_eq!(h.qualifier(), "stated by the user; holds until 2026-11-13T23:59:59+08:00");
        // The object the clue is about is named once.
        let mut h = hint("tool_observation");
        h.object = Some("obj://a/b".into());
        h.applies_to = vec!["obj://a/b".into()];
        assert_eq!(h.note(), "observed; noticed 2026-11-01T00:00:00Z; about obj://a/b");
        // Nothing known about its evidence: nothing claimed.
        let bare = Hint { time: "t".into(), ..Default::default() };
        assert_eq!(bare.qualifier(), "");
        assert_eq!(bare.note(), "noticed t");
    }
}
