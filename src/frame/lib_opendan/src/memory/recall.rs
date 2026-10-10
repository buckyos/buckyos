//! Two-layer recall (§6, A.6, A.9): `query_topic` for a Session's topic,
//! `query` for active questions. Cognitions and unconsolidated perceptions
//! come back separately; every recalled cognition carries the unconsolidated
//! corrections of its subject and scope (§4.3), whatever was asked.

use std::collections::{BTreeMap, BTreeSet};

use agent_tool::agent_memory::{
    fts_enters, fts_tokens, item_search_text, phrase_hit, AmbiguousAlias, DispositionState,
    GraphView, IndexUse, MemoryItem, RecallOutcome, RecallQuery,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::protocol::PerceptionRecord;

use super::hint::{Budget, Hint, HintLayer, HintState};
use super::perception::parse_cite;
use super::{
    Caller, DispositionOutcome, Memory, MemoryError, MemoryResult, MemorySnapshot, QueryScope,
    Scope,
};

/// What a Session is attending to: the host's base scope (subjects and
/// objects) plus the topic title and the model's current tags.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicScope {
    pub subjects: Vec<String>,
    pub objects: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub title: String,
}

impl TopicScope {
    pub fn with_title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    pub fn query_scope(&self) -> QueryScope {
        QueryScope {
            subjects: self.subjects.clone(),
            objects: self.objects.clone(),
        }
    }

    fn text(&self) -> String {
        let mut t = self.title.clone();
        for tag in &self.tags {
            t.push(' ');
            t.push_str(tag);
        }
        t
    }

    pub fn digest(&self) -> String {
        crate::ids::h(&[
            &self.subjects.join(","),
            &self.objects.join(","),
            &self.tags.join(","),
            &self.title,
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallStatus {
    /// No entry (objects, aliases, tags, text): nothing was looked up.
    NotTriggered,
    /// Looked up; the lists may be empty (that never means "no such memory").
    Recalled,
    /// Same identity, scope, versions and context: nothing new to show.
    Unchanged,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicQuery {
    pub scope: TopicScope,
    pub cognitions: bool,
    pub perceptions: bool,
    pub budget: Budget,
}

impl TopicQuery {
    pub fn new(scope: TopicScope, budget: Budget) -> Self {
        Self {
            scope,
            cognitions: true,
            perceptions: true,
            budget,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicResult {
    pub status: RecallStatus,
    pub cognitions: Vec<Hint>,
    pub perceptions: Vec<Hint>,
    pub clarifications: Vec<Hint>,
    /// Refs beyond the budget; the Session keeps them as pending.
    pub omitted: Vec<String>,
    pub truncated: bool,
    /// Taken before reading: later writes show up as changes after it.
    pub snapshot: MemorySnapshot,
    pub ambiguous_aliases: Vec<AmbiguousAlias>,
    /// Visible unconsolidated perceptions in range ("noted, not yet
    /// consolidated").
    pub pending_in_scope: usize,
    pub last_consolidated_at: Option<String>,
    pub index: Option<IndexUse>,
}

impl TopicResult {
    pub fn hints(&self) -> impl Iterator<Item = &Hint> {
        self.cognitions
            .iter()
            .chain(self.perceptions.iter())
            .chain(self.clarifications.iter())
    }
}

/// Page position of an active query (one snapshot and clock per paging).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCursor {
    pub snapshot: MemorySnapshot,
    pub offset: usize,
    pub now_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryQuery {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Empty = every principal the caller may read for.
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub objects: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub semantic_kinds: Vec<String>,
    #[serde(default)]
    pub include_recent_perceptions: bool,
    #[serde(default)]
    pub limit: usize,
    /// Group the cognitions by semantic kind (subject view, §6.3.3).
    #[serde(default)]
    pub by_subject: bool,
    #[serde(default)]
    pub cursor: Option<PageCursor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub status: RecallStatus,
    pub cognitions: Vec<Hint>,
    pub recent_perceptions: Vec<Hint>,
    /// Unconsolidated corrections of the returned cognitions: always present.
    pub corrections: Vec<Hint>,
    pub groups: BTreeMap<String, Vec<String>>,
    pub ambiguous_aliases: Vec<AmbiguousAlias>,
    pub snapshot: MemorySnapshot,
    pub pending_in_scope: usize,
    pub last_consolidated_at: Option<String>,
    pub next_cursor: Option<PageCursor>,
    pub index: Option<IndexUse>,
}

/// Pending-ref encoding kept by Sessions: `c:` cognition, `p:` perception,
/// `r:` review of a cognition, `q:` clarification.
pub(super) fn pending_ref(h: &Hint) -> String {
    match h.layer {
        HintLayer::Cognition => format!("c:{}", h.id),
        HintLayer::Perception => format!("p:{}", h.id),
        HintLayer::Clarification => format!("q:{}", h.id),
    }
}

/// Perceptions still waiting for consolidation: not cleared, not terminally
/// disposed, and (for recall) not past an expired deferral.
pub(super) fn is_pending(r: &PerceptionRecord, d: Option<&DispositionState>) -> bool {
    r.cleared.is_none() && d.is_none_or(|d| d.disposition.outcome == DispositionOutcome::Deferred)
}

pub(super) fn deferral_expired(d: Option<&DispositionState>, now: DateTime<Utc>) -> bool {
    d.and_then(|d| d.deadline_cap.as_deref())
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| t.with_timezone(&Utc) <= now)
}

pub(super) fn record_visible(r: &PerceptionRecord, caller: &Caller) -> bool {
    r.scope
        .as_ref()
        .is_none_or(|s| s.visible_to(&caller.grants))
}

/// Why a perception belongs to a topic (range match plus an entry), or None.
pub(super) fn perception_topic_hit(
    r: &PerceptionRecord,
    topic: &TopicScope,
) -> Option<Vec<String>> {
    if r.is_runtime() {
        return None;
    }
    let scope = r.scope.as_ref()?;
    let q = topic.query_scope();
    if !scope.matches(&q) {
        return None;
    }
    let mut reasons: Vec<String> = scope
        .object_hits(&q)
        .into_iter()
        .map(|o| format!("scope:{o}"))
        .collect();
    let content = r.summary.to_lowercase();
    for tag in &topic.tags {
        let t = tag.to_lowercase();
        if r.tags.iter().any(|x| x == &t) || phrase_hit(&content, &t) {
            reasons.push(format!("tag:{t}"));
        }
    }
    let query = fts_tokens(&topic.text());
    if !query.is_empty() && r.cleared.is_none() {
        let hits = query.intersection(&fts_tokens(&r.summary)).count();
        if fts_enters(query.len(), hits) {
            reasons.push("fts:perception".into());
        }
    }
    (!reasons.is_empty()).then_some(reasons)
}

/// Graph objects a topic names: object paths (and their prefixes) resolved
/// through path / DID aliases, visible ones only.
pub(super) fn topic_objects(
    view: &GraphView,
    topic: &TopicScope,
    caller: &Caller,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in &topic.objects {
        let segs: Vec<&str> = path.split('/').collect();
        for n in (1..=segs.len()).rev() {
            for id in view.resolve_alias(&segs[..n].join("/")) {
                if view.object_visible(&id, Some(&caller.grants)) {
                    out.insert(id);
                }
            }
        }
    }
    out
}

/// Why a cognition belongs to a topic, independent of its status (used for
/// change detection, where superseded / deleted states matter too).
pub(super) fn item_topic_hit(
    view: &GraphView,
    item: &MemoryItem,
    topic: &TopicScope,
    objects: &BTreeSet<String>,
    caller: &Caller,
) -> Option<Vec<String>> {
    if !item.visible_to(&caller.grants) || item.kind == agent_tool::agent_memory::ItemKind::Salience
    {
        return None;
    }
    let q = topic.query_scope();
    let mut reasons = Vec::new();
    if let Some(s) = &item.scope {
        if !s.matches(&q) {
            return None;
        }
        reasons.extend(s.object_hits(&q).into_iter().map(|o| format!("scope:{o}")));
    }
    for e in &item.entities {
        let e = view.resolve_object(e);
        if objects.contains(&e) {
            reasons.push(format!("entity:{e}"));
        }
    }
    let (own, obs) = item_search_text(view, item);
    for tag in &topic.tags {
        let t = tag.to_lowercase();
        if item.tags.iter().any(|x| x == &t) || phrase_hit(&own, &t) || phrase_hit(&obs, &t) {
            reasons.push(format!("tag:{t}"));
        }
    }
    let query = fts_tokens(&topic.text());
    if !query.is_empty() {
        let mut doc = fts_tokens(&own);
        doc.extend(fts_tokens(&obs));
        if fts_enters(query.len(), query.intersection(&doc).count()) {
            reasons.push("fts:item".into());
        }
    }
    (!reasons.is_empty()).then_some(reasons)
}

/// Unconsolidated corrections that may affect `item` (same subject and
/// scope), and whether one of them cites it (§4.3).
pub(super) fn corrections_for<'a>(
    item: &MemoryItem,
    pending: &'a [PerceptionRecord],
    caller: &Caller,
) -> (Vec<&'a PerceptionRecord>, bool) {
    let Some(item_scope) = &item.scope else {
        let cited: Vec<&PerceptionRecord> = pending
            .iter()
            .filter(|r| {
                r.suggested_kind.as_deref() == Some("correction") && record_visible(r, caller)
            })
            .filter(|r| cites_item(r, &item.item_id))
            .collect();
        let precise = !cited.is_empty();
        return (cited, precise);
    };
    let mut out = Vec::new();
    let mut precise = false;
    for r in pending {
        if r.suggested_kind.as_deref() != Some("correction") || !record_visible(r, caller) {
            continue;
        }
        let cites = cites_item(r, &item.item_id);
        let same_scope = r.scope.as_ref().is_some_and(|s| s.overlaps(item_scope));
        if cites || same_scope {
            precise |= cites;
            out.push(r);
        }
    }
    (out, precise)
}

pub(super) fn cites_item(r: &PerceptionRecord, item_id: &str) -> bool {
    r.cites
        .iter()
        .any(|c| parse_cite(c).map(|(id, _)| id == item_id).unwrap_or(false))
}

/// Decorate a cognition hint with its correction state.
pub(super) fn overlay_corrections(
    hint: &mut Hint,
    item: &MemoryItem,
    pending: &[PerceptionRecord],
    caller: &Caller,
) {
    let (corr, precise) = corrections_for(item, pending, caller);
    if corr.is_empty() {
        return;
    }
    hint.corrections = corr.iter().map(|r| r.reference()).collect();
    if matches!(hint.state, HintState::Active | HintState::Disputed) {
        hint.state = if precise {
            HintState::ReviewPending
        } else {
            HintState::MayBeCorrected
        };
    }
    hint.expand_recommended = true;
}

pub(super) struct Core {
    pub status: RecallStatus,
    pub cognitions: Vec<Hint>,
    pub perceptions: Vec<Hint>,
    pub corrections: Vec<Hint>,
    pub clarifications: Vec<Hint>,
    pub ambiguous: Vec<AmbiguousAlias>,
    pub pending_in_scope: usize,
    pub last_consolidated_at: Option<String>,
    pub index: Option<IndexUse>,
}

pub(super) struct Spec<'a> {
    pub topic: TopicScope,
    pub aliases: &'a [String],
    pub semantic_kinds: &'a [String],
    pub perceptions: bool,
    pub now: DateTime<Utc>,
}

impl Memory {
    pub(super) fn recall_core(&self, caller: &Caller, spec: &Spec<'_>) -> MemoryResult<Core> {
        let view = self.graph_view()?;
        let records = self.load_perceptions(None)?;
        let pending: Vec<PerceptionRecord> = records
            .into_iter()
            .filter(|r| {
                let d = view.disposition(&r.reference());
                is_pending(r, d) && !deferral_expired(d, spec.now)
            })
            .collect();
        let objects = topic_objects(&view, &spec.topic, caller);
        let q = RecallQuery {
            tags: spec.topic.tags.clone(),
            text: Some(spec.topic.title.clone()).filter(|t| !t.trim().is_empty()),
            objects: objects.iter().cloned().collect(),
            aliases: spec.aliases.to_vec(),
            scope: Some(spec.topic.query_scope()),
            grants: Some(caller.grants.clone()),
            now: Some(spec.now),
            ..RecallQuery::default()
        };
        let outcome = self.graph()?.recall(&q)?;
        let (items, ambiguous, index) = match outcome {
            RecallOutcome::NotTriggered => {
                return Ok(Core {
                    status: RecallStatus::NotTriggered,
                    cognitions: Vec::new(),
                    perceptions: Vec::new(),
                    corrections: Vec::new(),
                    clarifications: Vec::new(),
                    ambiguous: Vec::new(),
                    pending_in_scope: 0,
                    last_consolidated_at: None,
                    index: None,
                })
            }
            RecallOutcome::Recalled(r) => (r.items, r.ambiguous_aliases, r.index),
        };
        let mut cognitions = Vec::new();
        let mut correction_refs = BTreeSet::new();
        for ri in items {
            let Some(item) = view.item(&ri.item_id) else {
                continue;
            };
            if !spec.semantic_kinds.is_empty()
                && !item
                    .semantic_kind
                    .as_ref()
                    .is_some_and(|k| spec.semantic_kinds.contains(k))
            {
                continue;
            }
            let mut h = Hint::from_item(&view, item, ri.matched.clone(), ri.score, &caller.grants);
            overlay_corrections(&mut h, item, &pending, caller);
            correction_refs.extend(h.corrections.iter().cloned());
            cognitions.push(h);
        }
        let corrections: Vec<Hint> = pending
            .iter()
            .filter(|r| correction_refs.contains(&r.reference()))
            .map(|r| {
                let mut h = Hint::from_perception(
                    r,
                    view.disposition(&r.reference()),
                    vec!["correction:same-scope".into()],
                );
                h.state = HintState::Pending;
                h
            })
            .collect();
        let q_scope = spec.topic.query_scope();
        let mut pending_in_scope = 0;
        let mut perceptions = Vec::new();
        let mut clarifications = Vec::new();
        for r in &pending {
            if r.is_runtime() || !record_visible(r, caller) {
                continue;
            }
            if r.scope.as_ref().is_some_and(|s| s.matches(&q_scope)) {
                pending_in_scope += 1;
            }
            let Some(reasons) = perception_topic_hit(r, &spec.topic) else {
                continue;
            };
            let d = view.disposition(&r.reference());
            if let Some(q) = d.and_then(|d| d.disposition.clarify.clone()) {
                let mut h = Hint::from_perception(r, d, reasons.clone());
                h.layer = HintLayer::Clarification;
                h.summary = q;
                clarifications.push(h);
            }
            if spec.perceptions && !correction_refs.contains(&r.reference()) {
                let mut h = Hint::from_perception(r, d, reasons);
                h.score = h.reasons.len() as f64 + r.at_ms as f64 / 1e15;
                perceptions.push(h);
            }
        }
        perceptions.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(Core {
            status: RecallStatus::Recalled,
            cognitions,
            perceptions,
            corrections,
            clarifications,
            ambiguous,
            pending_in_scope,
            last_consolidated_at: view.last_consolidation_at().map(str::to_string),
            index: Some(index),
        })
    }

    /// Initial recall of a topic (A.9): the snapshot is taken before
    /// reading, so anything written meanwhile shows up as a later change
    /// (possibly twice; Sessions dedupe by reference and revision).
    pub fn query_topic(&self, caller: &Caller, q: &TopicQuery) -> MemoryResult<TopicResult> {
        let snapshot = self.snapshot()?;
        let core = self.recall_core(
            caller,
            &Spec {
                topic: q.scope.clone(),
                aliases: &[],
                semantic_kinds: &[],
                perceptions: q.perceptions,
                now: self.now(),
            },
        )?;
        if core.status == RecallStatus::NotTriggered {
            return Ok(TopicResult {
                status: RecallStatus::NotTriggered,
                cognitions: Vec::new(),
                perceptions: Vec::new(),
                clarifications: Vec::new(),
                omitted: Vec::new(),
                truncated: false,
                snapshot,
                ambiguous_aliases: Vec::new(),
                pending_in_scope: 0,
                last_consolidated_at: None,
                index: None,
            });
        }
        let mut validity = Vec::new();
        let mut plain = Vec::new();
        if q.cognitions {
            for h in core.cognitions {
                if h.state.is_validity_change() {
                    validity.push(h);
                } else {
                    plain.push(h);
                }
            }
        }
        validity.extend(core.corrections);
        let (delivered, rest) =
            q.budget
                .allocate([validity, plain, core.perceptions, core.clarifications]);
        let mut result = TopicResult {
            status: RecallStatus::Recalled,
            cognitions: Vec::new(),
            perceptions: Vec::new(),
            clarifications: Vec::new(),
            truncated: !rest.is_empty(),
            omitted: rest.iter().map(pending_ref).collect(),
            snapshot,
            ambiguous_aliases: core.ambiguous,
            pending_in_scope: core.pending_in_scope,
            last_consolidated_at: core.last_consolidated_at,
            index: core.index,
        };
        for h in delivered {
            match h.layer {
                HintLayer::Cognition => result.cognitions.push(h),
                HintLayer::Perception => result.perceptions.push(h),
                HintLayer::Clarification => result.clarifications.push(h),
            }
        }
        Ok(result)
    }

    /// Active query (§6.3.1, §6.3.3). Unconsolidated corrections of the
    /// returned cognitions come back even when recent perceptions are off.
    pub fn query(&self, caller: &Caller, q: &MemoryQuery) -> MemoryResult<QueryResult> {
        let snapshot = self.snapshot()?;
        let now_ms = match &q.cursor {
            Some(c) if c.snapshot != snapshot => {
                return Err(MemoryError::StaleCursor(format!(
                    "cursor from {}, memory now at {}",
                    c.snapshot.token(),
                    snapshot.token()
                )))
            }
            Some(c) => c.now_ms,
            None => self.now_ms(),
        };
        let now = super::iso_ms(now_ms);
        let now = DateTime::parse_from_rfc3339(&now)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| self.now());
        let subjects = if q.subjects.is_empty() {
            caller.grants.iter().cloned().collect()
        } else {
            q.subjects.clone()
        };
        let core = self.recall_core(
            caller,
            &Spec {
                topic: TopicScope {
                    subjects,
                    objects: q.objects.clone(),
                    tags: q.tags.clone(),
                    title: q.text.clone().unwrap_or_default(),
                },
                aliases: &q.aliases,
                semantic_kinds: &q.semantic_kinds,
                perceptions: q.include_recent_perceptions,
                now,
            },
        )?;
        let limit = if q.limit == 0 { 10 } else { q.limit };
        let offset = q.cursor.as_ref().map(|c| c.offset).unwrap_or(0);
        let total = core.cognitions.len();
        let page: Vec<Hint> = core
            .cognitions
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        let next_cursor = (offset + page.len() < total).then(|| PageCursor {
            snapshot: snapshot.clone(),
            offset: offset + page.len(),
            now_ms,
        });
        let page_corrections: BTreeSet<&String> =
            page.iter().flat_map(|h| h.corrections.iter()).collect();
        let corrections: Vec<Hint> = core
            .corrections
            .into_iter()
            .filter(|h| page_corrections.contains(&h.reference))
            .collect();
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if q.by_subject {
            for h in &page {
                groups
                    .entry(h.kind.clone().unwrap_or_else(|| "cognition".into()))
                    .or_default()
                    .push(h.reference.clone());
            }
        }
        Ok(QueryResult {
            status: core.status,
            cognitions: page,
            recent_perceptions: core.perceptions,
            corrections,
            groups,
            ambiguous_aliases: core.ambiguous,
            snapshot,
            pending_in_scope: core.pending_in_scope,
            last_consolidated_at: core.last_consolidated_at,
            next_cursor,
            index: core.index,
        })
    }
}

/// Scopes of two records overlap (used for grouping deferred context).
pub(super) fn scopes_overlap(a: Option<&Scope>, b: Option<&Scope>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.overlaps(b),
        _ => false,
    }
}
