use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::hint_recall::{
    DefaultRecallService, DidObjectRecallProvider, HintRecallEngine, LlmRecallService,
    MemoryRecallProvider, NotebookRecallProvider, RecallHintType, RecallInput, RecallItem,
    RecallPayload, RecallProvider, RecallResult, RecallService, RecallSourceSystem, RecallTarget,
    SessionTopicRecallProvider,
};

const META_DIR: &str = ".meta";
const TOPIC_FILE: &str = "topic.md";
const TOPIC_LOG_FILE: &str = "topic_log.jsonl";
const TAG_SET_FILE: &str = "tag_set.json";
const SUBSCRIPTIONS_FILE: &str = "subscriptions.json";
const TOPIC_DOC_SCHEMA: &str = "opendan.session_topic";
const TOPIC_DOC_VERSION: u32 = 1;
const TAG_ACTIVE_REINFORCE_THRESHOLD: f32 = 3.0;

#[derive(Debug, Error)]
pub enum SessionTopicError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct UpdateSessionTopicInput {
    pub session_id: String,
    pub session_dir: PathBuf,
    pub topic: String,
    pub tags: Vec<TagInput>,
    pub current_turn: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TagInput {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct UpdateSessionTopicResult {
    pub tag_set_diff: TagSetDiff,
    pub recall: Option<RecallPayload>,
    pub recall_status: RecallStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SessionTopicHistoryRecord {
    pub session_id: String,
    pub topic: String,
    pub tags: Vec<String>,
    pub tag_reasons: BTreeMap<String, String>,
    pub current_turn: u32,
    pub updated_at: String,
    pub topic_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TagSetDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub current: Vec<TagEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RecallStatus {
    NotTriggered,
    Mechanical { ms: u32 },
    Llm { ms: u32 },
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Subscription {
    pub id: String,
    pub kind: String,
    pub hint: String,
    #[serde(default)]
    pub bound_tags: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TagSet {
    #[serde(default = "default_tag_capacity")]
    pub capacity: usize,
    #[serde(default)]
    pub tags: Vec<TagEntry>,
    #[serde(default)]
    pub last_recall_turn: Option<u32>,
    #[serde(default)]
    pub last_recall_at: Option<String>,
}

impl Default for TagSet {
    fn default() -> Self {
        Self {
            capacity: default_tag_capacity(),
            tags: Vec::new(),
            last_recall_turn: None,
            last_recall_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TagEntry {
    pub name: String,
    pub weight: f32,
    pub last_touched: String,
    pub tier: TagTier,
    #[serde(default)]
    pub position: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TagTier {
    Pinned,
    Active,
    Transient,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecallMode {
    Auto,
    Mechanical,
    Llm,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecallPolicy {
    pub tag_capacity: usize,
    pub decay_tau_seconds: f64,
    pub distance_threshold_turns: u32,
    pub change_threshold: f32,
    pub mode: RecallMode,
    pub llm_timeout_ms: u64,
    pub max_hints: usize,
    pub source_budgets: RecallSourceBudgets,
    pub memory_type_budgets: MemoryTypeBudgets,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RecallPolicyOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RecallMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hints: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_budgets: Option<RecallSourceBudgetsOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_type_budgets: Option<MemoryTypeBudgetsOverride>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RecallSourceBudgetsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notebook: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_raw: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_object: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_event: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct MemoryTypeBudgetsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_raw: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_observation: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_relation: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<usize>,
}

pub fn apply_recall_policy_override(
    policy: &mut RecallPolicy,
    override_policy: Option<&RecallPolicyOverride>,
) {
    let Some(override_policy) = override_policy else {
        return;
    };
    if let Some(mode) = override_policy.mode {
        policy.mode = mode;
    }
    if let Some(timeout) = override_policy.llm_timeout_ms {
        policy.llm_timeout_ms = timeout;
    }
    if let Some(max_hints) = override_policy.max_hints {
        policy.max_hints = max_hints.max(1);
    }
    if let Some(source) = &override_policy.source_budgets {
        if let Some(value) = source.memory {
            policy.source_budgets.memory = value;
        }
        if let Some(value) = source.notebook {
            policy.source_budgets.notebook = value;
        }
        if let Some(value) = source.session_raw {
            policy.source_budgets.session_raw = value;
        }
        if let Some(value) = source.did_object {
            policy.source_budgets.did_object = value;
        }
        if let Some(value) = source.background_event {
            policy.source_budgets.background_event = value;
        }
    }
    if let Some(memory) = &override_policy.memory_type_budgets {
        if let Some(value) = memory.session_raw {
            policy.memory_type_budgets.session_raw = value;
        }
        if let Some(value) = memory.event {
            policy.memory_type_budgets.event = value;
        }
        if let Some(value) = memory.entity_observation {
            policy.memory_type_budgets.entity_observation = value;
        }
        if let Some(value) = memory.entity_relation {
            policy.memory_type_budgets.entity_relation = value;
        }
        if let Some(value) = memory.free {
            policy.memory_type_budgets.free = value;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecallSourceBudgets {
    pub memory: usize,
    pub notebook: usize,
    pub session_raw: usize,
    pub did_object: usize,
    pub background_event: usize,
}

impl Default for RecallSourceBudgets {
    fn default() -> Self {
        Self {
            memory: 4,
            notebook: 3,
            session_raw: 3,
            did_object: 0,
            background_event: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemoryTypeBudgets {
    pub session_raw: usize,
    pub event: usize,
    pub entity_observation: usize,
    pub entity_relation: usize,
    pub free: usize,
}

impl Default for MemoryTypeBudgets {
    fn default() -> Self {
        Self {
            session_raw: 2,
            event: 2,
            entity_observation: 2,
            entity_relation: 2,
            free: 2,
        }
    }
}

impl Default for RecallPolicy {
    fn default() -> Self {
        Self {
            tag_capacity: default_tag_capacity(),
            decay_tau_seconds: 30.0 * 60.0,
            distance_threshold_turns: 5,
            change_threshold: 0.5,
            mode: RecallMode::Auto,
            llm_timeout_ms: 10_000,
            max_hints: 8,
            source_budgets: RecallSourceBudgets::default(),
            memory_type_budgets: MemoryTypeBudgets::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallDecision {
    NotTriggered,
    Mechanical,
    Llm,
}

pub struct SessionTopicUpdater {
    recall_service: Arc<dyn RecallService>,
    policy: RecallPolicy,
}

impl SessionTopicUpdater {
    pub fn new(recall_service: Arc<dyn RecallService>, policy: RecallPolicy) -> Self {
        Self {
            recall_service,
            policy,
        }
    }

    pub fn with_default_recall(policy: RecallPolicy) -> Self {
        Self::new(Arc::new(DefaultRecallService::default()), policy)
    }

    pub fn with_default_retrieval(policy: RecallPolicy) -> Self {
        Self::with_default_recall(policy)
    }

    pub fn with_local_recall(
        policy: RecallPolicy,
        memory_root: impl Into<PathBuf>,
        notebook_root: impl Into<PathBuf>,
    ) -> Self {
        Self::new(
            Arc::new(DefaultRecallService::with_local_roots(
                memory_root,
                notebook_root,
            )),
            policy,
        )
    }

    pub async fn update(
        &self,
        input: UpdateSessionTopicInput,
    ) -> Result<UpdateSessionTopicResult, SessionTopicError> {
        Ok(self.update_with_record(input).await?.0)
    }

    pub async fn update_with_record(
        &self,
        input: UpdateSessionTopicInput,
    ) -> Result<(UpdateSessionTopicResult, SessionTopicHistoryRecord), SessionTopicError> {
        let topic = normalize_topic(&input.topic)?;
        let tags = normalize_tags(&input.tags)?;
        let tag_names: Vec<String> = tags.iter().map(|tag| tag.name.clone()).collect();
        let tag_reasons: BTreeMap<String, String> = tags
            .iter()
            .map(|tag| (tag.name.clone(), tag.reason.clone()))
            .collect();
        let now = now_string();
        let meta_dir = input.session_dir.join(META_DIR);
        fs::create_dir_all(&meta_dir)?;

        let topic_path = meta_dir.join(TOPIC_FILE);
        let old_topic = read_topic_doc(&topic_path).ok();
        let topic_changed = old_topic
            .as_ref()
            .map(|old| {
                old.topic != topic || old.tags != tag_names || old.tag_reasons != tag_reasons
            })
            .unwrap_or(true);
        if topic_changed {
            write_topic_doc(
                &topic_path,
                &input.session_id,
                &topic,
                &tag_names,
                &tag_reasons,
                &now,
            )?;
        }
        let topic_record = SessionTopicHistoryRecord {
            session_id: input.session_id.clone(),
            topic: topic.clone(),
            tags: tag_names.clone(),
            tag_reasons: tag_reasons.clone(),
            current_turn: input.current_turn,
            updated_at: now.clone(),
            topic_changed,
        };
        append_topic_log(&meta_dir.join(TOPIC_LOG_FILE), &topic_record)?;

        let tag_path = meta_dir.join(TAG_SET_FILE);
        let mut tag_set = read_tag_set(&tag_path)?;
        tag_set.capacity = self.policy.tag_capacity.max(1);
        let tag_set_diff = update_tag_set(&mut tag_set, &tags, &now, &self.policy);
        write_json_pretty(&tag_path, &tag_set)?;

        if !tag_set_diff.removed.is_empty() {
            cleanup_subscriptions_for_removed_tags(&meta_dir, &tag_set_diff.removed)?;
        }

        let decision = decide_recall(&tag_set, &tag_set_diff, input.current_turn, &self.policy);
        let mut recall = None;
        let mut recall_status = RecallStatus::NotTriggered;

        match decision {
            RecallDecision::NotTriggered => {}
            RecallDecision::Mechanical | RecallDecision::Llm => {
                let mode = match decision {
                    RecallDecision::Mechanical => RecallMode::Mechanical,
                    RecallDecision::Llm => RecallMode::Llm,
                    RecallDecision::NotTriggered => RecallMode::Auto,
                };
                let started = Instant::now();
                let result = if matches!(mode, RecallMode::Llm) {
                    match tokio::time::timeout(
                        Duration::from_millis(self.policy.llm_timeout_ms),
                        self.recall_service.recall(
                            RecallInput {
                                session_id: &input.session_id,
                                session_dir: &input.session_dir,
                                topic: &topic,
                                tags: &tag_set,
                            },
                            mode,
                            &self.policy,
                        ),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => RecallResult::Failed {
                            reason: format!(
                                "LLM recall timed out after {}ms",
                                self.policy.llm_timeout_ms
                            ),
                        },
                    }
                } else {
                    self.recall_service
                        .recall(
                            RecallInput {
                                session_id: &input.session_id,
                                session_dir: &input.session_dir,
                                topic: &topic,
                                tags: &tag_set,
                            },
                            mode,
                            &self.policy,
                        )
                        .await
                };
                let ms = elapsed_ms(started);
                match result {
                    RecallResult::NotTriggered => {
                        recall_status = RecallStatus::NotTriggered;
                    }
                    RecallResult::Recalled {
                        items,
                        subscriptions,
                    } => {
                        if !subscriptions.is_empty() {
                            merge_subscriptions(&meta_dir, subscriptions.clone())?;
                        }
                        recall = Some(RecallPayload {
                            items,
                            subscriptions,
                        });
                        recall_status = match mode {
                            RecallMode::Mechanical => RecallStatus::Mechanical { ms },
                            RecallMode::Llm => RecallStatus::Llm { ms },
                            RecallMode::Auto => RecallStatus::NotTriggered,
                        };
                        tag_set.last_recall_turn = Some(input.current_turn);
                        tag_set.last_recall_at = Some(now_string());
                        write_json_pretty(&tag_path, &tag_set)?;
                    }
                    RecallResult::Failed { reason } => {
                        recall_status = RecallStatus::Failed { reason };
                        tag_set.last_recall_turn = Some(input.current_turn);
                        tag_set.last_recall_at = Some(now_string());
                        write_json_pretty(&tag_path, &tag_set)?;
                    }
                }
            }
        }

        Ok((
            UpdateSessionTopicResult {
                tag_set_diff: TagSetDiff {
                    added: tag_set_diff.added,
                    removed: tag_set_diff.removed,
                    current: tag_set.tags,
                },
                recall,
                recall_status,
            },
            topic_record,
        ))
    }
}

impl Default for SessionTopicUpdater {
    fn default() -> Self {
        Self::with_default_recall(RecallPolicy::default())
    }
}

pub fn decide_recall(
    tag_set: &TagSet,
    diff: &TagSetDiff,
    current_turn: u32,
    policy: &RecallPolicy,
) -> RecallDecision {
    match policy.mode {
        RecallMode::Mechanical => return RecallDecision::Mechanical,
        RecallMode::Llm => return RecallDecision::Llm,
        RecallMode::Auto => {}
    }

    let total = tag_set.tags.len().max(1) as f32;
    let change_ratio = (diff.added.len() + diff.removed.len()) as f32 / total;
    if change_ratio >= policy.change_threshold {
        return RecallDecision::Llm;
    }

    let turns_since_recall = tag_set
        .last_recall_turn
        .map(|last| current_turn.saturating_sub(last))
        .unwrap_or(current_turn);
    if turns_since_recall >= policy.distance_threshold_turns {
        return RecallDecision::Mechanical;
    }

    RecallDecision::NotTriggered
}

fn update_tag_set(
    tag_set: &mut TagSet,
    incoming: &[TagInput],
    now: &str,
    policy: &RecallPolicy,
) -> TagSetDiff {
    let mut added = Vec::new();
    let mut by_name: HashMap<String, usize> = tag_set
        .tags
        .iter()
        .enumerate()
        .map(|(idx, t)| (t.name.clone(), idx))
        .collect();

    for (position, tag) in incoming.iter().enumerate() {
        if let Some(idx) = by_name.get(&tag.name).copied() {
            let entry = &mut tag_set.tags[idx];
            entry.weight += 1.0;
            entry.last_touched = now.to_string();
            entry.tier = reinforce_tag_tier(entry.tier, entry.weight);
            entry.position = position as u32;
            entry.reason = Some(tag.reason.clone());
        } else {
            let idx = tag_set.tags.len();
            tag_set.tags.push(TagEntry {
                name: tag.name.clone(),
                weight: 1.0,
                last_touched: now.to_string(),
                tier: TagTier::Transient,
                position: position as u32,
                reason: Some(tag.reason.clone()),
            });
            by_name.insert(tag.name.clone(), idx);
            added.push(tag.name.clone());
        }
    }
    let incoming_names: HashSet<&str> = incoming.iter().map(|tag| tag.name.as_str()).collect();
    let mut retained_position = incoming.len() as u32;
    let mut retained: Vec<_> = tag_set
        .tags
        .iter_mut()
        .filter(|tag| !incoming_names.contains(tag.name.as_str()))
        .collect();
    retained.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
    });
    for tag in retained {
        tag.position = retained_position;
        retained_position = retained_position.saturating_add(1);
    }

    let mut removed = Vec::new();
    while tag_set.tags.len() > tag_set.capacity {
        let idx = choose_eviction_index(tag_set, now, policy);
        removed.push(tag_set.tags.remove(idx).name);
    }

    normalize_tag_positions(&mut tag_set.tags);
    TagSetDiff {
        added,
        removed,
        current: tag_set.tags.clone(),
    }
}

fn normalize_tag_positions(tags: &mut [TagEntry]) {
    tags.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
    });
    for (idx, tag) in tags.iter_mut().enumerate() {
        tag.position = idx as u32;
    }
}

fn reinforce_tag_tier(current: TagTier, weight: f32) -> TagTier {
    match current {
        TagTier::Pinned => TagTier::Pinned,
        TagTier::Active => TagTier::Active,
        TagTier::Transient if weight >= TAG_ACTIVE_REINFORCE_THRESHOLD => TagTier::Active,
        TagTier::Transient => TagTier::Transient,
    }
}

fn choose_eviction_index(tag_set: &TagSet, now: &str, policy: &RecallPolicy) -> usize {
    let candidates = [TagTier::Transient, TagTier::Active, TagTier::Pinned];
    for tier in candidates {
        let mut best: Option<(usize, f64)> = None;
        for (idx, tag) in tag_set.tags.iter().enumerate() {
            if tag.tier != tier {
                continue;
            }
            let score = decayed_score(tag, now, policy);
            if best.map(|(_, s)| score < s).unwrap_or(true) {
                best = Some((idx, score));
            }
        }
        if let Some((idx, _)) = best {
            return idx;
        }
    }
    0
}

fn decayed_score(tag: &TagEntry, now: &str, policy: &RecallPolicy) -> f64 {
    let dt = parse_time(now)
        .zip(parse_time(&tag.last_touched))
        .map(|(now, touched)| (now - touched).num_seconds().max(0) as f64)
        .unwrap_or(0.0);
    let tau = policy.decay_tau_seconds.max(1.0);
    (tag.weight as f64) * (-dt / tau).exp()
}

fn read_tag_set(path: &Path) -> Result<TagSet, SessionTopicError> {
    if !path.exists() {
        return Ok(TagSet::default());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(TagSet::default());
    }
    Ok(serde_json::from_str(&text)?)
}

fn write_topic_doc(
    path: &Path,
    session_id: &str,
    topic: &str,
    tags: &[String],
    tag_reasons: &BTreeMap<String, String>,
    now: &str,
) -> Result<(), SessionTopicError> {
    let tags_json = serde_json::to_string(tags)?;
    let reasons_json = serde_json::to_string(tag_reasons)?;
    let body = format!(
        "---\nschema: {}\nversion: {}\nsession_id: {}\nupdated_at: {}\ntags: {}\ntag_reasons: {}\n---\n\n{}\n",
        TOPIC_DOC_SCHEMA, TOPIC_DOC_VERSION, session_id, now, tags_json, reasons_json, topic
    );
    write_atomic(path, body.as_bytes())?;
    Ok(())
}

pub(crate) fn read_topic_doc(path: &Path) -> Result<TopicDoc, SessionTopicError> {
    let text = fs::read_to_string(path)?;
    parse_topic_doc(&text)
}

fn parse_topic_doc(text: &str) -> Result<TopicDoc, SessionTopicError> {
    if !text.starts_with("---\n") {
        return Ok(TopicDoc {
            schema: None,
            version: None,
            session_id: String::new(),
            tags: Vec::new(),
            tag_reasons: BTreeMap::new(),
            topic: text.trim().to_string(),
        });
    }
    let Some(end) = text[4..].find("\n---\n") else {
        return Err(SessionTopicError::InvalidInput(
            "topic.md frontmatter is not closed".to_string(),
        ));
    };
    let fm = &text[4..4 + end];
    let body = text[4 + end + 5..].trim().to_string();
    let mut schema = None;
    let mut version = None;
    let mut session_id = String::new();
    let mut tags = Vec::new();
    let mut tag_reasons = BTreeMap::new();
    for line in fm.lines() {
        let line = line.trim();
        if let Some(raw) = line.strip_prefix("schema:") {
            schema = Some(raw.trim().to_string());
        } else if let Some(raw) = line.strip_prefix("version:") {
            version = raw.trim().parse::<u32>().ok();
        } else if let Some(raw) = line.strip_prefix("session_id:") {
            session_id = raw.trim().to_string();
        } else if let Some(raw) = line.strip_prefix("tags:") {
            tags = serde_json::from_str(raw.trim()).unwrap_or_default();
        } else if let Some(raw) = line.strip_prefix("tag_reasons:") {
            tag_reasons = serde_json::from_str(raw.trim()).unwrap_or_default();
        }
    }
    Ok(TopicDoc {
        schema,
        version,
        session_id,
        tags,
        tag_reasons,
        topic: body,
    })
}

pub(crate) fn read_topic_log(
    path: &Path,
) -> Result<Vec<SessionTopicHistoryRecord>, SessionTopicError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        records.push(serde_json::from_str(line)?);
    }
    Ok(records)
}

fn append_topic_log(
    path: &Path,
    record: &SessionTopicHistoryRecord,
) -> Result<(), SessionTopicError> {
    let line = serde_json::json!({
        "session_id": record.session_id,
        "updated_at": record.updated_at,
        "topic": record.topic,
        "tags": record.tags,
        "tag_reasons": record.tag_reasons,
        "current_turn": record.current_turn,
        "topic_changed": record.topic_changed,
    });
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(&line)?)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct TopicDoc {
    pub(crate) schema: Option<String>,
    pub(crate) version: Option<u32>,
    pub(crate) session_id: String,
    pub(crate) tags: Vec<String>,
    pub(crate) tag_reasons: BTreeMap<String, String>,
    pub(crate) topic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SubscriptionSet {
    #[serde(default)]
    subscriptions: Vec<Subscription>,
}

fn cleanup_subscriptions_for_removed_tags(
    meta_dir: &Path,
    removed: &[String],
) -> Result<(), SessionTopicError> {
    let path = meta_dir.join(SUBSCRIPTIONS_FILE);
    if !path.exists() {
        return Ok(());
    }
    let mut set = read_subscription_set(&path)?;
    let removed: HashSet<&str> = removed.iter().map(String::as_str).collect();
    set.subscriptions.retain(|sub| {
        !sub.bound_tags
            .iter()
            .any(|tag| removed.contains(tag.as_str()))
    });
    write_json_pretty(&path, &set)?;
    Ok(())
}

fn merge_subscriptions(
    meta_dir: &Path,
    subscriptions: Vec<Subscription>,
) -> Result<(), SessionTopicError> {
    let path = meta_dir.join(SUBSCRIPTIONS_FILE);
    let mut set = read_subscription_set(&path)?;
    for sub in subscriptions {
        if let Some(existing) = set
            .subscriptions
            .iter_mut()
            .find(|item| item.kind == sub.kind && item.bound_tags == sub.bound_tags)
        {
            *existing = sub;
        } else {
            set.subscriptions.push(sub);
        }
    }
    write_json_pretty(&path, &set)?;
    Ok(())
}

fn read_subscription_set(path: &Path) -> Result<SubscriptionSet, SessionTopicError> {
    if !path.exists() {
        return Ok(SubscriptionSet::default());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(SubscriptionSet::default());
    }
    Ok(serde_json::from_str(&text)?)
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<(), SessionTopicError> {
    let data = serde_json::to_vec_pretty(value)?;
    write_atomic(path, &data)?;
    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), SessionTopicError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn normalize_topic(topic: &str) -> Result<String, SessionTopicError> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err(SessionTopicError::InvalidInput(
            "`topic` must not be empty".to_string(),
        ));
    }
    if topic.contains('\n') || topic.contains('\r') {
        return Err(SessionTopicError::InvalidInput(
            "`topic` must be a single line".to_string(),
        ));
    }
    if topic.chars().count() > 120 {
        return Err(SessionTopicError::InvalidInput(
            "`topic` must be 120 characters or fewer".to_string(),
        ));
    }
    Ok(topic.to_string())
}

fn normalize_tags(tags: &[TagInput]) -> Result<Vec<TagInput>, SessionTopicError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in tags {
        let name = raw.name.trim().to_lowercase();
        if name.is_empty() {
            continue;
        }
        if name.contains('\n') || name.contains('\r') {
            return Err(SessionTopicError::InvalidInput(
                "`tags[].name` entries must be single-line strings".to_string(),
            ));
        }
        if name.chars().count() > 48 {
            return Err(SessionTopicError::InvalidInput(
                "`tags[].name` entries must be 48 characters or fewer".to_string(),
            ));
        }
        let reason = raw.reason.trim().to_string();
        if reason.is_empty() {
            return Err(SessionTopicError::InvalidInput(
                "`tags[].reason` must not be empty".to_string(),
            ));
        }
        if reason.contains('\n') || reason.contains('\r') {
            return Err(SessionTopicError::InvalidInput(
                "`tags[].reason` entries must be single-line strings".to_string(),
            ));
        }
        if reason.chars().count() > 160 {
            return Err(SessionTopicError::InvalidInput(
                "`tags[].reason` entries must be 160 characters or fewer".to_string(),
            ));
        }
        if seen.insert(name.clone()) {
            out.push(TagInput { name, reason });
        }
    }
    Ok(out)
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(s: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn elapsed_ms(started: Instant) -> u32 {
    started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}

fn default_tag_capacity() -> usize {
    8
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tool::agent_notebook::{
        AgentNotebook, AgentNotebookConfig, AppendNoteInput, Confidence, WriteReason,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockRecallService {
        result: Mutex<Option<RecallResult>>,
    }

    #[async_trait]
    impl RecallService for MockRecallService {
        async fn recall(
            &self,
            _input: RecallInput<'_>,
            _mode: RecallMode,
            _policy: &RecallPolicy,
        ) -> RecallResult {
            self.result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(RecallResult::Recalled {
                    items: Vec::new(),
                    subscriptions: Vec::new(),
                })
        }
    }

    #[test]
    fn decision_prefers_llm_on_large_change() {
        let policy = RecallPolicy::default();
        let tag_set = TagSet {
            tags: vec![tag("a", 1.0, "2026-05-19T00:00:00Z")],
            ..TagSet::default()
        };
        let diff = TagSetDiff {
            added: vec!["a".to_string()],
            removed: Vec::new(),
            current: tag_set.tags.clone(),
        };
        assert_eq!(
            decide_recall(&tag_set, &diff, 0, &policy),
            RecallDecision::Llm
        );
    }

    #[test]
    fn decision_uses_distance_after_small_change() {
        let policy = RecallPolicy::default();
        let tag_set = TagSet {
            tags: vec![tag("a", 1.0, "2026-05-19T00:00:00Z")],
            last_recall_turn: Some(1),
            ..TagSet::default()
        };
        let diff = TagSetDiff {
            added: Vec::new(),
            removed: Vec::new(),
            current: tag_set.tags.clone(),
        };
        assert_eq!(
            decide_recall(&tag_set, &diff, 6, &policy),
            RecallDecision::Mechanical
        );
    }

    #[test]
    fn tag_update_reinforces_and_evicts_lowest_decayed_transient() {
        let now = "2026-05-19T01:00:00Z";
        let mut set = TagSet {
            capacity: 2,
            tags: vec![
                tag("old", 5.0, "2026-05-18T00:00:00Z"),
                tag("fresh", 1.0, now),
            ],
            ..TagSet::default()
        };
        let policy = RecallPolicy {
            tag_capacity: 2,
            ..RecallPolicy::default()
        };
        let diff = update_tag_set(
            &mut set,
            &[
                tag_input("new", "new topic focus"),
                tag_input("fresh", "still relevant"),
            ],
            now,
            &policy,
        );
        assert_eq!(diff.added, vec!["new"]);
        assert_eq!(diff.removed, vec!["old"]);
        let fresh = set.tags.iter().find(|t| t.name == "fresh").unwrap();
        assert_eq!(fresh.weight, 2.0);
        assert_eq!(fresh.reason.as_deref(), Some("still relevant"));
    }

    #[test]
    fn tag_eviction_prefers_lower_tier_before_score() {
        let now = "2026-05-19T01:00:00Z";
        let policy = RecallPolicy::default();
        let set = TagSet {
            capacity: 3,
            tags: vec![
                tag_with_tier(
                    "active-old",
                    0.1,
                    "2026-05-18T00:00:00Z",
                    TagTier::Active,
                    0,
                ),
                tag_with_tier(
                    "pinned-old",
                    0.1,
                    "2026-05-18T00:00:00Z",
                    TagTier::Pinned,
                    1,
                ),
                tag_with_tier("transient-fresh", 10.0, now, TagTier::Transient, 2),
            ],
            ..TagSet::default()
        };
        assert_eq!(
            set.tags[choose_eviction_index(&set, now, &policy)].name,
            "transient-fresh"
        );

        let set = TagSet {
            capacity: 2,
            tags: vec![
                tag_with_tier(
                    "active-old",
                    0.1,
                    "2026-05-18T00:00:00Z",
                    TagTier::Active,
                    0,
                ),
                tag_with_tier(
                    "pinned-old",
                    0.1,
                    "2026-05-18T00:00:00Z",
                    TagTier::Pinned,
                    1,
                ),
            ],
            ..TagSet::default()
        };
        assert_eq!(
            set.tags[choose_eviction_index(&set, now, &policy)].name,
            "active-old"
        );

        let set = TagSet {
            capacity: 1,
            tags: vec![tag_with_tier(
                "pinned-only",
                0.1,
                "2026-05-18T00:00:00Z",
                TagTier::Pinned,
                0,
            )],
            ..TagSet::default()
        };
        assert_eq!(
            set.tags[choose_eviction_index(&set, now, &policy)].name,
            "pinned-only"
        );
    }

    #[test]
    fn tag_update_promotes_active_and_preserves_pinned() {
        let now = "2026-05-19T01:00:00Z";
        let policy = RecallPolicy::default();
        let mut set = TagSet {
            capacity: 4,
            tags: vec![
                tag_with_tier("alpha", 2.0, now, TagTier::Transient, 0),
                tag_with_tier("beta", 1.0, now, TagTier::Pinned, 1),
            ],
            ..TagSet::default()
        };

        update_tag_set(
            &mut set,
            &[
                tag_input("alpha", "third reinforcement"),
                tag_input("beta", "explicitly pinned before this update"),
            ],
            now,
            &policy,
        );

        let alpha = set.tags.iter().find(|tag| tag.name == "alpha").unwrap();
        let beta = set.tags.iter().find(|tag| tag.name == "beta").unwrap();
        assert_eq!(alpha.weight, 3.0);
        assert_eq!(alpha.tier, TagTier::Active);
        assert_eq!(beta.tier, TagTier::Pinned);
    }

    #[test]
    fn tag_update_persists_current_input_order_as_position() {
        let now = "2026-05-19T01:00:00Z";
        let policy = RecallPolicy::default();
        let mut set = TagSet {
            capacity: 4,
            tags: vec![
                tag_with_tier("zeta", 1.0, now, TagTier::Transient, 0),
                tag_with_tier("alpha", 1.0, now, TagTier::Transient, 1),
            ],
            ..TagSet::default()
        };

        update_tag_set(
            &mut set,
            &[
                tag_input("beta", "first current focus"),
                tag_input("zeta", "second current focus"),
            ],
            now,
            &policy,
        );

        let names: Vec<_> = set.tags.iter().map(|tag| tag.name.as_str()).collect();
        let positions: Vec<_> = set.tags.iter().map(|tag| tag.position).collect();
        assert_eq!(names, vec!["beta", "zeta", "alpha"]);
        assert_eq!(positions, vec![0, 1, 2]);
    }

    #[test]
    fn tag_normalization_requires_reason() {
        let err = normalize_tags(&[tag_input("Design", "   ")]).unwrap_err();
        assert!(matches!(err, SessionTopicError::InvalidInput(_)));
    }

    #[test]
    fn recall_policy_override_only_touches_recall_fields() {
        let mut policy = RecallPolicy::default();
        apply_recall_policy_override(
            &mut policy,
            Some(&RecallPolicyOverride {
                mode: Some(RecallMode::Mechanical),
                max_hints: Some(3),
                source_budgets: Some(RecallSourceBudgetsOverride {
                    memory: Some(1),
                    did_object: Some(2),
                    ..RecallSourceBudgetsOverride::default()
                }),
                memory_type_budgets: Some(MemoryTypeBudgetsOverride {
                    free: Some(0),
                    event: Some(1),
                    ..MemoryTypeBudgetsOverride::default()
                }),
                ..RecallPolicyOverride::default()
            }),
        );
        assert_eq!(policy.mode, RecallMode::Mechanical);
        assert_eq!(policy.max_hints, 3);
        assert_eq!(policy.source_budgets.memory, 1);
        assert_eq!(policy.source_budgets.did_object, 2);
        assert_eq!(policy.memory_type_budgets.event, 1);
        assert_eq!(policy.memory_type_budgets.free, 0);
        assert_eq!(policy.tag_capacity, default_tag_capacity());
        assert_eq!(
            policy.decay_tau_seconds,
            RecallPolicy::default().decay_tau_seconds
        );
    }

    #[tokio::test]
    async fn updater_writes_topic_tag_set_and_recall_payload() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(MockRecallService {
            result: Mutex::new(Some(RecallResult::Recalled {
                items: vec![RecallItem {
                    source_system: RecallSourceSystem::SessionRaw,
                    hint_type: RecallHintType::SessionRaw,
                    target: RecallTarget {
                        kind: "session".to_string(),
                        id: "s-old".to_string(),
                        uri: Some("/tmp/s-old".to_string()),
                    },
                    title: Some("old topic".to_string()),
                    hint: "Related previous session topic: old topic".to_string(),
                    reason: "matched tags: design".to_string(),
                    matched_tags: vec!["design".to_string()],
                    score: 2.0,
                    suggested_action: "open_session_history_if_needed".to_string(),
                    debug: BTreeMap::new(),
                }],
                subscriptions: vec![Subscription {
                    id: "sub-1".to_string(),
                    kind: "state".to_string(),
                    hint: "watch state".to_string(),
                    bound_tags: vec!["design".to_string()],
                    created_at: "2026-05-19T00:00:00Z".to_string(),
                }],
            })),
        });
        let updater = SessionTopicUpdater::new(service, RecallPolicy::default());
        let out = updater
            .update(UpdateSessionTopicInput {
                session_id: "s1".to_string(),
                session_dir: dir.path().join("s1"),
                topic: "Discuss session topic implementation".to_string(),
                tags: vec![tag_input("Design", "current implementation topic")],
                current_turn: 0,
            })
            .await
            .unwrap();
        assert!(matches!(out.recall_status, RecallStatus::Llm { .. }));
        let recall_item = &out.recall.unwrap().items[0];
        assert_eq!(recall_item.source_system, RecallSourceSystem::SessionRaw);
        assert_eq!(recall_item.hint_type, RecallHintType::SessionRaw);
        assert_eq!(recall_item.target.id, "s-old");
        assert_eq!(recall_item.matched_tags, vec!["design"]);
        assert!(dir
            .path()
            .join("s1/.meta/topic.md")
            .read_to_string()
            .unwrap()
            .contains("Discuss session topic implementation"));
        let topic_doc = dir
            .path()
            .join("s1/.meta/topic.md")
            .read_to_string()
            .unwrap();
        assert!(topic_doc.contains("schema: opendan.session_topic"));
        assert!(topic_doc.contains("version: 1"));
        let parsed = read_topic_doc(&dir.path().join("s1/.meta/topic.md")).unwrap();
        assert_eq!(parsed.schema.as_deref(), Some("opendan.session_topic"));
        assert_eq!(parsed.version, Some(1));
        let tag_set: TagSet = serde_json::from_str(
            &fs::read_to_string(dir.path().join("s1/.meta/tag_set.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(tag_set.tags[0].name, "design");
        assert_eq!(
            tag_set.tags[0].reason.as_deref(),
            Some("current implementation topic")
        );
        let topic_log = fs::read_to_string(dir.path().join("s1/.meta/topic_log.jsonl")).unwrap();
        assert!(topic_log.contains("current implementation topic"));
        assert!(dir.path().join("s1/.meta/subscriptions.json").exists());
    }

    #[tokio::test]
    async fn updater_keeps_success_when_recall_fails() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(MockRecallService {
            result: Mutex::new(Some(RecallResult::Failed {
                reason: "boom".to_string(),
            })),
        });
        let updater = SessionTopicUpdater::new(service, RecallPolicy::default());
        let out = updater
            .update(UpdateSessionTopicInput {
                session_id: "s1".to_string(),
                session_dir: dir.path().join("s1"),
                topic: "Discuss recall failure handling".to_string(),
                tags: vec![tag_input("ops", "checking failure handling")],
                current_turn: 0,
            })
            .await
            .unwrap();
        assert_eq!(
            out.recall_status,
            RecallStatus::Failed {
                reason: "boom".to_string()
            }
        );
        assert!(dir.path().join("s1/.meta/tag_set.json").exists());
    }

    #[tokio::test]
    async fn updater_returns_notebook_hints_from_local_recall() {
        let root = tempfile::tempdir().unwrap();
        let notebook_root = root.path().join("notebook");
        let memory_root = root.path().join("memory");
        let sessions_root = root.path().join("sessions");
        let session_dir = sessions_root.join("s-current");
        fs::create_dir_all(&session_dir).unwrap();

        let notebook = AgentNotebook::open(AgentNotebookConfig::new(&notebook_root)).unwrap();
        notebook
            .append_note(AppendNoteInput {
                session_id: Some("seed".to_string()),
                notebook_id: "projects/agent-memory".to_string(),
                title: "Agent memory recall".to_string(),
                content: "Notebook content should not be copied into the recall hint.".to_string(),
                source_excerpt: None,
                source_ref: None,
                source_session_id: Some("seed".to_string()),
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: Some(Confidence::High),
                tags: vec!["memory".to_string()],
                detect_conflicts: false,
            })
            .unwrap();

        let updater = SessionTopicUpdater::with_local_recall(
            RecallPolicy {
                mode: RecallMode::Mechanical,
                ..RecallPolicy::default()
            },
            memory_root,
            notebook_root,
        );
        let out = updater
            .update(UpdateSessionTopicInput {
                session_id: "s-current".to_string(),
                session_dir,
                topic: "Discuss agent memory recall".to_string(),
                tags: vec![tag_input("memory", "current topic tag")],
                current_turn: 0,
            })
            .await
            .unwrap();
        assert!(matches!(out.recall_status, RecallStatus::Mechanical { .. }));
        let items = out.recall.unwrap().items;
        let notebook_hint = items
            .iter()
            .find(|item| item.source_system == RecallSourceSystem::Notebook)
            .expect("notebook hint should be returned");
        assert_eq!(notebook_hint.target.id, "projects/agent-memory");
        assert_eq!(notebook_hint.matched_tags, vec!["memory"]);
        assert!(notebook_hint
            .debug
            .get("version")
            .is_some_and(|version| !version.is_empty()));
        assert!(!notebook_hint
            .hint
            .contains("Notebook content should not be copied"));
    }

    #[tokio::test]
    async fn repeated_same_topic_keeps_topic_doc_content_stable() {
        let dir = tempfile::tempdir().unwrap();
        let updater = SessionTopicUpdater::new(
            Arc::new(MockRecallService::default()),
            RecallPolicy {
                change_threshold: 2.0,
                distance_threshold_turns: u32::MAX,
                ..RecallPolicy::default()
            },
        );
        let input = UpdateSessionTopicInput {
            session_id: "s1".to_string(),
            session_dir: dir.path().join("s1"),
            topic: "Discuss idempotent topic writes".to_string(),
            tags: vec![tag_input("idempotent", "same topic repeated")],
            current_turn: 0,
        };
        updater.update(input.clone()).await.unwrap();
        let path = dir.path().join("s1/.meta/topic.md");
        let first = fs::read_to_string(&path).unwrap();
        updater.update(input).await.unwrap();
        let second = fs::read_to_string(path).unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn updater_appends_topic_timeline_and_keeps_latest_projection() {
        let dir = tempfile::tempdir().unwrap();
        let updater = SessionTopicUpdater::new(
            Arc::new(MockRecallService::default()),
            RecallPolicy {
                change_threshold: 2.0,
                distance_threshold_turns: u32::MAX,
                ..RecallPolicy::default()
            },
        );
        let session_dir = dir.path().join("s1");

        let (_, first_record) = updater
            .update_with_record(UpdateSessionTopicInput {
                session_id: "s1".to_string(),
                session_dir: session_dir.clone(),
                topic: "Plan storage history".to_string(),
                tags: vec![tag_input("Storage", "topic history design")],
                current_turn: 1,
            })
            .await
            .unwrap();
        let (_, second_record) = updater
            .update_with_record(UpdateSessionTopicInput {
                session_id: "s1".to_string(),
                session_dir: session_dir.clone(),
                topic: "Implement storage history".to_string(),
                tags: vec![tag_input("History", "topic timeline implementation")],
                current_turn: 2,
            })
            .await
            .unwrap();

        assert_eq!(first_record.topic, "Plan storage history");
        assert_eq!(first_record.tags, vec!["storage"]);
        assert_eq!(first_record.current_turn, 1);
        assert!(first_record.topic_changed);
        assert_eq!(second_record.topic, "Implement storage history");
        assert_eq!(second_record.tags, vec!["history"]);
        assert_eq!(second_record.current_turn, 2);
        assert!(second_record.topic_changed);

        let topic_doc = fs::read_to_string(session_dir.join(".meta/topic.md")).unwrap();
        assert!(topic_doc.contains("Implement storage history"));
        assert!(!topic_doc.contains("Plan storage history"));

        let topic_log = fs::read_to_string(session_dir.join(".meta/topic_log.jsonl")).unwrap();
        let lines: Vec<serde_json::Value> = topic_log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["topic"], "Plan storage history");
        assert_eq!(lines[0]["current_turn"], 1);
        assert_eq!(lines[1]["topic"], "Implement storage history");
        assert_eq!(lines[1]["current_turn"], 2);
        let records = read_topic_log(&session_dir.join(".meta/topic_log.jsonl")).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].topic, "Plan storage history");
        assert_eq!(records[1].tags, vec!["history"]);
    }

    trait ReadToString {
        fn read_to_string(&self) -> std::io::Result<String>;
    }

    impl ReadToString for PathBuf {
        fn read_to_string(&self) -> std::io::Result<String> {
            fs::read_to_string(self)
        }
    }

    fn tag(name: &str, weight: f32, last_touched: &str) -> TagEntry {
        tag_with_tier(name, weight, last_touched, TagTier::Transient, 0)
    }

    fn tag_with_tier(
        name: &str,
        weight: f32,
        last_touched: &str,
        tier: TagTier,
        position: u32,
    ) -> TagEntry {
        TagEntry {
            name: name.to_string(),
            weight,
            last_touched: last_touched.to_string(),
            tier,
            position,
            reason: None,
        }
    }

    fn tag_input(name: &str, reason: &str) -> TagInput {
        TagInput {
            name: name.to_string(),
            reason: reason.to_string(),
        }
    }
}
