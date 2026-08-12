use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const SKILLS_DIR: &str = "skills";
pub const SOURCES_DIR: &str = "sources";
pub const CANDIDATES_DIR: &str = "candidates";
pub const ACTIVE_DIR: &str = "active";
pub const ARCHIVED_DIR: &str = "archived";
pub const INDEXES_DIR: &str = "indexes";
pub const USAGE_DIR: &str = "usage";
pub const BACKUPS_DIR: &str = "backups";
pub const SOURCE_META_FILE: &str = "source.json";
pub const SKILL_META_FILE: &str = "skill.yaml";
pub const SKILL_BODY_FILE: &str = "SKILL.md";
pub const SELECTION_FILE: &str = "selection.json";
pub const USAGE_LOG_FILE: &str = "usage.jsonl";
pub const AUDIT_LOG_FILE: &str = "audit.jsonl";
pub const BLOCKLIST_LOG_FILE: &str = "blocklist.jsonl";
pub const PROPOSALS_LOG_FILE: &str = "proposals.jsonl";
pub const LOCK_FILE: &str = ".lock";
pub const DEFAULT_MAX_HINTS: usize = 10;
pub const DEFAULT_RENDER_TOKEN_BUDGET: usize = 12_000;
pub const DEFAULT_MAX_SELECTED_SKILLS: usize = 3;

#[derive(Debug, Error)]
pub enum SkillsMgrError {
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("already_exists: {0}")]
    AlreadyExists(String),
    #[error("invalid_input: {0}")]
    InvalidInput(String),
    #[error("permission_denied: {0}")]
    PermissionDenied(String),
    #[error("version_conflict: {0}")]
    VersionConflict(String),
    #[error("lock_contention: {0}")]
    LockTimeout(String),
    #[error("storage_error: {0}")]
    Storage(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl SkillsMgrError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::AlreadyExists(_) => "already_exists",
            Self::InvalidInput(_) => "invalid_input",
            Self::PermissionDenied(_) => "permission_denied",
            Self::VersionConflict(_) => "version_conflict",
            Self::LockTimeout(_) => "lock_contention",
            Self::Storage(_) | Self::Io(_) | Self::Json(_) => "storage_error",
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            Self::InvalidInput(_) => 2,
            Self::NotFound(_) => 3,
            Self::PermissionDenied(_) => 4,
            Self::VersionConflict(_) => 5,
            Self::LockTimeout(_) => 6,
            _ => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, SkillsMgrError>;

#[derive(Clone, Debug)]
pub struct SkillsMgrConfig {
    pub root: PathBuf,
    pub lock_timeout: Duration,
}

impl SkillsMgrConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceType {
    SystemBuiltin,
    HubInstalled,
    TeamShared,
    OwnerInstalled,
    AgentDiscovered,
    AgentCurated,
    ExternalReference,
}

impl SkillSourceType {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "system_builtin" => Self::SystemBuiltin,
            "hub_installed" => Self::HubInstalled,
            "team_shared" => Self::TeamShared,
            "owner_installed" => Self::OwnerInstalled,
            "agent_discovered" => Self::AgentDiscovered,
            "agent_curated" => Self::AgentCurated,
            "external_reference" => Self::ExternalReference,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown source type `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillType {
    DataAcquisition,
    Exploration,
    Delivery,
    Workflow,
    Paradigm,
}

impl SkillType {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "data_acquisition" => Self::DataAcquisition,
            "exploration" => Self::Exploration,
            "delivery" => Self::Delivery,
            "workflow" => Self::Workflow,
            "paradigm" => Self::Paradigm,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown skill type `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerScope {
    Agent,
    Owner,
    Team,
    Zone,
}

impl OwnerScope {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "agent" => Self::Agent,
            "owner" => Self::Owner,
            "team" => Self::Team,
            "zone" => Self::Zone,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown owner scope `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Candidate,
    Verifying,
    Active,
    Preferred,
    NeedsReverification,
    Stale,
    Archived,
    Rejected,
    Blocked,
    Restored,
}

impl LifecycleState {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "candidate" => Self::Candidate,
            "verifying" => Self::Verifying,
            "active" => Self::Active,
            "preferred" => Self::Preferred,
            "needs_reverification" => Self::NeedsReverification,
            "stale" => Self::Stale,
            "archived" => Self::Archived,
            "rejected" => Self::Rejected,
            "blocked" => Self::Blocked,
            "restored" => Self::Restored,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown lifecycle state `{other}`"
                )))
            }
        })
    }

    pub fn is_loadable(self) -> bool {
        matches!(
            self,
            Self::Active | Self::Preferred | Self::NeedsReverification | Self::Stale
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Unverified,
    StaticChecked,
    Simulated,
    ManualVerified,
    UsageVerified,
    Failed,
    Expired,
    Unsafe,
}

impl VerificationStatus {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "unverified" => Self::Unverified,
            "static_checked" => Self::StaticChecked,
            "simulated" => Self::Simulated,
            "manual_verified" => Self::ManualVerified,
            "usage_verified" => Self::UsageVerified,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            "unsafe" => Self::Unsafe,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown verification status `{other}`"
                )))
            }
        })
    }

    fn strength(self) -> u8 {
        match self {
            Self::Unverified => 0,
            Self::Failed | Self::Expired | Self::Unsafe => 0,
            Self::StaticChecked => 1,
            Self::Simulated => 2,
            Self::ManualVerified => 3,
            Self::UsageVerified => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "critical" => Self::Critical,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown risk level `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageMode {
    Loaded,
    Referenced,
    Applied,
    RejectedAfterView,
}

impl UsageMode {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "loaded" => Self::Loaded,
            "referenced" => Self::Referenced,
            "applied" => Self::Applied,
            "rejected_after_view" => Self::RejectedAfterView,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown usage mode `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTaskResult {
    Success,
    Partial,
    Failed,
    NotApplicable,
}

impl SkillTaskResult {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "success" => Self::Success,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "not_applicable" => Self::NotApplicable,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown task result `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserFeedback {
    Accepted,
    Rejected,
    Corrected,
}

impl UserFeedback {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw.trim() {
            "accepted" => Self::Accepted,
            "rejected" => Self::Rejected,
            "corrected" => Self::Corrected,
            other => {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "unknown user feedback `{other}`"
                )))
            }
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillSourceRef {
    pub r#type: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillObjectRef {
    pub object_id: String,
    pub role: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillTrigger {
    #[serde(default)]
    pub when_to_use: Vec<String>,
    #[serde(default)]
    pub intent_tags: Vec<String>,
    #[serde(default)]
    pub object_types: Vec<String>,
    #[serde(default)]
    pub negative_triggers: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillRequires {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub agent_tools: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub optional_tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRisk {
    pub risk_level: RiskLevel,
    #[serde(default)]
    pub side_effects: Vec<String>,
    #[serde(default)]
    pub approval_policy: Option<String>,
    #[serde(default)]
    pub external_impact: Option<String>,
}

impl Default for SkillRisk {
    fn default() -> Self {
        Self {
            risk_level: RiskLevel::Low,
            side_effects: vec!["read_only".to_string()],
            approval_policy: None,
            external_impact: Some("none".to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillLifecycle {
    pub state: LifecycleState,
    pub verification_status: VerificationStatus,
    pub version: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub protected: bool,
}

impl SkillLifecycle {
    fn new_candidate(now: String, verification_status: VerificationStatus) -> Self {
        Self {
            state: LifecycleState::Candidate,
            verification_status,
            version: "0.1.0".to_string(),
            created_at: now.clone(),
            updated_at: now,
            last_used_at: None,
            last_verified_at: None,
            expires_at: None,
            pinned: false,
            protected: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillAverageCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_time_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRanking {
    pub rank: i64,
    pub score: f64,
    pub usage_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_cost: Option<SkillAverageCost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_latency_ms: Option<u64>,
}

impl Default for SkillRanking {
    fn default() -> Self {
        Self {
            rank: 1,
            score: 0.5,
            usage_count: 0,
            success_count: 0,
            failure_count: 0,
            average_cost: None,
            average_latency_ms: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillCompat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_agent_runtime: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillPackageMeta {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub r#type: SkillType,
    pub group_id: String,
    pub origin: SkillSourceType,
    pub owner_scope: OwnerScope,
    #[serde(default)]
    pub source_refs: Vec<SkillSourceRef>,
    #[serde(default)]
    pub source_event_ids: Vec<String>,
    #[serde(default)]
    pub source_session_ids: Vec<String>,
    #[serde(default)]
    pub object_refs: Vec<SkillObjectRef>,
    #[serde(default)]
    pub trigger: SkillTrigger,
    #[serde(default)]
    pub requires: SkillRequires,
    #[serde(default)]
    pub risk: SkillRisk,
    pub lifecycle: SkillLifecycle,
    #[serde(default)]
    pub ranking: SkillRanking,
    #[serde(default)]
    pub compat: SkillCompat,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillSource {
    pub schema_version: u32,
    pub source_id: String,
    pub source_type: SkillSourceType,
    pub source_uri: String,
    pub installed_by: String,
    pub target_scope: OwnerScope,
    pub trust_hint: String,
    pub digest: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct CreateCandidateInput {
    pub source_uri: String,
    pub source_type: SkillSourceType,
    pub installed_by: String,
    pub target_scope: OwnerScope,
    pub trust_hint: String,
    pub package_path: Option<PathBuf>,
    pub body: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub skill_type: Option<SkillType>,
    pub group_id: Option<String>,
    pub intent_tags: Vec<String>,
    pub required_tools: Vec<String>,
    pub risk_level: Option<RiskLevel>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallResult {
    pub source: SkillSource,
    pub candidate: SkillHint,
    pub static_check: StaticCheckReport,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StaticCheckReport {
    pub ok: bool,
    pub missing_sections: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillHint {
    pub skill_id: String,
    pub group_id: String,
    pub title: String,
    pub sentence: String,
    pub score: f64,
    pub risk_level: RiskLevel,
    pub lifecycle_state: LifecycleState,
    pub verification_status: VerificationStatus,
    pub r#type: SkillType,
    pub rank: i64,
}

#[derive(Clone, Debug, Default)]
pub struct ListSkillsInput {
    pub intent_tags: Vec<String>,
    pub object_types: Vec<String>,
    pub required_tools: Vec<String>,
    pub risk_budget: Option<RiskLevel>,
    pub states: Vec<LifecycleState>,
    pub types: Vec<SkillType>,
    pub verification_status: Option<VerificationStatus>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillPackageSummary {
    pub meta: SkillPackageMeta,
    pub body: String,
    pub package_path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillPackageFile {
    pub skill_id: String,
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ValidateSelectionInput {
    pub requested: Vec<String>,
    pub max_count: usize,
    pub allowed_states: Vec<LifecycleState>,
    pub risk_budget: RiskLevel,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillSelectionValidation {
    pub ok: bool,
    pub resolved: Vec<ResolvedSkillSelection>,
    pub rejected: Vec<RejectedSkillSelection>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResolvedSkillSelection {
    pub requested: String,
    pub skill_id: String,
    pub title: String,
    pub state: LifecycleState,
    pub verification_status: VerificationStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct RejectedSkillSelection {
    pub requested: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectedSkillSet {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    pub skill_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct RenderSelectedInput {
    pub session_id: String,
    pub todo_id: Option<String>,
    pub max_skills: usize,
    pub token_budget: usize,
    pub include_metadata: bool,
    pub include_usage_instructions: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PromptFragment {
    pub block_type: &'static str,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    pub selected_skill_ids: Vec<String>,
    pub usage_ids: Vec<String>,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillUsageCost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_time_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillUsage {
    pub usage_id: String,
    pub skill_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    pub loaded_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<String>,
    pub usage_mode: UsageMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_result: Option<SkillTaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<UserFeedback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub cost: SkillUsageCost,
    #[serde(default)]
    pub output_refs: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub action: String,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub created_at: String,
    pub detail: Json,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SelectionIndex {
    #[serde(default)]
    sessions: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    todos: BTreeMap<String, Vec<String>>,
}

#[derive(Clone)]
pub struct SkillsMgr {
    cfg: SkillsMgrConfig,
}

impl SkillsMgr {
    pub fn open(cfg: SkillsMgrConfig) -> Result<Self> {
        let mgr = Self { cfg };
        mgr.ensure_initialized()?;
        Ok(mgr)
    }

    pub fn init(cfg: SkillsMgrConfig) -> Result<Self> {
        Self::open(cfg)
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    pub fn create_candidate(&self, input: CreateCandidateInput) -> Result<InstallResult> {
        self.with_writer_lock(|_| {
            let now = now_iso8601();
            let (body, support_root, imported_meta) = self.read_candidate_material(&input)?;
            let check = static_check_skill_body(&body);
            let source = self.write_source(&input, &body, &now)?;
            let mut meta = imported_meta.unwrap_or_else(|| {
                build_default_meta(
                    &input,
                    &source,
                    &body,
                    &now,
                    if check.ok {
                        VerificationStatus::StaticChecked
                    } else {
                        VerificationStatus::Unverified
                    },
                )
            });
            normalize_meta_from_input(&mut meta, &input, &source, &body, &now);
            meta.lifecycle.state = LifecycleState::Candidate;
            if meta.lifecycle.verification_status == VerificationStatus::Unverified && check.ok {
                meta.lifecycle.verification_status = VerificationStatus::StaticChecked;
            }
            let skill_id = meta.id.clone();
            let candidate_dir = self.package_dir(CANDIDATES_DIR, &skill_id);
            if candidate_dir.exists() {
                return Err(SkillsMgrError::AlreadyExists(format!(
                    "candidate `{skill_id}` already exists"
                )));
            }
            fs::create_dir_all(&candidate_dir)?;
            write_json_pretty(&candidate_dir.join(SKILL_META_FILE), &meta)?;
            atomic_write(&candidate_dir.join(SKILL_BODY_FILE), body.as_bytes())?;
            if let Some(root) = support_root {
                copy_support_files(&root, &candidate_dir)?;
            }
            self.append_audit(AuditEvent {
                event_id: make_id("audit", &skill_id),
                action: "candidate_created".to_string(),
                actor: input.installed_by.clone(),
                skill_id: Some(skill_id.clone()),
                source_id: Some(source.source_id.clone()),
                created_at: now_iso8601(),
                detail: json!({
                    "source_uri": input.source_uri,
                    "static_check": check,
                }),
            })?;
            let hint = skill_hint(&meta);
            let next_actions = if check.ok {
                vec![
                    "request_verification".to_string(),
                    "promote_when_ready".to_string(),
                ]
            } else {
                vec!["patch_candidate_missing_sections".to_string()]
            };
            Ok(InstallResult {
                source,
                candidate: hint,
                static_check: check,
                next_actions,
            })
        })
    }

    pub fn list_skills(&self, input: ListSkillsInput) -> Result<Vec<SkillHint>> {
        let mut metas = self.load_all_skill_metas()?;
        metas.retain(|meta| skill_matches_filter(meta, &input));
        metas.sort_by(compare_skill_for_hint);
        let limit = input.limit.unwrap_or(DEFAULT_MAX_HINTS);
        Ok(metas
            .into_iter()
            .take(limit)
            .map(|m| skill_hint(&m))
            .collect())
    }

    pub fn skill_hints_for_explorer(&self, mut input: ListSkillsInput) -> Result<Vec<SkillHint>> {
        input.types = vec![SkillType::DataAcquisition, SkillType::Exploration];
        input.risk_budget = Some(RiskLevel::Low);
        input.limit = Some(input.limit.unwrap_or(3));
        self.list_skills(input)
    }

    pub fn skill_view(&self, skill_ref: &str, path: Option<&str>) -> Result<Json> {
        let resolved = self.resolve_skill(skill_ref)?;
        let package_dir = self.existing_package_dir(&resolved)?;
        if let Some(path) = path {
            let rel = validate_package_relative_path(path)?;
            let full = package_dir.join(&rel);
            let content = fs::read_to_string(&full).map_err(|err| {
                SkillsMgrError::NotFound(format!("package file `{}`: {err}", rel.display()))
            })?;
            return Ok(serde_json::to_value(SkillPackageFile {
                skill_id: resolved.id,
                path: rel.to_string_lossy().to_string(),
                content,
            })?);
        }
        let meta = read_meta(&package_dir)?;
        let body = read_body(&package_dir)?;
        Ok(serde_json::to_value(SkillPackageSummary {
            meta,
            body,
            package_path: package_dir.to_string_lossy().to_string(),
        })?)
    }

    pub fn promote(&self, skill_ref: &str, actor: &str) -> Result<SkillHint> {
        self.with_writer_lock(|_| {
            let resolved = self.resolve_skill_in_dir(CANDIDATES_DIR, skill_ref)?;
            let src = self.package_dir(CANDIDATES_DIR, &resolved.id);
            let dst = self.package_dir(ACTIVE_DIR, &resolved.id);
            if dst.exists() {
                return Err(SkillsMgrError::AlreadyExists(format!(
                    "active skill `{}` already exists",
                    resolved.id
                )));
            }
            let backup_id = self.backup_package(&src, &resolved.id, "promote")?;
            copy_dir_recursive(&src, &dst)?;
            let mut meta = read_meta(&dst)?;
            let now = now_iso8601();
            meta.lifecycle.state = LifecycleState::Active;
            meta.lifecycle.updated_at = now.clone();
            if meta.lifecycle.verification_status == VerificationStatus::Unverified {
                meta.lifecycle.verification_status = VerificationStatus::StaticChecked;
            }
            write_json_pretty(&dst.join(SKILL_META_FILE), &meta)?;
            fs::remove_dir_all(&src)?;
            self.append_audit(AuditEvent {
                event_id: make_id("audit", &resolved.id),
                action: "promote".to_string(),
                actor: actor.to_string(),
                skill_id: Some(resolved.id.clone()),
                source_id: None,
                created_at: now,
                detail: json!({ "backup_id": backup_id }),
            })?;
            Ok(skill_hint(&meta))
        })
    }

    pub fn archive(&self, skill_ref: &str, actor: &str, reason: &str) -> Result<SkillHint> {
        self.with_writer_lock(|_| {
            let resolved = self.resolve_skill_in_dir(ACTIVE_DIR, skill_ref)?;
            let src = self.package_dir(ACTIVE_DIR, &resolved.id);
            let dst = self.package_dir(ARCHIVED_DIR, &resolved.id);
            if dst.exists() {
                return Err(SkillsMgrError::AlreadyExists(format!(
                    "archived skill `{}` already exists",
                    resolved.id
                )));
            }
            let backup_id = self.backup_package(&src, &resolved.id, "archive")?;
            copy_dir_recursive(&src, &dst)?;
            let mut meta = read_meta(&dst)?;
            let now = now_iso8601();
            meta.lifecycle.state = LifecycleState::Archived;
            meta.lifecycle.updated_at = now.clone();
            write_json_pretty(&dst.join(SKILL_META_FILE), &meta)?;
            fs::remove_dir_all(&src)?;
            self.append_audit(AuditEvent {
                event_id: make_id("audit", &resolved.id),
                action: "archive".to_string(),
                actor: actor.to_string(),
                skill_id: Some(resolved.id.clone()),
                source_id: None,
                created_at: now,
                detail: json!({ "reason": reason, "backup_id": backup_id }),
            })?;
            Ok(skill_hint(&meta))
        })
    }

    pub fn restore(&self, skill_ref: &str, actor: &str, reason: &str) -> Result<SkillHint> {
        self.with_writer_lock(|_| {
            let resolved = self.resolve_skill_in_dir(ARCHIVED_DIR, skill_ref)?;
            let src = self.package_dir(ARCHIVED_DIR, &resolved.id);
            let dst = self.package_dir(ACTIVE_DIR, &resolved.id);
            if dst.exists() {
                return Err(SkillsMgrError::AlreadyExists(format!(
                    "active skill `{}` already exists",
                    resolved.id
                )));
            }
            let backup_id = self.backup_package(&src, &resolved.id, "restore")?;
            copy_dir_recursive(&src, &dst)?;
            let mut meta = read_meta(&dst)?;
            let now = now_iso8601();
            meta.lifecycle.state = LifecycleState::Active;
            meta.lifecycle.updated_at = now.clone();
            write_json_pretty(&dst.join(SKILL_META_FILE), &meta)?;
            fs::remove_dir_all(&src)?;
            self.append_audit(AuditEvent {
                event_id: make_id("audit", &resolved.id),
                action: "restore".to_string(),
                actor: actor.to_string(),
                skill_id: Some(resolved.id.clone()),
                source_id: None,
                created_at: now,
                detail: json!({ "reason": reason, "backup_id": backup_id }),
            })?;
            Ok(skill_hint(&meta))
        })
    }

    pub fn validate_selection(
        &self,
        input: ValidateSelectionInput,
    ) -> Result<SkillSelectionValidation> {
        let mut resolved = Vec::new();
        let mut rejected = Vec::new();
        let mut seen = BTreeSet::new();
        let allowed_states = if input.allowed_states.is_empty() {
            vec![LifecycleState::Active, LifecycleState::Preferred]
        } else {
            input.allowed_states
        };
        for requested in input.requested {
            if resolved.len() >= input.max_count {
                rejected.push(RejectedSkillSelection {
                    requested,
                    reason: "too_many".to_string(),
                });
                continue;
            }
            match self.resolve_skill(&requested) {
                Ok(skill) => {
                    if !seen.insert(skill.id.clone()) {
                        continue;
                    }
                    if !allowed_states.contains(&skill.meta.lifecycle.state) {
                        rejected.push(RejectedSkillSelection {
                            requested,
                            reason: "not_loadable".to_string(),
                        });
                        continue;
                    }
                    if matches!(
                        skill.meta.lifecycle.state,
                        LifecycleState::Blocked | LifecycleState::Rejected
                    ) || skill.meta.lifecycle.verification_status == VerificationStatus::Unsafe
                    {
                        rejected.push(RejectedSkillSelection {
                            requested,
                            reason: "blocked".to_string(),
                        });
                        continue;
                    }
                    if skill.meta.risk.risk_level > input.risk_budget {
                        rejected.push(RejectedSkillSelection {
                            requested,
                            reason: "risk_exceeded".to_string(),
                        });
                        continue;
                    }
                    resolved.push(ResolvedSkillSelection {
                        requested,
                        skill_id: skill.id.clone(),
                        title: skill.meta.title.clone(),
                        state: skill.meta.lifecycle.state,
                        verification_status: skill.meta.lifecycle.verification_status,
                    });
                }
                Err(SkillsMgrError::NotFound(_)) => rejected.push(RejectedSkillSelection {
                    requested,
                    reason: "not_found".to_string(),
                }),
                Err(err) => return Err(err),
            }
        }
        Ok(SkillSelectionValidation {
            ok: rejected.is_empty(),
            resolved,
            rejected,
        })
    }

    pub fn select_for_session(
        &self,
        session_id: &str,
        input: ValidateSelectionInput,
    ) -> Result<SelectedSkillSet> {
        self.update_selection(session_id, None, input, true)
    }

    pub fn select_for_todo(
        &self,
        session_id: &str,
        todo_id: &str,
        input: ValidateSelectionInput,
    ) -> Result<SelectedSkillSet> {
        self.update_selection(session_id, Some(todo_id), input, true)
    }

    pub fn unselect_for_session(
        &self,
        session_id: &str,
        skill_refs: &[String],
    ) -> Result<SelectedSkillSet> {
        self.update_unselect(session_id, None, skill_refs)
    }

    pub fn unselect_for_todo(
        &self,
        session_id: &str,
        todo_id: &str,
        skill_refs: &[String],
    ) -> Result<SelectedSkillSet> {
        self.update_unselect(session_id, Some(todo_id), skill_refs)
    }

    pub fn selected_list(
        &self,
        session_id: &str,
        todo_id: Option<&str>,
    ) -> Result<SelectedSkillSet> {
        let index = self.load_selection_index()?;
        let ids = match todo_id {
            Some(todo_id) => index
                .todos
                .get(&selection_todo_key(session_id, todo_id))
                .cloned()
                .unwrap_or_default(),
            None => index.sessions.get(session_id).cloned().unwrap_or_default(),
        };
        Ok(SelectedSkillSet {
            session_id: session_id.to_string(),
            todo_id: todo_id.map(str::to_string),
            skill_ids: ids,
        })
    }

    pub fn render_selected(&self, input: RenderSelectedInput) -> Result<PromptFragment> {
        let selected = self.selected_list(&input.session_id, input.todo_id.as_deref())?;
        let mut content = String::new();
        let mut usage_ids = Vec::new();
        let mut truncated = false;
        let mut budget = input.token_budget.saturating_mul(4);
        for skill_id in selected.skill_ids.iter().take(input.max_skills) {
            let resolved = self.resolve_skill(skill_id)?;
            if !resolved.meta.lifecycle.state.is_loadable() {
                continue;
            }
            let package_dir = self.existing_package_dir(&resolved)?;
            let body = read_body(&package_dir)?;
            let mut block = String::new();
            if input.include_metadata {
                block.push_str(&format!(
                    "## Skill: {}\n\nid: {}\ngroup_id: {}\ntype: {:?}\nrisk_level: {:?}\nverification_status: {:?}\n\n",
                    resolved.meta.title,
                    resolved.meta.id,
                    resolved.meta.group_id,
                    resolved.meta.r#type,
                    resolved.meta.risk.risk_level,
                    resolved.meta.lifecycle.verification_status
                ));
            } else {
                block.push_str(&format!("## Skill: {}\n\n", resolved.meta.title));
            }
            block.push_str(&body);
            if input.include_usage_instructions {
                block.push_str("\n\nReport skill usage result after the task finishes.\n");
            }
            block.push_str("\n\n");
            if block.len() > budget {
                let mut end = budget;
                while end > 0 && !block.is_char_boundary(end) {
                    end -= 1;
                }
                content.push_str(&block[..end]);
                truncated = true;
                let usage = self.record_loaded(
                    &resolved.meta.id,
                    &input.session_id,
                    input.todo_id.as_deref(),
                )?;
                usage_ids.push(usage.usage_id);
                break;
            }
            budget = budget.saturating_sub(block.len());
            content.push_str(&block);
            let usage = self.record_loaded(
                &resolved.meta.id,
                &input.session_id,
                input.todo_id.as_deref(),
            )?;
            usage_ids.push(usage.usage_id);
        }
        Ok(PromptFragment {
            block_type: "selected_skills",
            session_id: input.session_id,
            todo_id: input.todo_id,
            selected_skill_ids: selected.skill_ids,
            usage_ids,
            content,
            truncated,
        })
    }

    pub fn record_loaded(
        &self,
        skill_id: &str,
        session_id: &str,
        todo_id: Option<&str>,
    ) -> Result<SkillUsage> {
        let usage = SkillUsage {
            usage_id: make_id("skill_usage", skill_id),
            skill_id: skill_id.to_string(),
            session_id: session_id.to_string(),
            todo_id: todo_id.map(str::to_string),
            loaded_at: now_iso8601(),
            used_at: None,
            usage_mode: UsageMode::Loaded,
            task_result: None,
            user_feedback: None,
            failure_reason: None,
            cost: SkillUsageCost::default(),
            output_refs: Vec::new(),
            evidence: Vec::new(),
        };
        self.append_usage(&usage)?;
        Ok(usage)
    }

    pub fn record_used(
        &self,
        usage_id: &str,
        mode: UsageMode,
        evidence: Vec<String>,
    ) -> Result<SkillUsage> {
        self.update_usage_record(usage_id, |usage| {
            usage.used_at = Some(now_iso8601());
            usage.usage_mode = mode;
            usage.evidence = evidence.clone();
        })
    }

    pub fn record_result(
        &self,
        usage_id: &str,
        result: SkillTaskResult,
        feedback: Option<UserFeedback>,
        failure_reason: Option<String>,
        output_refs: Vec<String>,
    ) -> Result<SkillUsage> {
        let usage = self.update_usage_record(usage_id, |usage| {
            usage.task_result = Some(result);
            usage.user_feedback = feedback;
            usage.failure_reason = failure_reason.clone();
            usage.output_refs = output_refs.clone();
        })?;
        self.apply_usage_stats(&usage)?;
        Ok(usage)
    }

    fn ensure_initialized(&self) -> Result<()> {
        fs::create_dir_all(&self.cfg.root)?;
        for dir in [
            SOURCES_DIR,
            CANDIDATES_DIR,
            ACTIVE_DIR,
            ARCHIVED_DIR,
            INDEXES_DIR,
            USAGE_DIR,
            BACKUPS_DIR,
        ] {
            fs::create_dir_all(self.cfg.root.join(dir))?;
        }
        for file in [
            self.usage_dir().join(USAGE_LOG_FILE),
            self.usage_dir().join(AUDIT_LOG_FILE),
            self.usage_dir().join(BLOCKLIST_LOG_FILE),
            self.usage_dir().join(PROPOSALS_LOG_FILE),
        ] {
            OpenOptions::new().create(true).append(true).open(file)?;
        }
        let selection_path = self.indexes_dir().join(SELECTION_FILE);
        if !selection_path.exists() {
            write_json_pretty(&selection_path, &SelectionIndex::default())?;
        }
        OpenOptions::new()
            .create(true)
            .write(true)
            .open(self.cfg.root.join(LOCK_FILE))?;
        Ok(())
    }

    fn indexes_dir(&self) -> PathBuf {
        self.cfg.root.join(INDEXES_DIR)
    }

    fn usage_dir(&self) -> PathBuf {
        self.cfg.root.join(USAGE_DIR)
    }

    fn package_dir(&self, kind: &str, skill_id: &str) -> PathBuf {
        self.cfg.root.join(kind).join(path_key(skill_id))
    }

    fn source_dir(&self, source_id: &str) -> PathBuf {
        self.cfg.root.join(SOURCES_DIR).join(path_key(source_id))
    }

    fn read_candidate_material(
        &self,
        input: &CreateCandidateInput,
    ) -> Result<(String, Option<PathBuf>, Option<SkillPackageMeta>)> {
        if let Some(body) = input.body.as_ref() {
            return Ok((body.clone(), None, None));
        }
        let Some(path) = input.package_path.as_ref() else {
            return Err(SkillsMgrError::InvalidInput(
                "candidate requires package_path or body".to_string(),
            ));
        };
        let path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()?.join(path)
        };
        if path.is_file() {
            let body = fs::read_to_string(&path)?;
            return Ok((body, None, None));
        }
        if !path.is_dir() {
            return Err(SkillsMgrError::InvalidInput(format!(
                "package path is not a file or directory: {}",
                path.display()
            )));
        }
        let body_path = path.join(SKILL_BODY_FILE);
        let body = fs::read_to_string(&body_path).map_err(|err| {
            SkillsMgrError::InvalidInput(format!("read `{}` failed: {err}", body_path.display()))
        })?;
        let meta_path = path.join(SKILL_META_FILE);
        let imported_meta = if meta_path.exists() {
            let raw = fs::read_to_string(&meta_path)?;
            Some(
                serde_json::from_str::<SkillPackageMeta>(&raw).map_err(|err| {
                    SkillsMgrError::InvalidInput(format!(
                        "`{}` must be JSON-compatible skill.yaml in this phase: {err}",
                        meta_path.display()
                    ))
                })?,
            )
        } else {
            None
        };
        Ok((body, Some(path), imported_meta))
    }

    fn write_source(
        &self,
        input: &CreateCandidateInput,
        body: &str,
        now: &str,
    ) -> Result<SkillSource> {
        let digest = content_digest(body.as_bytes());
        let source_id = make_id("skill_source", &format!("{}:{digest}", input.source_uri));
        let source = SkillSource {
            schema_version: SCHEMA_VERSION,
            source_id: source_id.clone(),
            source_type: input.source_type,
            source_uri: input.source_uri.clone(),
            installed_by: input.installed_by.clone(),
            target_scope: input.target_scope,
            trust_hint: input.trust_hint.clone(),
            digest,
            created_at: now.to_string(),
        };
        let dir = self.source_dir(&source_id);
        if !dir.exists() {
            fs::create_dir_all(dir.join("raw"))?;
            write_json_pretty(&dir.join(SOURCE_META_FILE), &source)?;
            atomic_write(&dir.join("raw").join(SKILL_BODY_FILE), body.as_bytes())?;
        }
        Ok(source)
    }

    fn load_all_skill_metas(&self) -> Result<Vec<SkillPackageMeta>> {
        let mut out = Vec::new();
        for kind in [ACTIVE_DIR, CANDIDATES_DIR, ARCHIVED_DIR] {
            let dir = self.cfg.root.join(kind);
            if !dir.exists() {
                continue;
            }
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let path = entry.path();
                    if path.join(SKILL_META_FILE).exists() {
                        out.push(read_meta(&path)?);
                    }
                }
            }
        }
        Ok(out)
    }

    fn resolve_skill(&self, skill_ref: &str) -> Result<ResolvedSkill> {
        for kind in [ACTIVE_DIR, CANDIDATES_DIR, ARCHIVED_DIR] {
            if let Ok(found) = self.resolve_skill_in_dir(kind, skill_ref) {
                return Ok(found);
            }
        }
        Err(SkillsMgrError::NotFound(skill_ref.to_string()))
    }

    fn resolve_skill_in_dir(&self, kind: &str, skill_ref: &str) -> Result<ResolvedSkill> {
        let direct = self.package_dir(kind, skill_ref);
        if direct.exists() {
            let meta = read_meta(&direct)?;
            return Ok(ResolvedSkill {
                id: meta.id.clone(),
                meta,
                kind: kind.to_string(),
            });
        }
        let mut matches = Vec::new();
        let dir = self.cfg.root.join(kind);
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let meta = read_meta(&entry.path())?;
                if meta.id == skill_ref || meta.name == skill_ref || path_key(&meta.id) == skill_ref
                {
                    matches.push(ResolvedSkill {
                        id: meta.id.clone(),
                        meta,
                        kind: kind.to_string(),
                    });
                }
            }
        }
        match matches.len() {
            0 => Err(SkillsMgrError::NotFound(skill_ref.to_string())),
            1 => Ok(matches.remove(0)),
            _ => Err(SkillsMgrError::InvalidInput(format!(
                "skill ref `{skill_ref}` is ambiguous"
            ))),
        }
    }

    fn existing_package_dir(&self, skill: &ResolvedSkill) -> Result<PathBuf> {
        let dir = self.package_dir(&skill.kind, &skill.id);
        if dir.exists() {
            Ok(dir)
        } else {
            Err(SkillsMgrError::NotFound(skill.id.clone()))
        }
    }

    fn load_selection_index(&self) -> Result<SelectionIndex> {
        let path = self.indexes_dir().join(SELECTION_FILE);
        if !path.exists() {
            return Ok(SelectionIndex::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    fn save_selection_index(&self, index: &SelectionIndex) -> Result<()> {
        write_json_pretty(&self.indexes_dir().join(SELECTION_FILE), index)
    }

    fn update_selection(
        &self,
        session_id: &str,
        todo_id: Option<&str>,
        input: ValidateSelectionInput,
        append: bool,
    ) -> Result<SelectedSkillSet> {
        self.with_writer_lock(|_| {
            let validation = self.validate_selection(input)?;
            if !validation.rejected.is_empty() {
                return Err(SkillsMgrError::InvalidInput(format!(
                    "selection rejected: {}",
                    serde_json::to_string(&validation.rejected)?
                )));
            }
            let mut index = self.load_selection_index()?;
            let key = todo_id
                .map(|todo| selection_todo_key(session_id, todo))
                .unwrap_or_else(|| session_id.to_string());
            let target = if todo_id.is_some() {
                index.todos.entry(key).or_default()
            } else {
                index.sessions.entry(key).or_default()
            };
            if !append {
                target.clear();
            }
            for item in validation.resolved {
                if !target.contains(&item.skill_id) {
                    target.push(item.skill_id);
                }
            }
            let skill_ids = target.clone();
            self.save_selection_index(&index)?;
            Ok(SelectedSkillSet {
                session_id: session_id.to_string(),
                todo_id: todo_id.map(str::to_string),
                skill_ids,
            })
        })
    }

    fn update_unselect(
        &self,
        session_id: &str,
        todo_id: Option<&str>,
        skill_refs: &[String],
    ) -> Result<SelectedSkillSet> {
        self.with_writer_lock(|_| {
            let mut remove_ids = BTreeSet::new();
            for skill_ref in skill_refs {
                if let Ok(skill) = self.resolve_skill(skill_ref) {
                    remove_ids.insert(skill.id);
                } else {
                    remove_ids.insert(skill_ref.clone());
                }
            }
            let mut index = self.load_selection_index()?;
            let key = todo_id
                .map(|todo| selection_todo_key(session_id, todo))
                .unwrap_or_else(|| session_id.to_string());
            let target = if todo_id.is_some() {
                index.todos.entry(key).or_default()
            } else {
                index.sessions.entry(key).or_default()
            };
            target.retain(|skill_id| !remove_ids.contains(skill_id));
            let skill_ids = target.clone();
            self.save_selection_index(&index)?;
            Ok(SelectedSkillSet {
                session_id: session_id.to_string(),
                todo_id: todo_id.map(str::to_string),
                skill_ids,
            })
        })
    }

    fn append_usage(&self, usage: &SkillUsage) -> Result<()> {
        append_jsonl(&self.usage_dir().join(USAGE_LOG_FILE), usage)
    }

    fn update_usage_record<F>(&self, usage_id: &str, mut update: F) -> Result<SkillUsage>
    where
        F: FnMut(&mut SkillUsage),
    {
        self.with_writer_lock(|_| {
            let path = self.usage_dir().join(USAGE_LOG_FILE);
            let raw = fs::read_to_string(&path)?;
            let mut usages = Vec::new();
            let mut found = None;
            for line in raw.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let mut usage: SkillUsage = serde_json::from_str(line)?;
                if usage.usage_id == usage_id {
                    update(&mut usage);
                    found = Some(usage.clone());
                }
                usages.push(usage);
            }
            let Some(updated) = found else {
                return Err(SkillsMgrError::NotFound(format!("usage `{usage_id}`")));
            };
            let mut out = String::new();
            for usage in usages {
                out.push_str(&serde_json::to_string(&usage)?);
                out.push('\n');
            }
            atomic_write(&path, out.as_bytes())?;
            Ok(updated)
        })
    }

    fn apply_usage_stats(&self, usage: &SkillUsage) -> Result<()> {
        self.with_writer_lock(|_| {
            let resolved = self.resolve_skill(&usage.skill_id)?;
            let dir = self.existing_package_dir(&resolved)?;
            let mut meta = read_meta(&dir)?;
            meta.ranking.usage_count = meta.ranking.usage_count.saturating_add(1);
            meta.lifecycle.last_used_at = Some(now_iso8601());
            match usage.task_result {
                Some(SkillTaskResult::Success) => {
                    meta.ranking.success_count = meta.ranking.success_count.saturating_add(1);
                    if meta.ranking.success_count >= 3
                        && meta.lifecycle.verification_status.strength()
                            < VerificationStatus::UsageVerified.strength()
                    {
                        meta.lifecycle.verification_status = VerificationStatus::UsageVerified;
                    }
                }
                Some(SkillTaskResult::Failed | SkillTaskResult::NotApplicable) => {
                    meta.ranking.failure_count = meta.ranking.failure_count.saturating_add(1);
                    if meta.ranking.failure_count >= 2 {
                        meta.lifecycle.state = LifecycleState::NeedsReverification;
                    }
                }
                Some(SkillTaskResult::Partial) | None => {}
            }
            meta.ranking.score = compute_score(&meta);
            meta.lifecycle.updated_at = now_iso8601();
            write_json_pretty(&dir.join(SKILL_META_FILE), &meta)?;
            Ok(())
        })
    }

    fn backup_package(&self, package_dir: &Path, skill_id: &str, action: &str) -> Result<String> {
        let backup_id = make_id("backup", &format!("{skill_id}:{action}"));
        let dst = self.cfg.root.join(BACKUPS_DIR).join(&backup_id);
        copy_dir_recursive(package_dir, &dst)?;
        Ok(backup_id)
    }

    fn append_audit(&self, event: AuditEvent) -> Result<()> {
        append_jsonl(&self.usage_dir().join(AUDIT_LOG_FILE), &event)
    }

    fn with_writer_lock<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&File) -> Result<T>,
    {
        let lock_path = self.cfg.root.join(LOCK_FILE);
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        let deadline = Instant::now() + self.cfg.lock_timeout;
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(SkillsMgrError::LockTimeout(format!(
                            "{} after {:?}",
                            lock_path.display(),
                            self.cfg.lock_timeout
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(err) => return Err(SkillsMgrError::Io(err)),
            }
        }
        let result = f(&lock_file);
        let _ = FileExt::unlock(&lock_file);
        result
    }
}

#[derive(Clone, Debug)]
struct ResolvedSkill {
    id: String,
    meta: SkillPackageMeta,
    kind: String,
}

fn build_default_meta(
    input: &CreateCandidateInput,
    source: &SkillSource,
    body: &str,
    now: &str,
    verification_status: VerificationStatus,
) -> SkillPackageMeta {
    let title = input
        .title
        .clone()
        .or_else(|| extract_markdown_title(body))
        .unwrap_or_else(|| "Untitled Skill".to_string());
    let name = input.name.clone().unwrap_or_else(|| slugify(&title));
    let group_id = input
        .group_id
        .clone()
        .unwrap_or_else(|| name.replace('-', "_"));
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| format!("skill://local/{name}"));
    let description = input
        .description
        .clone()
        .or_else(|| extract_first_sentence(body))
        .unwrap_or_else(|| title.clone());
    SkillPackageMeta {
        schema_version: SCHEMA_VERSION,
        id,
        name,
        title,
        description,
        r#type: input.skill_type.unwrap_or(SkillType::Workflow),
        group_id,
        origin: input.source_type,
        owner_scope: input.target_scope,
        source_refs: vec![SkillSourceRef {
            r#type: "source".to_string(),
            uri: source.source_uri.clone(),
            digest: Some(source.digest.clone()),
        }],
        source_event_ids: Vec::new(),
        source_session_ids: Vec::new(),
        object_refs: Vec::new(),
        trigger: SkillTrigger {
            when_to_use: extract_section_lines(body, "When to Use"),
            intent_tags: input.intent_tags.clone(),
            object_types: Vec::new(),
            negative_triggers: Vec::new(),
        },
        requires: SkillRequires {
            tools: input.required_tools.clone(),
            agent_tools: Vec::new(),
            permissions: Vec::new(),
            environment: Vec::new(),
            optional_tools: Vec::new(),
        },
        risk: SkillRisk {
            risk_level: input.risk_level.unwrap_or(RiskLevel::Low),
            ..SkillRisk::default()
        },
        lifecycle: SkillLifecycle::new_candidate(now.to_string(), verification_status),
        ranking: SkillRanking::default(),
        compat: SkillCompat::default(),
    }
}

fn normalize_meta_from_input(
    meta: &mut SkillPackageMeta,
    input: &CreateCandidateInput,
    source: &SkillSource,
    body: &str,
    now: &str,
) {
    meta.schema_version = SCHEMA_VERSION;
    if let Some(id) = input.id.as_ref() {
        meta.id = id.clone();
    }
    if let Some(name) = input.name.as_ref() {
        meta.name = name.clone();
    }
    if let Some(title) = input.title.as_ref() {
        meta.title = title.clone();
    }
    if let Some(description) = input.description.as_ref() {
        meta.description = description.clone();
    }
    if let Some(skill_type) = input.skill_type {
        meta.r#type = skill_type;
    }
    if let Some(group_id) = input.group_id.as_ref() {
        meta.group_id = group_id.clone();
    }
    if let Some(risk_level) = input.risk_level {
        meta.risk.risk_level = risk_level;
    }
    meta.origin = input.source_type;
    meta.owner_scope = input.target_scope;
    for tag in &input.intent_tags {
        if !meta.trigger.intent_tags.contains(tag) {
            meta.trigger.intent_tags.push(tag.clone());
        }
    }
    if meta.trigger.when_to_use.is_empty() {
        meta.trigger.when_to_use = extract_section_lines(body, "When to Use");
    }
    for tool in &input.required_tools {
        if !meta.requires.tools.contains(tool) {
            meta.requires.tools.push(tool.clone());
        }
    }
    if meta.source_refs.is_empty() {
        meta.source_refs.push(SkillSourceRef {
            r#type: "source".to_string(),
            uri: source.source_uri.clone(),
            digest: Some(source.digest.clone()),
        });
    }
    if meta.lifecycle.created_at.trim().is_empty() {
        meta.lifecycle.created_at = now.to_string();
    }
    meta.lifecycle.updated_at = now.to_string();
}

fn static_check_skill_body(body: &str) -> StaticCheckReport {
    let required = [
        "When to Use",
        "Inputs",
        "Procedure",
        "Pitfalls",
        "Verification",
        "Rollback",
        "Report",
    ];
    let headings = markdown_headings(body);
    let missing_sections = required
        .iter()
        .filter(|section| !headings.iter().any(|h| h.eq_ignore_ascii_case(section)))
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    StaticCheckReport {
        ok: missing_sections.is_empty(),
        missing_sections,
        warnings: Vec::new(),
    }
}

fn skill_matches_filter(meta: &SkillPackageMeta, input: &ListSkillsInput) -> bool {
    if input.states.is_empty() {
        if !meta.lifecycle.state.is_loadable() {
            return false;
        }
    } else if !input.states.contains(&meta.lifecycle.state) {
        return false;
    }
    if !input.types.is_empty() && !input.types.contains(&meta.r#type) {
        return false;
    }
    if let Some(risk_budget) = input.risk_budget {
        if meta.risk.risk_level > risk_budget {
            return false;
        }
    }
    if let Some(status) = input.verification_status {
        if meta.lifecycle.verification_status != status {
            return false;
        }
    }
    if !input.intent_tags.is_empty()
        && !input
            .intent_tags
            .iter()
            .any(|tag| meta.trigger.intent_tags.iter().any(|t| t == tag))
    {
        return false;
    }
    if !input.object_types.is_empty()
        && !input
            .object_types
            .iter()
            .any(|ty| meta.trigger.object_types.iter().any(|t| t == ty))
    {
        return false;
    }
    if !input.required_tools.is_empty()
        && !input
            .required_tools
            .iter()
            .all(|tool| meta.requires.tools.iter().any(|t| t == tool))
    {
        return false;
    }
    true
}

fn compare_skill_for_hint(a: &SkillPackageMeta, b: &SkillPackageMeta) -> Ordering {
    state_weight(b.lifecycle.state)
        .cmp(&state_weight(a.lifecycle.state))
        .then_with(|| b.ranking.score.total_cmp(&a.ranking.score))
        .then_with(|| a.ranking.rank.cmp(&b.ranking.rank))
        .then_with(|| {
            b.lifecycle
                .verification_status
                .strength()
                .cmp(&a.lifecycle.verification_status.strength())
        })
        .then_with(|| a.title.cmp(&b.title))
}

fn state_weight(state: LifecycleState) -> u8 {
    match state {
        LifecycleState::Preferred => 4,
        LifecycleState::Active => 3,
        LifecycleState::NeedsReverification => 2,
        LifecycleState::Stale => 1,
        _ => 0,
    }
}

fn skill_hint(meta: &SkillPackageMeta) -> SkillHint {
    SkillHint {
        skill_id: meta.id.clone(),
        group_id: meta.group_id.clone(),
        title: meta.title.clone(),
        sentence: meta.description.clone(),
        score: meta.ranking.score,
        risk_level: meta.risk.risk_level,
        lifecycle_state: meta.lifecycle.state,
        verification_status: meta.lifecycle.verification_status,
        r#type: meta.r#type,
        rank: meta.ranking.rank,
    }
}

fn compute_score(meta: &SkillPackageMeta) -> f64 {
    let total = meta.ranking.success_count + meta.ranking.failure_count;
    let success_rate = if total == 0 {
        0.5
    } else {
        meta.ranking.success_count as f64 / total as f64
    };
    let verification_bonus = meta.lifecycle.verification_status.strength() as f64 * 0.08;
    let risk_penalty = match meta.risk.risk_level {
        RiskLevel::Low => 0.0,
        RiskLevel::Medium => 0.05,
        RiskLevel::High => 0.12,
        RiskLevel::Critical => 0.25,
    };
    (success_rate + verification_bonus - risk_penalty).clamp(0.0, 1.0)
}

fn read_meta(package_dir: &Path) -> Result<SkillPackageMeta> {
    let raw = fs::read_to_string(package_dir.join(SKILL_META_FILE))?;
    Ok(serde_json::from_str(&raw)?)
}

fn read_body(package_dir: &Path) -> Result<String> {
    Ok(fs::read_to_string(package_dir.join(SKILL_BODY_FILE))?)
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(serde_json::to_string(value)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        SkillsMgrError::InvalidInput(format!("path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        "{}.tmp.{}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "skills".to_string()),
        make_id("tmp", path.to_string_lossy().as_ref())
    ));
    {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> Result<()> {
    let file = File::open(path)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> Result<()> {
    Ok(())
}

fn copy_support_files(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == SKILL_META_FILE || name == SKILL_BODY_FILE {
            continue;
        }
        let target = dst.join(name);
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(SkillsMgrError::AlreadyExists(format!(
            "destination exists: {}",
            dst.display()
        )));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn validate_package_relative_path(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(SkillsMgrError::InvalidInput(
            "package path must be relative".to_string(),
        ));
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => {
                return Err(SkillsMgrError::InvalidInput(
                    "package path cannot contain parent or prefix components".to_string(),
                ))
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(SkillsMgrError::InvalidInput(
            "package path cannot be empty".to_string(),
        ));
    }
    Ok(out)
}

fn selection_todo_key(session_id: &str, todo_id: &str) -> String {
    format!("{session_id}\n{todo_id}")
}

fn path_key(value: &str) -> String {
    let mut out = String::new();
    for b in value.as_bytes() {
        match *b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    if out.is_empty() {
        "empty".to_string()
    } else {
        out
    }
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn make_id(prefix: &str, seed: &str) -> String {
    let now = now_iso8601();
    let hash = blake3::hash(format!("{prefix}:{seed}:{now}:{}", std::process::id()).as_bytes());
    format!("{prefix}_{}", &hash.to_hex()[..16])
}

fn content_digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn extract_markdown_title(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

fn extract_first_sentence(body: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        return Some(line.chars().take(240).collect());
    }
    None
}

fn markdown_headings(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') {
                return None;
            }
            let heading = trimmed.trim_start_matches('#').trim();
            (!heading.is_empty()).then(|| heading.to_string())
        })
        .collect()
}

fn extract_section_lines(body: &str, section: &str) -> Vec<String> {
    let mut in_section = false;
    let mut lines = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim();
            if in_section && !heading.eq_ignore_ascii_case(section) {
                break;
            }
            in_section = heading.eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            let item = trimmed
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim()
                .to_string();
            if !item.is_empty() {
                lines.push(item);
            }
        }
    }
    lines
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "skill".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn body() -> String {
        r#"# Demo Skill

## When to Use
- demo task

## Inputs
- repo

## Procedure
1. do it

## Pitfalls
- avoid noise

## Verification
- test

## Rollback
- revert

## Report
- summarize
"#
        .to_string()
    }

    fn open_tmp() -> (TempDir, SkillsMgr) {
        let tmp = TempDir::new().unwrap();
        let mgr = SkillsMgr::open(SkillsMgrConfig::new(tmp.path().join(SKILLS_DIR))).unwrap();
        (tmp, mgr)
    }

    #[test]
    fn create_promote_list_and_view_candidate() {
        let (_tmp, mgr) = open_tmp();
        let result = mgr
            .create_candidate(CreateCandidateInput {
                source_uri: "inline".to_string(),
                source_type: SkillSourceType::OwnerInstalled,
                installed_by: "tester".to_string(),
                target_scope: OwnerScope::Agent,
                trust_hint: "medium".to_string(),
                package_path: None,
                body: Some(body()),
                id: Some("skill://local/demo".to_string()),
                name: Some("demo".to_string()),
                title: None,
                description: None,
                skill_type: Some(SkillType::Workflow),
                group_id: Some("demo_group".to_string()),
                intent_tags: vec!["demo".to_string()],
                required_tools: vec!["terminal".to_string()],
                risk_level: Some(RiskLevel::Medium),
            })
            .unwrap();
        assert!(result.static_check.ok);
        assert!(mgr
            .list_skills(ListSkillsInput::default())
            .unwrap()
            .is_empty());
        let promoted = mgr.promote("demo", "tester").unwrap();
        assert_eq!(promoted.lifecycle_state, LifecycleState::Active);
        let hints = mgr
            .list_skills(ListSkillsInput {
                intent_tags: vec!["demo".to_string()],
                risk_budget: Some(RiskLevel::Medium),
                ..ListSkillsInput::default()
            })
            .unwrap();
        assert_eq!(hints.len(), 1);
        let view = mgr.skill_view("demo", None).unwrap();
        assert_eq!(view["meta"]["id"], "skill://local/demo");
    }

    #[test]
    fn selection_render_records_usage() {
        let (_tmp, mgr) = open_tmp();
        mgr.create_candidate(CreateCandidateInput {
            source_uri: "inline".to_string(),
            source_type: SkillSourceType::OwnerInstalled,
            installed_by: "tester".to_string(),
            target_scope: OwnerScope::Agent,
            trust_hint: "medium".to_string(),
            package_path: None,
            body: Some(body()),
            id: Some("skill://local/demo".to_string()),
            name: Some("demo".to_string()),
            title: None,
            description: None,
            skill_type: Some(SkillType::Workflow),
            group_id: Some("demo_group".to_string()),
            intent_tags: vec![],
            required_tools: vec![],
            risk_level: Some(RiskLevel::Low),
        })
        .unwrap();
        mgr.promote("demo", "tester").unwrap();
        let selected = mgr
            .select_for_session(
                "s1",
                ValidateSelectionInput {
                    requested: vec!["demo".to_string()],
                    max_count: 3,
                    allowed_states: vec![LifecycleState::Active],
                    risk_budget: RiskLevel::Low,
                },
            )
            .unwrap();
        assert_eq!(selected.skill_ids, vec!["skill://local/demo"]);
        let rendered = mgr
            .render_selected(RenderSelectedInput {
                session_id: "s1".to_string(),
                todo_id: None,
                max_skills: 3,
                token_budget: DEFAULT_RENDER_TOKEN_BUDGET,
                include_metadata: true,
                include_usage_instructions: true,
            })
            .unwrap();
        assert!(rendered.content.contains("Demo Skill"));
        assert_eq!(rendered.usage_ids.len(), 1);
        let usage = mgr
            .record_result(
                &rendered.usage_ids[0],
                SkillTaskResult::Success,
                Some(UserFeedback::Accepted),
                None,
                Vec::new(),
            )
            .unwrap();
        assert_eq!(usage.task_result, Some(SkillTaskResult::Success));
    }
}
