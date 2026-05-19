use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const META_DIR: &str = ".meta";
const TOPIC_FILE: &str = "topic.md";
const TOPIC_LOG_FILE: &str = "topic_log.jsonl";
const TAG_SET_FILE: &str = "tag_set.json";
const SUBSCRIPTIONS_FILE: &str = "subscriptions.json";
const DEFAULT_RECALL_LIMIT: usize = 8;

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
    pub tags: Vec<String>,
    pub current_turn: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct UpdateSessionTopicResult {
    pub tag_set_diff: TagSetDiff,
    pub recall: Option<RecallPayload>,
    pub recall_status: RecallStatus,
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
pub struct RecallPayload {
    pub items: Vec<RecallItem>,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RecallItem {
    pub session_id: String,
    pub session_dir: String,
    pub topic: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub score: f32,
    pub reason: String,
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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallDecision {
    NotTriggered,
    Mechanical,
    Llm,
}

#[derive(Debug, Clone)]
pub struct RecallInput<'a> {
    pub session_id: &'a str,
    pub session_dir: &'a Path,
    pub topic: &'a str,
    pub tags: &'a TagSet,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecallResult {
    NotTriggered,
    Recalled {
        items: Vec<RecallItem>,
        subscriptions: Vec<Subscription>,
    },
    Failed {
        reason: String,
    },
}

#[async_trait]
pub trait RecallService: Send + Sync {
    async fn recall(
        &self,
        input: RecallInput<'_>,
        mode: RecallMode,
        policy: &RecallPolicy,
    ) -> RecallResult;
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

    pub async fn update(
        &self,
        input: UpdateSessionTopicInput,
    ) -> Result<UpdateSessionTopicResult, SessionTopicError> {
        let topic = normalize_topic(&input.topic)?;
        let tags = normalize_tags(&input.tags)?;
        let now = now_string();
        let meta_dir = input.session_dir.join(META_DIR);
        fs::create_dir_all(&meta_dir)?;

        let topic_path = meta_dir.join(TOPIC_FILE);
        let old_topic = read_topic_doc(&topic_path).ok();
        let topic_changed = old_topic
            .as_ref()
            .map(|old| old.topic != topic || old.tags != tags)
            .unwrap_or(true);
        if topic_changed {
            write_topic_doc(&topic_path, &input.session_id, &topic, &tags, &now)?;
        }
        append_topic_log(
            &meta_dir.join(TOPIC_LOG_FILE),
            &input.session_id,
            &topic,
            &tags,
            topic_changed,
            &now,
        )?;

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

        Ok(UpdateSessionTopicResult {
            tag_set_diff: TagSetDiff {
                added: tag_set_diff.added,
                removed: tag_set_diff.removed,
                current: tag_set.tags,
            },
            recall,
            recall_status,
        })
    }
}

impl Default for SessionTopicUpdater {
    fn default() -> Self {
        Self::with_default_recall(RecallPolicy::default())
    }
}

#[derive(Default)]
pub struct DefaultRecallService {
    mechanical: MechanicalRecallService,
    llm: LlmRecallService,
}

#[async_trait]
impl RecallService for DefaultRecallService {
    async fn recall(
        &self,
        input: RecallInput<'_>,
        mode: RecallMode,
        policy: &RecallPolicy,
    ) -> RecallResult {
        match mode {
            RecallMode::Mechanical => self.mechanical.recall(input, mode, policy).await,
            RecallMode::Llm => self.llm.recall(input, mode, policy).await,
            RecallMode::Auto => RecallResult::NotTriggered,
        }
    }
}

#[derive(Default)]
pub struct MechanicalRecallService;

#[async_trait]
impl RecallService for MechanicalRecallService {
    async fn recall(
        &self,
        input: RecallInput<'_>,
        _mode: RecallMode,
        _policy: &RecallPolicy,
    ) -> RecallResult {
        let Some(sessions_root) = input.session_dir.parent() else {
            return RecallResult::Recalled {
                items: Vec::new(),
                subscriptions: Vec::new(),
            };
        };
        let query_tags: HashSet<String> = input.tags.tags.iter().map(|t| t.name.clone()).collect();
        if query_tags.is_empty() {
            return RecallResult::Recalled {
                items: Vec::new(),
                subscriptions: Vec::new(),
            };
        }

        let mut items = Vec::new();
        let Ok(entries) = fs::read_dir(sessions_root) else {
            return RecallResult::Recalled {
                items: Vec::new(),
                subscriptions: Vec::new(),
            };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == input.session_dir || !path.is_dir() {
                continue;
            }
            let topic_path = path.join(META_DIR).join(TOPIC_FILE);
            let Ok(doc) = read_topic_doc(&topic_path) else {
                continue;
            };
            if doc.session_id == input.session_id {
                continue;
            }
            let item_tags: HashSet<String> = doc.tags.iter().cloned().collect();
            let matched: Vec<String> = query_tags.intersection(&item_tags).cloned().collect();
            let mut score = (matched.len() as f32) * 2.0;
            let topic_lc = doc.topic.to_lowercase();
            for tag in &query_tags {
                if topic_lc.contains(tag) {
                    score += 1.0;
                }
            }
            if score <= 0.0 {
                continue;
            }
            items.push(RecallItem {
                session_id: if doc.session_id.is_empty() {
                    path.file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or_default()
                        .to_string()
                } else {
                    doc.session_id
                },
                session_dir: path.display().to_string(),
                topic: doc.topic,
                tags: doc.tags,
                score,
                reason: if matched.is_empty() {
                    "topic text matched current tags".to_string()
                } else {
                    format!("matched tags: {}", matched.join(", "))
                },
            });
        }
        items.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        items.truncate(DEFAULT_RECALL_LIMIT);
        RecallResult::Recalled {
            items,
            subscriptions: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct LlmRecallService;

#[async_trait]
impl RecallService for LlmRecallService {
    async fn recall(
        &self,
        _input: RecallInput<'_>,
        _mode: RecallMode,
        _policy: &RecallPolicy,
    ) -> RecallResult {
        RecallResult::Failed {
            reason: "LLM recall backend is not configured".to_string(),
        }
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
    incoming: &[String],
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

    for tag in incoming {
        if let Some(idx) = by_name.get(tag).copied() {
            let entry = &mut tag_set.tags[idx];
            entry.weight += 1.0;
            entry.last_touched = now.to_string();
            entry.tier = TagTier::Transient;
        } else {
            let idx = tag_set.tags.len();
            tag_set.tags.push(TagEntry {
                name: tag.clone(),
                weight: 1.0,
                last_touched: now.to_string(),
                tier: TagTier::Transient,
            });
            by_name.insert(tag.clone(), idx);
            added.push(tag.clone());
        }
    }

    let mut removed = Vec::new();
    while tag_set.tags.len() > tag_set.capacity {
        let idx = choose_eviction_index(tag_set, now, policy);
        removed.push(tag_set.tags.remove(idx).name);
    }

    tag_set.tags.sort_by(|a, b| a.name.cmp(&b.name));
    TagSetDiff {
        added,
        removed,
        current: tag_set.tags.clone(),
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
    now: &str,
) -> Result<(), SessionTopicError> {
    let tags_json = serde_json::to_string(tags)?;
    let body = format!(
        "---\nsession_id: {}\nupdated_at: {}\ntags: {}\n---\n\n{}\n",
        session_id, now, tags_json, topic
    );
    write_atomic(path, body.as_bytes())?;
    Ok(())
}

fn read_topic_doc(path: &Path) -> Result<TopicDoc, SessionTopicError> {
    let text = fs::read_to_string(path)?;
    parse_topic_doc(&text)
}

fn parse_topic_doc(text: &str) -> Result<TopicDoc, SessionTopicError> {
    if !text.starts_with("---\n") {
        return Ok(TopicDoc {
            session_id: String::new(),
            tags: Vec::new(),
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
    let mut session_id = String::new();
    let mut tags = Vec::new();
    for line in fm.lines() {
        let line = line.trim();
        if let Some(raw) = line.strip_prefix("session_id:") {
            session_id = raw.trim().to_string();
        } else if let Some(raw) = line.strip_prefix("tags:") {
            tags = serde_json::from_str(raw.trim()).unwrap_or_default();
        }
    }
    Ok(TopicDoc {
        session_id,
        tags,
        topic: body,
    })
}

fn append_topic_log(
    path: &Path,
    session_id: &str,
    topic: &str,
    tags: &[String],
    topic_changed: bool,
    now: &str,
) -> Result<(), SessionTopicError> {
    let line = serde_json::json!({
        "session_id": session_id,
        "updated_at": now,
        "topic": topic,
        "tags": tags,
        "topic_changed": topic_changed,
    });
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(&line)?)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct TopicDoc {
    session_id: String,
    tags: Vec<String>,
    topic: String,
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

fn normalize_tags(tags: &[String]) -> Result<Vec<String>, SessionTopicError> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in tags {
        let tag = raw.trim().to_lowercase();
        if tag.is_empty() {
            continue;
        }
        if tag.contains('\n') || tag.contains('\r') {
            return Err(SessionTopicError::InvalidInput(
                "`tags` entries must be single-line strings".to_string(),
            ));
        }
        if tag.chars().count() > 48 {
            return Err(SessionTopicError::InvalidInput(
                "`tags` entries must be 48 characters or fewer".to_string(),
            ));
        }
        if seen.insert(tag.clone()) {
            out.push(tag);
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
            &["new".to_string(), "fresh".to_string()],
            now,
            &policy,
        );
        assert_eq!(diff.added, vec!["new"]);
        assert_eq!(diff.removed, vec!["old"]);
        let fresh = set.tags.iter().find(|t| t.name == "fresh").unwrap();
        assert_eq!(fresh.weight, 2.0);
    }

    #[tokio::test]
    async fn updater_writes_topic_tag_set_and_recall_payload() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(MockRecallService {
            result: Mutex::new(Some(RecallResult::Recalled {
                items: vec![RecallItem {
                    session_id: "s-old".to_string(),
                    session_dir: "/tmp/s-old".to_string(),
                    topic: "old topic".to_string(),
                    tags: vec!["design".to_string()],
                    score: 2.0,
                    reason: "matched tags: design".to_string(),
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
                tags: vec!["Design".to_string()],
                current_turn: 0,
            })
            .await
            .unwrap();
        assert!(matches!(out.recall_status, RecallStatus::Llm { .. }));
        assert_eq!(out.recall.unwrap().items[0].session_id, "s-old");
        assert!(dir
            .path()
            .join("s1/.meta/topic.md")
            .read_to_string()
            .unwrap()
            .contains("Discuss session topic implementation"));
        let tag_set: TagSet = serde_json::from_str(
            &fs::read_to_string(dir.path().join("s1/.meta/tag_set.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(tag_set.tags[0].name, "design");
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
                tags: vec!["ops".to_string()],
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
            tags: vec!["idempotent".to_string()],
            current_turn: 0,
        };
        updater.update(input.clone()).await.unwrap();
        let path = dir.path().join("s1/.meta/topic.md");
        let first = fs::read_to_string(&path).unwrap();
        updater.update(input).await.unwrap();
        let second = fs::read_to_string(path).unwrap();
        assert_eq!(first, second);
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
        TagEntry {
            name: name.to_string(),
            weight,
            last_touched: last_touched.to_string(),
            tier: TagTier::Transient,
        }
    }
}
