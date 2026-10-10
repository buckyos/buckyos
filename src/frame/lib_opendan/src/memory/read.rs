//! Reading known references (§6.3.2, §6.3.4): a cognition at its current or
//! a fixed revision, its provenance, or a perception by `<sid>:<seq>`. None
//! of these depends on the search index; all of them filter visibility
//! first and answer "not found" for what the caller may not see.

use agent_tool::agent_memory::{AffectedFilter, ItemStatus, MemoryItem, ObservationKind};
use serde::{Deserialize, Serialize};

use crate::protocol::PerceptionRecord;

use super::hint::{Hint, HintState};
use super::recall::{corrections_for, is_pending, overlay_corrections, record_visible};
use super::{Caller, Disposition, Memory, MemoryError, MemoryResult, ProducedBy};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadView {
    Cognition,
    Provenance,
    Perception,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CognitionView {
    /// The requested revision (fixed content) or the current one.
    pub item: MemoryItem,
    pub current_revision: u64,
    pub current_status: ItemStatus,
    pub hint: Hint,
    /// Unconsolidated corrections in the same subject and scope (§4.3).
    pub corrections: Vec<Hint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAvailability {
    Available,
    /// The original is gone (purged history); never reconstructed.
    Unreadable,
    /// The evidence exists but its subjects are outside the caller's grants.
    Restricted,
    /// No source resolver is bound.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BasisEntry {
    pub observation_id: String,
    pub kind: ObservationKind,
    pub content: String,
    pub event_ref: Option<String>,
    pub occurred_at: Option<String>,
    pub excerpt: Option<String>,
    pub availability: SourceAvailability,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevisionEntry {
    pub revision: u64,
    pub status: ItemStatus,
    pub statement: String,
    pub source_occasion: String,
    pub noticed_at: String,
    pub produced_by: Option<ProducedBy>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceView {
    pub item_id: String,
    pub revision: u64,
    pub statement: String,
    /// Evidence of the current revision.
    pub current_basis: Vec<BasisEntry>,
    /// Evidence that only earlier revisions used.
    pub historical_basis: Vec<BasisEntry>,
    pub revisions: Vec<RevisionEntry>,
    /// Perceptions consolidated into this cognition.
    pub absorbed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerceptionView {
    pub reference: String,
    /// Body removed once the record is cleared.
    pub record: PerceptionRecord,
    pub cleared: bool,
    pub disposition: Option<Disposition>,
    pub hint: Hint,
}

fn not_found(r: &str) -> MemoryError {
    MemoryError::NotFound(r.to_string())
}

impl Memory {
    /// `item_x` (current) or `item_x@N` (fixed revision).
    pub fn read_cognition(&self, caller: &Caller, reference: &str) -> MemoryResult<CognitionView> {
        let view = self.graph_view()?;
        let (id, rev) = super::perception::parse_cite(reference)?;
        let current = view.item(&id).ok_or_else(|| not_found(reference))?;
        let item = match rev {
            Some(r) => view.item_at(&id, r).ok_or_else(|| not_found(reference))?,
            None => current,
        };
        if !current.visible_to(&caller.grants) || !item.visible_to(&caller.grants) {
            return Err(not_found(reference));
        }
        let pending: Vec<PerceptionRecord> = self
            .load_perceptions(None)?
            .into_iter()
            .filter(|r| is_pending(r, view.disposition(&r.reference())))
            .collect();
        let mut hint = Hint::from_item(
            &view,
            item,
            vec!["read:reference".into()],
            0.0,
            &caller.grants,
        );
        if item.revision != current.revision {
            hint.state = HintState::of_item(current.status);
            if hint.state == HintState::Active {
                hint.state = HintState::Superseded;
            }
            hint.replaced_by = Some(current.reference());
        }
        overlay_corrections(&mut hint, current, &pending, caller);
        let (corr, _) = corrections_for(current, &pending, caller);
        let corrections = corr
            .into_iter()
            .map(|r| {
                Hint::from_perception(
                    r,
                    view.disposition(&r.reference()),
                    vec!["correction:same-scope".into()],
                )
            })
            .collect();
        Ok(CognitionView {
            item: item.clone(),
            current_revision: current.revision,
            current_status: current.status,
            hint,
            corrections,
        })
    }

    pub fn read_provenance(&self, caller: &Caller, item_id: &str) -> MemoryResult<ProvenanceView> {
        let view = self.graph_view()?;
        let item = view.item(item_id).ok_or_else(|| not_found(item_id))?;
        if !item.visible_to(&caller.grants) {
            return Err(not_found(item_id));
        }
        let entry = |obs_id: &str| -> Option<BasisEntry> {
            let o = view.observation(obs_id)?;
            let visible = o
                .scope
                .as_ref()
                .is_none_or(|s| s.visible_to(&caller.grants));
            let event_ref = o
                .source_ref
                .as_ref()
                .and_then(|s| s.event_ref.clone().or_else(|| s.uri.clone()));
            if !visible {
                return Some(BasisEntry {
                    observation_id: o.observation_id.clone(),
                    kind: o.kind,
                    content: String::new(),
                    event_ref: None,
                    occurred_at: None,
                    excerpt: None,
                    availability: SourceAvailability::Restricted,
                });
            }
            let (availability, excerpt) = match (&self.config().sources, &event_ref) {
                (Some(src), Some(ev)) => match src.lookup(ev) {
                    Some(info) if info.readable => (SourceAvailability::Available, info.excerpt),
                    _ => (SourceAvailability::Unreadable, None),
                },
                _ => (SourceAvailability::Unknown, None),
            };
            Some(BasisEntry {
                observation_id: o.observation_id.clone(),
                kind: o.kind,
                content: o.content.clone(),
                event_ref,
                occurred_at: o.occurred_at.clone(),
                excerpt,
                availability,
            })
        };
        let current_basis: Vec<BasisEntry> =
            item.evidence.iter().filter_map(|e| entry(e)).collect();
        let mut historical = Vec::new();
        for rev in view.item_history(item_id) {
            for e in &rev.evidence {
                if !item.evidence.contains(e)
                    && !historical
                        .iter()
                        .any(|b: &BasisEntry| &b.observation_id == e)
                {
                    if let Some(b) = entry(e) {
                        historical.push(b);
                    }
                }
            }
        }
        let revisions = view
            .item_history(item_id)
            .iter()
            .filter(|r| r.visible_to(&caller.grants))
            .map(|r| RevisionEntry {
                revision: r.revision,
                status: r.status,
                statement: r.statement(),
                source_occasion: r.source_occasion.clone(),
                noticed_at: r.noticed_at.clone(),
                produced_by: r.produced_by.clone(),
            })
            .collect();
        let absorbed = view
            .dispositions()
            .iter()
            .filter(|(_, d)| {
                d.disposition
                    .cognition_refs
                    .iter()
                    .any(|c| c.split('@').next() == Some(item_id))
            })
            .map(|(r, _)| r.clone())
            .filter(|r| {
                self.perception(r)
                    .ok()
                    .flatten()
                    .is_some_and(|rec| record_visible(&rec, caller))
            })
            .collect();
        Ok(ProvenanceView {
            item_id: item_id.to_string(),
            revision: item.revision,
            statement: item.statement(),
            current_basis,
            historical_basis: historical,
            revisions,
            absorbed,
        })
    }

    /// A perception by reference; a cleared one returns its disposition and
    /// carrying cognitions, never the old body (A.9).
    pub fn read_perception(
        &self,
        caller: &Caller,
        reference: &str,
    ) -> MemoryResult<PerceptionView> {
        let r = self
            .perception(reference)?
            .ok_or_else(|| not_found(reference))?;
        if !record_visible(&r, caller) {
            return Err(not_found(reference));
        }
        let view = self.graph_view()?;
        let d = view.disposition(reference);
        let hint = Hint::from_perception(&r, d, vec!["read:reference".into()]);
        Ok(PerceptionView {
            reference: reference.to_string(),
            cleared: r.cleared.is_some(),
            disposition: hint.disposition.clone(),
            record: r,
            hint,
        })
    }

    /// Cognitions that depend on a source event, task, goal run or object
    /// (M-27); visible ones only.
    pub fn affected_by(&self, caller: &Caller, filter: &AffectedFilter) -> MemoryResult<Vec<Hint>> {
        let view = self.graph_view()?;
        Ok(view
            .items_affected_by(filter)
            .into_iter()
            .filter(|i| i.visible_to(&caller.grants))
            .map(|i| Hint::from_item(&view, i, vec!["affected".into()], 0.0, &caller.grants))
            .collect())
    }
}
