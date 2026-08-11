use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use agent_did_object_lib::{AdapterType, ObjectRouteConfig, RouteMethod};
use agent_tool::{
    agent_notebook::{AgentNotebook, AgentNotebookConfig, BuildHintsInput, HintReason},
    AgentMemory, AgentMemoryConfig, MemoryHintBudget, MemoryHintType, MemoryRecallOptions,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session_topic::{
    read_topic_doc, read_topic_log, RecallMode, RecallPolicy, SessionTopicHistoryRecord,
    Subscription, TagSet,
};

const META_DIR: &str = ".meta";
const TOPIC_FILE: &str = "topic.md";
const TOPIC_LOG_FILE: &str = "topic_log.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecallSourceSystem {
    Memory,
    Notebook,
    DidObject,
    SessionRaw,
    BackgroundEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecallHintType {
    SessionRaw,
    Event,
    EntityObservation,
    EntityRelation,
    Free,
    TopicRelevance,
    CrossSessionUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RecallTarget {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RecallItem {
    pub source_system: RecallSourceSystem,
    pub hint_type: RecallHintType,
    pub target: RecallTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub hint: String,
    pub reason: String,
    #[serde(default)]
    pub matched_tags: Vec<String>,
    pub score: f32,
    pub suggested_action: String,
    #[serde(default)]
    pub debug: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct RecallPayload {
    pub items: Vec<RecallItem>,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
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

#[async_trait]
pub trait RecallProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn recall(
        &self,
        input: RecallInput<'_>,
        policy: &RecallPolicy,
    ) -> Result<Vec<RecallItem>, String>;
}

pub struct DefaultRecallService {
    mechanical: HintRecallEngine,
    llm: LlmRecallService,
}

impl Default for DefaultRecallService {
    fn default() -> Self {
        Self {
            mechanical: HintRecallEngine::default(),
            llm: LlmRecallService,
        }
    }
}

impl DefaultRecallService {
    pub fn with_local_roots(
        memory_root: impl Into<PathBuf>,
        notebook_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            mechanical: HintRecallEngine::with_local_roots(memory_root, notebook_root),
            llm: LlmRecallService,
        }
    }

    pub fn with_local_roots_and_did_objects(
        memory_root: impl Into<PathBuf>,
        notebook_root: impl Into<PathBuf>,
        did_object_config: ObjectRouteConfig,
    ) -> Self {
        Self {
            mechanical: HintRecallEngine::with_local_roots_and_did_objects(
                memory_root,
                notebook_root,
                did_object_config,
            ),
            llm: LlmRecallService,
        }
    }
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

pub struct HintRecallEngine {
    providers: Vec<Arc<dyn RecallProvider>>,
}

impl HintRecallEngine {
    pub fn new(providers: Vec<Arc<dyn RecallProvider>>) -> Self {
        Self { providers }
    }

    pub fn with_local_roots(
        memory_root: impl Into<PathBuf>,
        notebook_root: impl Into<PathBuf>,
    ) -> Self {
        Self::new(vec![
            Arc::new(SessionTopicRecallProvider),
            Arc::new(NotebookRecallProvider::new(notebook_root)),
            Arc::new(MemoryRecallProvider::new(memory_root)),
            Arc::new(DidObjectRecallProvider::default()),
        ])
    }

    pub fn with_local_roots_and_did_objects(
        memory_root: impl Into<PathBuf>,
        notebook_root: impl Into<PathBuf>,
        did_object_config: ObjectRouteConfig,
    ) -> Self {
        Self::new(vec![
            Arc::new(SessionTopicRecallProvider),
            Arc::new(NotebookRecallProvider::new(notebook_root)),
            Arc::new(MemoryRecallProvider::new(memory_root)),
            Arc::new(DidObjectRecallProvider::new(did_object_config)),
        ])
    }
}

impl Default for HintRecallEngine {
    fn default() -> Self {
        Self::new(vec![
            Arc::new(SessionTopicRecallProvider),
            Arc::new(NotebookRecallProvider::default()),
            Arc::new(MemoryRecallProvider::default()),
            Arc::new(DidObjectRecallProvider::default()),
        ])
    }
}

#[async_trait]
impl RecallService for HintRecallEngine {
    async fn recall(
        &self,
        input: RecallInput<'_>,
        _mode: RecallMode,
        policy: &RecallPolicy,
    ) -> RecallResult {
        let mut items = Vec::new();
        for provider in &self.providers {
            match provider.recall(input.clone(), policy).await {
                Ok(mut provider_items) => items.append(&mut provider_items),
                Err(reason) => {
                    return RecallResult::Failed {
                        reason: format!("{} recall failed: {reason}", provider.name()),
                    };
                }
            }
        }
        items = dedupe_recall_items(items);
        items.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.target.id.cmp(&b.target.id))
        });
        items.truncate(policy.max_hints);
        RecallResult::Recalled {
            items,
            subscriptions: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct SessionTopicRecallProvider;

#[async_trait]
impl RecallProvider for SessionTopicRecallProvider {
    fn name(&self) -> &'static str {
        "session_topic"
    }

    async fn recall(
        &self,
        input: RecallInput<'_>,
        policy: &RecallPolicy,
    ) -> Result<Vec<RecallItem>, String> {
        let budget = policy.source_budgets.session_raw;
        if budget == 0 {
            return Ok(Vec::new());
        }
        let Some(sessions_root) = input.session_dir.parent() else {
            return Ok(Vec::new());
        };
        let query_tags: HashSet<String> = input.tags.tags.iter().map(|t| t.name.clone()).collect();
        if query_tags.is_empty() {
            return Ok(Vec::new());
        }

        let entries = match fs::read_dir(sessions_root) {
            Ok(entries) => entries,
            Err(_) => return Ok(Vec::new()),
        };
        let mut items = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path == input.session_dir || !path.is_dir() {
                continue;
            }
            let topic_path = path.join(META_DIR).join(TOPIC_FILE);
            if let Ok(doc) = read_topic_doc(&topic_path) {
                if doc.session_id != input.session_id {
                    if let Some(mut item) = recall_item_from_session_topic(
                        self.name(),
                        &path,
                        doc.session_id,
                        doc.topic,
                        doc.tags,
                        &query_tags,
                        "current",
                        None,
                    ) {
                        if let Some(schema) = doc.schema {
                            item.debug.insert("schema".to_string(), schema);
                        }
                        if let Some(version) = doc.version {
                            item.debug
                                .insert("version".to_string(), version.to_string());
                        }
                        items.push(item);
                    }
                }
            }

            let log_path = path.join(META_DIR).join(TOPIC_LOG_FILE);
            for record in read_topic_log(&log_path).unwrap_or_default() {
                if record.session_id == input.session_id {
                    continue;
                }
                if let Some(item) =
                    recall_item_from_topic_log(self.name(), &path, record, &query_tags)
                {
                    items.push(item);
                }
            }
        }
        items.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.target.id.cmp(&b.target.id))
        });
        items.truncate(budget);
        Ok(items)
    }
}

fn recall_item_from_topic_log(
    provider_name: &'static str,
    session_dir: &Path,
    record: SessionTopicHistoryRecord,
    query_tags: &HashSet<String>,
) -> Option<RecallItem> {
    recall_item_from_session_topic(
        provider_name,
        session_dir,
        record.session_id,
        record.topic,
        record.tags,
        query_tags,
        "topic_log",
        Some((record.current_turn, record.updated_at)),
    )
}

fn recall_item_from_session_topic(
    provider_name: &'static str,
    session_dir: &Path,
    session_id: String,
    topic: String,
    tags: Vec<String>,
    query_tags: &HashSet<String>,
    topic_source: &'static str,
    history_meta: Option<(u32, String)>,
) -> Option<RecallItem> {
    let item_tags: HashSet<String> = tags.iter().cloned().collect();
    let mut matched: Vec<String> = query_tags.intersection(&item_tags).cloned().collect();
    matched.sort();
    let mut score = (matched.len() as f32) * 2.0;
    let topic_lc = topic.to_lowercase();
    for tag in query_tags {
        if topic_lc.contains(tag) {
            score += 1.0;
        }
    }
    if score <= 0.0 {
        return None;
    }

    let session_id = if session_id.is_empty() {
        session_dir
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        session_id
    };
    let target_kind = if topic_source == "topic_log" {
        "session_topic_history"
    } else {
        "session"
    };
    let target_id = if let Some((turn, _)) = &history_meta {
        format!("{session_id}@turn:{turn}")
    } else {
        session_id.clone()
    };
    let uri = if let Some((turn, _)) = &history_meta {
        Some(format!("{}#topic_log:{turn}", session_dir.display()))
    } else {
        Some(session_dir.display().to_string())
    };
    let reason = if matched.is_empty() {
        "topic title matched current tags".to_string()
    } else {
        format!("matched tags: {}", matched.join(", "))
    };
    let mut debug = BTreeMap::new();
    debug.insert("provider".to_string(), provider_name.to_string());
    debug.insert("session_dir".to_string(), session_dir.display().to_string());
    debug.insert("tags".to_string(), tags.join(","));
    debug.insert("topic_source".to_string(), topic_source.to_string());
    if let Some((turn, updated_at)) = history_meta {
        debug.insert("current_turn".to_string(), turn.to_string());
        debug.insert("updated_at".to_string(), updated_at);
    }
    Some(RecallItem {
        source_system: RecallSourceSystem::SessionRaw,
        hint_type: RecallHintType::SessionRaw,
        target: RecallTarget {
            kind: target_kind.to_string(),
            id: target_id,
            uri,
        },
        title: Some(topic.clone()),
        hint: format!("Related previous session topic: {topic}"),
        reason,
        matched_tags: matched,
        score,
        suggested_action: "open_session_history_if_needed".to_string(),
        debug,
    })
}

#[derive(Default)]
pub struct NotebookRecallProvider {
    root: Option<PathBuf>,
}

impl NotebookRecallProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
        }
    }
}

#[async_trait]
impl RecallProvider for NotebookRecallProvider {
    fn name(&self) -> &'static str {
        "notebook"
    }

    async fn recall(
        &self,
        input: RecallInput<'_>,
        policy: &RecallPolicy,
    ) -> Result<Vec<RecallItem>, String> {
        let budget = policy.source_budgets.notebook;
        if budget == 0 {
            return Ok(Vec::new());
        }
        let Some(root) = &self.root else {
            return Ok(Vec::new());
        };
        let notebook =
            AgentNotebook::open(AgentNotebookConfig::new(root)).map_err(|err| err.to_string())?;
        let topic_tags = input.tags.tags.iter().map(|tag| tag.name.clone()).collect();
        let ctx = notebook
            .build_notebook_hints(BuildHintsInput {
                session_id: input.session_id.to_string(),
                topic_tags: Some(topic_tags),
                candidate_notebook_ids: None,
                max_hints: Some(budget),
            })
            .map_err(|err| err.to_string())?;
        Ok(ctx
            .hints
            .into_iter()
            .map(|hint| {
                let matched_tags = hint.matched_tags.unwrap_or_default();
                let reason = format!("{:?}", hint.reason);
                let hint_type = match hint.reason {
                    HintReason::CrossSessionUpdate => RecallHintType::CrossSessionUpdate,
                    HintReason::TopicRelevance | HintReason::NearTitleUpdate => {
                        RecallHintType::TopicRelevance
                    }
                };
                let mut debug = BTreeMap::new();
                debug.insert("provider".to_string(), self.name().to_string());
                debug.insert("version".to_string(), hint.version.clone());
                RecallItem {
                    source_system: RecallSourceSystem::Notebook,
                    hint_type,
                    target: RecallTarget {
                        kind: "notebook".to_string(),
                        id: hint.notebook_id,
                        uri: None,
                    },
                    title: hint.title,
                    hint: hint.text,
                    reason,
                    matched_tags: matched_tags.clone(),
                    score: 1.5 + matched_tags.len() as f32,
                    suggested_action: "read_notebook_if_needed".to_string(),
                    debug,
                }
            })
            .collect())
    }
}

#[derive(Default)]
pub struct MemoryRecallProvider {
    root: Option<PathBuf>,
}

impl MemoryRecallProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
        }
    }
}

#[async_trait]
impl RecallProvider for MemoryRecallProvider {
    fn name(&self) -> &'static str {
        "memory"
    }

    async fn recall(
        &self,
        input: RecallInput<'_>,
        policy: &RecallPolicy,
    ) -> Result<Vec<RecallItem>, String> {
        let budget = policy.source_budgets.memory;
        if budget == 0 {
            return Ok(Vec::new());
        }
        let Some(root) = &self.root else {
            return Ok(Vec::new());
        };
        let memory =
            AgentMemory::open(AgentMemoryConfig::new(root)).map_err(|err| err.to_string())?;
        let tags: Vec<String> = input.tags.tags.iter().map(|tag| tag.name.clone()).collect();
        let items = memory
            .recall_hints(&tags, memory_recall_options_from_policy(policy, budget))
            .map_err(|err| err.to_string())?;
        Ok(items
            .into_iter()
            .map(|item| {
                let matched_tags = item
                    .matched
                    .iter()
                    .filter_map(|hit| hit.strip_prefix("tag:").map(ToOwned::to_owned))
                    .collect::<Vec<_>>();
                let hint_type = recall_hint_type_from_memory(item.hint_type);
                let mut debug = BTreeMap::new();
                debug.insert("provider".to_string(), self.name().to_string());
                debug.insert("kind".to_string(), item.kind.clone());
                debug.insert("source_occasion".to_string(), item.source_occasion);
                debug.insert("noticed_at".to_string(), item.noticed_at);
                if !item.evidence.is_empty() {
                    debug.insert("evidence".to_string(), item.evidence.join(","));
                }
                RecallItem {
                    source_system: RecallSourceSystem::Memory,
                    hint_type,
                    target: RecallTarget {
                        kind: item.target_kind,
                        id: item.target_id,
                        uri: item.uri,
                    },
                    title: None,
                    hint: item.hint,
                    reason: item.reason,
                    matched_tags,
                    score: item.score,
                    suggested_action: "load_memory_item_if_needed".to_string(),
                    debug,
                }
            })
            .collect())
    }
}

#[derive(Default)]
pub struct DidObjectRecallProvider {
    config: Option<ObjectRouteConfig>,
}

impl DidObjectRecallProvider {
    pub fn new(config: ObjectRouteConfig) -> Self {
        Self {
            config: Some(config),
        }
    }
}

#[async_trait]
impl RecallProvider for DidObjectRecallProvider {
    fn name(&self) -> &'static str {
        "did_object"
    }

    async fn recall(
        &self,
        input: RecallInput<'_>,
        policy: &RecallPolicy,
    ) -> Result<Vec<RecallItem>, String> {
        let budget = policy.source_budgets.did_object;
        if budget == 0 {
            return Ok(Vec::new());
        }
        let Some(config) = &self.config else {
            return Ok(Vec::new());
        };
        let query_tags: HashSet<String> = input.tags.tags.iter().map(|t| t.name.clone()).collect();
        if query_tags.is_empty() && input.topic.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut items = Vec::new();
        for route in &config.routes {
            let Some(adapter) = config.adapter(&route.adapter) else {
                continue;
            };
            if adapter.adapter_type != AdapterType::DidObject
                || !route.allows_method(RouteMethod::Read)
            {
                continue;
            }
            let object = route
                .options
                .get("object")
                .and_then(|value| value.as_str())
                .unwrap_or(&route.pattern)
                .trim();
            if object.is_empty() {
                continue;
            }
            let title = route
                .options
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let object_tags = route
                .options
                .get("tags")
                .and_then(|value| value.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(|value| value.trim().to_ascii_lowercase())
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let haystack = did_object_match_text(route, adapter, title.as_deref(), &object_tags);
            let mut matched_tags = object_tags
                .iter()
                .filter(|tag| query_tags.contains(*tag))
                .cloned()
                .collect::<Vec<_>>();
            matched_tags.sort();
            matched_tags.dedup();
            let mut score = (matched_tags.len() as f32) * 2.0;
            for tag in &query_tags {
                if haystack.contains(tag) {
                    score += 1.0;
                }
            }
            if !input.topic.trim().is_empty() {
                let topic_lc = input.topic.to_ascii_lowercase();
                if title
                    .as_deref()
                    .map(|title| topic_lc.contains(&title.to_ascii_lowercase()))
                    .unwrap_or(false)
                {
                    score += 1.0;
                }
            }
            if score <= 0.0 {
                continue;
            }

            let hint = route
                .options
                .get("hint")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    title
                        .as_ref()
                        .map(|title| format!("Related DID object: {title}"))
                        .unwrap_or_else(|| format!("Related DID object: {object}"))
                });
            let kind = route
                .options
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("did_object")
                .trim()
                .to_string();
            let mut debug = BTreeMap::new();
            debug.insert("provider".to_string(), self.name().to_string());
            debug.insert("route_id".to_string(), route.id.clone());
            debug.insert("adapter".to_string(), route.adapter.clone());
            debug.insert("match_type".to_string(), format!("{:?}", route.match_type));
            debug.insert("pattern".to_string(), route.pattern.clone());
            items.push(RecallItem {
                source_system: RecallSourceSystem::DidObject,
                hint_type: RecallHintType::EntityObservation,
                target: RecallTarget {
                    kind,
                    id: object.to_string(),
                    uri: Some(object.to_string()),
                },
                title,
                hint,
                reason: if matched_tags.is_empty() {
                    "DID object metadata matched current topic title".to_string()
                } else {
                    format!("matched tags: {}", matched_tags.join(", "))
                },
                matched_tags,
                score,
                suggested_action: "read_did_object_if_needed".to_string(),
                debug,
            });
        }
        items.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.target.id.cmp(&b.target.id))
        });
        items.truncate(budget);
        Ok(items)
    }
}

fn did_object_match_text(
    route: &agent_did_object_lib::ObjectRoute,
    adapter: &agent_did_object_lib::AdapterConfig,
    title: Option<&str>,
    tags: &[String],
) -> String {
    let mut parts = vec![
        route.id.as_str(),
        route.pattern.as_str(),
        route.adapter.as_str(),
        adapter.id.as_str(),
    ];
    if let Some(title) = title {
        parts.push(title);
    }
    let mut text = parts.join(" ").to_ascii_lowercase();
    for tag in tags {
        text.push(' ');
        text.push_str(tag);
    }
    for key in ["description", "kind", "hint"] {
        if let Some(value) = route.options.get(key).and_then(|value| value.as_str()) {
            text.push(' ');
            text.push_str(&value.to_ascii_lowercase());
        }
    }
    text
}

fn memory_recall_options_from_policy(
    policy: &RecallPolicy,
    source_budget: usize,
) -> MemoryRecallOptions {
    let type_budgets = &policy.memory_type_budgets;
    MemoryRecallOptions {
        max_hints: source_budget.min(policy.max_hints),
        body_truncate_bytes: 160,
        session_raw: MemoryHintBudget::new(type_budgets.session_raw * 3, type_budgets.session_raw),
        event: MemoryHintBudget::new(type_budgets.event * 3, type_budgets.event),
        entity_observation: MemoryHintBudget::new(
            type_budgets.entity_observation * 3,
            type_budgets.entity_observation,
        ),
        entity_relation: MemoryHintBudget::new(
            type_budgets.entity_relation * 3,
            type_budgets.entity_relation,
        ),
        free: MemoryHintBudget::new(type_budgets.free * 3, type_budgets.free),
        current_time: None,
    }
}

fn recall_hint_type_from_memory(hint_type: MemoryHintType) -> RecallHintType {
    match hint_type {
        MemoryHintType::SessionRaw => RecallHintType::SessionRaw,
        MemoryHintType::Event => RecallHintType::Event,
        MemoryHintType::EntityObservation => RecallHintType::EntityObservation,
        MemoryHintType::EntityRelation => RecallHintType::EntityRelation,
        MemoryHintType::Free => RecallHintType::Free,
    }
}

fn dedupe_recall_items(items: Vec<RecallItem>) -> Vec<RecallItem> {
    let mut by_key: BTreeMap<String, RecallItem> = BTreeMap::new();
    for item in items {
        let mut matched = item.matched_tags.clone();
        matched.sort();
        let key = format!(
            "{:?}|{:?}|{}|{}|{}|{}",
            item.source_system,
            item.hint_type,
            item.target.kind,
            item.target.id,
            item.target.uri.clone().unwrap_or_default(),
            matched.join(",")
        );
        if let Some(existing) = by_key.get_mut(&key) {
            merge_recall_item(existing, item);
        } else {
            by_key.insert(key, item);
        }
    }
    by_key.into_values().collect()
}

fn merge_recall_item(existing: &mut RecallItem, incoming: RecallItem) {
    let prior_score = existing.score;
    let incoming_score = incoming.score;
    if incoming.score > existing.score {
        let mut old_debug = std::mem::take(&mut existing.debug);
        old_debug.insert(
            "merged_lower_score".to_string(),
            format!("{prior_score:.3}"),
        );
        *existing = incoming;
        existing.debug.extend(old_debug);
    } else {
        if incoming.reason.len() > existing.reason.len() {
            existing.reason = incoming.reason;
        }
        existing.debug.insert(
            "merged_lower_score".to_string(),
            format!("{incoming_score:.3}"),
        );
        existing.debug.extend(
            incoming
                .debug
                .into_iter()
                .map(|(k, v)| (format!("merged_{k}"), v)),
        );
    }
    let merged_count = existing
        .debug
        .get("merged_count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        + 1;
    existing
        .debug
        .insert("merged_count".to_string(), merged_count.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_topic::{TagEntry, TagTier};
    use agent_did_object_lib::{AdapterConfig, ObjectRoute, RouteMatchType};
    use serde_json::json;

    #[tokio::test]
    async fn session_topic_provider_returns_unified_hints() {
        let root = tempfile::tempdir().unwrap();
        let current_dir = root.path().join("s-current");
        let old_dir = root.path().join("s-old");
        let older_dir = root.path().join("s-older");
        let no_topic_dir = root.path().join("s-empty");
        fs::create_dir_all(current_dir.join(".meta")).unwrap();
        fs::create_dir_all(old_dir.join(".meta")).unwrap();
        fs::create_dir_all(older_dir.join(".meta")).unwrap();
        fs::create_dir_all(&no_topic_dir).unwrap();
        fs::write(
            current_dir.join(".meta/topic.md"),
            "---\nsession_id: s-current\nupdated_at: 2026-05-19T00:00:00Z\ntags: [\"agent\"]\ntag_reasons: {}\n---\n\nCurrent topic\n",
        )
        .unwrap();
        fs::write(
            old_dir.join(".meta/topic.md"),
            "---\nsession_id: s-old\nupdated_at: 2026-05-18T00:00:00Z\ntags: [\"agent\",\"memory\"]\ntag_reasons: {}\n---\n\nAgent memory recall design\n",
        )
        .unwrap();
        fs::write(
            older_dir.join(".meta/topic.md"),
            "---\nsession_id: s-older\nupdated_at: 2026-05-17T00:00:00Z\ntags: [\"memory\"]\ntag_reasons: {}\n---\n\nMemory-only recall notes\n",
        )
        .unwrap();

        let tags = TagSet {
            tags: vec![
                TagEntry {
                    name: "agent".to_string(),
                    weight: 1.0,
                    last_touched: "2026-05-19T00:00:00Z".to_string(),
                    tier: TagTier::Transient,
                    position: 0,
                    reason: None,
                },
                TagEntry {
                    name: "memory".to_string(),
                    weight: 1.0,
                    last_touched: "2026-05-19T00:00:00Z".to_string(),
                    tier: TagTier::Transient,
                    position: 1,
                    reason: None,
                },
            ],
            ..TagSet::default()
        };
        let items = SessionTopicRecallProvider
            .recall(
                RecallInput {
                    session_id: "s-current",
                    session_dir: &current_dir,
                    topic: "Current topic",
                    tags: &tags,
                },
                &RecallPolicy::default(),
            )
            .await
            .unwrap();

        assert_eq!(items.len(), 2);
        let item = &items[0];
        assert_eq!(item.source_system, RecallSourceSystem::SessionRaw);
        assert_eq!(item.hint_type, RecallHintType::SessionRaw);
        assert_eq!(item.target.kind, "session");
        assert_eq!(item.target.id, "s-old");
        assert_eq!(item.title.as_deref(), Some("Agent memory recall design"));
        assert_eq!(item.matched_tags, vec!["agent", "memory"]);
        assert!(item.hint.contains("Related previous session topic"));
        assert!(!item.hint.contains("Current topic"));
        assert!(item.score > 0.0);
        assert_eq!(items[1].target.id, "s-older");
        assert_eq!(items[1].matched_tags, vec!["memory"]);
    }

    #[tokio::test]
    async fn session_topic_provider_recalls_topic_log_titles() {
        let root = tempfile::tempdir().unwrap();
        let current_dir = root.path().join("s-current");
        let old_dir = root.path().join("s-old");
        fs::create_dir_all(current_dir.join(".meta")).unwrap();
        fs::create_dir_all(old_dir.join(".meta")).unwrap();
        fs::write(
            old_dir.join(".meta/topic.md"),
            "---\nsession_id: s-old\nupdated_at: 2026-05-19T00:00:00Z\ntags: [\"other\"]\ntag_reasons: {}\n---\n\nCurrent unrelated title\n",
        )
        .unwrap();
        fs::write(
            old_dir.join(".meta/topic_log.jsonl"),
            serde_json::to_string(&serde_json::json!({
                "session_id": "s-old",
                "topic": "Booking object integration",
                "tags": ["booking"],
                "tag_reasons": {"booking": "old focus"},
                "current_turn": 3,
                "updated_at": "2026-05-18T00:00:00Z",
                "topic_changed": true
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let tags = TagSet {
            tags: vec![TagEntry {
                name: "booking".to_string(),
                weight: 1.0,
                last_touched: "2026-05-19T00:00:00Z".to_string(),
                tier: TagTier::Transient,
                position: 0,
                reason: None,
            }],
            ..TagSet::default()
        };
        let items = SessionTopicRecallProvider
            .recall(
                RecallInput {
                    session_id: "s-current",
                    session_dir: &current_dir,
                    topic: "Current topic",
                    tags: &tags,
                },
                &RecallPolicy::default(),
            )
            .await
            .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target.kind, "session_topic_history");
        assert_eq!(items[0].target.id, "s-old@turn:3");
        assert_eq!(
            items[0].debug.get("topic_source").map(String::as_str),
            Some("topic_log")
        );
        assert_eq!(items[0].matched_tags, vec!["booking"]);
    }

    #[tokio::test]
    async fn did_object_provider_returns_static_metadata_hint() {
        let config = ObjectRouteConfig {
            version: 1,
            adapters: vec![AdapterConfig {
                id: "did-object".to_string(),
                adapter_type: AdapterType::DidObject,
                endpoint: None,
                auth_token_env: None,
                options: json!({}),
            }],
            routes: vec![ObjectRoute {
                id: "reservation".to_string(),
                priority: 10,
                match_type: RouteMatchType::Exact,
                pattern: "https://booking.example/reservations/r1".to_string(),
                adapter: "did-object".to_string(),
                methods: vec![RouteMethod::Read],
                options: json!({
                    "title": "Dinner reservation",
                    "tags": ["booking", "dinner"],
                    "kind": "reservation",
                    "hint": "Dinner reservation object may affect this task"
                }),
            }],
        };
        let tags = TagSet {
            tags: vec![TagEntry {
                name: "booking".to_string(),
                weight: 1.0,
                last_touched: "2026-05-19T00:00:00Z".to_string(),
                tier: TagTier::Transient,
                position: 0,
                reason: None,
            }],
            ..TagSet::default()
        };
        let policy = RecallPolicy {
            source_budgets: crate::session_topic::RecallSourceBudgets {
                did_object: 2,
                ..crate::session_topic::RecallSourceBudgets::default()
            },
            ..RecallPolicy::default()
        };
        let items = DidObjectRecallProvider::new(config)
            .recall(
                RecallInput {
                    session_id: "s-current",
                    session_dir: std::path::Path::new("/tmp/s-current"),
                    topic: "Check booking conflicts",
                    tags: &tags,
                },
                &policy,
            )
            .await
            .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_system, RecallSourceSystem::DidObject);
        assert_eq!(items[0].target.kind, "reservation");
        assert_eq!(
            items[0].target.id,
            "https://booking.example/reservations/r1"
        );
        assert_eq!(items[0].matched_tags, vec!["booking"]);
        assert!(!items[0].hint.contains("profile"));
    }
}
