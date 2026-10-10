//! Cognition recall over a replayed Graph (A.6.1).
//!
//! Hard filters first (visibility, range, status, validity, evidence), then
//! candidates from object / pair / directed relation hops, scope objects,
//! tags and full text; the complete score is kept for the caller.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::*;
use super::state::GraphView;
use super::text::{
    collapse_whitespace, fts_enters, fts_hits, fts_tokens, phrase_hit, truncate_at_char_boundary,
};

pub const DEFAULT_MAX_RECORDS: usize = 50;
pub const DEFAULT_MAX_BYTES: usize = 65536;
pub const DEFAULT_BODY_TRUNCATE_BYTES: usize = 4096;
const MAX_EXPANSION: usize = 32;

/// Time fade of recall ranking (M-20). Explicit cognitions never fade.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FadeConfig {
    /// One point of penalty per this many days without a revision.
    pub days_per_point: f64,
    pub max_penalty: f64,
}

impl Default for FadeConfig {
    fn default() -> Self {
        Self {
            days_per_point: 30.0,
            max_penalty: 20.0,
        }
    }
}

impl FadeConfig {
    pub fn penalty(&self, item: &MemoryItem, now: DateTime<Utc>) -> f64 {
        if item.explicit {
            return 0.0;
        }
        let Ok(dt) = DateTime::parse_from_rfc3339(&item.noticed_at) else {
            return self.max_penalty;
        };
        let days = (now - dt.with_timezone(&Utc)).num_seconds().max(0) as f64 / 86_400.0;
        (days / self.days_per_point.max(f64::EPSILON)).min(self.max_penalty)
    }
}

#[derive(Clone, Debug)]
pub struct RecallQuery {
    pub tags: Vec<String>,
    /// Free text (topic title, question); tokenized for full text.
    pub text: Option<String>,
    /// Object ids (`obj_…`).
    pub objects: Vec<String>,
    pub aliases: Vec<String>,
    /// Range filter, and scope objects as structural entries.
    pub scope: Option<QueryScope>,
    /// Visibility (A.3.7). `None` = unrestricted administrative read.
    pub grants: Option<Grants>,
    pub max_records: usize,
    pub max_bytes: usize,
    pub body_truncate_bytes: usize,
    pub now: Option<DateTime<Utc>>,
    /// Directed relation hops to expand from the query objects (0..=2).
    pub expand_hops: u8,
    pub fade: FadeConfig,
}

impl Default for RecallQuery {
    fn default() -> Self {
        Self {
            tags: Vec::new(),
            text: None,
            objects: Vec::new(),
            aliases: Vec::new(),
            scope: None,
            grants: None,
            max_records: DEFAULT_MAX_RECORDS,
            max_bytes: DEFAULT_MAX_BYTES,
            body_truncate_bytes: DEFAULT_BODY_TRUNCATE_BYTES,
            now: None,
            expand_hops: 2,
            fade: FadeConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScoreParts {
    pub structural: f64,
    pub tag: f64,
    pub fulltext: f64,
    pub weight: f64,
    pub confidence: f64,
    pub fade: f64,
}

impl ScoreParts {
    pub fn total(&self) -> f64 {
        self.structural + self.tag + self.fulltext + self.weight + self.confidence - self.fade
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallItem {
    pub item_id: String,
    pub revision: u64,
    pub kind: ItemKind,
    pub semantic_kind: Option<String>,
    pub entities: Vec<String>,
    pub scope: Option<Scope>,
    pub basis: Option<Basis>,
    pub explicit: bool,
    pub status: ItemStatus,
    pub weight: f64,
    pub confidence: f64,
    pub score: f64,
    pub score_parts: ScoreParts,
    pub matched: Vec<String>,
    pub source_occasion: String,
    pub noticed_at: String,
    pub evidence: Vec<String>,
    pub valid_until: Option<String>,
    pub review_when: Vec<String>,
    pub content: String,
    pub size: usize,
    pub truncated: bool,
    pub expand_recommended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasCandidate {
    pub object_id: String,
    pub canonical_name: String,
    pub kind: ObjectKind,
    /// Why the object is a candidate (alias hit and its evidence).
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguousAlias {
    pub alias: String,
    pub candidates: Vec<AliasCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexUse {
    /// SQLite FTS narrowed the full-text candidates.
    Fts,
    /// Full text scanned in memory; the reason the index was not used.
    Scan(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallResult {
    pub items: Vec<RecallItem>,
    pub ambiguous_aliases: Vec<AmbiguousAlias>,
    pub ignored_tags: Vec<String>,
    /// Candidates beyond the record / byte budget were dropped.
    pub truncated: bool,
    pub total_candidates: usize,
    pub index: IndexUse,
    pub graph_seq: u64,
}

/// Recall distinguishes "nothing to look for" from "looked, found nothing"
/// (TD-10); failures are errors, never an empty list (TD-01).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RecallOutcome {
    NotTriggered,
    Recalled(RecallResult),
}

impl RecallOutcome {
    pub fn items(&self) -> &[RecallItem] {
        match self {
            RecallOutcome::NotTriggered => &[],
            RecallOutcome::Recalled(r) => &r.items,
        }
    }
}

/// Text an item can be found by: claim, tags, semantic kind and the content
/// of its evidence observations.
pub fn item_search_text(view: &GraphView, item: &MemoryItem) -> (String, String) {
    let own = collapse_whitespace(&format!(
        "{} {} {}",
        item.statement(),
        item.tags.join(" "),
        item.semantic_kind.clone().unwrap_or_default()
    ))
    .to_lowercase();
    let mut obs = String::new();
    for e in &item.evidence {
        if let Some(o) = view.observation(e) {
            obs.push_str(&o.content);
            obs.push(' ');
        }
    }
    (own, collapse_whitespace(&obs).to_lowercase())
}

impl GraphView {
    /// Objects referenced by visible items or observations (objects carry
    /// no scope of their own, A.3.7).
    pub fn object_visible(&self, object_id: &str, grants: Option<&Grants>) -> bool {
        let Some(grants) = grants else {
            return true;
        };
        self.items().any(|i| {
            i.visible_to(grants)
                && i.entities
                    .iter()
                    .any(|e| self.resolve_object(e) == object_id)
        }) || self.observations().any(|o| {
            o.scope
                .as_ref()
                .map(|s| s.visible_to(grants))
                .unwrap_or(true)
                && o.entities
                    .iter()
                    .any(|e| self.resolve_object(e) == object_id)
        })
    }

    /// Resolve aliases to visible objects; several visible hits are returned
    /// as an ambiguity instead of being picked (A.6.3).
    pub fn resolve_aliases_visible(
        &self,
        aliases: &[String],
        grants: Option<&Grants>,
    ) -> (BTreeSet<String>, Vec<AmbiguousAlias>) {
        let mut objects = BTreeSet::new();
        let mut ambiguous = Vec::new();
        for alias in aliases {
            let hits: Vec<String> = self
                .resolve_alias(alias)
                .into_iter()
                .filter(|id| self.object_visible(id, grants))
                .collect();
            match hits.len() {
                0 => {}
                1 => {
                    objects.insert(hits[0].clone());
                }
                _ => ambiguous.push(AmbiguousAlias {
                    alias: alias.clone(),
                    candidates: hits
                        .iter()
                        .filter_map(|id| self.object(id))
                        .map(|o| AliasCandidate {
                            object_id: o.object_id.clone(),
                            canonical_name: o.canonical_name.clone(),
                            kind: o.kind,
                            evidence: o
                                .aliases
                                .iter()
                                .filter(|a| {
                                    super::text::normalize_alias(&a.alias)
                                        == super::text::normalize_alias(alias)
                                })
                                .flat_map(|a| a.evidence.clone())
                                .collect(),
                        })
                        .collect(),
                }),
            }
        }
        (objects, ambiguous)
    }

    /// Objects reached through active relations, following their direction
    /// (subject → object), `hops` deep.
    fn expand(
        &self,
        from: &BTreeSet<String>,
        hops: u8,
        now: DateTime<Utc>,
        grants: Option<&Grants>,
    ) -> Vec<(String, u8, String)> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = from.iter().cloned().collect();
        let mut frontier: Vec<String> = from.iter().cloned().collect();
        for hop in 1..=hops.min(2) {
            let mut next = Vec::new();
            for item in self.items() {
                if item.kind != ItemKind::Relation
                    || !item.is_current(now)
                    || grants.is_some_and(|g| !item.visible_to(g))
                {
                    continue;
                }
                let s = item
                    .claim
                    .get("subject")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let o = item
                    .claim
                    .get("object")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let (s, o) = (self.resolve_object(s), self.resolve_object(o));
                if frontier.contains(&s) && seen.insert(o.clone()) {
                    out.push((o.clone(), hop, item.item_id.clone()));
                    next.push(o);
                    if out.len() >= MAX_EXPANSION {
                        return out;
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        out
    }

    /// Evidence still usable: none deleted or superseded.
    pub fn evidence_usable(&self, item: &MemoryItem) -> bool {
        item.evidence.iter().all(|id| {
            self.observation(id).is_some_and(|o| {
                matches!(
                    o.status,
                    ObservationStatus::Active | ObservationStatus::Disputed
                )
            })
        })
    }

    /// Hard filters of A.6.1 step 2.
    pub fn passes_filters(
        &self,
        item: &MemoryItem,
        grants: Option<&Grants>,
        scope: Option<&QueryScope>,
        now: DateTime<Utc>,
    ) -> bool {
        if !item.is_current(now) || !self.evidence_usable(item) {
            return false;
        }
        if let Some(g) = grants {
            if !item.visible_to(g) {
                return false;
            }
        }
        match (scope, &item.scope) {
            (Some(q), Some(s)) => s.matches(q),
            _ => true,
        }
    }

    /// The recall pipeline. `fts` narrows full-text scoring to the refs the
    /// FTS index matched (`None` = scan every candidate).
    pub fn recall(
        &self,
        q: &RecallQuery,
        tags: &[String],
        ignored_tags: Vec<String>,
        fts: Option<&HashSet<String>>,
        index: IndexUse,
    ) -> RecallOutcome {
        let now = q.now.unwrap_or_else(Utc::now);
        let grants = q.grants.as_ref();
        let mut query_objects: BTreeSet<String> = q
            .objects
            .iter()
            .map(|o| self.resolve_object(o))
            .filter(|o| self.object_visible(o, grants))
            .collect();
        let (alias_objects, ambiguous) = self.resolve_aliases_visible(&q.aliases, grants);
        query_objects.extend(alias_objects);
        let scope_objects = q
            .scope
            .as_ref()
            .map(|s| !s.objects.is_empty())
            .unwrap_or(false);
        let mut text = q.text.clone().unwrap_or_default();
        for t in tags {
            text.push(' ');
            text.push_str(t);
        }
        let query_tokens = fts_tokens(&text);
        let triggered = !query_objects.is_empty()
            || !q.objects.is_empty()
            || !q.aliases.is_empty()
            || !tags.is_empty()
            || !query_tokens.is_empty()
            || scope_objects;
        if !triggered {
            return RecallOutcome::NotTriggered;
        }

        let expansion = self.expand(&query_objects, q.expand_hops, now, grants);
        let mut ranked: Vec<(ScoreParts, RecallItemDraft)> = Vec::new();
        for item in self.items() {
            if !self.passes_filters(item, grants, q.scope.as_ref(), now) {
                continue;
            }
            let mut parts = ScoreParts::default();
            let mut matched = Vec::new();
            let entities: Vec<String> = item
                .entities
                .iter()
                .map(|e| self.resolve_object(e))
                .collect();
            let hits: Vec<&String> = entities
                .iter()
                .filter(|e| query_objects.contains(*e))
                .collect();
            if !hits.is_empty() {
                parts.structural += 12.0;
                matched.extend(hits.iter().map(|e| format!("entity:{e}")));
            }
            if item.kind == ItemKind::Relation {
                let s = self.resolve_object(
                    item.claim
                        .get("subject")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                let o = self.resolve_object(
                    item.claim
                        .get("object")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                if query_objects.contains(&s) && query_objects.contains(&o) {
                    parts.structural += 18.0;
                    matched.push(format!("pair:{s}:{o}"));
                }
            }
            if hits.is_empty() {
                if let Some((obj, hop, via)) =
                    expansion.iter().find(|(obj, _, _)| entities.contains(obj))
                {
                    parts.structural += if *hop == 1 { 6.0 } else { 3.0 };
                    matched.push(format!("hop{hop}:{obj}:via:{via}"));
                }
            }
            if let (Some(qs), Some(s)) = (q.scope.as_ref(), item.scope.as_ref()) {
                let scope_hits = s.object_hits(qs);
                if !scope_hits.is_empty() {
                    parts.structural += 12.0;
                    matched.extend(scope_hits.iter().map(|o| format!("scope:{o}")));
                }
            }
            let (own_text, obs_text) = item_search_text(self, item);
            for (idx, tag) in tags.iter().enumerate() {
                if item.tags.iter().any(|t| t == tag)
                    || phrase_hit(&own_text, tag)
                    || phrase_hit(&obs_text, tag)
                {
                    parts.tag += match idx {
                        0 => 8.0,
                        1 => 4.0,
                        2 => 2.0,
                        _ => 1.0,
                    };
                    matched.push(format!("tag:{tag}"));
                }
            }
            let fts_allowed = fts.map_or(true, |set| {
                set.contains(&item.item_id) || item.evidence.iter().any(|e| set.contains(e))
            });
            if !query_tokens.is_empty() && fts_allowed {
                let own_hits = fts_hits(&query_tokens, &fts_tokens(&own_text));
                let obs_hits = fts_hits(&query_tokens, &fts_tokens(&obs_text));
                let all: BTreeSet<&String> = own_hits.iter().chain(obs_hits.iter()).collect();
                if fts_enters(query_tokens.len(), all.len()) {
                    parts.fulltext += 2.0 * all.len().min(5) as f64;
                    if !own_hits.is_empty() {
                        matched.push("fts:item".to_string());
                    }
                    if !obs_hits.is_empty() {
                        matched.push("fts:observation".to_string());
                    }
                }
            }
            if parts.structural == 0.0 && parts.tag == 0.0 && parts.fulltext == 0.0 {
                continue;
            }
            parts.weight = item.weight * 10.0;
            parts.confidence = item.confidence * 6.0;
            parts.fade = q.fade.penalty(item, now);
            ranked.push((parts, RecallItemDraft { item, matched }));
        }
        ranked.sort_by(|a, b| {
            b.0.total()
                .partial_cmp(&a.0.total())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.1.item
                        .weight
                        .partial_cmp(&a.1.item.weight)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    b.1.item
                        .confidence
                        .partial_cmp(&a.1.item.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.1.item.item_id.cmp(&b.1.item.item_id))
        });

        let total = ranked.len();
        let mut items = Vec::new();
        let mut bytes = 0usize;
        for (parts, draft) in ranked {
            if items.len() >= q.max_records {
                break;
            }
            let item = draft.item;
            let (content, truncated) =
                truncate_at_char_boundary(&item.statement(), q.body_truncate_bytes);
            let size = content.len();
            if bytes.saturating_add(size) > q.max_bytes && !items.is_empty() {
                break;
            }
            bytes += size;
            items.push(RecallItem {
                item_id: item.item_id.clone(),
                revision: item.revision,
                kind: item.kind,
                semantic_kind: item.semantic_kind.clone(),
                entities: item.entities.clone(),
                scope: item.scope.clone(),
                basis: item.basis,
                explicit: item.explicit,
                status: item.status,
                weight: item.weight,
                confidence: item.confidence,
                score: parts.total(),
                score_parts: parts,
                matched: draft.matched,
                source_occasion: item.source_occasion.clone(),
                noticed_at: item.noticed_at.clone(),
                evidence: item.evidence.clone(),
                valid_until: item.valid_until.clone(),
                review_when: item.review_when.clone(),
                content,
                size,
                truncated,
                expand_recommended: truncated
                    || !item.review_when.is_empty()
                    || item.status == ItemStatus::Disputed,
            });
        }
        RecallOutcome::Recalled(RecallResult {
            truncated: items.len() < total,
            items,
            ambiguous_aliases: ambiguous,
            ignored_tags,
            total_candidates: total,
            index,
            graph_seq: self.seq(),
        })
    }

    /// Full-text documents for the FTS index: `(ref_id, ref_type, tokens)`.
    pub fn fts_documents(&self) -> Vec<(String, &'static str, String)> {
        let mut out = Vec::new();
        for item in self.items() {
            if !item.status.recallable() || item.kind == ItemKind::Salience {
                continue;
            }
            let (own, _) = item_search_text(self, item);
            out.push((item.item_id.clone(), "item", join_tokens(&own)));
        }
        for obs in self.observations() {
            if obs.status == ObservationStatus::Deleted {
                continue;
            }
            out.push((
                obs.observation_id.clone(),
                "observation",
                join_tokens(&obs.content),
            ));
        }
        out
    }
}

fn join_tokens(s: &str) -> String {
    fts_tokens(s).into_iter().collect::<Vec<_>>().join(" ")
}

struct RecallItemDraft<'a> {
    item: &'a MemoryItem,
    matched: Vec<String>,
}

/// Group recall items by semantic kind for subject views (§6.3.3).
pub fn group_by_semantic_kind(items: &[RecallItem]) -> BTreeMap<String, Vec<RecallItem>> {
    let mut out: BTreeMap<String, Vec<RecallItem>> = BTreeMap::new();
    for i in items {
        out.entry(
            i.semantic_kind
                .clone()
                .unwrap_or_else(|| i.kind.to_string()),
        )
        .or_default()
        .push(i.clone());
    }
    out
}
