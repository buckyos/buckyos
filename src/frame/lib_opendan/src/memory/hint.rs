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
    /// `stable | contextual | volatile` (cognitions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<String>,
    #[serde(default)]
    pub expand_recommended: bool,
    /// The object it is about, as a URL `read` follows (TODO A4): its first object URL, else the URL it was
    /// observed from. The hint is not the object; read gets the object as it is now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_url: Option<String>,
}

/// The first object of `scope` named by a URL.
pub fn scope_object_url(scope: Option<&Scope>) -> Option<String> {
    scope?.objects.iter().find_map(|o| buckyos_api::readable_object_url(o))
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
            stability: Some(item.stability.as_str().to_string()),
            expand_recommended: !item.review_when.is_empty() || item.status == ItemStatus::Disputed,
            object_url: scope_object_url(item.scope.as_ref()),
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
            stability: None,
            expand_recommended: false,
            object_url: r
                .objects
                .iter()
                .find_map(|o| buckyos_api::readable_object_url(o))
                .or_else(|| r.source_ref.as_ref().and_then(|s| s.uri.as_deref()).and_then(buckyos_api::readable_object_url)),
        }
    }

    pub fn is_correction(&self) -> bool {
        self.layer == HintLayer::Perception && self.kind.as_deref() == Some("correction")
    }

    /// What its scope limits it to: the workspaces among its subjects and
    /// its scope objects.
    pub fn applies_to(&self) -> Vec<String> {
        self.scope
            .as_ref()
            .map(|s| {
                s.subjects
                    .iter()
                    .filter(|x| x.starts_with("workspace:"))
                    .chain(s.objects.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What a reader must know to not take the clue for more than it is:
    /// the nature of its evidence, its range and its end. Empty when the
    /// cognition carries none of them.
    pub fn qualifier(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match (self.basis.map(|b| b.as_str()), self.explicit) {
            (Some("user_statement"), true) => parts.push("the user's explicit statement".into()),
            (Some("user_statement"), false) => parts.push("stated by the user".into()),
            (Some("tool_observation"), _) => parts.push("observed".into()),
            (Some("third_party_claim"), _) => {
                parts.push("a third party's claim, not verified".into())
            }
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
            .applies_to()
            .into_iter()
            .filter(|a| Some(a.as_str()) != self.object_url.as_deref())
            .collect();
        if !applies.is_empty() {
            parts.push(format!("applies to {}", list(&applies)));
        }
        let except = self
            .scope
            .as_ref()
            .map(|s| s.exceptions.clone())
            .unwrap_or_default();
        if !except.is_empty() {
            parts.push(format!("not to {}", list(&except)));
        }
        match (&self.valid_until, self.stability.as_deref()) {
            (Some(t), _) if self.layer == HintLayer::Cognition => {
                parts.push(format!("holds until {t}"))
            }
            (None, Some("volatile")) => parts.push("short-lived".into()),
            _ => {}
        }
        parts.join("; ")
    }

    /// The parenthesis after the statement of a cognition line: qualifier,
    /// when it was noticed, the object it is about and its reference.
    pub fn note(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let q = self.qualifier();
        if !q.is_empty() {
            parts.push(q);
        }
        if let Some(t) = &self.observed_at {
            parts.push(format!("noticed {t}"));
        }
        if let Some(o) = &self.object_url {
            parts.push(format!("about {o}"));
        }
        parts.push(format!("ref {}", self.reference));
        parts.join("; ")
    }
}

/// Evidence below this is said to be weak next to the clue.
const WEAK_CONFIDENCE: f64 = 0.5;
const QUALIFIER_ENTRIES: usize = 3;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(basis: Basis) -> Hint {
        Hint {
            layer: HintLayer::Cognition,
            reference: "item_a@1".into(),
            id: "item_a".into(),
            revision: Some(1),
            summary: "s".into(),
            scope: None,
            basis: Some(basis),
            kind: None,
            explicit: false,
            memory_intent: None,
            state: HintState::Active,
            weight: None,
            confidence: Some(0.8),
            score: 0.0,
            reasons: Vec::new(),
            sources: Vec::new(),
            observed_at: Some("2026-11-01T00:00:00Z".into()),
            session_id: None,
            corrections: Vec::new(),
            cites: Vec::new(),
            disposition: None,
            review_when: Vec::new(),
            replaced_by: None,
            valid_until: None,
            stability: Some("contextual".into()),
            expand_recommended: false,
            object_url: None,
        }
    }

    #[test]
    fn an_inference_is_never_shown_as_the_users_words() {
        assert_eq!(hint(Basis::Inference).qualifier(), "the agent's inference, not confirmed");
        assert_eq!(hint(Basis::UserStatement).qualifier(), "stated by the user");
        let mut h = hint(Basis::UserStatement);
        h.explicit = true;
        assert_eq!(h.qualifier(), "the user's explicit statement");
        assert_eq!(
            hint(Basis::ThirdPartyClaim).qualifier(),
            "a third party's claim, not verified"
        );
        let mut h = hint(Basis::ToolObservation);
        h.confidence = Some(0.4);
        assert_eq!(h.qualifier(), "observed; weak evidence");
    }

    #[test]
    fn range_and_end_are_part_of_the_clue() {
        let mut h = hint(Basis::ToolObservation);
        h.scope = Some(Scope {
            subjects: vec!["did:bns:alice".into(), "workspace:proj-a".into()],
            objects: vec!["tool-x/v1".into()],
            exceptions: vec!["tool-x/v2".into()],
        });
        assert_eq!(
            h.qualifier(),
            "observed; applies to workspace:proj-a, tool-x/v1; not to tool-x/v2"
        );
        // A short-lived clue says when it ends, or at least that it does.
        let mut h = hint(Basis::UserStatement);
        h.stability = Some("volatile".into());
        assert_eq!(h.qualifier(), "stated by the user; short-lived");
        h.valid_until = Some("2026-11-13T23:59:59+08:00".into());
        assert_eq!(
            h.qualifier(),
            "stated by the user; holds until 2026-11-13T23:59:59+08:00"
        );
        // The object the clue is about is named once; its reference ends it.
        let mut h = hint(Basis::ToolObservation);
        h.object_url = Some("obj://a/b".into());
        h.scope = Some(Scope {
            subjects: vec!["did:bns:alice".into()],
            objects: vec!["obj://a/b".into()],
            exceptions: Vec::new(),
        });
        assert_eq!(
            h.note(),
            "observed; noticed 2026-11-01T00:00:00Z; about obj://a/b; ref item_a@1"
        );
        // Nothing known about its evidence: nothing claimed.
        let mut bare = hint(Basis::Inference);
        bare.basis = None;
        bare.confidence = None;
        bare.stability = None;
        assert_eq!(bare.qualifier(), "");
        assert_eq!(bare.note(), "noticed 2026-11-01T00:00:00Z; ref item_a@1");
    }
}
