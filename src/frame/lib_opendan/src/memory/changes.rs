//! `changes_since` (A.9): what changed after a snapshot, for one Session's
//! topic and read set. Memory keeps no subscription state; the Session
//! holds its snapshot, pending refs and read set and pulls before each
//! inference. An unchanged snapshot answers without opening the Graph.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use agent_tool::agent_memory::{GraphView, ItemStatus, MemoryItem};
use serde::{Deserialize, Serialize};

use crate::protocol::PerceptionRecord;

use super::hint::{Hint, HintState};
use super::perception::parse_cite;
use super::recall::{
    cites_item, deferral_expired, is_pending, item_topic_hit, overlay_corrections,
    perception_topic_hit, record_visible, topic_objects, TopicScope,
};
use super::{Caller, Memory, MemoryResult, MemorySnapshot};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChangeKind {
    NewPerception,
    /// A perception was disposed; its hint carries the outcome and the
    /// carrying cognitions (no body once cleared).
    PerceptionDisposed,
    NewCognition,
    RevisedCognition {
        from: u64,
    },
    CognitionStatus {
        status: ItemStatus,
    },
    /// A correction citing a cognition is waiting for consolidation.
    ReviewPending {
        by: Vec<String>,
    },
    /// A cognition the Session read before was revised or changed status by
    /// a consolidation (read set, M-21).
    ReadRevision {
        from: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    pub hint: Hint,
}

impl Change {
    /// Budget pool: 0 validity change, 1 cognition, 2 perception, 3 marker.
    pub fn pool(&self) -> usize {
        match &self.kind {
            ChangeKind::ReviewPending { .. }
            | ChangeKind::ReadRevision { .. }
            | ChangeKind::CognitionStatus { .. } => 0,
            ChangeKind::RevisedCognition { .. } | ChangeKind::NewCognition => {
                if self.hint.state.is_validity_change() {
                    0
                } else {
                    1
                }
            }
            ChangeKind::NewPerception => 2,
            ChangeKind::PerceptionDisposed => 3,
        }
    }

    /// The ref a Session keeps when this change does not fit its budget.
    pub fn pending_ref(&self) -> String {
        match &self.kind {
            ChangeKind::ReviewPending { .. } => format!("r:{}", self.hint.id),
            ChangeKind::NewPerception | ChangeKind::PerceptionDisposed => {
                format!("p:{}", self.hint.id)
            }
            _ => format!("c:{}", self.hint.id),
        }
    }

    /// Dedupe key: same reference and same state.
    pub fn key(&self) -> String {
        format!("{}|{:?}", self.hint.reference, self.hint.state)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
    /// Where the Session moves to once it has assembled these changes.
    pub snapshot: MemorySnapshot,
    /// Too many changes: rebuild with `query_topic`.
    pub resync_required: bool,
    /// Answered from the snapshot alone (no Graph read, no lock).
    pub fast_path: bool,
}

pub(super) struct Ctx {
    pub view: std::sync::Arc<GraphView>,
    pub records: HashMap<String, PerceptionRecord>,
    pub pending: Vec<PerceptionRecord>,
    pub objects: BTreeSet<String>,
}

impl Memory {
    pub(super) fn change_ctx(&self, caller: &Caller, topic: &TopicScope) -> MemoryResult<Ctx> {
        let view = self.graph_view()?;
        let all = self.load_perceptions(None)?;
        let pending = all
            .iter()
            .filter(|r| is_pending(r, view.disposition(&r.reference())))
            .cloned()
            .collect();
        let objects = topic_objects(&view, topic, caller);
        Ok(Ctx {
            records: all.into_iter().map(|r| (r.reference(), r)).collect(),
            pending,
            objects,
            view,
        })
    }

    fn cognition_change(
        &self,
        ctx: &Ctx,
        caller: &Caller,
        topic: &TopicScope,
        item: &MemoryItem,
        read_set: &BTreeMap<String, u64>,
    ) -> Option<Change> {
        if !item.visible_to(&caller.grants) {
            return None;
        }
        if let Some(from) = read_set.get(&item.item_id).copied() {
            if item.revision > from {
                let mut hint = Hint::from_item(
                    &ctx.view,
                    item,
                    vec![format!("read_set:@{from}")],
                    0.0,
                    &caller.grants,
                );
                overlay_corrections(&mut hint, item, &ctx.pending, caller);
                return Some(Change {
                    kind: ChangeKind::ReadRevision { from },
                    hint,
                });
            }
            return None;
        }
        let reasons = item_topic_hit(&ctx.view, item, topic, &ctx.objects, caller)?;
        let mut hint = Hint::from_item(&ctx.view, item, reasons, 0.0, &caller.grants);
        overlay_corrections(&mut hint, item, &ctx.pending, caller);
        let kind = if !item.status.recallable() {
            ChangeKind::CognitionStatus {
                status: item.status,
            }
        } else if item.revision == 1 {
            ChangeKind::NewCognition
        } else {
            ChangeKind::RevisedCognition {
                from: item.revision - 1,
            }
        };
        Some(Change { kind, hint })
    }

    fn perception_change(
        &self,
        ctx: &Ctx,
        caller: &Caller,
        topic: &TopicScope,
        r: &PerceptionRecord,
    ) -> Vec<Change> {
        if r.session_id == caller.session_id || !record_visible(r, caller) {
            return Vec::new();
        }
        let Some(reasons) = perception_topic_hit(r, topic) else {
            return Vec::new();
        };
        let d = ctx.view.disposition(&r.reference());
        let hint = Hint::from_perception(r, d, reasons);
        if !is_pending(r, d) {
            return vec![Change {
                kind: ChangeKind::PerceptionDisposed,
                hint,
            }];
        }
        if deferral_expired(d, self.now()) {
            return Vec::new();
        }
        let mut out = vec![Change {
            kind: ChangeKind::NewPerception,
            hint,
        }];
        if r.suggested_kind.as_deref() == Some("correction") {
            for c in &r.cites {
                let Ok((id, _)) = parse_cite(c) else { continue };
                if let Some(item) = ctx
                    .view
                    .item(&id)
                    .filter(|i| i.visible_to(&caller.grants) && i.status.recallable())
                {
                    let mut hint = Hint::from_item(
                        &ctx.view,
                        item,
                        vec![format!("cited_by:{}", r.reference())],
                        0.0,
                        &caller.grants,
                    );
                    overlay_corrections(&mut hint, item, &ctx.pending, caller);
                    hint.state = HintState::ReviewPending;
                    out.push(Change {
                        kind: ChangeKind::ReviewPending {
                            by: hint.corrections.clone(),
                        },
                        hint,
                    });
                }
            }
        }
        out
    }

    /// Changes after `since` that concern this Session: perceptions and
    /// dispositions in its topic (its own perceptions excluded), cognitions
    /// added, revised or retired in its topic, corrections waiting on a
    /// cognition, and any consolidation change of a cognition in its read set
    /// even when the topic moved on. `read_set` maps `item_…` to the revision
    /// read; a `<sid>:<seq>` key marks a perception already shown, whose
    /// disposition is reported even if it no longer matches the topic.
    pub fn changes_since(
        &self,
        caller: &Caller,
        since: &MemorySnapshot,
        topic: &TopicScope,
        read_set: &BTreeMap<String, u64>,
        limit: usize,
    ) -> MemoryResult<ChangeSet> {
        let now_snap = self.snapshot()?;
        if &now_snap == since {
            return Ok(ChangeSet {
                changes: Vec::new(),
                snapshot: now_snap,
                resync_required: false,
                fast_path: true,
            });
        }
        let ctx = self.change_ctx(caller, topic)?;
        let mut changes: Vec<Change> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut push = |c: Change, changes: &mut Vec<Change>| {
            if seen.insert(c.key()) {
                changes.push(c);
            }
        };

        // Graph occasions after the snapshot (up to the new snapshot).
        let mut items = BTreeSet::new();
        let mut disposed = BTreeSet::new();
        for occ in ctx.view.occasions_after(since.graph_seq()) {
            if occ.seq > now_snap.graph_seq() {
                break;
            }
            for d in &occ.dispositions {
                disposed.insert(d.perception_ref.clone());
            }
        }
        for item in ctx.view.items() {
            let changed = ctx.view.item_changed_seq(&item.item_id);
            if changed > since.graph_seq() && changed <= now_snap.graph_seq() {
                items.insert(item.item_id.clone());
            }
        }
        for id in &items {
            if let Some(item) = ctx.view.item(id) {
                if let Some(c) = self.cognition_change(&ctx, caller, topic, item, read_set) {
                    push(c, &mut changes);
                }
            }
        }
        for r in &disposed {
            let Some(rec) = ctx.records.get(r) else {
                continue;
            };
            let found = self.perception_change(&ctx, caller, topic, rec);
            if !found.is_empty() {
                for c in found {
                    push(c, &mut changes);
                }
            } else if read_set.contains_key(r)
                && rec.session_id != caller.session_id
                && record_visible(rec, caller)
            {
                // Shown before, no longer matching (cleanup dropped its
                // body): the marker still arrives.
                let hint = Hint::from_perception(
                    rec,
                    ctx.view.disposition(r),
                    vec!["shown_before".into()],
                );
                push(
                    Change {
                        kind: ChangeKind::PerceptionDisposed,
                        hint,
                    },
                    &mut changes,
                );
            }
        }
        // New perception records after each file's component.
        let mut fresh: Vec<&PerceptionRecord> = ctx
            .records
            .values()
            .filter(|r| {
                r.seq > since.perception_seq(&r.session_id)
                    && r.seq <= now_snap.perception_seq(&r.session_id)
            })
            .collect();
        fresh.sort_by_key(|r| (r.at_ms, r.session_id.clone(), r.seq));
        for r in fresh {
            for c in self.perception_change(&ctx, caller, topic, r) {
                push(c, &mut changes);
            }
        }
        if changes.len() > limit {
            return Ok(ChangeSet {
                changes: Vec::new(),
                snapshot: now_snap,
                resync_required: true,
                fast_path: false,
            });
        }
        changes.sort_by_key(|c| c.pool());
        Ok(ChangeSet {
            changes,
            snapshot: now_snap,
            resync_required: false,
            fast_path: false,
        })
    }

    /// Re-derive a pending ref against the current state (visibility is
    /// checked again; a ref that no longer says anything new disappears).
    pub(super) fn resolve_pending(
        &self,
        ctx: &Ctx,
        caller: &Caller,
        topic: &TopicScope,
        read_set: &BTreeMap<String, u64>,
        r: &str,
    ) -> Option<Change> {
        let (kind, id) = r.split_once(':')?;
        match kind {
            "c" => {
                let item = ctx.view.item(id)?;
                if !item.visible_to(&caller.grants) {
                    return None;
                }
                if let Some(c) = self.cognition_change(ctx, caller, topic, item, read_set) {
                    return Some(c);
                }
                // Out of the topic now: still deliverable as a plain clue.
                let mut hint =
                    Hint::from_item(&ctx.view, item, vec!["pending".into()], 0.0, &caller.grants);
                overlay_corrections(&mut hint, item, &ctx.pending, caller);
                Some(Change {
                    kind: ChangeKind::NewCognition,
                    hint,
                })
            }
            "r" => {
                let item = ctx.view.item(id)?;
                if !item.visible_to(&caller.grants) {
                    return None;
                }
                let by: Vec<String> = ctx
                    .pending
                    .iter()
                    .filter(|p| {
                        p.suggested_kind.as_deref() == Some("correction") && cites_item(p, id)
                    })
                    .map(|p| p.reference())
                    .collect();
                if by.is_empty() {
                    return None;
                }
                let mut hint =
                    Hint::from_item(&ctx.view, item, vec!["pending".into()], 0.0, &caller.grants);
                overlay_corrections(&mut hint, item, &ctx.pending, caller);
                hint.state = HintState::ReviewPending;
                Some(Change {
                    kind: ChangeKind::ReviewPending { by },
                    hint,
                })
            }
            "p" | "q" => {
                let rec = ctx.records.get(id)?;
                if !record_visible(rec, caller) {
                    return None;
                }
                let d = ctx.view.disposition(id);
                let hint = Hint::from_perception(rec, d, vec!["pending".into()]);
                let kind = if is_pending(rec, d) {
                    ChangeKind::NewPerception
                } else {
                    ChangeKind::PerceptionDisposed
                };
                Some(Change { kind, hint })
            }
            _ => None,
        }
    }
}
