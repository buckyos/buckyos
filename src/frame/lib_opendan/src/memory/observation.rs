//! Session-side topic and observation progress (A.9, §6.6). The state lives
//! with the Session; Memory never learns who is watching.
//!
//! - `set_topic`: the model's title and tags over the host's base scope;
//!   tags are reinforced on mention, decay with time and are evicted when
//!   the capacity is reached; a change valve decides deep, light or no
//!   recall.
//! - `observe`: before each inference, pending refs first, then
//!   `changes_since`, split by the budget.
//! - `accept`: only after the material was assembled into the model input;
//!   it moves the snapshot, keeps what did not fit as pending and records
//!   delivered cognitions in the read set.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result as OdResult;
use crate::fsutil;

use super::changes::{Change, ChangeKind};
use super::hint::{Budget, Hint, HintLayer};
use super::recall::{RecallStatus, TopicQuery, TopicResult, TopicScope};
use super::{Caller, Memory, MemoryError, MemoryResult, MemorySnapshot, QueryScope};

/// Evaluation start values (§6.6), all configurable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicConfig {
    pub capacity: usize,
    pub decay_tau_ms: u64,
    /// Tags whose decayed score falls below this are dropped.
    pub min_score: f64,
    pub deep_ratio: f64,
    /// Title token drift (1 - Jaccard) that counts as a new topic.
    pub title_drift: f64,
    pub light_interval_ms: u64,
    pub light_rounds: u32,
    pub pending_cap: usize,
    pub max_changes: usize,
}

impl Default for TopicConfig {
    fn default() -> Self {
        Self {
            capacity: 8,
            decay_tau_ms: 30 * 60 * 1000,
            min_score: 0.05,
            deep_ratio: 0.5,
            title_drift: 0.5,
            light_interval_ms: 30 * 60 * 1000,
            light_rounds: 5,
            pending_cap: 32,
            max_changes: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicTag {
    pub tag: String,
    pub score: f64,
    pub touched_at_ms: u64,
}

impl TopicTag {
    fn effective(&self, now_ms: u64, tau_ms: u64) -> f64 {
        let age = now_ms.saturating_sub(self.touched_at_ms) as f64;
        self.score * (-age / tau_ms.max(1) as f64).exp()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TopicState {
    pub revision: u64,
    pub title: String,
    /// Built deterministically by the creator / host; never evicted.
    pub base_scope: QueryScope,
    /// Added by the model; bounded, decaying.
    pub tags: Vec<TopicTag>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastQuery {
    pub at_ms: u64,
    pub topic_revision: u64,
    pub rounds_since: u32,
    /// Identity + scope + context generation the recall was made for.
    pub key: String,
    pub snapshot: MemorySnapshot,
    pub had_corrections: bool,
    /// Earliest validity end / deferral deadline among what was shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservationState {
    pub enabled: bool,
    pub topic: TopicState,
    /// Last snapshot whose changes were assembled; `None` before the first
    /// accepted recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<MemorySnapshot>,
    /// Detected but not delivered (budget or failed assembly); next first.
    #[serde(default)]
    pub pending: Vec<String>,
    /// item_id → revision delivered (read → watched, M-21).
    #[serde(default)]
    pub read_set: BTreeMap<String, u64>,
    /// reference → state already shown in this topic phase (cleared when
    /// the topic switches, is cleared, or the model context is rebuilt).
    #[serde(default)]
    pub shown: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_query: Option<LastQuery>,
    /// Bumped by fork / compaction / rebuild: cached "unchanged" is void.
    #[serde(default)]
    pub context_generation: u64,
    #[serde(default)]
    pub resync_required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Valve {
    Deep,
    Light,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicUpdate {
    pub changed: bool,
    pub revision: u64,
    pub added: Vec<String>,
    pub evicted: Vec<String>,
    pub valve: Valve,
}

fn title_drift(a: &str, b: &str) -> f64 {
    let ta = agent_tool::agent_memory::fts_tokens(a);
    let tb = agent_tool::agent_memory::fts_tokens(b);
    if ta.is_empty() && tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f64;
    let union = ta.union(&tb).count() as f64;
    1.0 - inter / union.max(1.0)
}

fn state_key(h: &Hint) -> String {
    format!("{:?}@{}", h.state, h.revision.unwrap_or(0))
}

impl ObservationState {
    pub fn new(base_scope: QueryScope) -> Self {
        Self {
            enabled: true,
            topic: TopicState {
                base_scope,
                ..TopicState::default()
            },
            snapshot: None,
            pending: Vec::new(),
            read_set: BTreeMap::new(),
            shown: BTreeMap::new(),
            last_query: None,
            context_generation: 0,
            resync_required: false,
        }
    }

    pub fn tags(&self) -> Vec<String> {
        self.topic.tags.iter().map(|t| t.tag.clone()).collect()
    }

    pub fn query_scope(&self) -> TopicScope {
        TopicScope {
            subjects: self.topic.base_scope.subjects.clone(),
            objects: self.topic.base_scope.objects.clone(),
            tags: self.tags(),
            title: self.topic.title.clone(),
        }
    }

    /// Model-side topic update (§6.6). Same title and tags: idempotent (the
    /// tags are reinforced, the revision stays).
    pub fn set_topic(
        &mut self,
        title: &str,
        tags: &[String],
        now_ms: u64,
        cfg: &TopicConfig,
    ) -> MemoryResult<TopicUpdate> {
        let tags = agent_tool::agent_memory::normalize_tags(tags)?;
        let prev_len = self.topic.tags.len();
        let mut added = Vec::new();
        let mut evicted = Vec::new();
        for t in &tags {
            match self.topic.tags.iter_mut().find(|x| &x.tag == t) {
                Some(x) => {
                    x.score = x.effective(now_ms, cfg.decay_tau_ms) + 1.0;
                    x.touched_at_ms = now_ms;
                }
                None => {
                    self.topic.tags.push(TopicTag {
                        tag: t.clone(),
                        score: 1.0,
                        touched_at_ms: now_ms,
                    });
                    added.push(t.clone());
                }
            }
        }
        let tau = cfg.decay_tau_ms;
        let title = title.trim();
        let drift = title_drift(&self.topic.title, title);
        // A topic switch (the title drifted past the threshold) drops the
        // tags the model did not mention again.
        let switched = !self.topic.title.is_empty() && drift >= cfg.title_drift;
        self.topic.tags.retain(|x| {
            let keep =
                x.effective(now_ms, tau) >= cfg.min_score && (!switched || tags.contains(&x.tag));
            if !keep {
                evicted.push(x.tag.clone());
            }
            keep
        });
        while self.topic.tags.len() > cfg.capacity {
            let (idx, _) = self
                .topic
                .tags
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    a.1.effective(now_ms, tau)
                        .partial_cmp(&b.1.effective(now_ms, tau))
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.1.touched_at_ms.cmp(&b.1.touched_at_ms))
                })
                .expect("non-empty");
            evicted.push(self.topic.tags.remove(idx).tag);
        }
        added.retain(|t| !evicted.contains(t));
        let title_changed = self.topic.title != title;
        let changed = title_changed || !added.is_empty() || !evicted.is_empty();
        if changed {
            self.topic.revision += 1;
            self.topic.title = title.to_string();
        }
        // A new topic phase (re-showing is allowed) starts on a switch, not
        // on a refinement of the same topic.
        if drift >= cfg.title_drift {
            self.shown.clear();
        }
        let ratio = (added.len() + evicted.len()) as f64 / prev_len.max(1) as f64;
        let valve = match &self.last_query {
            _ if self.resync_required => Valve::Deep,
            None => Valve::Deep,
            Some(_) if ratio >= cfg.deep_ratio || drift >= cfg.title_drift => Valve::Deep,
            Some(q)
                if now_ms.saturating_sub(q.at_ms) >= cfg.light_interval_ms
                    || q.rounds_since >= cfg.light_rounds =>
            {
                Valve::Light
            }
            Some(_) => Valve::None,
        };
        Ok(TopicUpdate {
            changed,
            revision: self.topic.revision,
            added,
            evicted,
            valve,
        })
    }

    pub fn clear_topic(&mut self) {
        if !self.topic.title.is_empty() || !self.topic.tags.is_empty() {
            self.topic.title.clear();
            self.topic.tags.clear();
            self.topic.revision += 1;
            self.shown.clear();
        }
    }

    /// Fork, compaction or rebuild of the model context.
    pub fn bump_context_generation(&mut self) {
        self.context_generation += 1;
        self.shown.clear();
    }

    fn cache_key(&self, caller: &Caller) -> String {
        let grants: Vec<&str> = caller.grants.iter().map(String::as_str).collect();
        crate::ids::h(&[
            &grants.join(","),
            &self.query_scope().digest(),
            &self.context_generation.to_string(),
        ])
    }

    /// Accept an assembled topic recall. The first recall sets the snapshot;
    /// later ones keep the older snapshot so read-set revisions in between
    /// still arrive through `observe` (duplicates are deduped by `shown`).
    pub fn accept_recall(&mut self, r: &TopicResult, caller: &Caller, now_ms: u64) {
        if self.snapshot.is_none() || self.resync_required {
            self.snapshot = Some(r.snapshot.clone());
            self.resync_required = false;
        }
        let mut had_corrections = false;
        let mut expires: Option<String> = None;
        for h in r.hints() {
            self.mark_shown(h);
            had_corrections |= !h.corrections.is_empty() || h.is_correction();
            if let Some(v) = &h.valid_until {
                if expires.as_ref().is_none_or(|e| v < e) {
                    expires = Some(v.clone());
                }
            }
        }
        self.merge_pending(r.omitted.iter().cloned());
        self.last_query = Some(LastQuery {
            at_ms: now_ms,
            topic_revision: self.topic.revision,
            rounds_since: 0,
            key: self.cache_key(caller),
            snapshot: r.snapshot.clone(),
            had_corrections,
            expires_at: expires,
        });
    }

    fn mark_shown(&mut self, h: &Hint) {
        self.shown.insert(h.reference.clone(), state_key(h));
        if h.layer == HintLayer::Cognition {
            if let Some(rev) = h.revision {
                let e = self.read_set.entry(h.id.clone()).or_insert(rev);
                *e = (*e).max(rev);
            }
        }
    }

    fn merge_pending(&mut self, refs: impl Iterator<Item = String>) {
        for r in refs {
            if !self.pending.contains(&r) {
                self.pending.push(r);
            }
        }
    }

    /// Accept an assembled observation (only after successful assembly).
    pub fn accept(&mut self, d: &Delivery, cfg: &TopicConfig) {
        if d.disabled {
            return;
        }
        if d.resync_required {
            // The next set_topic rebuilds through a deep recall and adopts
            // its snapshot.
            self.resync_required = true;
            self.pending.clear();
            return;
        }
        if let Some(s) = &d.snapshot {
            self.snapshot = Some(s.clone());
        }
        self.pending.clear();
        self.merge_pending(d.overflow.iter().cloned());
        if self.pending.len() > cfg.pending_cap {
            self.pending.clear();
            self.resync_required = true;
        }
        for c in &d.changes {
            self.mark_shown(&c.hint);
        }
        if let Some(q) = &mut self.last_query {
            q.rounds_since += 1;
        }
    }

    fn already_shown(&self, c: &Change) -> bool {
        if self.shown.get(&c.hint.reference) == Some(&state_key(&c.hint)) {
            return true;
        }
        if c.hint.layer == HintLayer::Cognition && !c.hint.state.is_validity_change() {
            if let (Some(rev), Some(read)) = (c.hint.revision, self.read_set.get(&c.hint.id)) {
                return rev <= *read && !matches!(c.kind, ChangeKind::ReadRevision { .. });
            }
        }
        false
    }

    pub fn save(&self, path: &Path) -> OdResult<()> {
        fsutil::atomic_replace_json(path, self)
    }

    pub fn load(path: &Path) -> OdResult<Option<Self>> {
        fsutil::read_json_opt(path)
    }
}

/// One observation before assembly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub changes: Vec<Change>,
    /// Refs that did not fit the budget.
    pub overflow: Vec<String>,
    pub snapshot: Option<MemorySnapshot>,
    pub resync_required: bool,
    pub disabled: bool,
    pub fast_path: bool,
}

/// Pull what changed for this Session (A.9 `observe`). Pending refs come
/// first; the Session's own perceptions never come back to it.
pub fn observe(
    memory: &Memory,
    caller: &Caller,
    obs: &ObservationState,
    budget: &Budget,
    cfg: &TopicConfig,
) -> MemoryResult<Delivery> {
    if !obs.enabled {
        return Ok(Delivery {
            disabled: true,
            ..Delivery::default()
        });
    }
    let Some(since) = &obs.snapshot else {
        return Ok(Delivery {
            resync_required: true,
            ..Delivery::default()
        });
    };
    if obs.resync_required {
        return Ok(Delivery {
            resync_required: true,
            ..Delivery::default()
        });
    }
    let topic = obs.query_scope();
    // Perceptions already shown are watched for their disposition (the marker
    // must arrive even when cleanup removed what made them match the topic).
    let mut watched = obs.read_set.clone();
    for r in obs.shown.keys().filter(|r| !r.starts_with("item_")) {
        watched.insert(r.clone(), 0);
    }
    let set = memory.changes_since(caller, since, &topic, &watched, cfg.max_changes)?;
    if set.resync_required {
        return Ok(Delivery {
            resync_required: true,
            snapshot: Some(set.snapshot),
            ..Delivery::default()
        });
    }
    let mut all: Vec<Change> = Vec::new();
    if !obs.pending.is_empty() {
        let ctx = memory.change_ctx(caller, &topic)?;
        for r in &obs.pending {
            if let Some(c) = memory.resolve_pending(&ctx, caller, &topic, &obs.read_set, r) {
                all.push(c);
            }
        }
    }
    all.extend(set.changes);
    let mut seen = std::collections::BTreeSet::new();
    let mut pools: [Vec<Change>; 4] = Default::default();
    for c in all {
        if obs.already_shown(&c) || !seen.insert(c.key()) {
            continue;
        }
        pools[c.pool()].push(c);
    }
    let (delivered, rest) = budget.allocate(pools);
    Ok(Delivery {
        overflow: rest.iter().map(Change::pending_ref).collect(),
        changes: delivered,
        snapshot: Some(set.snapshot),
        resync_required: false,
        disabled: false,
        fast_path: set.fast_path && obs.pending.is_empty(),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TopicOutcome {
    pub update: TopicUpdate,
    /// The recall to assemble (`None` when the valve stayed closed).
    pub result: Option<TopicResult>,
}

/// Session-side `set_topic` (A.9): update the topic, then recall through
/// `query_topic` when the valve opens. Hints already shown in this topic
/// phase are not repeated; an unchanged light recall answers `Unchanged`.
pub fn set_topic(
    memory: &Memory,
    caller: &Caller,
    obs: &mut ObservationState,
    title: &str,
    tags: &[String],
    budget: &Budget,
    cfg: &TopicConfig,
) -> MemoryResult<TopicOutcome> {
    let now = memory.now_ms();
    let update = obs.set_topic(title, tags, now, cfg)?;
    let result = match update.valve {
        Valve::None => None,
        valve => {
            if valve == Valve::Light && unchanged(memory, caller, obs)? {
                Some(TopicResult {
                    status: RecallStatus::Unchanged,
                    cognitions: Vec::new(),
                    perceptions: Vec::new(),
                    clarifications: Vec::new(),
                    omitted: Vec::new(),
                    truncated: false,
                    snapshot: memory.snapshot()?,
                    ambiguous_aliases: Vec::new(),
                    pending_in_scope: 0,
                    last_consolidated_at: None,
                    index: None,
                })
            } else {
                let mut r = memory
                    .query_topic(caller, &TopicQuery::new(obs.query_scope(), budget.clone()))?;
                let fresh = |h: &Hint| obs.shown.get(&h.reference) != Some(&state_key(h));
                r.cognitions.retain(fresh);
                r.perceptions.retain(fresh);
                r.clarifications.retain(fresh);
                Some(r)
            }
        }
    };
    Ok(TopicOutcome { update, result })
}

/// Same identity, scope, versions and context generation as the last
/// accepted recall, nothing expired and no unconsolidated correction shown:
/// the light recall can answer "unchanged" (§6.4).
pub fn unchanged(memory: &Memory, caller: &Caller, obs: &ObservationState) -> MemoryResult<bool> {
    let Some(q) = &obs.last_query else {
        return Ok(false);
    };
    if q.key != obs.cache_key(caller) || q.had_corrections || q.topic_revision != obs.topic.revision
    {
        return Ok(false);
    }
    if let Some(exp) = &q.expires_at {
        if memory.now()
            >= chrono::DateTime::parse_from_rfc3339(exp)
                .map_err(|e| MemoryError::Corrupted(e.to_string()))?
        {
            return Ok(false);
        }
    }
    Ok(memory.snapshot()? == q.snapshot)
}
