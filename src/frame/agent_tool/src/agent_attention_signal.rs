use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use thiserror::Error;

use crate::tool::CallingConventions;
use crate::{AgentToolError, ToolCtx, TypedTool};

pub const SCHEMA_VERSION: &str = "0.1";
pub const DEFAULT_EXTRACTOR_VERSION: &str = "agent_attention_signal_stage1_v0.1";
pub const DEFAULT_PROMPT_VERSION: &str = "attention_signal_stage1_v0.1";
pub const TOOL_DISCOVER_EVENT: &str = "DiscoverEvent";
pub const TOOL_DISCOVER_OBJECT_OBSERVATION: &str = "DiscoverObjectObservation";
pub const TOOL_DISCOVER_RELATIONSHIP: &str = "DiscoverRelationship";
pub const TOOL_DISCOVER_SKILL_COVERAGE_GAP: &str = "DiscoverSkillCoverageGap";

pub type Result<T> = std::result::Result<T, AttentionSignalError>;

#[derive(Debug, Error)]
pub enum AttentionSignalError {
    #[error("invalid_input: {0}")]
    InvalidInput(String),
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("storage_error: {0}")]
    Storage(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<AttentionSignalError> for AgentToolError {
    fn from(err: AttentionSignalError) -> Self {
        match err {
            AttentionSignalError::InvalidInput(message) => AgentToolError::InvalidArgs(message),
            AttentionSignalError::NotFound(message) => AgentToolError::NotFound(message),
            other => AgentToolError::ExecFailed(other.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AttentionSignalStoreConfig {
    pub root: PathBuf,
    pub signal_ttl_hours: u64,
    pub light_dedup: bool,
}

impl AttentionSignalStoreConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            signal_ttl_hours: 72,
            light_dedup: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalConfig {
    pub extraction_window_minutes: u64,
    pub max_entries_per_extraction: usize,
    pub max_signals_per_session_window: usize,
    pub min_confidence_to_store: f64,
    pub default_signal_ttl_hours: u64,
    pub default_watching_ttl_hours: u64,
    pub enable_public_entity_filter: bool,
    pub enable_stage1_light_dedup: bool,
    pub enable_canonicalization_candidates: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_model: Option<String>,
    pub prompt_version: String,
}

impl Default for AttentionSignalConfig {
    fn default() -> Self {
        Self {
            extraction_window_minutes: 240,
            max_entries_per_extraction: 200,
            max_signals_per_session_window: 50,
            min_confidence_to_store: 0.55,
            default_signal_ttl_hours: 72,
            default_watching_ttl_hours: 72,
            enable_public_entity_filter: true,
            enable_stage1_light_dedup: true,
            enable_canonicalization_candidates: true,
            extractor_model: None,
            prompt_version: DEFAULT_PROMPT_VERSION.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    Event,
    ObjectObservation,
    Relationship,
    SkillCoverageGap,
}

impl SignalType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::ObjectObservation => "object_observation",
            Self::Relationship => "relationship",
            Self::SkillCoverageGap => "skill_coverage_gap",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalLifecycleStatus {
    PendingStage2,
    Watching,
    Consumed,
    Converted,
    Dropped,
    Expired,
}

impl SignalLifecycleStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::PendingStage2 => "pending_stage2",
            Self::Watching => "watching",
            Self::Consumed => "consumed",
            Self::Converted => "converted",
            Self::Dropped => "dropped",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventPhase {
    Idea,
    Planning,
    Scheduled,
    Active,
    Waiting,
    Blocked,
    Completed,
    Cancelled,
    Abandoned,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Person,
    Project,
    Component,
    Device,
    Organization,
    Location,
    Account,
    Email,
    Document,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObservationType {
    Status,
    Preference,
    Problem,
    Capability,
    Attribute,
    Role,
    Usage,
    Note,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipType {
    Likes,
    Dislikes,
    Owns,
    Uses,
    WorksWith,
    ResponsibleFor,
    ParticipatesIn,
    SentTo,
    ReceivedFrom,
    DependsOn,
    RelatedTo,
    AliasOf,
    ContactInfo,
    AttitudeTowards,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipPolarity {
    Positive,
    Negative,
    Neutral,
    Mixed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipStrength {
    Weak,
    Medium,
    Strong,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Message,
    Step,
    Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityLevel {
    Low,
    Medium,
    High,
}

impl AmbiguityLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedAction {
    ConsiderEvent,
    ConsiderObject,
    ConsiderRelationship,
    ConsiderSkillCoverageGap,
    ConsiderAlias,
    Watch,
    DropIfUnreinforced,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillCoverageGapKind {
    NoSkillAssigned,
    SuccessWithoutSkill,
    AssignedSkillUnused,
    MissingFallback,
    RepeatedManualPath,
    ExistingSkillMismatch,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillTaskResultStatus {
    Success,
    PartialSuccess,
    Failure,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetentionHint {
    ShortLived,
    Watch72h,
    LikelyPromotable,
    RequiresMoreEvidence,
}

impl RetentionHint {
    fn as_str(self) -> &'static str {
        match self {
            Self::ShortLived => "short_lived",
            Self::Watch72h => "watch_72h",
            Self::LikelyPromotable => "likely_promotable",
            Self::RequiresMoreEvidence => "requires_more_evidence",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyScope {
    UserPrivate,
    Public,
    Mixed,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EntityMention {
    pub mention_text: String,
    pub entity_type: EntityType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_id_candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alias_candidates: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_public_entity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_user_private_entity: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uniqueness_hint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TimeInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range_text: Option<String>,
    pub is_time_precise: bool,
}

impl Default for TimeInfo {
    fn default() -> Self {
        Self {
            start_time: None,
            end_time: None,
            time_range_text: None,
            is_time_precise: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipAttitude {
    pub polarity: RelationshipPolarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentiment_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<RelationshipStrength>,
    pub is_explicit: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TemporalContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range_text: Option<String>,
    pub is_time_bound: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RoundEntryRef {
    pub round_index: u64,
    pub entry_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_kind: Option<EntryKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_call: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Evidence {
    pub round_index: u64,
    pub entry_seq: u64,
    pub entry_kind: EntryKind,
    pub role: EvidenceRole,
    pub text_excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignalSource {
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub session_id: String,
    pub round_refs: Vec<RoundEntryRef>,
    pub window_start: String,
    pub window_end: String,
    pub source_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CanonicalizationCandidate {
    pub mention_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    pub candidate_source: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Stage2Preparation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_merge_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub canonicalization_candidates: Vec<CanonicalizationCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub possible_memory_path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_initial_attention_weight: Option<f64>,
    pub suggested_action: SuggestedAction,
    pub retention_hint: RetentionHint,
    pub privacy_scope: PrivacyScope,
    pub recall_candidate_hint: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionInfo {
    pub extractor_version: String,
    pub prompt_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub extracted_at: String,
    pub extraction_window_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SignalQuality {
    pub confidence: f64,
    pub ambiguity_level: AmbiguityLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_value_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_relevance_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_risk_score: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EventSignalPayload {
    pub title: String,
    pub phase: EventPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub time_info: TimeInfo,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<EntityMention>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ObjectObservationSignalPayload {
    pub object: EntityMention,
    pub observation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_type: Option<ObservationType>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RelationshipSignalPayload {
    pub subject: EntityMention,
    pub predicate: String,
    pub object: EntityMention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attitude: Option<RelationshipAttitude>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_type: Option<RelationshipType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal_context: Option<TemporalContext>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SkillCoverageGapSignalPayload {
    pub title: String,
    pub task_description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_objects: Vec<EntityMention>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assigned_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub critical_actions: Vec<String>,
    pub result_status: SkillTaskResultStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_kind: Option<SkillCoverageGapKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_context: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "signal_type")]
pub enum AttentionSignalPayload {
    #[serde(rename = "event")]
    Event(EventSignalPayload),
    #[serde(rename = "object_observation")]
    ObjectObservation(ObjectObservationSignalPayload),
    #[serde(rename = "relationship")]
    Relationship(RelationshipSignalPayload),
    #[serde(rename = "skill_coverage_gap")]
    SkillCoverageGap(SkillCoverageGapSignalPayload),
}

impl AttentionSignalPayload {
    fn signal_type(&self) -> SignalType {
        match self {
            Self::Event(_) => SignalType::Event,
            Self::ObjectObservation(_) => SignalType::ObjectObservation,
            Self::Relationship(_) => SignalType::Relationship,
            Self::SkillCoverageGap(_) => SignalType::SkillCoverageGap,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignal {
    pub id: String,
    pub signal_type: SignalType,
    pub lifecycle_status: SignalLifecycleStatus,
    pub payload: AttentionSignalPayload,
    pub source: SignalSource,
    pub evidence: Vec<Evidence>,
    pub extraction: ExtractionInfo,
    pub quality: SignalQuality,
    pub stage2_hints: Stage2Preparation,
    pub idempotency_key: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalWriteResult {
    pub signal: AttentionSignal,
    pub inserted: bool,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScanCheckpoint {
    pub id: String,
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scanned_round_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scanned_entry_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scanned_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_window_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_window_end: Option<String>,
    pub status: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MarkScannedInput {
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub session_id: String,
    pub last_scanned_round_index: u64,
    pub last_scanned_entry_seq: u64,
    pub scan_window_start: String,
    pub scan_window_end: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionWindow {
    pub id: String,
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub window_start: String,
    pub window_end: String,
    pub status: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateExtractionWindowInput {
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub window_start: String,
    pub window_end: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalTimeWindow {
    pub window_start: String,
    pub window_end: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalHistoryEntry {
    pub round_index: u64,
    pub entry_seq: u64,
    pub entry_kind: EntryKind,
    pub role: EvidenceRole,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalSessionWindow {
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub session_id: String,
    pub entries: Vec<AttentionSignalHistoryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalExtractionInput {
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub session_id: String,
    pub window_start: String,
    pub window_end: String,
    pub extraction_window_id: String,
    pub entries: Vec<AttentionSignalHistoryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalStage1RunReport {
    pub window_start: String,
    pub window_end: String,
    pub sessions_processed: usize,
    pub entries_scanned: usize,
    pub signals_created: usize,
    pub event_signals_created: usize,
    pub object_signals_created: usize,
    pub relationship_signals_created: usize,
    pub skill_coverage_gap_signals_created: usize,
    pub signals_rejected: usize,
    pub duplicate_signals_skipped: usize,
    pub avg_confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejection_errors: Vec<String>,
}

#[async_trait]
pub trait AttentionSignalHistoryReader: Send + Sync {
    async fn get_sessions_with_unscanned_entries(
        &self,
        window: &AttentionSignalTimeWindow,
    ) -> Result<Vec<AttentionSignalSessionWindow>>;
}

#[async_trait]
pub trait AttentionSignalExtractor: Send + Sync {
    async fn extract(&self, input: AttentionSignalExtractionInput) -> Result<Vec<AttentionSignal>>;
}

pub struct AgentAttentionSignalStage1Runner {
    store: Arc<AgentAttentionSignalStore>,
    history_reader: Arc<dyn AttentionSignalHistoryReader>,
    extractor: Arc<dyn AttentionSignalExtractor>,
}

impl AgentAttentionSignalStage1Runner {
    pub fn new(
        store: Arc<AgentAttentionSignalStore>,
        history_reader: Arc<dyn AttentionSignalHistoryReader>,
        extractor: Arc<dyn AttentionSignalExtractor>,
    ) -> Self {
        Self {
            store,
            history_reader,
            extractor,
        }
    }

    pub async fn run_window(
        &self,
        window: AttentionSignalTimeWindow,
    ) -> Result<AttentionSignalStage1RunReport> {
        require_non_empty(&window.window_start, "window_start")?;
        require_non_empty(&window.window_end, "window_end")?;
        let sessions = self
            .history_reader
            .get_sessions_with_unscanned_entries(&window)
            .await?;
        let mut report = AttentionSignalStage1RunReport {
            window_start: window.window_start.clone(),
            window_end: window.window_end.clone(),
            sessions_processed: 0,
            entries_scanned: 0,
            signals_created: 0,
            event_signals_created: 0,
            object_signals_created: 0,
            relationship_signals_created: 0,
            skill_coverage_gap_signals_created: 0,
            signals_rejected: 0,
            duplicate_signals_skipped: 0,
            avg_confidence: 0.0,
            rejection_errors: Vec::new(),
        };
        let mut confidence_sum = 0.0;

        for session in sessions {
            if session.entries.is_empty() {
                continue;
            }
            validate_identity(
                &session.owner_id,
                &session.agent_id,
                &session.agent_scope_id,
                &session.user_id,
                Some(&session.session_id),
            )?;
            let extraction_window =
                self.store
                    .create_extraction_window(CreateExtractionWindowInput {
                        owner_id: session.owner_id.clone(),
                        agent_id: session.agent_id.clone(),
                        agent_scope_id: session.agent_scope_id.clone(),
                        user_id: session.user_id.clone(),
                        window_start: window.window_start.clone(),
                        window_end: window.window_end.clone(),
                    })?;
            let input = AttentionSignalExtractionInput {
                owner_id: session.owner_id.clone(),
                agent_id: session.agent_id.clone(),
                agent_scope_id: session.agent_scope_id.clone(),
                user_id: session.user_id.clone(),
                session_id: session.session_id.clone(),
                window_start: window.window_start.clone(),
                window_end: window.window_end.clone(),
                extraction_window_id: extraction_window.id.clone(),
                entries: session.entries.clone(),
            };
            let signals = self.extractor.extract(input).await?;
            for signal in signals {
                match self.store.insert_signal(signal) {
                    Ok(write) => {
                        if write.duplicate {
                            report.duplicate_signals_skipped += 1;
                            continue;
                        }
                        report.signals_created += 1;
                        confidence_sum += write.signal.quality.confidence;
                        match write.signal.signal_type {
                            SignalType::Event => report.event_signals_created += 1,
                            SignalType::ObjectObservation => {
                                report.object_signals_created += 1;
                            }
                            SignalType::Relationship => report.relationship_signals_created += 1,
                            SignalType::SkillCoverageGap => {
                                report.skill_coverage_gap_signals_created += 1;
                            }
                        }
                    }
                    Err(AttentionSignalError::InvalidInput(message)) => {
                        report.signals_rejected += 1;
                        report.rejection_errors.push(message);
                    }
                    Err(err) => return Err(err),
                }
            }
            let (last_round, last_entry) = last_entry_ref(&session.entries);
            self.store.mark_scanned(MarkScannedInput {
                owner_id: session.owner_id,
                agent_id: session.agent_id,
                agent_scope_id: session.agent_scope_id,
                user_id: session.user_id,
                session_id: session.session_id,
                last_scanned_round_index: last_round,
                last_scanned_entry_seq: last_entry,
                scan_window_start: window.window_start.clone(),
                scan_window_end: window.window_end.clone(),
                status: Some("up_to_date".to_string()),
            })?;
            let _ = self
                .store
                .complete_extraction_window(&extraction_window.id)?;
            report.sessions_processed += 1;
            report.entries_scanned += session.entries.len();
        }

        if report.signals_created > 0 {
            report.avg_confidence = confidence_sum / report.signals_created as f64;
        }
        Ok(report)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AttentionSignalToolRuntime {
    pub owner_id: String,
    pub agent_id: String,
    pub agent_scope_id: String,
    pub user_id: String,
    pub session_id: String,
    pub window_start: String,
    pub window_end: String,
    pub extraction_window_id: String,
    #[serde(default)]
    pub extractor_version: Option<String>,
    #[serde(default)]
    pub prompt_version: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverEventArgs {
    pub title: String,
    pub phase: EventPhase,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub time_info: Option<TimeInfo>,
    #[serde(default)]
    pub participants: Vec<EntityMention>,
    pub evidence: Vec<Evidence>,
    pub confidence: f64,
    #[serde(default)]
    pub quality: Option<PartialSignalQuality>,
    #[serde(default)]
    pub stage2_hints: Option<Stage2Preparation>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverObjectObservationArgs {
    pub object: EntityMention,
    pub observation: String,
    #[serde(default)]
    pub observation_type: Option<ObservationType>,
    pub evidence: Vec<Evidence>,
    pub confidence: f64,
    #[serde(default)]
    pub quality: Option<PartialSignalQuality>,
    #[serde(default)]
    pub stage2_hints: Option<Stage2Preparation>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRelationshipArgs {
    pub subject: EntityMention,
    pub predicate: String,
    pub object: EntityMention,
    #[serde(default)]
    pub relation_type: Option<RelationshipType>,
    #[serde(default)]
    pub attitude: Option<RelationshipAttitude>,
    #[serde(default)]
    pub temporal_context: Option<TemporalContext>,
    pub evidence: Vec<Evidence>,
    pub confidence: f64,
    #[serde(default)]
    pub quality: Option<PartialSignalQuality>,
    #[serde(default)]
    pub stage2_hints: Option<Stage2Preparation>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverSkillCoverageGapArgs {
    pub title: String,
    pub task_description: String,
    #[serde(default)]
    pub related_objects: Vec<EntityMention>,
    #[serde(default)]
    pub assigned_skills: Vec<String>,
    #[serde(default)]
    pub used_skills: Vec<String>,
    #[serde(default)]
    pub critical_actions: Vec<String>,
    pub result_status: SkillTaskResultStatus,
    #[serde(default)]
    pub gap_kind: Option<SkillCoverageGapKind>,
    #[serde(default)]
    pub gap_description: Option<String>,
    #[serde(default)]
    pub trigger_context: Option<String>,
    pub evidence: Vec<Evidence>,
    pub confidence: f64,
    #[serde(default)]
    pub quality: Option<PartialSignalQuality>,
    #[serde(default)]
    pub stage2_hints: Option<Stage2Preparation>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartialSignalQuality {
    #[serde(default)]
    pub ambiguity_level: Option<AmbiguityLevel>,
    #[serde(default)]
    pub private_value_score: Option<f64>,
    #[serde(default)]
    pub user_relevance_score: Option<f64>,
    #[serde(default)]
    pub noise_risk_score: Option<f64>,
}

pub struct AgentAttentionSignalStore {
    cfg: AttentionSignalStoreConfig,
    conn: Mutex<Connection>,
}

impl AgentAttentionSignalStore {
    pub fn open(cfg: AttentionSignalStoreConfig) -> Result<Self> {
        std::fs::create_dir_all(&cfg.root)?;
        let db_path = cfg.root.join("attention_signal.sqlite");
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        ensure_schema(&conn)?;
        Ok(Self {
            cfg,
            conn: Mutex::new(conn),
        })
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    pub fn build_signal(
        &self,
        payload: AttentionSignalPayload,
        source: SignalSource,
        evidence: Vec<Evidence>,
        extraction: ExtractionInfo,
        quality: SignalQuality,
        stage2_hints: Stage2Preparation,
    ) -> Result<AttentionSignal> {
        let now = now_iso8601();
        let expires_at = if self.cfg.signal_ttl_hours == 0 {
            None
        } else {
            Some(
                (Utc::now() + ChronoDuration::hours(self.cfg.signal_ttl_hours as i64))
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
            )
        };
        let mut signal = AttentionSignal {
            id: String::new(),
            signal_type: payload.signal_type(),
            lifecycle_status: SignalLifecycleStatus::PendingStage2,
            payload,
            source,
            evidence,
            extraction,
            quality,
            stage2_hints,
            idempotency_key: String::new(),
            created_at: now.clone(),
            updated_at: now,
            expires_at,
        };
        validate_signal(&signal)?;
        let idem = build_idempotency_key(&signal)?;
        signal.idempotency_key = idem.clone();
        signal.id = format!("sig_{}", &blake3_hex(idem.as_bytes())[..24]);
        Ok(signal)
    }

    pub fn insert_signal(&self, signal: AttentionSignal) -> Result<AttentionSignalWriteResult> {
        validate_signal(&signal)?;
        let payload_json = serde_json::to_string(&signal.payload)?;
        let source_json = serde_json::to_string(&signal.source)?;
        let evidence_json = serde_json::to_string(&signal.evidence)?;
        let full_json = serde_json::to_string(&signal)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let row_count = tx.execute(
            "INSERT OR IGNORE INTO attention_signals(
                id, owner_id, agent_id, agent_scope_id, user_id, signal_type,
                lifecycle_status, payload_json, source_json, evidence_json,
                extraction_window_id, extractor_version, prompt_version,
                confidence, ambiguity_level, private_value_score, user_relevance_score,
                noise_risk_score, suggested_merge_key, suggested_initial_attention_weight,
                retention_hint, recall_candidate_hint, idempotency_key,
                created_at, updated_at, expires_at, full_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                signal.id,
                signal.source.owner_id,
                signal.source.agent_id,
                signal.source.agent_scope_id,
                signal.source.user_id,
                signal.signal_type.as_str(),
                signal.lifecycle_status.as_str(),
                payload_json,
                source_json,
                evidence_json,
                signal.extraction.extraction_window_id,
                signal.extraction.extractor_version,
                signal.extraction.prompt_version,
                signal.quality.confidence,
                signal.quality.ambiguity_level.as_str(),
                signal.quality.private_value_score,
                signal.quality.user_relevance_score,
                signal.quality.noise_risk_score,
                signal.stage2_hints.suggested_merge_key,
                signal.stage2_hints.suggested_initial_attention_weight,
                signal.stage2_hints.retention_hint.as_str(),
                if signal.stage2_hints.recall_candidate_hint { 1 } else { 0 },
                signal.idempotency_key,
                signal.created_at,
                signal.updated_at,
                signal.expires_at,
                full_json,
            ],
        )?;
        tx.commit()?;
        drop(conn);
        if row_count == 1 {
            self.append_jsonl(&signal)?;
            return Ok(AttentionSignalWriteResult {
                signal,
                inserted: true,
                duplicate: false,
            });
        }
        let existing = self
            .get_signal_by_idempotency_key(&signal.idempotency_key)?
            .ok_or_else(|| {
                AttentionSignalError::Storage("idempotency conflict without stored row".into())
            })?;
        Ok(AttentionSignalWriteResult {
            signal: existing,
            inserted: false,
            duplicate: true,
        })
    }

    pub fn list_pending_stage2(
        &self,
        agent_scope_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AttentionSignal>> {
        if agent_scope_id.trim().is_empty() {
            return Err(AttentionSignalError::InvalidInput(
                "agent_scope_id is empty".into(),
            ));
        }
        let limit = limit.unwrap_or(100).min(1000);
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT full_json FROM attention_signals
             WHERE agent_scope_id = ? AND lifecycle_status = 'pending_stage2'
             ORDER BY created_at ASC, id ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![agent_scope_id, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub fn list_pending_stage2_memory_signals(
        &self,
        agent_scope_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AttentionSignal>> {
        if agent_scope_id.trim().is_empty() {
            return Err(AttentionSignalError::InvalidInput(
                "agent_scope_id is empty".into(),
            ));
        }
        let limit = limit.unwrap_or(100).min(1000);
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT full_json FROM attention_signals
             WHERE agent_scope_id = ?
               AND lifecycle_status = 'pending_stage2'
               AND signal_type <> 'skill_coverage_gap'
             ORDER BY created_at ASC, id ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![agent_scope_id, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub fn list_pending_stage2_skill_coverage_gap_signals(
        &self,
        agent_scope_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AttentionSignal>> {
        if agent_scope_id.trim().is_empty() {
            return Err(AttentionSignalError::InvalidInput(
                "agent_scope_id is empty".into(),
            ));
        }
        let limit = limit.unwrap_or(100).min(1000);
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT full_json FROM attention_signals
             WHERE agent_scope_id = ?
               AND lifecycle_status = 'pending_stage2'
               AND signal_type = 'skill_coverage_gap'
             ORDER BY created_at ASC, id ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![agent_scope_id, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub fn get_signal(&self, id: &str) -> Result<Option<AttentionSignal>> {
        let conn = self.lock_conn()?;
        let value: Option<String> = conn
            .query_row(
                "SELECT full_json FROM attention_signals WHERE id = ?",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|raw| serde_json::from_str(&raw).map_err(AttentionSignalError::from))
            .transpose()
    }

    pub fn update_lifecycle_status(
        &self,
        id: &str,
        status: SignalLifecycleStatus,
    ) -> Result<AttentionSignal> {
        require_non_empty(id, "id")?;
        let mut signal = self
            .get_signal(id)?
            .ok_or_else(|| AttentionSignalError::NotFound(id.to_string()))?;
        signal.lifecycle_status = status;
        signal.updated_at = now_iso8601();
        let full_json = serde_json::to_string(&signal)?;
        let conn = self.lock_conn()?;
        let count = conn.execute(
            "UPDATE attention_signals
             SET lifecycle_status = ?, updated_at = ?, full_json = ?
             WHERE id = ?",
            params![
                signal.lifecycle_status.as_str(),
                signal.updated_at,
                full_json,
                id,
            ],
        )?;
        if count == 0 {
            return Err(AttentionSignalError::NotFound(id.to_string()));
        }
        Ok(signal)
    }

    pub fn create_extraction_window(
        &self,
        input: CreateExtractionWindowInput,
    ) -> Result<ExtractionWindow> {
        validate_identity(
            &input.owner_id,
            &input.agent_id,
            &input.agent_scope_id,
            &input.user_id,
            None,
        )?;
        require_non_empty(&input.window_start, "window_start")?;
        require_non_empty(&input.window_end, "window_end")?;
        let id = extraction_window_id(
            &input.agent_scope_id,
            &input.user_id,
            &input.window_start,
            &input.window_end,
        );
        let now = now_iso8601();
        let window = ExtractionWindow {
            id,
            owner_id: input.owner_id,
            agent_id: input.agent_id,
            agent_scope_id: input.agent_scope_id,
            user_id: input.user_id,
            window_start: input.window_start,
            window_end: input.window_end,
            status: "open".to_string(),
            created_at: now,
            completed_at: None,
        };
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO extraction_windows(
                id, owner_id, agent_id, agent_scope_id, user_id,
                window_start, window_end, status, created_at, completed_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
            params![
                window.id,
                window.owner_id,
                window.agent_id,
                window.agent_scope_id,
                window.user_id,
                window.window_start,
                window.window_end,
                window.status,
                window.created_at,
            ],
        )?;
        drop(conn);
        self.get_extraction_window(&window.id)?
            .ok_or_else(|| AttentionSignalError::Storage("created window missing".into()))
    }

    pub fn complete_extraction_window(&self, id: &str) -> Result<ExtractionWindow> {
        require_non_empty(id, "id")?;
        let now = now_iso8601();
        let conn = self.lock_conn()?;
        let count = conn.execute(
            "UPDATE extraction_windows SET status = 'completed', completed_at = ?
             WHERE id = ?",
            params![now, id],
        )?;
        if count == 0 {
            return Err(AttentionSignalError::NotFound(id.to_string()));
        }
        drop(conn);
        self.get_extraction_window(id)?
            .ok_or_else(|| AttentionSignalError::NotFound(id.to_string()))
    }

    pub fn get_extraction_window(&self, id: &str) -> Result<Option<ExtractionWindow>> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, owner_id, agent_id, agent_scope_id, user_id, window_start,
                    window_end, status, created_at, completed_at
             FROM extraction_windows WHERE id = ?",
            params![id],
            row_to_extraction_window,
        )
        .optional()
        .map_err(AttentionSignalError::from)
    }

    pub fn mark_scanned(&self, input: MarkScannedInput) -> Result<ScanCheckpoint> {
        validate_identity(
            &input.owner_id,
            &input.agent_id,
            &input.agent_scope_id,
            &input.user_id,
            Some(&input.session_id),
        )?;
        let id = checkpoint_id(&input.agent_scope_id, &input.user_id, &input.session_id);
        let now = now_iso8601();
        let status = input.status.unwrap_or_else(|| "up_to_date".to_string());
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO scan_checkpoints(
                id, owner_id, agent_id, agent_scope_id, user_id, session_id,
                last_scanned_round_index, last_scanned_entry_seq, last_scanned_at,
                scan_window_start, scan_window_end, status, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                last_scanned_round_index = excluded.last_scanned_round_index,
                last_scanned_entry_seq = excluded.last_scanned_entry_seq,
                last_scanned_at = excluded.last_scanned_at,
                scan_window_start = excluded.scan_window_start,
                scan_window_end = excluded.scan_window_end,
                status = excluded.status,
                updated_at = excluded.updated_at",
            params![
                id,
                input.owner_id,
                input.agent_id,
                input.agent_scope_id,
                input.user_id,
                input.session_id,
                input.last_scanned_round_index as i64,
                input.last_scanned_entry_seq as i64,
                now,
                input.scan_window_start,
                input.scan_window_end,
                status,
                now,
            ],
        )?;
        drop(conn);
        self.get_scan_checkpoint_by_id(&id)?
            .ok_or_else(|| AttentionSignalError::Storage("updated checkpoint missing".into()))
    }

    pub fn get_scan_checkpoint(
        &self,
        agent_scope_id: &str,
        user_id: &str,
        session_id: &str,
    ) -> Result<Option<ScanCheckpoint>> {
        let id = checkpoint_id(agent_scope_id, user_id, session_id);
        self.get_scan_checkpoint_by_id(&id)
    }

    fn get_scan_checkpoint_by_id(&self, id: &str) -> Result<Option<ScanCheckpoint>> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, owner_id, agent_id, agent_scope_id, user_id, session_id,
                    last_scanned_round_index, last_scanned_entry_seq, last_scanned_at,
                    scan_window_start, scan_window_end, status, updated_at
             FROM scan_checkpoints WHERE id = ?",
            params![id],
            row_to_scan_checkpoint,
        )
        .optional()
        .map_err(AttentionSignalError::from)
    }

    fn get_signal_by_idempotency_key(&self, key: &str) -> Result<Option<AttentionSignal>> {
        let conn = self.lock_conn()?;
        let value: Option<String> = conn
            .query_row(
                "SELECT full_json FROM attention_signals WHERE idempotency_key = ?",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|raw| serde_json::from_str(&raw).map_err(AttentionSignalError::from))
            .transpose()
    }

    fn append_jsonl(&self, signal: &AttentionSignal) -> Result<()> {
        let path = self.cfg.root.join("attention_signals.jsonl");
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, signal)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| AttentionSignalError::Storage(format!("connection lock poisoned: {e}")))
    }
}

#[derive(Clone)]
pub struct DiscoverEventTool {
    store: Arc<AgentAttentionSignalStore>,
    runtime: AttentionSignalToolRuntime,
}

impl DiscoverEventTool {
    pub fn new(store: Arc<AgentAttentionSignalStore>, runtime: AttentionSignalToolRuntime) -> Self {
        Self { store, runtime }
    }
}

#[async_trait]
impl TypedTool for DiscoverEventTool {
    type Args = DiscoverEventArgs;
    type Output = AttentionSignalWriteResult;

    fn name(&self) -> &str {
        TOOL_DISCOVER_EVENT
    }

    fn description(&self) -> &str {
        "Store a Stage-1 event attention signal from real session-history evidence. Use only for lifecycle events, not stable attributes or long-term memory."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM | CallingConventions::ACTION
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format_signal_summary(output)
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> std::result::Result<Self::Output, AgentToolError> {
        let signal = self.store.build_signal(
            AttentionSignalPayload::Event(EventSignalPayload {
                title: args.title,
                phase: args.phase,
                description: empty_to_none(args.description),
                time_info: args.time_info.unwrap_or_default(),
                participants: args.participants,
            }),
            build_source(&self.runtime, &args.evidence)?,
            args.evidence,
            build_extraction(&self.runtime),
            build_quality(args.confidence, args.quality)?,
            args.stage2_hints
                .unwrap_or_else(|| default_stage2_hints(SignalType::Event)),
        )?;
        Ok(self.store.insert_signal(signal)?)
    }
}

#[derive(Clone)]
pub struct DiscoverObjectObservationTool {
    store: Arc<AgentAttentionSignalStore>,
    runtime: AttentionSignalToolRuntime,
}

impl DiscoverObjectObservationTool {
    pub fn new(store: Arc<AgentAttentionSignalStore>, runtime: AttentionSignalToolRuntime) -> Self {
        Self { store, runtime }
    }
}

#[async_trait]
impl TypedTool for DiscoverObjectObservationTool {
    type Args = DiscoverObjectObservationArgs;
    type Output = AttentionSignalWriteResult;

    fn name(&self) -> &str {
        TOOL_DISCOVER_OBJECT_OBSERVATION
    }

    fn description(&self) -> &str {
        "Store a Stage-1 object observation signal from real session-history evidence. Use for one concrete private object plus one observation."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM | CallingConventions::ACTION
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format_signal_summary(output)
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> std::result::Result<Self::Output, AgentToolError> {
        let signal = self.store.build_signal(
            AttentionSignalPayload::ObjectObservation(ObjectObservationSignalPayload {
                object: args.object,
                observation: args.observation,
                observation_type: args.observation_type,
            }),
            build_source(&self.runtime, &args.evidence)?,
            args.evidence,
            build_extraction(&self.runtime),
            build_quality(args.confidence, args.quality)?,
            args.stage2_hints
                .unwrap_or_else(|| default_stage2_hints(SignalType::ObjectObservation)),
        )?;
        Ok(self.store.insert_signal(signal)?)
    }
}

#[derive(Clone)]
pub struct DiscoverRelationshipTool {
    store: Arc<AgentAttentionSignalStore>,
    runtime: AttentionSignalToolRuntime,
}

impl DiscoverRelationshipTool {
    pub fn new(store: Arc<AgentAttentionSignalStore>, runtime: AttentionSignalToolRuntime) -> Self {
        Self { store, runtime }
    }
}

#[async_trait]
impl TypedTool for DiscoverRelationshipTool {
    type Args = DiscoverRelationshipArgs;
    type Output = AttentionSignalWriteResult;

    fn name(&self) -> &str {
        TOOL_DISCOVER_RELATIONSHIP
    }

    fn description(&self) -> &str {
        "Store a Stage-1 relationship attention signal from real session-history evidence. Use for candidate edges, attitudes, preferences, ownership, responsibility, contact info, or dependencies."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM | CallingConventions::ACTION
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format_signal_summary(output)
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> std::result::Result<Self::Output, AgentToolError> {
        let signal = self.store.build_signal(
            AttentionSignalPayload::Relationship(RelationshipSignalPayload {
                subject: args.subject,
                predicate: args.predicate,
                object: args.object,
                relation_type: args.relation_type,
                attitude: args.attitude,
                temporal_context: args.temporal_context,
            }),
            build_source(&self.runtime, &args.evidence)?,
            args.evidence,
            build_extraction(&self.runtime),
            build_quality(args.confidence, args.quality)?,
            args.stage2_hints
                .unwrap_or_else(|| default_stage2_hints(SignalType::Relationship)),
        )?;
        Ok(self.store.insert_signal(signal)?)
    }
}

#[derive(Clone)]
pub struct DiscoverSkillCoverageGapTool {
    store: Arc<AgentAttentionSignalStore>,
    runtime: AttentionSignalToolRuntime,
}

impl DiscoverSkillCoverageGapTool {
    pub fn new(store: Arc<AgentAttentionSignalStore>, runtime: AttentionSignalToolRuntime) -> Self {
        Self { store, runtime }
    }
}

#[async_trait]
impl TypedTool for DiscoverSkillCoverageGapTool {
    type Args = DiscoverSkillCoverageGapArgs;
    type Output = AttentionSignalWriteResult;

    fn name(&self) -> &str {
        TOOL_DISCOVER_SKILL_COVERAGE_GAP
    }

    fn description(&self) -> &str {
        "Store a Stage-1 skill coverage gap signal from real session-history evidence. Use when a task succeeded or progressed through ad hoc actions because no suitable Skill covered it."
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM | CallingConventions::ACTION
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format_signal_summary(output)
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> std::result::Result<Self::Output, AgentToolError> {
        let signal = self.store.build_signal(
            AttentionSignalPayload::SkillCoverageGap(SkillCoverageGapSignalPayload {
                title: args.title,
                task_description: args.task_description,
                related_objects: args.related_objects,
                assigned_skills: args.assigned_skills,
                used_skills: args.used_skills,
                critical_actions: args.critical_actions,
                result_status: args.result_status,
                gap_kind: args.gap_kind,
                gap_description: empty_to_none(args.gap_description),
                trigger_context: empty_to_none(args.trigger_context),
            }),
            build_source(&self.runtime, &args.evidence)?,
            args.evidence,
            build_extraction(&self.runtime),
            build_quality(args.confidence, args.quality)?,
            args.stage2_hints
                .unwrap_or_else(|| default_stage2_hints(SignalType::SkillCoverageGap)),
        )?;
        Ok(self.store.insert_signal(signal)?)
    }
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS attention_signals (
            id                                  TEXT PRIMARY KEY,
            owner_id                            TEXT NOT NULL,
            agent_id                            TEXT NOT NULL,
            agent_scope_id                      TEXT NOT NULL,
            user_id                             TEXT NOT NULL,
            signal_type                         TEXT NOT NULL,
            lifecycle_status                    TEXT NOT NULL,
            payload_json                        TEXT NOT NULL,
            source_json                         TEXT NOT NULL,
            evidence_json                       TEXT NOT NULL,
            extraction_window_id                TEXT NOT NULL,
            extractor_version                   TEXT NOT NULL,
            prompt_version                      TEXT NOT NULL,
            confidence                          REAL NOT NULL,
            ambiguity_level                     TEXT,
            private_value_score                 REAL,
            user_relevance_score                REAL,
            noise_risk_score                    REAL,
            suggested_merge_key                 TEXT,
            suggested_initial_attention_weight  REAL,
            retention_hint                      TEXT,
            recall_candidate_hint               INTEGER,
            idempotency_key                     TEXT UNIQUE,
            created_at                          TEXT NOT NULL,
            updated_at                          TEXT NOT NULL,
            expires_at                          TEXT,
            full_json                           TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_attention_signals_stage2
            ON attention_signals(agent_scope_id, lifecycle_status, created_at);
         CREATE INDEX IF NOT EXISTS idx_attention_signals_source
            ON attention_signals(agent_scope_id, user_id, extraction_window_id);
         CREATE TABLE IF NOT EXISTS scan_checkpoints (
            id                         TEXT PRIMARY KEY,
            owner_id                   TEXT NOT NULL,
            agent_id                   TEXT NOT NULL,
            agent_scope_id             TEXT NOT NULL,
            user_id                    TEXT NOT NULL,
            session_id                 TEXT NOT NULL,
            last_scanned_round_index   INTEGER,
            last_scanned_entry_seq     INTEGER,
            last_scanned_at            TEXT,
            scan_window_start          TEXT,
            scan_window_end            TEXT,
            status                     TEXT NOT NULL,
            updated_at                 TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_scan_checkpoints_scope_session
            ON scan_checkpoints(agent_scope_id, user_id, session_id);
         CREATE TABLE IF NOT EXISTS extraction_windows (
            id               TEXT PRIMARY KEY,
            owner_id         TEXT NOT NULL,
            agent_id         TEXT NOT NULL,
            agent_scope_id   TEXT NOT NULL,
            user_id          TEXT NOT NULL,
            window_start     TEXT NOT NULL,
            window_end       TEXT NOT NULL,
            status           TEXT NOT NULL,
            created_at       TEXT NOT NULL,
            completed_at     TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_extraction_windows_scope_time
            ON extraction_windows(agent_scope_id, user_id, window_start, window_end);",
    )?;
    Ok(())
}

fn validate_signal(signal: &AttentionSignal) -> Result<()> {
    if signal.signal_type != signal.payload.signal_type() {
        return Err(AttentionSignalError::InvalidInput(
            "signal_type does not match payload".into(),
        ));
    }
    if signal.lifecycle_status != SignalLifecycleStatus::PendingStage2 {
        return Err(AttentionSignalError::InvalidInput(
            "stage-1 can only create pending_stage2 signals".into(),
        ));
    }
    validate_identity(
        &signal.source.owner_id,
        &signal.source.agent_id,
        &signal.source.agent_scope_id,
        &signal.source.user_id,
        Some(&signal.source.session_id),
    )?;
    if signal.source.source_type != "session_history" {
        return Err(AttentionSignalError::InvalidInput(
            "source_type must be session_history".into(),
        ));
    }
    if signal.source.round_refs.is_empty() {
        return Err(AttentionSignalError::InvalidInput(
            "source.round_refs cannot be empty".into(),
        ));
    }
    validate_evidence(&signal.evidence)?;
    validate_confidence(signal.quality.confidence, "quality.confidence")?;
    validate_extraction(&signal.extraction)?;
    validate_stage2_hints(&signal.stage2_hints)?;
    match &signal.payload {
        AttentionSignalPayload::Event(payload) => validate_event_payload(payload),
        AttentionSignalPayload::ObjectObservation(payload) => validate_object_payload(payload),
        AttentionSignalPayload::Relationship(payload) => validate_relationship_payload(payload),
        AttentionSignalPayload::SkillCoverageGap(payload) => {
            validate_skill_coverage_gap_payload(payload)
        }
    }
}

fn validate_event_payload(payload: &EventSignalPayload) -> Result<()> {
    require_non_empty(&payload.title, "title")?;
    Ok(())
}

fn validate_object_payload(payload: &ObjectObservationSignalPayload) -> Result<()> {
    validate_entity(&payload.object, "object")?;
    require_non_empty(&payload.observation, "observation")?;
    if is_generic_object_word(&payload.object.mention_text) {
        return Err(AttentionSignalError::InvalidInput(
            "object.mention_text is too generic".into(),
        ));
    }
    if payload.object.is_public_entity == Some(true)
        && payload.object.is_user_private_entity != Some(true)
        && !matches!(
            payload.observation_type,
            Some(ObservationType::Preference | ObservationType::Problem | ObservationType::Usage)
        )
    {
        return Err(AttentionSignalError::InvalidInput(
            "public object observations need private user value".into(),
        ));
    }
    Ok(())
}

fn validate_relationship_payload(payload: &RelationshipSignalPayload) -> Result<()> {
    validate_entity(&payload.subject, "subject")?;
    validate_entity(&payload.object, "object")?;
    require_non_empty(&payload.predicate, "predicate")?;
    if payload
        .subject
        .mention_text
        .trim()
        .eq_ignore_ascii_case(payload.object.mention_text.trim())
    {
        return Err(AttentionSignalError::InvalidInput(
            "subject and object cannot be the same mention".into(),
        ));
    }
    Ok(())
}

fn validate_skill_coverage_gap_payload(payload: &SkillCoverageGapSignalPayload) -> Result<()> {
    require_non_empty(&payload.title, "title")?;
    require_non_empty(&payload.task_description, "task_description")?;
    for object in &payload.related_objects {
        validate_entity(object, "related_objects")?;
    }
    for skill in &payload.assigned_skills {
        require_non_empty(skill, "assigned_skills")?;
    }
    for skill in &payload.used_skills {
        require_non_empty(skill, "used_skills")?;
    }
    for action in &payload.critical_actions {
        require_non_empty(action, "critical_actions")?;
    }
    if payload.assigned_skills.is_empty()
        && payload.used_skills.is_empty()
        && payload.critical_actions.is_empty()
        && payload
            .gap_description
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        && !matches!(
            payload.gap_kind,
            Some(
                SkillCoverageGapKind::NoSkillAssigned
                    | SkillCoverageGapKind::SuccessWithoutSkill
                    | SkillCoverageGapKind::AssignedSkillUnused
                    | SkillCoverageGapKind::MissingFallback
                    | SkillCoverageGapKind::RepeatedManualPath
                    | SkillCoverageGapKind::ExistingSkillMismatch
            )
        )
    {
        return Err(AttentionSignalError::InvalidInput(
            "skill coverage gap needs explicit missing coverage or ad hoc execution evidence"
                .into(),
        ));
    }
    Ok(())
}

fn validate_entity(entity: &EntityMention, field: &str) -> Result<()> {
    require_non_empty(&entity.mention_text, &format!("{field}.mention_text"))?;
    for alias in &entity.alias_candidates {
        require_non_empty(alias, &format!("{field}.alias_candidates"))?;
    }
    Ok(())
}

fn validate_evidence(evidence: &[Evidence]) -> Result<()> {
    if evidence.is_empty() {
        return Err(AttentionSignalError::InvalidInput(
            "evidence cannot be empty".into(),
        ));
    }
    for item in evidence {
        require_non_empty(&item.text_excerpt, "evidence.text_excerpt")?;
        if let (Some(start), Some(end)) = (item.start_offset, item.end_offset) {
            if start > end {
                return Err(AttentionSignalError::InvalidInput(
                    "evidence start_offset cannot exceed end_offset".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_extraction(extraction: &ExtractionInfo) -> Result<()> {
    require_non_empty(
        &extraction.extractor_version,
        "extraction.extractor_version",
    )?;
    require_non_empty(&extraction.prompt_version, "extraction.prompt_version")?;
    require_non_empty(&extraction.extracted_at, "extraction.extracted_at")?;
    require_non_empty(
        &extraction.extraction_window_id,
        "extraction.extraction_window_id",
    )?;
    Ok(())
}

fn validate_stage2_hints(hints: &Stage2Preparation) -> Result<()> {
    if let Some(weight) = hints.suggested_initial_attention_weight {
        validate_confidence(weight, "stage2_hints.suggested_initial_attention_weight")?;
    }
    for candidate in &hints.canonicalization_candidates {
        require_non_empty(
            &candidate.mention_text,
            "stage2_hints.canonicalization_candidates.mention_text",
        )?;
        validate_confidence(
            candidate.confidence,
            "stage2_hints.canonicalization_candidates.confidence",
        )?;
    }
    Ok(())
}

fn validate_confidence(value: f64, field: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(AttentionSignalError::InvalidInput(format!(
            "{field} must be between 0 and 1"
        )));
    }
    Ok(())
}

fn validate_identity(
    owner_id: &str,
    agent_id: &str,
    agent_scope_id: &str,
    user_id: &str,
    session_id: Option<&str>,
) -> Result<()> {
    require_non_empty(owner_id, "owner_id")?;
    require_non_empty(agent_id, "agent_id")?;
    require_non_empty(agent_scope_id, "agent_scope_id")?;
    require_non_empty(user_id, "user_id")?;
    if let Some(session_id) = session_id {
        require_non_empty(session_id, "session_id")?;
    }
    Ok(())
}

fn require_non_empty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(AttentionSignalError::InvalidInput(format!(
            "{field} cannot be empty"
        )));
    }
    Ok(())
}

fn build_source(
    runtime: &AttentionSignalToolRuntime,
    evidence: &[Evidence],
) -> Result<SignalSource> {
    validate_identity(
        &runtime.owner_id,
        &runtime.agent_id,
        &runtime.agent_scope_id,
        &runtime.user_id,
        Some(&runtime.session_id),
    )?;
    require_non_empty(&runtime.window_start, "window_start")?;
    require_non_empty(&runtime.window_end, "window_end")?;
    let mut refs: BTreeMap<(u64, u64, EntryKind), RoundEntryRef> = BTreeMap::new();
    for item in evidence {
        refs.insert(
            (item.round_index, item.entry_seq, item.entry_kind),
            RoundEntryRef {
                round_index: item.round_index,
                entry_seq: item.entry_seq,
                entry_kind: Some(item.entry_kind),
                llm_call: None,
            },
        );
    }
    Ok(SignalSource {
        owner_id: runtime.owner_id.clone(),
        agent_id: runtime.agent_id.clone(),
        agent_scope_id: runtime.agent_scope_id.clone(),
        user_id: runtime.user_id.clone(),
        session_id: runtime.session_id.clone(),
        round_refs: refs.into_values().collect(),
        window_start: runtime.window_start.clone(),
        window_end: runtime.window_end.clone(),
        source_type: "session_history".to_string(),
    })
}

fn last_entry_ref(entries: &[AttentionSignalHistoryEntry]) -> (u64, u64) {
    entries
        .iter()
        .max_by_key(|entry| (entry.round_index, entry.entry_seq))
        .map(|entry| (entry.round_index, entry.entry_seq))
        .unwrap_or((0, 0))
}

fn build_extraction(runtime: &AttentionSignalToolRuntime) -> ExtractionInfo {
    ExtractionInfo {
        extractor_version: runtime
            .extractor_version
            .clone()
            .unwrap_or_else(|| DEFAULT_EXTRACTOR_VERSION.to_string()),
        prompt_version: runtime
            .prompt_version
            .clone()
            .unwrap_or_else(|| DEFAULT_PROMPT_VERSION.to_string()),
        model_name: runtime.model_name.clone(),
        extracted_at: now_iso8601(),
        extraction_window_id: runtime.extraction_window_id.clone(),
    }
}

fn build_quality(confidence: f64, partial: Option<PartialSignalQuality>) -> Result<SignalQuality> {
    validate_confidence(confidence, "confidence")?;
    let partial = partial.unwrap_or(PartialSignalQuality {
        ambiguity_level: None,
        private_value_score: None,
        user_relevance_score: None,
        noise_risk_score: None,
    });
    if let Some(value) = partial.private_value_score {
        validate_confidence(value, "private_value_score")?;
    }
    if let Some(value) = partial.user_relevance_score {
        validate_confidence(value, "user_relevance_score")?;
    }
    if let Some(value) = partial.noise_risk_score {
        validate_confidence(value, "noise_risk_score")?;
    }
    Ok(SignalQuality {
        confidence,
        ambiguity_level: partial.ambiguity_level.unwrap_or_else(|| {
            if confidence >= 0.8 {
                AmbiguityLevel::Low
            } else if confidence >= 0.6 {
                AmbiguityLevel::Medium
            } else {
                AmbiguityLevel::High
            }
        }),
        private_value_score: partial.private_value_score,
        user_relevance_score: partial.user_relevance_score,
        noise_risk_score: partial.noise_risk_score,
    })
}

fn default_stage2_hints(signal_type: SignalType) -> Stage2Preparation {
    let suggested_action = match signal_type {
        SignalType::Event => SuggestedAction::ConsiderEvent,
        SignalType::ObjectObservation => SuggestedAction::ConsiderObject,
        SignalType::Relationship => SuggestedAction::ConsiderRelationship,
        SignalType::SkillCoverageGap => SuggestedAction::ConsiderSkillCoverageGap,
    };
    Stage2Preparation {
        suggested_merge_key: None,
        canonicalization_candidates: Vec::new(),
        possible_memory_path: Vec::new(),
        suggested_initial_attention_weight: None,
        suggested_action,
        retention_hint: RetentionHint::RequiresMoreEvidence,
        privacy_scope: PrivacyScope::Unknown,
        recall_candidate_hint: false,
    }
}

fn build_idempotency_key(signal: &AttentionSignal) -> Result<String> {
    let payload_core = match &signal.payload {
        AttentionSignalPayload::Event(payload) => Json::String(format!(
            "{}\n{:?}\n{}",
            payload.title.trim(),
            payload.phase,
            payload.description.as_deref().unwrap_or("").trim()
        )),
        AttentionSignalPayload::ObjectObservation(payload) => Json::String(format!(
            "{}\n{}",
            payload.object.mention_text.trim(),
            payload.observation.trim()
        )),
        AttentionSignalPayload::Relationship(payload) => Json::String(format!(
            "{}\n{}\n{}",
            payload.subject.mention_text.trim(),
            payload.predicate.trim(),
            payload.object.mention_text.trim()
        )),
        AttentionSignalPayload::SkillCoverageGap(payload) => Json::String(format!(
            "{}\n{}\n{}\n{}",
            payload.title.trim(),
            payload.task_description.trim(),
            payload.gap_description.as_deref().unwrap_or("").trim(),
            payload.critical_actions.join("\n")
        )),
    };
    let mut evidence_refs: Vec<_> = signal
        .evidence
        .iter()
        .map(|e| {
            Json::Array(vec![
                Json::from(e.round_index),
                Json::from(e.entry_seq),
                Json::String(format!("{:?}", e.entry_kind)),
            ])
        })
        .collect();
    evidence_refs.sort_by_key(canonical_json);
    let value = serde_json::json!({
        "agent_scope_id": signal.source.agent_scope_id,
        "user_id": signal.source.user_id,
        "session_id": signal.source.session_id,
        "signal_type": signal.signal_type.as_str(),
        "evidence": evidence_refs,
        "payload_core": payload_core,
    });
    Ok(format!(
        "blake3:{}",
        blake3_hex(canonical_json(&value).as_bytes())
    ))
}

fn row_to_extraction_window(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtractionWindow> {
    Ok(ExtractionWindow {
        id: row.get(0)?,
        owner_id: row.get(1)?,
        agent_id: row.get(2)?,
        agent_scope_id: row.get(3)?,
        user_id: row.get(4)?,
        window_start: row.get(5)?,
        window_end: row.get(6)?,
        status: row.get(7)?,
        created_at: row.get(8)?,
        completed_at: row.get(9)?,
    })
}

fn row_to_scan_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanCheckpoint> {
    let round_index: Option<i64> = row.get(6)?;
    let entry_seq: Option<i64> = row.get(7)?;
    Ok(ScanCheckpoint {
        id: row.get(0)?,
        owner_id: row.get(1)?,
        agent_id: row.get(2)?,
        agent_scope_id: row.get(3)?,
        user_id: row.get(4)?,
        session_id: row.get(5)?,
        last_scanned_round_index: round_index.and_then(|v| u64::try_from(v).ok()),
        last_scanned_entry_seq: entry_seq.and_then(|v| u64::try_from(v).ok()),
        last_scanned_at: row.get(8)?,
        scan_window_start: row.get(9)?,
        scan_window_end: row.get(10)?,
        status: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn extraction_window_id(
    agent_scope_id: &str,
    user_id: &str,
    window_start: &str,
    window_end: &str,
) -> String {
    let raw = format!("{agent_scope_id}\n{user_id}\n{window_start}\n{window_end}");
    format!("win_{}", &blake3_hex(raw.as_bytes())[..24])
}

fn checkpoint_id(agent_scope_id: &str, user_id: &str, session_id: &str) -> String {
    let raw = format!("{agent_scope_id}\n{user_id}\n{session_id}");
    format!("ckp_{}", &blake3_hex(raw.as_bytes())[..24])
}

fn is_generic_object_word(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    matches!(
        normalized.as_str(),
        "object"
            | "thing"
            | "something"
            | "someone"
            | "person"
            | "project"
            | "device"
            | "对象"
            | "某人"
            | "某个对象"
            | "东西"
    )
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn format_signal_summary(output: &AttentionSignalWriteResult) -> String {
    let action = if output.inserted {
        "stored"
    } else {
        "duplicate"
    };
    format!(
        "{} {} signal {}",
        action,
        output.signal.signal_type.as_str(),
        output.signal.id
    )
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn canonical_json(v: &Json) -> String {
    let canon = canonicalize(v);
    serde_json::to_string(&canon).unwrap_or_default()
}

fn canonicalize(v: &Json) -> Json {
    match v {
        Json::Object(map) => {
            let mut sorted: BTreeMap<String, Json> = BTreeMap::new();
            for (k, val) in map {
                sorted.insert(k.clone(), canonicalize(val));
            }
            let m: serde_json::Map<String, Json> = sorted.into_iter().collect();
            Json::Object(m)
        }
        Json::Array(values) => Json::Array(values.iter().map(canonicalize).collect()),
        _ => v.clone(),
    }
}

fn random_window_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    blake3_hex(nanos.as_bytes())[..8].to_string()
}

pub fn new_extraction_window_id() -> String {
    format!("win_{}", random_window_suffix())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::{AgentTool, SessionRuntimeContext, TypedToolHandle};

    fn open_tmp() -> (TempDir, Arc<AgentAttentionSignalStore>) {
        let tmp = TempDir::new().unwrap();
        let store = AgentAttentionSignalStore::open(AttentionSignalStoreConfig::new(tmp.path()))
            .expect("open store");
        (tmp, Arc::new(store))
    }

    fn runtime() -> AttentionSignalToolRuntime {
        AttentionSignalToolRuntime {
            owner_id: "owner".into(),
            agent_id: "agent".into(),
            agent_scope_id: "scope".into(),
            user_id: "user".into(),
            session_id: "session".into(),
            window_start: "2026-06-03T08:00:00Z".into(),
            window_end: "2026-06-03T12:00:00Z".into(),
            extraction_window_id: "win_test".into(),
            extractor_version: None,
            prompt_version: None,
            model_name: None,
        }
    }

    fn evidence() -> Vec<Evidence> {
        vec![Evidence {
            round_index: 1,
            entry_seq: 2,
            entry_kind: EntryKind::Message,
            role: EvidenceRole::User,
            text_excerpt: "我的笔记本电脑最近很慢。".into(),
            start_offset: None,
            end_offset: None,
            created_at: Some("2026-06-03T08:10:00Z".into()),
        }]
    }

    fn source() -> SignalSource {
        build_source(&runtime(), &evidence()).unwrap()
    }

    fn extraction() -> ExtractionInfo {
        build_extraction(&runtime())
    }

    #[test]
    fn duplicate_insert_returns_existing_signal() {
        let (_tmp, store) = open_tmp();
        let payload = AttentionSignalPayload::ObjectObservation(ObjectObservationSignalPayload {
            object: EntityMention {
                mention_text: "用户的笔记本电脑".into(),
                entity_type: EntityType::Device,
                canonical_id_candidate: None,
                alias_candidates: Vec::new(),
                is_public_entity: Some(false),
                is_user_private_entity: Some(true),
                uniqueness_hint: None,
            },
            observation: "用户认为自己的笔记本电脑最近很慢。".into(),
            observation_type: Some(ObservationType::Problem),
        });
        let signal = store
            .build_signal(
                payload.clone(),
                source(),
                evidence(),
                extraction(),
                build_quality(0.95, None).unwrap(),
                default_stage2_hints(SignalType::ObjectObservation),
            )
            .unwrap();
        let first = store.insert_signal(signal.clone()).unwrap();
        let second = store.insert_signal(signal).unwrap();
        assert!(first.inserted);
        assert!(second.duplicate);
        assert_eq!(first.signal.id, second.signal.id);
        assert_eq!(store.list_pending_stage2("scope", None).unwrap().len(), 1);
    }

    #[test]
    fn rejects_relationship_with_same_subject_and_object() {
        let payload = RelationshipSignalPayload {
            subject: EntityMention {
                mention_text: "Bob".into(),
                entity_type: EntityType::Person,
                canonical_id_candidate: None,
                alias_candidates: Vec::new(),
                is_public_entity: None,
                is_user_private_entity: Some(true),
                uniqueness_hint: None,
            },
            predicate: "is Bob".into(),
            object: EntityMention {
                mention_text: "Bob".into(),
                entity_type: EntityType::Person,
                canonical_id_candidate: None,
                alias_candidates: Vec::new(),
                is_public_entity: None,
                is_user_private_entity: Some(true),
                uniqueness_hint: None,
            },
            attitude: None,
            relation_type: Some(RelationshipType::Unknown),
            temporal_context: None,
        };
        let (_tmp, store) = open_tmp();
        let err = store
            .build_signal(
                AttentionSignalPayload::Relationship(payload),
                source(),
                evidence(),
                extraction(),
                build_quality(0.8, None).unwrap(),
                default_stage2_hints(SignalType::Relationship),
            )
            .unwrap_err();
        assert!(matches!(err, AttentionSignalError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn discover_event_tool_stores_signal() {
        let (_tmp, store) = open_tmp();
        let tool = TypedToolHandle::with_null_host(DiscoverEventTool::new(store, runtime()));
        let ctx = SessionRuntimeContext {
            trace_id: "trace".into(),
            agent_name: "agent".into(),
            behavior: "test".into(),
            step_idx: 0,
            wakeup_id: String::new(),
            session_id: "session".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        };
        let args = json!({
            "title": "Self Improve 第一阶段设计讨论",
            "phase": "active",
            "time_info": {"is_time_precise": false},
            "evidence": [{
                "round_index": 1,
                "entry_seq": 1,
                "entry_kind": "message",
                "role": "user",
                "text_excerpt": "我们现在先讨论 Self Improve 第一阶段。"
            }],
            "confidence": 0.91
        });
        let result = tool.call(&ctx, args).await.unwrap();
        assert_eq!(result.tool.as_deref(), Some(TOOL_DISCOVER_EVENT));
        assert_eq!(result.details["inserted"], true);
        assert_eq!(
            result.details["signal"]["lifecycle_status"],
            "pending_stage2"
        );
    }

    #[tokio::test]
    async fn discover_skill_coverage_gap_tool_stores_signal_but_memory_query_excludes_it() {
        let (_tmp, store) = open_tmp();
        let tool = TypedToolHandle::with_null_host(DiscoverSkillCoverageGapTool::new(
            store.clone(),
            runtime(),
        ));
        let ctx = SessionRuntimeContext {
            trace_id: "trace".into(),
            agent_name: "agent".into(),
            behavior: "test".into(),
            step_idx: 0,
            wakeup_id: String::new(),
            session_id: "session".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        };
        let args = json!({
            "title": "Log parser coverage gap",
            "task_description": "Analyze unknown-format logs and identify the failure pattern.",
            "assigned_skills": [],
            "used_skills": [],
            "critical_actions": [
                "Explored log directory structure",
                "Wrote a temporary parser",
                "Grouped failures by signature"
            ],
            "result_status": "success",
            "gap_kind": "success_without_skill",
            "gap_description": "The task succeeded through a repeatable ad hoc log-analysis path without Skill coverage.",
            "evidence": [{
                "round_index": 1,
                "entry_seq": 1,
                "entry_kind": "message",
                "role": "assistant",
                "text_excerpt": "I wrote a temporary parser and grouped the failures by signature."
            }],
            "confidence": 0.78
        });
        let result = tool.call(&ctx, args).await.unwrap();
        assert_eq!(
            result.tool.as_deref(),
            Some(TOOL_DISCOVER_SKILL_COVERAGE_GAP)
        );
        assert_eq!(
            result.details["signal"]["signal_type"],
            "skill_coverage_gap"
        );
        assert_eq!(store.list_pending_stage2("scope", None).unwrap().len(), 1);
        assert_eq!(
            store
                .list_pending_stage2_memory_signals("scope", None)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            store
                .list_pending_stage2_skill_coverage_gap_signals("scope", None)
                .unwrap()
                .len(),
            1
        );
    }
}
