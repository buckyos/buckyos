//! Writing perceptions (§4.2, A.5): the model gives content and object
//! hints; the host binds identity, subjects, source event, time and policy.

use agent_tool::agent_memory::{validate_object_path, GraphView};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lock::Lease;
use crate::protocol::{Mention, MentionCandidate, PerceptionAnchors, PerceptionRecord};
use crate::state::perception::{
    append_locked, content_digest, lock_perception_file, read_records, tail_seq,
};

use super::{ActorKind, Caller, Memory, MemoryError, MemoryResult, RecordPolicy, Scope, SourceRef};

pub const MAX_PERCEPTION_BYTES: usize = 8 * 1024;
const MAX_RELATED: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PerceptionInput {
    pub content: String,
    /// Structured attributes of the observation (tool result fields…).
    #[serde(default)]
    pub attributes: Value,
    /// Narrows the caller's default subjects; must stay within its grants.
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub objects: Vec<String>,
    #[serde(default)]
    pub exceptions: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub suggested_kind: Option<String>,
    #[serde(default)]
    pub memory_intent: Option<String>,
    /// Cognitions (`item_…@rev`) the observation derives from or corrects.
    #[serde(default)]
    pub cites: Vec<String>,
    #[serde(default)]
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub anchors: Option<PerceptionAnchors>,
    /// Raw mentions ("Bob"); the component attaches candidates, never picks.
    #[serde(default)]
    pub mentions: Vec<String>,
    /// Original event, bound by the host.
    #[serde(default)]
    pub source: Option<SourceRef>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Runtime record kind (`run_digest`, `task_outcome`, `task_discarded`);
    /// `None` = an observation.
    #[serde(default)]
    pub kind: Option<String>,
    /// Seq chosen by the caller when it allocates seqs for this file (a
    /// session driver); `None` = next after the tail. One file has one
    /// allocator.
    #[serde(default)]
    pub seq: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Recorded,
    /// Same idempotency key and content: the earlier record.
    Replayed,
    /// The Session's policy said not to remember; nothing was written.
    Skipped,
}

/// Write receipt: the perception is recorded, nothing more (§4.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerceptionReceipt {
    pub status: ReceiptStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub seq: u64,
    /// The record was already disposed and cleaned (a replay never revives it).
    #[serde(default)]
    pub cleared: bool,
    /// Visible cognitions in the same subject and scope (optional hint for
    /// correction references; omitted when the Graph cannot be read).
    #[serde(default)]
    pub related_cognitions: Vec<String>,
    #[serde(default)]
    pub mentions: Vec<Mention>,
}

fn invalid(m: impl Into<String>) -> MemoryError {
    MemoryError::Invalid(m.into())
}

pub(super) fn parse_cite(c: &str) -> MemoryResult<(String, Option<u64>)> {
    let (id, rev) = match c.split_once('@') {
        Some((id, r)) => (
            id,
            Some(
                r.parse::<u64>()
                    .map_err(|_| invalid(format!("bad cite {c}")))?,
            ),
        ),
        None => (c, None),
    };
    if !id.starts_with("item_") || id.len() <= 5 {
        return Err(invalid(format!(
            "a cite must name a cognition `item_…@rev`: {c}"
        )));
    }
    Ok((id.to_string(), rev))
}

impl Memory {
    /// Record one perception for `caller` (A.5). The lease is the caller's
    /// Session lease; the short file lock serializes it with cleanup.
    pub fn record_perception(
        &self,
        caller: &Caller,
        lease: &Lease,
        input: PerceptionInput,
    ) -> MemoryResult<PerceptionReceipt> {
        if caller.record_policy == RecordPolicy::DoNotRecord {
            return Ok(PerceptionReceipt {
                status: ReceiptStatus::Skipped,
                reference: None,
                seq: 0,
                cleared: false,
                related_cognitions: Vec::new(),
                mentions: Vec::new(),
            });
        }
        if lease.resource() != format!("session:{}", caller.session_id) {
            return Err(MemoryError::PermissionDenied(format!(
                "perceptions of {} are written under its session lease, not {}",
                caller.session_id,
                lease.resource()
            )));
        }
        lease.check()?;
        let kind = input
            .kind
            .clone()
            .unwrap_or_else(|| "observation".to_string());
        let runtime = matches!(
            kind.as_str(),
            "run_digest" | "task_outcome" | "task_discarded"
        );
        if !runtime && kind != "observation" {
            return Err(invalid(format!("unknown perception kind {kind}")));
        }
        let content = input.content.trim().to_string();
        if !runtime && content.is_empty() {
            return Err(invalid("a perception needs content"));
        }
        if content.len() > MAX_PERCEPTION_BYTES {
            return Err(invalid(format!(
                "perception content exceeds {MAX_PERCEPTION_BYTES} bytes"
            )));
        }
        let subjects = if input.subjects.is_empty() {
            caller.default_subjects.clone()
        } else {
            input.subjects.clone()
        };
        if let Some(s) = subjects.iter().find(|s| !caller.grants.contains(*s)) {
            return Err(MemoryError::PermissionDenied(format!(
                "subject {s} is outside the session's grants"
            )));
        }
        let scope = Scope {
            subjects,
            objects: input.objects.clone(),
            exceptions: input.exceptions.clone(),
        }
        .normalized();
        if !runtime || !scope.subjects.is_empty() {
            scope.validate()?;
        }
        for o in scope.objects.iter().chain(&scope.exceptions) {
            validate_object_path(o)?;
        }
        if let Some(t) = &input.occurred_at {
            chrono::DateTime::parse_from_rfc3339(t)
                .map_err(|_| invalid(format!("occurred_at {t}")))?;
        }
        if let Some(k) = &input.idempotency_key {
            if k.trim().is_empty() || k.chars().any(char::is_control) {
                return Err(invalid("invalid idempotency_key"));
            }
        }
        let source = self.check_source(input.source.as_ref(), runtime)?;
        let tags = agent_tool::agent_memory::normalize_tags(&input.tags)?;

        let view = self.graph_view().ok();
        let mut cites = Vec::new();
        for c in &input.cites {
            let (id, rev) = parse_cite(c)?;
            let item = view
                .as_ref()
                .and_then(|v| v.item(&id).cloned())
                .filter(|i| i.visible_to(&caller.grants))
                .ok_or_else(|| invalid(format!("cited cognition {c} is unknown")))?;
            cites.push(format!("{id}@{}", rev.unwrap_or(item.revision)));
        }
        let mentions = match &view {
            Some(v) => mention_candidates(v, &input.mentions, caller),
            None => input
                .mentions
                .iter()
                .map(|t| Mention {
                    text: t.clone(),
                    candidates: Vec::new(),
                })
                .collect(),
        };

        let mut record = PerceptionRecord {
            seq: 0,
            at_ms: self.now_ms(),
            session_id: caller.session_id.clone(),
            kind,
            source: "session".to_string(),
            tags,
            objects: scope.objects.clone(),
            summary: content,
            payload: input.attributes.clone(),
            refs: Value::Null,
            idempotency_key: input.idempotency_key.clone(),
            content_digest: None,
            scope: (!scope.subjects.is_empty()).then_some(scope.clone()),
            suggested_kind: input.suggested_kind.clone(),
            memory_intent: input.memory_intent.clone(),
            cites,
            anchors: input.anchors.clone(),
            occurred_at: input.occurred_at.clone(),
            source_ref: source,
            mentions: mentions.clone(),
            backfilled: false,
            cleared: None,
        };
        record.content_digest = Some(content_digest(&record));

        let path = self.layout().perception_file(&caller.session_id);
        let _lock = lock_perception_file(
            self.layout(),
            &caller.session_id,
            self.config().lock_timeout,
        )?;
        if let Some(key) = &record.idempotency_key {
            if let Some(prev) = read_records(&path)?
                .into_iter()
                .find(|r| r.idempotency_key.as_ref() == Some(key))
            {
                if prev.content_digest != record.content_digest {
                    return Err(MemoryError::Conflict(format!(
                        "idempotency key {key} was recorded as {} with different content",
                        prev.reference()
                    )));
                }
                return Ok(PerceptionReceipt {
                    status: ReceiptStatus::Replayed,
                    reference: Some(prev.reference()),
                    seq: prev.seq,
                    cleared: prev.cleared.is_some(),
                    related_cognitions: Vec::new(),
                    mentions: prev.mentions,
                });
            }
        }
        let tail = tail_seq(&path)?;
        record.seq = input.seq.unwrap_or(tail + 1);
        append_locked(lease, &path, vec![record.clone()])?;
        if record.seq <= tail {
            return Ok(PerceptionReceipt {
                status: ReceiptStatus::Replayed,
                reference: Some(record.reference()),
                seq: record.seq,
                cleared: false,
                related_cognitions: Vec::new(),
                mentions,
            });
        }
        let related = view
            .as_ref()
            .map(|v| related_cognitions(v, &scope, caller))
            .unwrap_or_default();
        Ok(PerceptionReceipt {
            status: ReceiptStatus::Recorded,
            reference: Some(record.reference()),
            seq: record.seq,
            cleared: false,
            related_cognitions: related,
            mentions,
        })
    }

    fn check_source(
        &self,
        source: Option<&SourceRef>,
        runtime: bool,
    ) -> MemoryResult<Option<SourceRef>> {
        let Some(s) = source else {
            if runtime {
                return Ok(None);
            }
            return Err(invalid(
                "a perception needs its source event (bound by the host)",
            ));
        };
        s.validate()?;
        if !runtime && s.actor_kind == Some(ActorKind::Runtime) {
            return Err(invalid(
                "Runtime-attached Memory material is not a source event (E-15)",
            ));
        }
        if let (Some(src), Some(ev)) = (&self.config().sources, &s.event_ref) {
            match src.lookup(ev) {
                None => return Err(invalid(format!("unknown source event {ev}"))),
                Some(info) if info.runtime_attachment => {
                    return Err(invalid(format!(
                        "{ev} is a Runtime-attached Memory block, not a source event (E-15)"
                    )))
                }
                Some(info) => {
                    let mut s = s.clone();
                    if s.actor_kind.is_none() {
                        s.actor_kind = info.actor_kind;
                    }
                    return Ok(Some(s));
                }
            }
        }
        Ok(Some(s.clone()))
    }

    /// Every record of every perception file, optionally only up to a
    /// snapshot's components.
    pub(super) fn load_perceptions(
        &self,
        upto: Option<&super::MemorySnapshot>,
    ) -> MemoryResult<Vec<PerceptionRecord>> {
        let mut out = Vec::new();
        for sid in super::snapshot::perception_sids(self.layout())? {
            let limit = upto.map(|s| s.perception_seq(&sid));
            for r in read_records(&self.layout().perception_file(&sid))? {
                if limit.is_none_or(|l| r.seq <= l) {
                    out.push(r);
                }
            }
        }
        Ok(out)
    }

    /// One perception by `<sid>:<seq>`.
    pub(super) fn perception(&self, reference: &str) -> MemoryResult<Option<PerceptionRecord>> {
        let (sid, seq) = reference
            .rsplit_once(':')
            .and_then(|(s, q)| q.parse::<u64>().ok().map(|q| (s, q)))
            .ok_or_else(|| invalid(format!("perception ref must be <sid>:<seq>: {reference}")))?;
        crate::ids::validate_session_id(sid)?;
        Ok(read_records(&self.layout().perception_file(sid))?
            .into_iter()
            .find(|r| r.seq == seq))
    }
}

fn mention_candidates(view: &GraphView, mentions: &[String], caller: &Caller) -> Vec<Mention> {
    mentions
        .iter()
        .map(|text| Mention {
            text: text.clone(),
            candidates: view
                .resolve_alias(text)
                .into_iter()
                .filter(|id| view.object_visible(id, Some(&caller.grants)))
                .filter_map(|id| view.object(&id))
                .map(|o| MentionCandidate {
                    object_id: o.object_id.clone(),
                    canonical_name: o.canonical_name.clone(),
                    basis: format!("alias:{text}"),
                })
                .collect(),
        })
        .collect()
}

fn related_cognitions(view: &GraphView, scope: &Scope, caller: &Caller) -> Vec<String> {
    view.items()
        .filter(|i| i.status.recallable() && i.visible_to(&caller.grants))
        .filter(|i| i.scope.as_ref().is_some_and(|s| s.overlaps(scope)))
        .take(MAX_RELATED)
        .map(|i| i.reference())
        .collect()
}
