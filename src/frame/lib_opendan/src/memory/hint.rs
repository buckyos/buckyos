//! Hints: short clues that point at a cognition or an unconsolidated
//! perception, labelled with their layer, state, scope and evidence
//! character so the two layers are never merged into undifferentiated facts.

use agent_tool::agent_memory::{DispositionState, Grants, GraphView, ItemStatus, MemoryItem};
use serde::{Deserialize, Serialize};

use crate::protocol::PerceptionRecord;

use super::{iso_ms, Basis, Disposition, DispositionOutcome, Scope};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintLayer {
    Cognition,
    Perception,
    /// A low-priority question a deferred material waits on (§5.9).
    Clarification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintState {
    Active,
    Disputed,
    /// A correction citing this cognition is not consolidated yet (§4.3).
    ReviewPending,
    /// An unconsolidated correction in the same subject and scope exists.
    MayBeCorrected,
    Superseded,
    Stale,
    Deleted,
    /// Unconsolidated perception.
    Pending,
    /// Perception deferred by consolidation, still pending.
    Deferred,
    /// Perception disposed (absorbed / duplicate / discarded).
    Disposed,
}

impl HintState {
    pub fn of_item(status: ItemStatus) -> Self {
        match status {
            ItemStatus::Active => HintState::Active,
            ItemStatus::Disputed => HintState::Disputed,
            ItemStatus::Superseded => HintState::Superseded,
            ItemStatus::Stale => HintState::Stale,
            ItemStatus::Deleted => HintState::Deleted,
        }
    }

    /// The clue changes whether an earlier conclusion still holds.
    pub fn is_validity_change(self) -> bool {
        matches!(
            self,
            HintState::ReviewPending
                | HintState::MayBeCorrected
                | HintState::Superseded
                | HintState::Stale
                | HintState::Deleted
                | HintState::Disputed
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    pub layer: HintLayer,
    /// `item_x@3` for a cognition, `<sid>:<seq>` for a perception.
    pub reference: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
    /// Semantic kind of a cognition, suggested kind of a perception.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_intent: Option<String>,
    pub state: HintState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub score: f64,
    /// Why it surfaced (matched handles: scope, tag, full text, read set…).
    #[serde(default)]
    pub reasons: Vec<String>,
    /// Original events behind it.
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Unconsolidated corrections that may affect this cognition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cites: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<Disposition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_when: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    /// End of validity (cognition) or of the deferral window (perception).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default)]
    pub expand_recommended: bool,
}

impl Hint {
    pub fn from_item(
        view: &GraphView,
        item: &MemoryItem,
        reasons: Vec<String>,
        score: f64,
        grants: &Grants,
    ) -> Self {
        Hint {
            layer: HintLayer::Cognition,
            reference: item.reference(),
            id: item.item_id.clone(),
            revision: Some(item.revision),
            summary: item.statement(),
            scope: item.scope.clone(),
            basis: item.basis,
            kind: item
                .semantic_kind
                .clone()
                .or_else(|| Some(item.kind.to_string())),
            explicit: item.explicit,
            memory_intent: None,
            state: HintState::of_item(item.status),
            weight: Some(item.weight),
            confidence: Some(item.confidence),
            score,
            reasons,
            sources: view.item_source_events(item, Some(grants)),
            observed_at: Some(item.noticed_at.clone()),
            session_id: None,
            corrections: Vec::new(),
            cites: Vec::new(),
            disposition: None,
            review_when: item.review_when.clone(),
            replaced_by: item.replaced_by.clone(),
            valid_until: item.valid_until.clone(),
            expand_recommended: !item.review_when.is_empty() || item.status == ItemStatus::Disputed,
        }
    }

    pub fn from_perception(
        r: &PerceptionRecord,
        disposition: Option<&DispositionState>,
        reasons: Vec<String>,
    ) -> Self {
        let deadline = disposition.and_then(|d| d.deadline_cap.clone());
        let disposed = disposition.map(|d| d.disposition.clone());
        let state = match (&r.cleared, &disposed) {
            (Some(_), _) => HintState::Disposed,
            (None, Some(d)) if d.outcome == DispositionOutcome::Deferred => HintState::Deferred,
            (None, Some(_)) => HintState::Disposed,
            (None, None) => HintState::Pending,
        };
        let disposition = disposed.or_else(|| {
            r.cleared.as_ref().map(|c| Disposition {
                perception_ref: r.reference(),
                outcome: c.outcome.parse().unwrap_or(DispositionOutcome::Absorbed),
                cognition_refs: c.cognition_refs.clone(),
                reason: None,
                reevaluate_when: Vec::new(),
                deadline: None,
                clarify: None,
            })
        });
        Hint {
            layer: HintLayer::Perception,
            reference: r.reference(),
            id: r.reference(),
            revision: None,
            summary: if r.cleared.is_some() {
                String::new()
            } else {
                r.summary.clone()
            },
            scope: r.scope.clone(),
            basis: None,
            kind: r.suggested_kind.clone().or_else(|| Some(r.kind.clone())),
            explicit: r.memory_intent.as_deref() == Some("explicit"),
            memory_intent: r.memory_intent.clone(),
            state,
            weight: None,
            confidence: None,
            score: 0.0,
            reasons,
            sources: r
                .source_ref
                .as_ref()
                .and_then(|s| s.event_ref.clone().or_else(|| s.uri.clone()))
                .into_iter()
                .collect(),
            observed_at: Some(iso_ms(r.at_ms)),
            session_id: Some(r.session_id.clone()),
            corrections: Vec::new(),
            cites: r.cites.clone(),
            disposition,
            review_when: Vec::new(),
            replaced_by: None,
            valid_until: deadline,
            expand_recommended: false,
        }
    }

    pub fn is_correction(&self) -> bool {
        self.layer == HintLayer::Perception && self.kind.as_deref() == Some("correction")
    }
}

/// Context budget of one delivery, split into pools (M-20): validity
/// changes first, then reserved shares for cognitions and new perceptions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub max_items: usize,
    pub cognitions: usize,
    pub perceptions: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_items: 8,
            cognitions: 3,
            perceptions: 3,
        }
    }
}

impl Budget {
    pub fn zero() -> Self {
        Self {
            max_items: 0,
            cognitions: 0,
            perceptions: 0,
        }
    }

    pub fn of(max_items: usize) -> Self {
        Self {
            max_items,
            cognitions: max_items.div_ceil(3),
            perceptions: max_items.div_ceil(3),
        }
    }

    /// Split items (already ordered inside each pool) into delivered and
    /// overflow. Pools: 0 = validity change, 1 = cognition, 2 = perception,
    /// 3 = other (markers, clarifications).
    pub fn allocate<T>(&self, mut pools: [Vec<T>; 4]) -> (Vec<T>, Vec<T>) {
        let mut out = Vec::new();
        let mut rest = Vec::new();
        let max = self.max_items;
        let take = |v: &mut Vec<T>, n: usize, out: &mut Vec<T>| {
            let n = n.min(v.len());
            out.extend(v.drain(..n));
        };
        take(&mut pools[0], max, &mut out);
        let room = max - out.len();
        let c = self.cognitions.min(pools[1].len()).min(room);
        take(&mut pools[1], c, &mut out);
        let room = max - out.len();
        let p = self.perceptions.min(pools[2].len()).min(room);
        take(&mut pools[2], p, &mut out);
        for pool in pools.iter_mut().skip(1) {
            let room = max - out.len();
            take(pool, room, &mut out);
        }
        for pool in pools.iter_mut() {
            rest.append(pool);
        }
        (out, rest)
    }
}
