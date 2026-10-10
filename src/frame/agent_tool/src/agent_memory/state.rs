//! Replayed Graph state and the application of occasions.
//!
//! Replay (`strict = false`) applies what the log says; a commit
//! (`strict = true`) applies the same code with the validation of A.3–A.4
//! on a copy of the state, so nothing reaches the log unless it applies.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::model::*;
use super::text::{normalize_alias, normalize_tags};
use super::{AgentMemoryError, Result};

pub const DEFAULT_FREE_WEIGHT: f64 = 0.5;
pub const DEFAULT_FREE_CONFIDENCE: f64 = 0.5;
pub const MAX_SEGMENT_BYTES: usize = 200;

/// Latest disposition of a perception (`<sid>:<seq>`).
#[derive(Clone, Debug, PartialEq)]
pub struct DispositionState {
    pub disposition: Disposition,
    pub occasion_id: String,
    pub seq: u64,
    pub at: String,
    /// Earliest deadline the material ever got; a later deferral cannot
    /// extend the observation window (§5.9).
    pub deadline_cap: Option<String>,
}

/// One replayed view of the Graph. Immutable once built; callers keep an
/// `Arc` and read it without locks.
#[derive(Clone, Debug, Default)]
pub struct GraphView {
    pub(super) occasions: Vec<MemoryOccasion>,
    pub(super) occasion_index: HashMap<String, usize>,
    pub(super) objects: BTreeMap<String, MemoryObject>,
    pub(super) observations: BTreeMap<String, MemoryObservation>,
    pub(super) items: BTreeMap<String, MemoryItem>,
    pub(super) history: BTreeMap<String, Vec<MemoryItem>>,
    pub(super) changed_seq: BTreeMap<String, u64>,
    pub(super) free_keys: BTreeMap<String, String>,
    pub(super) relation_keys: BTreeMap<String, String>,
    pub(super) dispositions: BTreeMap<String, DispositionState>,
    pub(super) idempotency: BTreeMap<String, String>,
    pub(super) max_seq: u64,
    pub(super) max_lease_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectOutcome {
    Created,
    Updated,
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedAlias {
    pub alias: String,
    pub other_objects: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectResult {
    pub object_id: String,
    pub outcome: ObjectOutcome,
    /// Aliases this object now shares with other active objects; the other
    /// objects keep them (conflicts are reported, never overwritten).
    pub shared_aliases: Vec<SharedAlias>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemChange {
    pub item_id: String,
    pub revision: u64,
    pub status: ItemStatus,
    pub created: bool,
}

/// What one occasion changed, with the real generated ids (TD-17).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub objects: Vec<ObjectResult>,
    pub observations: Vec<String>,
    pub items: Vec<ItemChange>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ApplyCtx {
    /// Validate as a new commit (A.3–A.4), not as a replay.
    pub strict: bool,
    /// A consolidation commit: cognitions need evidence, scope and basis;
    /// evidence observations need a traceable source.
    pub consolidation: bool,
}

impl ApplyCtx {
    pub const REPLAY: ApplyCtx = ApplyCtx {
        strict: false,
        consolidation: false,
    };
}

fn invalid(msg: impl Into<String>) -> AgentMemoryError {
    AgentMemoryError::Invalid(msg.into())
}

fn conflict(msg: impl Into<String>) -> AgentMemoryError {
    AgentMemoryError::Conflict(msg.into())
}

pub fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

pub fn validate_probability(value: f64, name: &str) -> Result<()> {
    if value.is_nan() || !(0.0..=1.0).contains(&value) {
        return Err(invalid(format!("{name} must be in 0.0..=1.0")));
    }
    Ok(())
}

/// Ids become path segments of derived indexes: prefix plus a closed
/// charset (TD-11).
pub fn validate_id(value: &str, prefix: &str, name: &str) -> Result<()> {
    if !value.starts_with(prefix) || value.len() <= prefix.len() || value.len() > MAX_SEGMENT_BYTES
    {
        return Err(invalid(format!(
            "{name} must look like `{prefix}…`: {value:?}"
        )));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        || value.contains("..")
    {
        return Err(invalid(format!(
            "{name} may only use ASCII letters, digits and `_ - .`: {value:?}"
        )));
    }
    Ok(())
}

pub fn validate_iso8601(value: &str) -> Result<String> {
    let dt = DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid(format!("invalid RFC 3339 timestamp: {value}")))?;
    Ok(dt
        .with_timezone(&Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

pub fn validate_reason(reason: &str, name: &str) -> Result<()> {
    if reason.trim().is_empty() || reason.contains('\0') {
        return Err(invalid(format!("{name} is empty")));
    }
    Ok(())
}

pub fn validate_predicate(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(invalid(format!(
            "predicate must be lowercase snake_case: {value:?}"
        )));
    }
    Ok(())
}

/// Free-item key: absolute `/`-separated path, no relative or reserved
/// segments.
pub fn normalize_key(raw: &str) -> Result<String> {
    const RESERVED: &[&str] = &[
        ".meta",
        "memory.sqlite",
        "graph",
        "occasion",
        "object",
        "observation",
        "item",
        "index",
    ];
    if !raw.starts_with('/') {
        return Err(invalid(format!("key must start with '/': {raw}")));
    }
    if raw.chars().any(char::is_control) {
        return Err(invalid("key contains control characters"));
    }
    let mut segs = Vec::new();
    for seg in raw.split('/').filter(|s| !s.is_empty()) {
        if seg == "." || seg == ".." {
            return Err(invalid(format!("key has invalid segment '{seg}'")));
        }
        if seg.len() > MAX_SEGMENT_BYTES {
            return Err(invalid(format!(
                "key segment exceeds {MAX_SEGMENT_BYTES} bytes"
            )));
        }
        segs.push(seg);
    }
    if segs.is_empty() {
        return Err(invalid("key has no segments"));
    }
    if RESERVED.contains(&segs[0]) {
        return Err(invalid(format!(
            "key first segment must not be reserved: {}",
            segs[0]
        )));
    }
    Ok(format!("/{}", segs.join("/")))
}

fn validate_meta(meta: &ItemMeta, ctx: ApplyCtx) -> Result<()> {
    if let Some(scope) = &meta.scope {
        scope.validate()?;
    } else if ctx.consolidation {
        return Err(invalid("a consolidated cognition needs a scope (M-34)"));
    }
    if ctx.consolidation && meta.basis.is_none() {
        return Err(invalid("a consolidated cognition needs a basis (M-06)"));
    }
    if let Some(t) = &meta.valid_until {
        validate_iso8601(t)?;
    }
    if let Some(k) = &meta.semantic_kind {
        if k.trim().is_empty() || k.chars().any(char::is_control) {
            return Err(invalid("semantic_kind is empty"));
        }
    }
    normalize_tags(&meta.tags)?;
    Ok(())
}

impl GraphView {
    pub fn seq(&self) -> u64 {
        self.max_seq
    }

    pub fn max_lease_epoch(&self) -> u64 {
        self.max_lease_epoch
    }

    pub fn occasions(&self) -> &[MemoryOccasion] {
        &self.occasions
    }

    pub fn occasion(&self, id: &str) -> Option<&MemoryOccasion> {
        self.occasion_index.get(id).map(|i| &self.occasions[*i])
    }

    /// Occasions committed after `seq`, in order.
    pub fn occasions_after(&self, seq: u64) -> &[MemoryOccasion] {
        let start = self.occasions.partition_point(|o| o.seq <= seq);
        &self.occasions[start..]
    }

    pub fn objects(&self) -> impl Iterator<Item = &MemoryObject> {
        self.objects.values()
    }

    pub fn object(&self, id: &str) -> Option<&MemoryObject> {
        self.objects.get(id)
    }

    pub fn observations(&self) -> impl Iterator<Item = &MemoryObservation> {
        self.observations.values()
    }

    pub fn observation(&self, id: &str) -> Option<&MemoryObservation> {
        self.observations.get(id)
    }

    pub fn items(&self) -> impl Iterator<Item = &MemoryItem> {
        self.items.values()
    }

    pub fn item(&self, id: &str) -> Option<&MemoryItem> {
        self.items.get(id)
    }

    /// A fixed revision; its content never changes.
    pub fn item_at(&self, id: &str, revision: u64) -> Option<&MemoryItem> {
        self.history
            .get(id)
            .and_then(|h| h.iter().find(|i| i.revision == revision))
    }

    pub fn item_history(&self, id: &str) -> &[MemoryItem] {
        self.history.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Seq of the occasion that produced the current revision.
    pub fn item_changed_seq(&self, id: &str) -> u64 {
        self.changed_seq.get(id).copied().unwrap_or(0)
    }

    pub fn free_item(&self, key: &str) -> Option<&MemoryItem> {
        self.free_keys.get(key).and_then(|id| self.items.get(id))
    }

    pub fn free_keys(&self) -> impl Iterator<Item = (&String, &String)> {
        self.free_keys.iter()
    }

    pub fn disposition(&self, perception_ref: &str) -> Option<&DispositionState> {
        self.dispositions.get(perception_ref)
    }

    pub fn dispositions(&self) -> &BTreeMap<String, DispositionState> {
        &self.dispositions
    }

    pub fn occasion_for_key(&self, key: &str) -> Option<&MemoryOccasion> {
        self.idempotency.get(key).and_then(|id| self.occasion(id))
    }

    /// Time of the last consolidation commit.
    pub fn last_consolidation_at(&self) -> Option<&str> {
        self.occasions
            .iter()
            .rev()
            .find(|o| o.occasion_type == "consolidation")
            .map(|o| o.noticed_at.as_str())
    }

    /// Follow `merged_into` to the surviving object.
    pub fn resolve_object(&self, object_id: &str) -> String {
        let mut current = object_id.to_string();
        let mut seen = HashSet::new();
        while seen.insert(current.clone()) {
            match self.objects.get(&current) {
                Some(o) if o.status == ObjectStatus::Merged => match &o.merged_into {
                    Some(next) => current = next.clone(),
                    None => break,
                },
                _ => break,
            }
        }
        current
    }

    /// Objects an alias resolves to (merged objects redirect, A.6.3).
    pub fn resolve_alias(&self, alias: &str) -> Vec<String> {
        let norm = normalize_alias(alias);
        let mut out: Vec<String> = self
            .objects
            .values()
            .filter(|o| matches!(o.status, ObjectStatus::Active | ObjectStatus::Merged))
            .filter(|o| {
                o.aliases.iter().any(|a| {
                    matches!(a.status, AliasStatus::Active | AliasStatus::Merged)
                        && normalize_alias(&a.alias) == norm
                })
            })
            .map(|o| self.resolve_object(&o.object_id))
            .filter(|id| {
                self.objects
                    .get(id)
                    .is_some_and(|o| o.status == ObjectStatus::Active)
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Source events behind an item's evidence; with `grants`, only those of
    /// evidence the reader may see.
    pub fn item_source_events(&self, item: &MemoryItem, grants: Option<&Grants>) -> Vec<String> {
        let mut out = Vec::new();
        for obs in &item.evidence {
            if let Some(ev) = self
                .observations
                .get(obs)
                .filter(|o| grants.is_none_or(|g| o.scope.as_ref().is_none_or(|s| s.visible_to(g))))
                .and_then(|o| o.source_ref.as_ref())
                .and_then(|s| s.event_ref.clone().or_else(|| s.uri.clone()))
            {
                if !out.contains(&ev) {
                    out.push(ev);
                }
            }
        }
        out
    }

    /// Cognitions whose current or past evidence points at a source event,
    /// task or goal run (reverse lookup for discarded tasks and changed
    /// world objects, M-27).
    pub fn items_affected_by(&self, filter: &AffectedFilter) -> Vec<&MemoryItem> {
        let matches_obs = |obs: &MemoryObservation| {
            let s = obs.source_ref.as_ref();
            filter
                .event_ref
                .as_ref()
                .is_some_and(|e| s.and_then(|s| s.event_ref.as_ref()) == Some(e))
                || filter
                    .task_ref
                    .as_ref()
                    .is_some_and(|t| s.and_then(|s| s.task_ref.as_ref()) == Some(t))
                || filter
                    .session_id
                    .as_ref()
                    .is_some_and(|t| s.and_then(|s| s.session_id.as_ref()) == Some(t))
                || filter
                    .object
                    .as_ref()
                    .is_some_and(|o| obs.entities.iter().any(|e| &self.resolve_object(e) == o))
        };
        self.items
            .values()
            .filter(|item| {
                let hist = self.item_history(&item.item_id);
                hist.iter().any(|rev| {
                    rev.evidence
                        .iter()
                        .filter_map(|o| self.observations.get(o))
                        .any(matches_obs)
                        || filter.goal_run.as_ref().is_some_and(|g| {
                            rev.produced_by.as_ref().and_then(|p| p.goal_run.as_ref()) == Some(g)
                        })
                        || filter.object.as_ref().is_some_and(|o| {
                            rev.entities.iter().any(|e| &self.resolve_object(e) == o)
                        })
                })
            })
            .collect()
    }
}

/// Reverse-lookup filter; any field that is set must match.
#[derive(Clone, Debug, Default)]
pub struct AffectedFilter {
    pub event_ref: Option<String>,
    pub task_ref: Option<String>,
    pub session_id: Option<String>,
    pub goal_run: Option<String>,
    pub object: Option<String>,
}

pub(super) fn apply_occasion(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    ctx: ApplyCtx,
) -> Result<ApplyReport> {
    if ctx.strict && occasion.seq <= state.max_seq {
        return Err(conflict(format!(
            "occasion seq {} is not after {}",
            occasion.seq, state.max_seq
        )));
    }
    let mut report = ApplyReport::default();
    for (idx, op) in occasion.operations.iter().enumerate() {
        match op {
            GraphOperation::SetFree(op) => {
                apply_set_free(occasion, state, idx, op, ctx, &mut report)?
            }
            GraphOperation::RemoveFree(op) => {
                apply_remove_free(occasion, state, op, ctx, &mut report)?
            }
            GraphOperation::AddObservation(op) => {
                apply_add_observation(occasion, state, idx, op, ctx, &mut report)?
            }
            GraphOperation::UpsertObject(op) => {
                apply_upsert_object(occasion, state, idx, op, ctx, &mut report)?
            }
            GraphOperation::ReinforceObjectWeight(op) => {
                apply_reinforce_object(occasion, state, idx, op, ctx, &mut report)?
            }
            GraphOperation::UpsertRelation(op) => {
                apply_relation(occasion, state, idx, op, ctx, &mut report)?
            }
            GraphOperation::SetStatus(op) => {
                apply_set_status(occasion, state, op, ctx, &mut report)?
            }
            GraphOperation::PutItem(op) => {
                apply_put_item(occasion, state, idx, op, ctx, &mut report)?
            }
        }
    }
    if ctx.strict {
        validate_dispositions(occasion, state)?;
    }
    for d in &occasion.dispositions {
        let cap = match (state.dispositions.get(&d.perception_ref), &d.deadline) {
            (Some(prev), Some(new)) => Some(match &prev.deadline_cap {
                Some(old) if old <= new => old.clone(),
                _ => new.clone(),
            }),
            (Some(prev), None) => prev.deadline_cap.clone(),
            (None, dl) => dl.clone(),
        };
        state.dispositions.insert(
            d.perception_ref.clone(),
            DispositionState {
                disposition: d.clone(),
                occasion_id: occasion.occasion_id.clone(),
                seq: occasion.seq,
                at: occasion.noticed_at.clone(),
                deadline_cap: cap,
            },
        );
    }
    state.max_seq = state.max_seq.max(occasion.seq);
    if let Some(e) = occasion.lease_epoch {
        state.max_lease_epoch = state.max_lease_epoch.max(e);
    }
    if let Some(k) = &occasion.idempotency_key {
        state
            .idempotency
            .insert(k.clone(), occasion.occasion_id.clone());
    }
    state
        .occasion_index
        .insert(occasion.occasion_id.clone(), state.occasions.len());
    state.occasions.push(occasion.clone());
    Ok(report)
}

/// A.4.1 checks: terminal materials cannot be disposed again; a deferral
/// needs reason and window and cannot extend an earlier window; every
/// cognition reference must exist.
fn validate_dispositions(occasion: &MemoryOccasion, state: &GraphView) -> Result<()> {
    let mut seen = HashSet::new();
    for d in &occasion.dispositions {
        if !seen.insert(d.perception_ref.as_str()) {
            return Err(invalid(format!("{} is disposed twice", d.perception_ref)));
        }
        validate_perception_ref(&d.perception_ref)?;
        if let Some(prev) = state.dispositions.get(&d.perception_ref) {
            if prev.disposition.outcome.is_terminal() {
                return Err(conflict(format!(
                    "{} is already {} by {}",
                    d.perception_ref, prev.disposition.outcome, prev.occasion_id
                )));
            }
            if d.outcome == DispositionOutcome::Deferred {
                if let (Some(cap), Some(new)) = (&prev.deadline_cap, &d.deadline) {
                    if new > cap {
                        return Err(conflict(format!(
                            "{} cannot extend its deferral window beyond {cap}",
                            d.perception_ref
                        )));
                    }
                }
            }
        }
        match d.outcome {
            DispositionOutcome::Absorbed | DispositionOutcome::Duplicate => {
                if d.cognition_refs.is_empty() && d.outcome == DispositionOutcome::Absorbed {
                    return Err(invalid(format!(
                        "absorbed {} needs the cognition that carries it",
                        d.perception_ref
                    )));
                }
                if d.outcome == DispositionOutcome::Duplicate
                    && d.cognition_refs.is_empty()
                    && d.reason.as_deref().unwrap_or("").trim().is_empty()
                {
                    return Err(invalid(format!(
                        "duplicate {} needs a cognition reference or a reason",
                        d.perception_ref
                    )));
                }
            }
            DispositionOutcome::Discarded => {
                validate_reason(d.reason.as_deref().unwrap_or(""), "discarded reason")?;
            }
            DispositionOutcome::Deferred => {
                validate_reason(d.reason.as_deref().unwrap_or(""), "deferred reason")?;
                if d.reevaluate_when.is_empty() {
                    return Err(invalid(format!(
                        "deferred {} needs reevaluate_when",
                        d.perception_ref
                    )));
                }
                let dl = d.deadline.as_deref().ok_or_else(|| {
                    invalid(format!("deferred {} needs a deadline", d.perception_ref))
                })?;
                validate_iso8601(dl)?;
            }
        }
        for c in &d.cognition_refs {
            let id = c.split('@').next().unwrap_or(c);
            if !state.items.contains_key(id) {
                return Err(invalid(format!(
                    "{} refers to unknown cognition {c}",
                    d.perception_ref
                )));
            }
        }
    }
    Ok(())
}

pub fn validate_perception_ref(r: &str) -> Result<()> {
    match r.rsplit_once(':') {
        Some((sid, seq)) if !sid.is_empty() && seq.parse::<u64>().is_ok() => Ok(()),
        _ => Err(invalid(format!(
            "perception_ref must be <sid>:<seq>: {r:?}"
        ))),
    }
}

fn store_item(state: &mut GraphView, item: MemoryItem, seq: u64) {
    state
        .history
        .entry(item.item_id.clone())
        .or_default()
        .push(item.clone());
    state.changed_seq.insert(item.item_id.clone(), seq);
    state.items.insert(item.item_id.clone(), item);
}

fn check_expected(item: &MemoryItem, expected: Option<u64>, ctx: ApplyCtx) -> Result<()> {
    if !ctx.strict {
        return Ok(());
    }
    match expected {
        None => Err(conflict(format!(
            "{} exists at revision {}; pass expected_revision to revise it",
            item.item_id, item.revision
        ))),
        Some(r) if r != item.revision => Err(conflict(format!(
            "{} is at revision {}, not {r}; re-read and merge",
            item.item_id, item.revision
        ))),
        _ => Ok(()),
    }
}

fn check_evidence(state: &GraphView, evidence: &[String], ctx: ApplyCtx, what: &str) -> Result<()> {
    if !ctx.strict {
        return Ok(());
    }
    if ctx.consolidation && evidence.is_empty() {
        return Err(invalid(format!(
            "{what} needs evidence observations (M-16)"
        )));
    }
    for id in evidence {
        match state.observations.get(id) {
            Some(o) if o.status != ObservationStatus::Deleted => {}
            Some(_) => return Err(invalid(format!("{what}: evidence {id} is deleted"))),
            None => return Err(invalid(format!("{what}: unknown evidence {id}"))),
        }
    }
    Ok(())
}

fn check_entities(state: &GraphView, entities: &[String], ctx: ApplyCtx) -> Result<()> {
    for e in entities {
        validate_id(e, "obj_", "entity")?;
        if ctx.strict && !state.objects.contains_key(e) {
            return Err(invalid(format!("unknown entity {e}")));
        }
    }
    Ok(())
}

/// Supersede `replaced` items in favour of `by` (each gets a status revision).
fn supersede(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    replaced: &[String],
    by: &str,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    for id in replaced {
        if id == by {
            return Err(invalid(format!("{id} cannot replace itself")));
        }
        let Some(cur) = state.items.get(id).cloned() else {
            if ctx.strict {
                return Err(invalid(format!("replaces unknown item {id}")));
            }
            continue;
        };
        if ctx.strict && !cur.status.can_become(ItemStatus::Superseded, false) {
            return Err(conflict(format!(
                "{id} is {}; it cannot be superseded",
                cur.status
            )));
        }
        let mut next = cur;
        next.revision += 1;
        next.status = ItemStatus::Superseded;
        next.replaced_by = Some(by.to_string());
        next.source_occasion = occasion.occasion_id.clone();
        next.noticed_at = occasion.noticed_at.clone();
        if let Some(key) = &next.free_key {
            if state.free_keys.get(key) == Some(id) {
                state.free_keys.remove(key);
            }
        }
        report.items.push(ItemChange {
            item_id: id.clone(),
            revision: next.revision,
            status: next.status,
            created: false,
        });
        store_item(state, next, occasion.seq);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_item(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    item_id: String,
    kind: ItemKind,
    entities: Vec<String>,
    claim: Value,
    weight: f64,
    confidence: f64,
    evidence: Vec<String>,
    write_reason: String,
    replaces: Vec<String>,
    expected_revision: Option<u64>,
    meta: &ItemMeta,
    free_key: Option<String>,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    let tags = normalize_tags(&meta.tags)?;
    let existing = state.items.get(&item_id).cloned();
    let (revision, first_occasion) = match &existing {
        Some(cur) => {
            if ctx.strict && cur.status == ItemStatus::Deleted {
                return Err(conflict(format!("{item_id} is deleted; it is not revived")));
            }
            check_expected(cur, expected_revision, ctx)?;
            (cur.revision + 1, cur.first_occasion.clone())
        }
        None => {
            if ctx.strict && expected_revision.is_some_and(|r| r != 0) {
                return Err(conflict(format!(
                    "{item_id} does not exist (expected revision {})",
                    expected_revision.unwrap_or(0)
                )));
            }
            (1, occasion.occasion_id.clone())
        }
    };
    supersede(occasion, state, &replaces, &item_id, ctx, report)?;
    let item = MemoryItem {
        item_id: item_id.clone(),
        revision,
        kind,
        semantic_kind: meta.semantic_kind.clone(),
        entities,
        claim,
        scope: meta.scope.as_ref().map(Scope::normalized),
        basis: meta.basis,
        explicit: meta.explicit,
        weight: clamp01(weight),
        confidence: clamp01(confidence),
        evidence,
        source_occasion: occasion.occasion_id.clone(),
        first_occasion,
        noticed_at: occasion.noticed_at.clone(),
        valid_until: meta.valid_until.clone(),
        review_when: meta.review_when.clone(),
        produced_by: occasion.produced_by.clone(),
        write_reason,
        status: ItemStatus::Active,
        replaces,
        replaced_by: None,
        free_key,
        tags,
    };
    report.items.push(ItemChange {
        item_id: item_id.clone(),
        revision,
        status: ItemStatus::Active,
        created: existing.is_none(),
    });
    store_item(state, item, occasion.seq);
    Ok(())
}

fn gen_id(prefix: &str, seq: u64, idx: usize) -> String {
    format!("{prefix}{seq:016}_{idx:02}")
}

fn apply_set_free(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    idx: usize,
    op: &FlatSetOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    let key = normalize_key(&op.key)?;
    if ctx.strict {
        if op.content.trim().is_empty() {
            return Err(invalid("free content is empty"));
        }
        validate_reason(&op.reason, "reason")?;
        validate_probability(op.weight.unwrap_or(DEFAULT_FREE_WEIGHT), "weight")?;
        validate_probability(
            op.confidence.unwrap_or(DEFAULT_FREE_CONFIDENCE),
            "confidence",
        )?;
        validate_meta(&op.meta, ctx)?;
        check_entities(state, &op.entities, ctx)?;
        check_evidence(state, &op.evidence, ctx, "free item")?;
    }
    let (item_id, expected) = match state.free_keys.get(&key) {
        Some(id) => {
            // Free keys overwrite in admin use; a consolidation revises with
            // an explicit expected revision like any other cognition.
            let cur = state.items.get(id).map(|i| i.revision);
            let expected = if ctx.consolidation {
                op.expected_revision
            } else {
                op.expected_revision.or(cur)
            };
            (id.clone(), expected)
        }
        None => (gen_id("item_", occasion.seq, idx), None),
    };
    write_item(
        occasion,
        state,
        item_id.clone(),
        ItemKind::Free,
        op.entities.clone(),
        json!({ "type": "free", "key": key, "statement": op.content }),
        op.weight.unwrap_or(DEFAULT_FREE_WEIGHT),
        op.confidence.unwrap_or(DEFAULT_FREE_CONFIDENCE),
        op.evidence.clone(),
        op.reason.clone(),
        Vec::new(),
        expected,
        &op.meta,
        Some(key.clone()),
        ctx,
        report,
    )?;
    state.free_keys.insert(key, item_id);
    Ok(())
}

fn apply_remove_free(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    op: &FlatRemoveOp,
    _ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    let key = normalize_key(&op.key)?;
    if let Some(item_id) = state.free_keys.remove(&key) {
        if let Some(cur) = state.items.get(&item_id).cloned() {
            if cur.status != ItemStatus::Deleted {
                let mut next = cur;
                next.revision += 1;
                next.status = ItemStatus::Deleted;
                next.source_occasion = occasion.occasion_id.clone();
                next.noticed_at = occasion.noticed_at.clone();
                report.items.push(ItemChange {
                    item_id: item_id.clone(),
                    revision: next.revision,
                    status: next.status,
                    created: false,
                });
                store_item(state, next, occasion.seq);
            }
        }
    }
    Ok(())
}

fn apply_add_observation(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    idx: usize,
    op: &AddObservationOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    let observation_id = op
        .observation_id
        .clone()
        .unwrap_or_else(|| gen_id("obs_", occasion.seq, idx));
    validate_id(&observation_id, "obs_", "observation_id")?;
    if ctx.strict {
        if state.observations.contains_key(&observation_id) {
            return Err(conflict(format!(
                "observation {observation_id} already exists"
            )));
        }
        if op.content.trim().is_empty() {
            return Err(invalid("observation content is empty"));
        }
        validate_probability(op.confidence, "confidence")?;
        check_entities(state, &op.entities, ctx)?;
        if let Some(s) = &op.source_ref {
            s.validate()?;
        }
        if ctx.consolidation && !op.source_ref.as_ref().is_some_and(SourceRef::is_traceable) {
            return Err(invalid(format!(
                "evidence observation {observation_id} needs source_ref.event_ref or uri (M-16)"
            )));
        }
        if let Some(scope) = &op.scope {
            scope.validate()?;
        }
        if let Some(t) = &op.occurred_at {
            validate_iso8601(t)?;
        }
    }
    state.observations.insert(
        observation_id.clone(),
        MemoryObservation {
            observation_id: observation_id.clone(),
            kind: op.kind,
            source_occasion: occasion.occasion_id.clone(),
            entities: op.entities.clone(),
            content: op.content.clone(),
            source_excerpt: op.source_excerpt.clone(),
            source_ref: op.source_ref.clone(),
            occurred_at: op.occurred_at.clone(),
            scope: op.scope.as_ref().map(Scope::normalized),
            confidence: clamp01(op.confidence),
            noticed_at: occasion.noticed_at.clone(),
            status: ObservationStatus::Active,
        },
    );
    report.observations.push(observation_id);
    Ok(())
}

fn apply_upsert_object(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    idx: usize,
    op: &UpsertObjectOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    if ctx.strict {
        if op.canonical_name.trim().is_empty() {
            return Err(invalid("canonical_name is empty"));
        }
        validate_probability(op.confidence, "confidence")?;
        if let Some(w) = op.weight {
            validate_probability(w, "weight")?;
        }
        for a in &op.aliases {
            if normalize_alias(&a.alias).is_empty() {
                return Err(invalid("alias is empty"));
            }
            validate_probability(a.confidence, "alias confidence")?;
        }
        if !op.aliases.is_empty() && op.evidence.is_empty() {
            return Err(invalid("new aliases need evidence"));
        }
        check_evidence(
            state,
            &op.evidence,
            ApplyCtx {
                consolidation: false,
                ..ctx
            },
            "object",
        )?;
    }

    if let Some(target) = &op.merge_into {
        let object_id = op
            .object_id
            .clone()
            .ok_or_else(|| invalid("merge_into needs the object_id being merged"))?;
        if ctx.strict {
            match state.objects.get(&object_id) {
                Some(o) if o.status.can_become(ObjectStatus::Merged) => {}
                Some(o) => {
                    return Err(conflict(format!(
                        "{object_id} is {}; cannot merge",
                        o.status
                    )))
                }
                None => return Err(invalid(format!("unknown object {object_id}"))),
            }
            match state.objects.get(target) {
                Some(o) if o.status == ObjectStatus::Active && target != &object_id => {}
                _ => {
                    return Err(invalid(format!(
                        "merge target {target} is not an active object"
                    )))
                }
            }
        }
        if let Some(object) = state.objects.get_mut(&object_id) {
            object.status = ObjectStatus::Merged;
            object.merged_into = Some(target.clone());
            object.last_occasion = occasion.occasion_id.clone();
            object.noticed_at = occasion.noticed_at.clone();
            for alias in &mut object.aliases {
                if alias.status == AliasStatus::Active {
                    alias.status = AliasStatus::Merged;
                    alias.noticed_at = occasion.noticed_at.clone();
                }
            }
            merge_unique(&mut object.evidence, &op.evidence);
        }
        report.objects.push(ObjectResult {
            object_id,
            outcome: ObjectOutcome::Merged,
            shared_aliases: Vec::new(),
        });
        return Ok(());
    }

    let alias_norms: Vec<String> = op
        .aliases
        .iter()
        .map(|a| normalize_alias(&a.alias))
        .collect();
    let object_id = match &op.object_id {
        Some(id) => {
            validate_id(id, "obj_", "object_id")?;
            id.clone()
        }
        None => {
            // TD-17: match an existing object through its aliases.
            let mut hits: Vec<String> = alias_norms
                .iter()
                .flat_map(|a| state.resolve_alias(a))
                .collect();
            hits.sort();
            hits.dedup();
            match hits.len() {
                0 => gen_id("obj_", occasion.seq, idx),
                1 => hits.remove(0),
                _ => {
                    return Err(AgentMemoryError::Ambiguous {
                        alias: op
                            .aliases
                            .first()
                            .map(|a| a.alias.clone())
                            .unwrap_or_default(),
                        candidates: hits,
                    })
                }
            }
        }
    };
    if ctx.strict {
        if let Some(o) = state.objects.get(&object_id) {
            if o.status != ObjectStatus::Active {
                return Err(conflict(format!(
                    "{object_id} is {}; it is not updated",
                    o.status
                )));
            }
        }
    }

    let mut shared = Vec::new();
    for (a, norm) in op.aliases.iter().zip(&alias_norms) {
        let others: Vec<String> = state
            .objects
            .values()
            .filter(|o| o.object_id != object_id && o.status == ObjectStatus::Active)
            .filter(|o| {
                o.aliases
                    .iter()
                    .any(|x| x.status == AliasStatus::Active && &normalize_alias(&x.alias) == norm)
            })
            .map(|o| o.object_id.clone())
            .collect();
        if !others.is_empty() {
            shared.push(SharedAlias {
                alias: a.alias.clone(),
                other_objects: others,
            });
        }
    }

    let created = !state.objects.contains_key(&object_id);
    let object = state
        .objects
        .entry(object_id.clone())
        .or_insert_with(|| MemoryObject {
            object_id: object_id.clone(),
            kind: op.kind,
            canonical_name: op.canonical_name.clone(),
            aliases: Vec::new(),
            weight: clamp01(op.weight.unwrap_or(DEFAULT_FREE_WEIGHT)),
            confidence: clamp01(op.confidence),
            evidence: Vec::new(),
            source_occasion: occasion.occasion_id.clone(),
            last_occasion: occasion.occasion_id.clone(),
            noticed_at: occasion.noticed_at.clone(),
            status: ObjectStatus::Active,
            merged_into: None,
        });
    object.kind = op.kind;
    object.canonical_name = op.canonical_name.clone();
    object.weight = clamp01(op.weight.unwrap_or(object.weight));
    object.confidence = object.confidence.max(clamp01(op.confidence));
    object.last_occasion = occasion.occasion_id.clone();
    object.noticed_at = occasion.noticed_at.clone();
    merge_unique(&mut object.evidence, &op.evidence);
    for (a, norm) in op.aliases.iter().zip(&alias_norms) {
        if let Some(existing) = object
            .aliases
            .iter_mut()
            .find(|x| &normalize_alias(&x.alias) == norm)
        {
            existing.confidence = existing.confidence.max(clamp01(a.confidence));
            existing.noticed_at = occasion.noticed_at.clone();
            merge_unique(&mut existing.evidence, &op.evidence);
        } else {
            object.aliases.push(ObjectAlias {
                alias: a.alias.clone(),
                alias_type: a.alias_type,
                confidence: clamp01(a.confidence),
                evidence: op.evidence.clone(),
                source_occasion: occasion.occasion_id.clone(),
                noticed_at: occasion.noticed_at.clone(),
                status: AliasStatus::Active,
            });
        }
    }
    report.objects.push(ObjectResult {
        object_id,
        outcome: if created {
            ObjectOutcome::Created
        } else {
            ObjectOutcome::Updated
        },
        shared_aliases: shared,
    });
    Ok(())
}

fn apply_reinforce_object(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    idx: usize,
    op: &ReinforceObjectWeightOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    if ctx.strict {
        validate_id(&op.object_id, "obj_", "object_id")?;
        validate_reason(&op.reason, "reason")?;
        if op.delta.is_nan() || !(-1.0..=1.0).contains(&op.delta) {
            return Err(invalid("delta must be in -1.0..=1.0"));
        }
        check_evidence(
            state,
            &op.evidence,
            ApplyCtx {
                consolidation: false,
                ..ctx
            },
            "reinforce",
        )?;
    }
    let object = state
        .objects
        .get_mut(&op.object_id)
        .ok_or_else(|| AgentMemoryError::NotFound(op.object_id.clone()))?;
    object.weight = clamp01(object.weight + op.delta);
    object.last_occasion = occasion.occasion_id.clone();
    object.noticed_at = occasion.noticed_at.clone();
    let (weight, confidence) = (object.weight, object.confidence);
    let item_id = gen_id("item_", occasion.seq, idx);
    let item = MemoryItem {
        item_id: item_id.clone(),
        revision: 1,
        kind: ItemKind::Salience,
        semantic_kind: None,
        entities: vec![op.object_id.clone()],
        claim: json!({
            "type": "salience",
            "object_id": op.object_id,
            "reason": op.reason,
            "delta": op.delta,
        }),
        scope: None,
        basis: None,
        explicit: false,
        weight,
        confidence,
        evidence: op.evidence.clone(),
        source_occasion: occasion.occasion_id.clone(),
        first_occasion: occasion.occasion_id.clone(),
        noticed_at: occasion.noticed_at.clone(),
        valid_until: None,
        review_when: Vec::new(),
        produced_by: occasion.produced_by.clone(),
        write_reason: op.reason.clone(),
        status: ItemStatus::Active,
        replaces: Vec::new(),
        replaced_by: None,
        free_key: None,
        tags: Vec::new(),
    };
    report.items.push(ItemChange {
        item_id: item_id.clone(),
        revision: 1,
        status: ItemStatus::Active,
        created: true,
    });
    store_item(state, item, occasion.seq);
    Ok(())
}

pub fn relation_key(subject: &str, predicate: &str, object: &str, scope: Option<&Scope>) -> String {
    format!(
        "{subject}|{predicate}|{object}|{}",
        scope.map(Scope::key).unwrap_or_default()
    )
}

fn apply_relation(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    idx: usize,
    op: &UpsertRelationOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    validate_id(&op.subject, "obj_", "subject")?;
    validate_id(&op.object, "obj_", "object")?;
    validate_predicate(&op.predicate)?;
    if ctx.strict {
        for end in [&op.subject, &op.object] {
            match state.objects.get(end) {
                Some(o) if o.status == ObjectStatus::Active => {}
                _ => {
                    return Err(invalid(format!(
                        "relation end {end} is not an active object"
                    )))
                }
            }
        }
        validate_probability(op.weight, "weight")?;
        validate_probability(op.confidence, "confidence")?;
        validate_reason(&op.write_reason, "write_reason")?;
        validate_meta(&op.meta, ctx)?;
        check_evidence(state, &op.evidence, ctx, "relation")?;
    }
    let key = relation_key(
        &op.subject,
        &op.predicate,
        &op.object,
        op.meta.scope.as_ref(),
    );
    let item_id = match (&op.item_id, state.relation_keys.get(&key)) {
        (Some(id), _) => {
            validate_id(id, "item_", "item_id")?;
            id.clone()
        }
        (None, Some(existing))
            if state.items.get(existing).is_some_and(|i| {
                !matches!(i.status, ItemStatus::Deleted | ItemStatus::Superseded)
            }) =>
        {
            existing.clone()
        }
        (None, _) => gen_id("item_", occasion.seq, idx),
    };
    write_item(
        occasion,
        state,
        item_id.clone(),
        ItemKind::Relation,
        vec![op.subject.clone(), op.object.clone()],
        json!({
            "type": "relation",
            "subject": op.subject,
            "predicate": op.predicate,
            "object": op.object,
        }),
        op.weight,
        op.confidence,
        op.evidence.clone(),
        op.write_reason.clone(),
        op.replaces.clone(),
        op.expected_revision,
        &op.meta,
        None,
        ctx,
        report,
    )?;
    state.relation_keys.insert(key, item_id);
    Ok(())
}

fn apply_put_item(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    idx: usize,
    op: &PutItemOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    if ctx.strict {
        if matches!(op.kind, ItemKind::Salience | ItemKind::Relation) {
            return Err(invalid(format!(
                "{} items are written through their own operation",
                op.kind
            )));
        }
        validate_claim(op.kind, &op.claim)?;
        validate_probability(op.weight, "weight")?;
        validate_probability(op.confidence, "confidence")?;
        validate_reason(&op.write_reason, "write_reason")?;
        validate_meta(&op.meta, ctx)?;
        check_entities(state, &op.entities, ctx)?;
        check_evidence(state, &op.evidence, ctx, "cognition")?;
    }
    let item_id = op
        .item_id
        .clone()
        .unwrap_or_else(|| gen_id("item_", occasion.seq, idx));
    validate_id(&item_id, "item_", "item_id")?;
    write_item(
        occasion,
        state,
        item_id,
        op.kind,
        op.entities.clone(),
        op.claim.clone(),
        op.weight,
        op.confidence,
        op.evidence.clone(),
        op.write_reason.clone(),
        op.replaces.clone(),
        op.expected_revision,
        &op.meta,
        None,
        ctx,
        report,
    )
}

fn apply_set_status(
    occasion: &MemoryOccasion,
    state: &mut GraphView,
    op: &SetStatusOp,
    ctx: ApplyCtx,
    report: &mut ApplyReport,
) -> Result<()> {
    if ctx.strict {
        validate_reason(&op.reason, "reason")?;
    }
    match op.target_kind {
        TargetKind::Item => {
            let status: ItemStatus = op.status.parse()?;
            let cur = state
                .items
                .get(&op.target_id)
                .cloned()
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            if ctx.strict {
                if let Some(r) = op.expected_revision {
                    if r != cur.revision {
                        return Err(conflict(format!(
                            "{} is at revision {}, not {r}",
                            cur.item_id, cur.revision
                        )));
                    }
                } else if ctx.consolidation {
                    check_expected(&cur, None, ctx)?;
                }
                if !cur.status.can_become(status, !op.evidence.is_empty()) {
                    return Err(conflict(format!(
                        "illegal transition {} → {status} for {}",
                        cur.status, cur.item_id
                    )));
                }
                check_evidence(
                    state,
                    &op.evidence,
                    ApplyCtx {
                        consolidation: false,
                        ..ctx
                    },
                    "status change",
                )?;
                if let Some(by) = &op.replaced_by {
                    if !state.items.contains_key(by) {
                        return Err(invalid(format!("replaced_by {by} is unknown")));
                    }
                }
            }
            let mut next = cur;
            next.revision += 1;
            next.status = status;
            next.source_occasion = occasion.occasion_id.clone();
            next.noticed_at = occasion.noticed_at.clone();
            if op.replaced_by.is_some() {
                next.replaced_by = op.replaced_by.clone();
            }
            merge_unique(&mut next.evidence, &op.evidence);
            if let Some(key) = &next.free_key {
                if status == ItemStatus::Active {
                    state.free_keys.insert(key.clone(), next.item_id.clone());
                } else if state.free_keys.get(key) == Some(&next.item_id) {
                    state.free_keys.remove(key);
                }
            }
            report.items.push(ItemChange {
                item_id: next.item_id.clone(),
                revision: next.revision,
                status,
                created: false,
            });
            store_item(state, next, occasion.seq);
        }
        TargetKind::Object => {
            let status: ObjectStatus = op.status.parse()?;
            let replaced_by = op.replaced_by.clone();
            if ctx.strict {
                let cur = state
                    .objects
                    .get(&op.target_id)
                    .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
                if !cur.status.can_become(status) {
                    return Err(conflict(format!(
                        "illegal transition {} → {status} for {}",
                        cur.status, cur.object_id
                    )));
                }
                if status == ObjectStatus::Merged {
                    match replaced_by.as_ref().and_then(|t| state.objects.get(t)) {
                        Some(t) if t.status == ObjectStatus::Active => {}
                        _ => return Err(invalid("merging an object needs an active replaced_by")),
                    }
                }
            }
            let object = state
                .objects
                .get_mut(&op.target_id)
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            object.status = status;
            object.last_occasion = occasion.occasion_id.clone();
            object.noticed_at = occasion.noticed_at.clone();
            if status == ObjectStatus::Merged {
                object.merged_into = replaced_by;
            }
            if matches!(status, ObjectStatus::Merged | ObjectStatus::Deleted) {
                let cascade = if status == ObjectStatus::Merged {
                    AliasStatus::Merged
                } else {
                    AliasStatus::Deleted
                };
                for alias in &mut object.aliases {
                    if matches!(alias.status, AliasStatus::Active | AliasStatus::Deprecated) {
                        alias.status = cascade;
                        alias.noticed_at = occasion.noticed_at.clone();
                    }
                }
            }
        }
        TargetKind::Observation => {
            let status: ObservationStatus = op.status.parse()?;
            let observation = state
                .observations
                .get_mut(&op.target_id)
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            if ctx.strict && !observation.status.can_become(status) {
                return Err(conflict(format!(
                    "illegal transition {} → {status} for {}",
                    observation.status, observation.observation_id
                )));
            }
            observation.status = status;
            observation.noticed_at = occasion.noticed_at.clone();
        }
        TargetKind::Alias => {
            let status: AliasStatus = op.status.parse()?;
            let (object_id, alias_text) = op
                .target_id
                .split_once(':')
                .ok_or_else(|| invalid("alias target must be <object_id>:<alias>"))?;
            let object = state
                .objects
                .get_mut(object_id)
                .ok_or_else(|| AgentMemoryError::NotFound(object_id.to_string()))?;
            let norm = normalize_alias(alias_text);
            let alias = object
                .aliases
                .iter_mut()
                .find(|a| normalize_alias(&a.alias) == norm)
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            if ctx.strict && !alias.status.can_become(status) {
                return Err(conflict(format!(
                    "illegal transition {} → {status} for alias {}",
                    alias.status, op.target_id
                )));
            }
            alias.status = status;
            alias.noticed_at = occasion.noticed_at.clone();
        }
    }
    Ok(())
}

pub fn merge_unique(target: &mut Vec<String>, values: &[String]) {
    for v in values {
        if !target.contains(v) {
            target.push(v.clone());
        }
    }
}
