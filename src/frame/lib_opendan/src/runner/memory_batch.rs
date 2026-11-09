//! The Memory material of a controlled input batch (recall wiring TODO
//! notepads/agent-memory-recall-wiring-todo.md §3).
//!
//! Every batch of a UI or Work session with new input first forgets the
//! topic tags the conversation stopped mentioning, then recalls and
//! observes through the Memory facade as the session's host-bound identity
//! ([`super::consolidation::memory_caller`]): a UI batch recalls by its topic
//! and the text of its messages, a Work only on its first batch (by its
//! title and objective, kept as its topic) or when its observation must be
//! rebuilt; both observe what changed after their snapshot. One budget
//! splits both (UI 8, Work 4; validity changes first).
//!
//! The material becomes `<<background_context>>` lines, one per clue. A
//! clue counts as shown only when the rendered input contains its line;
//! [`settle`] turns that into the session's next [`ObservationState`],
//! which the batch's receipt carries (a batch that is never committed
//! changes nothing). A recall that fails shows one `memory unavailable`
//! line and moves no progress.

use std::collections::{BTreeMap, BTreeSet};

use crate::memory::{
    observe_batch, BatchDelivery, BatchEntry, Budget, Caller, Change, ChangeKind,
    DispositionOutcome, HintLayer, HintState, ObservationState, QueryScope, TopicConfig,
};
use crate::protocol::*;
use crate::Session;

use super::background::{one_line, BackgroundLine};
use super::shared::Shared;

/// Key prefix of a clue's line (`memory:<reference>|<state>`).
pub const KEY_PREFIX: &str = "memory:";
/// Key of the line that says the material could not be read.
pub const KEY_UNAVAILABLE: &str = "memory:unavailable";

/// What a batch tells its receipt about Memory.
#[derive(Debug, Clone)]
pub(super) struct MemoryReceipt {
    pub state: ObservationState,
    /// The topic other sessions see (UI sessions).
    pub topic: Option<Topic>,
    pub error: Option<String>,
}

/// One batch's material before rendering.
pub(super) struct Prepared {
    obs: ObservationState,
    caller: Caller,
    delivery: Option<BatchDelivery>,
    kind: SessionKind,
    /// One line per delivered clue, in delivery order.
    pub lines: Vec<BackgroundLine>,
    /// Why nothing could be read (its line goes through the background
    /// selection: shown once while it lasts).
    pub error: Option<String>,
}

/// The same budget for every batch of a kind (recall wiring TODO §3.4).
pub fn budget(kind: SessionKind) -> Budget {
    match kind {
        SessionKind::Work => Budget {
            max_items: 4,
            cognitions: 2,
            perceptions: 1,
        },
        _ => Budget::default(),
    }
}

/// Whether this batch takes Memory material: a UI or Work session whose
/// template loads hints, with new input (or its first batch).
pub(super) fn wanted(sh: &Shared, cfg: &SessionConfig, state: &SessionState, new_input: bool) -> bool {
    sh.deps.options.load_hints
        && cfg.session.policy.load_hints
        && matches!(cfg.session.kind, SessionKind::Ui | SessionKind::Work)
        && (!state.bootstrap_done || new_input)
}

/// The observation a session starts with: its base scope (the subjects it
/// may read for, the Agent itself among them, never forgotten), and for a
/// Work its title and objective as the topic its host gave it.
pub fn initial(cfg: &SessionConfig, caller: &Caller) -> ObservationState {
    let mut obs = ObservationState::new(QueryScope {
        subjects: caller.grants.iter().cloned().collect(),
        objects: Vec::new(),
    });
    if cfg.session.kind == SessionKind::Work {
        obs.topic.title = work_title(cfg);
    }
    obs
}

fn work_title(cfg: &SessionConfig) -> String {
    let title = cfg.session.title.clone().unwrap_or_default();
    let objective = cfg.session.objective.trim();
    let mut t = title.trim().to_string();
    if !objective.is_empty() && objective != t {
        if !t.is_empty() {
            t.push('\n');
        }
        t.push_str(objective);
    }
    t
}

/// The session's observation for this batch: restored as it was saved
/// (scores, times, snapshot, pending, read set), its base scope following
/// the session's current identity.
fn current(cfg: &SessionConfig, state: &SessionState, caller: &Caller) -> ObservationState {
    let mut obs = state
        .memory
        .clone()
        .unwrap_or_else(|| initial(cfg, caller));
    let subjects: Vec<String> = caller.grants.iter().cloned().collect();
    if obs.topic.base_scope.subjects != subjects {
        obs.topic.base_scope.subjects = subjects;
    }
    obs
}

/// The topic a UI session shows other sessions.
pub fn topic_view(obs: &ObservationState) -> Topic {
    Topic {
        title: obs.topic.title.clone(),
        tags: obs.tags(),
    }
}

/// Forget, recall and observe for this batch. Never fails: what cannot be
/// read is said by one line.
pub(super) async fn prepare(
    sh: &Shared,
    cfg: &SessionConfig,
    state: &SessionState,
    picked: &[FetchedInput],
    now_ms: u64,
) -> Prepared {
    let caller = super::consolidation::memory_caller(cfg, &super::consolidation::caps(sh));
    let mut obs = current(cfg, state, &caller);
    let topic_cfg = TopicConfig::default();
    obs.decay(now_ms, &topic_cfg);
    let kind = cfg.session.kind;
    let text: Vec<String> = if kind == SessionKind::Ui {
        picked
            .iter()
            .filter_map(|m| m.msg().map(|msg| msg.msg.content.content.trim().to_string()))
            .filter(|t| !t.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    let entry = BatchEntry {
        recall: !text.is_empty(),
        text: text.join("\n"),
    };
    let mut p = Prepared {
        obs,
        caller,
        delivery: None,
        kind,
        lines: Vec::new(),
        error: None,
    };
    let memory = match super::consolidation::memory(sh) {
        Ok(m) => m,
        Err(e) => {
            p.error = Some(e.to_string());
            return p;
        }
    };
    let (obs, caller, budget) = (p.obs.clone(), p.caller.clone(), budget(kind));
    let r = tokio::task::spawn_blocking(move || {
        observe_batch(&memory, &caller, &obs, &entry, &budget, &topic_cfg)
    })
    .await;
    match r {
        Ok(Ok(d)) => {
            p.lines = d.items.iter().map(|c| line(c, &d.corrections)).collect();
            p.delivery = Some(d);
        }
        Ok(Err(e)) => p.error = Some(e.to_string()),
        Err(e) => p.error = Some(e.to_string()),
    }
    if let Some(e) = &p.error {
        log::warn!("session {}: memory unavailable: {e}", cfg.session_id());
    }
    p
}

impl Prepared {
    /// The line standing for an error (`None`: there is none).
    pub fn error_line(&self) -> Option<BackgroundLine> {
        self.error.as_ref().map(|e| {
            BackgroundLine::new(KEY_UNAVAILABLE, format!("memory unavailable: {}", one_line(e, 300)))
        })
    }

    /// The template variable `hints`: the clue lines of this batch.
    pub fn texts(&self) -> Vec<String> {
        self.lines.iter().map(|l| l.text.clone()).collect()
    }
}

/// After rendering: the session's observation once `text` is committed.
pub(super) fn settle(p: Prepared, text: &str, now_ms: u64) -> MemoryReceipt {
    let Prepared {
        mut obs,
        caller,
        delivery,
        kind,
        lines,
        error,
    } = p;
    if let Some(d) = &delivery {
        let displayed: BTreeSet<String> = lines
            .iter()
            .filter(|l| text.contains(&l.text))
            .filter_map(|l| l.key.strip_prefix(KEY_PREFIX).map(str::to_string))
            .collect();
        obs.commit_batch(d, &displayed, &caller, now_ms, &TopicConfig::default());
    }
    MemoryReceipt {
        topic: (kind == SessionKind::Ui).then(|| topic_view(&obs)),
        state: obs,
        error,
    }
}

/// A structured reply stated the conversation's topic: its title and the
/// tags it mentions are reinforced (others keep decaying), and the view
/// other sessions see follows.
pub(super) fn apply_reply_topic(sh: &Shared, s: &mut Session, topic: &Topic) {
    let caller = super::consolidation::memory_caller(&s.config, &super::consolidation::caps(sh));
    let mut obs = current(&s.config, &s.state, &caller);
    match obs.reply_topic(&topic.title, &topic.tags, crate::now_ms(), &TopicConfig::default()) {
        Ok(_) => {
            s.state.topic = topic_view(&obs);
            s.state.memory = Some(obs);
        }
        Err(e) => log::warn!("session {}: topic not applied: {e}", s.sid()),
    }
}

fn hint_text(h: &crate::memory::Hint) -> String {
    one_line(&h.summary, 300)
}

/// One clue's line (recall wiring TODO §3.4): what it is (a cognition, a
/// note not yet consolidated, a validity change, a question, a note's
/// disposition) is said before its words.
pub fn line(c: &Change, corrections: &BTreeMap<String, String>) -> BackgroundLine {
    let h = &c.hint;
    let key = format!("{KEY_PREFIX}{}", c.key());
    let text = match h.layer {
        HintLayer::Cognition => {
            let stmt = hint_text(h);
            let note = one_line(&h.note(), 400);
            match h.state {
                HintState::Superseded | HintState::Deleted | HintState::Stale => {
                    let why = match (h.state, &h.replaced_by) {
                        (HintState::Superseded, Some(r)) => format!("superseded by {r}"),
                        (HintState::Superseded, None) => "superseded".to_string(),
                        (HintState::Deleted, _) => "deleted".to_string(),
                        _ => "no longer current".to_string(),
                    };
                    format!("hint withdrawn ({why}), do not rely on it: {stmt} ({note})")
                }
                HintState::Disputed => format!("hint disputed: {stmt} ({note})"),
                HintState::ReviewPending | HintState::MayBeCorrected => {
                    let words: Vec<String> = h
                        .corrections
                        .iter()
                        .filter_map(|r| corrections.get(r))
                        .map(|t| format!("\"{}\"", one_line(t, 200)))
                        .collect();
                    let by = if words.is_empty() {
                        "a note not yet consolidated".to_string()
                    } else {
                        words.join("; ")
                    };
                    format!("hint: {stmt} ({note}) — may have been corrected: {by}; clarify before relying on it")
                }
                _ => match &c.kind {
                    ChangeKind::ReadRevision { from } => {
                        format!("hint revised (you saw @{from}): {stmt} ({note})")
                    }
                    _ => format!("hint: {stmt} ({note})"),
                },
            }
        }
        HintLayer::Perception => match h.state {
            HintState::Disposed => {
                let d = h.disposition.as_ref();
                let into = d
                    .map(|d| d.cognition_refs.join(", "))
                    .filter(|r| !r.is_empty());
                let what = match (d.map(|d| d.outcome), into) {
                    (Some(DispositionOutcome::Duplicate), Some(r)) => format!("was a duplicate of {r}"),
                    (Some(DispositionOutcome::Discarded), _) => "was discarded by consolidation".to_string(),
                    (_, Some(r)) => format!("was consolidated into {r}"),
                    _ => "was consolidated".to_string(),
                };
                format!("note {} {what} (the same material, not new evidence)", h.reference)
            }
            _ => {
                let what = if h.is_correction() {
                    "correction noted"
                } else if h.explicit {
                    "asked to remember"
                } else {
                    "noted"
                };
                let status = if h.state == HintState::Deferred {
                    "consolidation deferred"
                } else {
                    "not yet consolidated"
                };
                let mut note = Vec::new();
                if let Some(sid) = &h.session_id {
                    note.push(format!("from session {sid}"));
                }
                if let Some(t) = &h.observed_at {
                    note.push(format!("noticed {t}"));
                }
                note.push(format!("ref {}", h.reference));
                format!("{what}, {status}: {} ({})", hint_text(h), note.join("; "))
            }
        },
        HintLayer::Clarification => format!(
            "memory question pending: {} (about note {})",
            hint_text(h),
            h.reference
        ),
    };
    BackgroundLine::new(key, text)
}
