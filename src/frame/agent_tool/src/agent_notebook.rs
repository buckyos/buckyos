//! Agent Notebook v0.2 — long-term factual layer.
//!
//! See `doc/opendan/Agent Notebook.md`. This module owns Notebook containers,
//! Note Item rows, supersede edges, cross-session events, and a per-session
//! read cache. Persistence is a single SQLite file; tag filtering is a
//! relational JOIN on a `notebook_item_tags` table — no FTS, no BM25, no
//! embedding index (§4.2).
//!
//! The implementation covers the MVP scope listed in §15.1:
//!
//! - data model: Notebook / NotebookItem / Edge / Event / ReadCache;
//! - tag normalization (§3.6), shared by write and read paths;
//! - `list_notebooks`, `create_or_update_notebook`;
//! - `read_notebook` with `all_active` / `latest` / `title` / `tags` / `items`
//!   recall modes, `unchanged` short-circuit, and `(updated_at DESC,
//!   created_at DESC, item_id ASC)` ordering for every mode;
//! - `append_note` with tag-overlap conflict detection;
//! - `mark_note_status` and `promote_to_system_notebook` with the
//!   10-active-item ceiling;
//! - `build_notebook_registry_context`, `build_system_notebook_context`,
//!   `build_notebook_hints` with cross-session update events + watermark.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use thiserror::Error;

pub const SCHEMA_VERSION: &str = "0.4";
pub const SYSTEM_NOTEBOOK_ID: &str = "system";
pub const SYSTEM_NOTEBOOK_MAX_ACTIVE: usize = 10;

pub const DEFAULT_MAX_ITEMS: usize = 10;
pub const DEFAULT_MAX_BYTES: usize = 12_000;
pub const DEFAULT_MAX_HINTS: usize = 3;

pub const MIN_TAG_BYTES: usize = 2;
pub const MAX_TAG_BYTES: usize = 32;
pub const MAX_TAGS_PER_ITEM: usize = 32;

// ============================================================ error / result

#[derive(Debug, Error)]
pub enum NotebookError {
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("permission_denied: {0}")]
    PermissionDenied(String),
    #[error("invalid_input: {0}")]
    InvalidInput(String),
    #[error("invalid_tag: {0}")]
    InvalidTag(String),
    #[error("version_conflict: {0}")]
    VersionConflict(String),
    #[error("limit_exceeded: {0}")]
    LimitExceeded(String),
    #[error("item_search_unavailable: {0}")]
    ItemSearchUnavailable(String),
    #[error("storage_error: {0}")]
    Storage(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl NotebookError {
    /// Stable error code string matching §10.1.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::PermissionDenied(_) => "permission_denied",
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidTag(_) => "invalid_tag",
            Self::VersionConflict(_) => "version_conflict",
            Self::LimitExceeded(_) => "limit_exceeded",
            Self::ItemSearchUnavailable(_) => "item_search_unavailable",
            Self::Storage(_) | Self::Io(_) | Self::Sqlite(_) | Self::Json(_) => "storage_error",
        }
    }
}

pub type Result<T> = std::result::Result<T, NotebookError>;

// ============================================================ enums

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotebookKind {
    Normal,
    Project,
    System,
    Agent,
}

impl NotebookKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Project => "project",
            Self::System => "system",
            Self::Agent => "agent",
        }
    }

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "normal" => Self::Normal,
            "project" => Self::Project,
            "system" => Self::System,
            "agent" => Self::Agent,
            other => {
                return Err(NotebookError::Storage(format!(
                    "bad notebook kind: {other}"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotebookStatus {
    Active,
    Archived,
    Deleted,
}

impl NotebookStatus {
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "active" => Self::Active,
            "archived" => Self::Archived,
            "deleted" => Self::Deleted,
            other => {
                return Err(NotebookError::Storage(format!(
                    "bad notebook status: {other}"
                )))
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotebookItemStatus {
    Active,
    Stale,
    Superseded,
    Deleted,
}

impl NotebookItemStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Deleted => "deleted",
        }
    }
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "active" => Self::Active,
            "stale" => Self::Stale,
            "superseded" => Self::Superseded,
            "deleted" => Self::Deleted,
            other => return Err(NotebookError::Storage(format!("bad item status: {other}"))),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            other => return Err(NotebookError::Storage(format!("bad confidence: {other}"))),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteReason {
    UserExplicit,
    StrongRule,
    ProjectState,
    CuratorExtracted,
    CuratorCleanup,
    ManualAdmin,
}

impl WriteReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::UserExplicit => "user_explicit",
            Self::StrongRule => "strong_rule",
            Self::ProjectState => "project_state",
            Self::CuratorExtracted => "curator_extracted",
            Self::CuratorCleanup => "curator_cleanup",
            Self::ManualAdmin => "manual_admin",
        }
    }
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "user_explicit" => Self::UserExplicit,
            "strong_rule" => Self::StrongRule,
            "project_state" => Self::ProjectState,
            "curator_extracted" => Self::CuratorExtracted,
            "curator_cleanup" => Self::CuratorCleanup,
            "manual_admin" => Self::ManualAdmin,
            other => return Err(NotebookError::Storage(format!("bad write reason: {other}"))),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotebookItemEdgeType {
    Supersedes,
    Related,
    ConflictsWith,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotebookEventType {
    #[serde(rename = "notebook.created")]
    NotebookCreated,
    #[serde(rename = "notebook.updated")]
    NotebookUpdated,
    #[serde(rename = "item.appended")]
    ItemAppended,
    #[serde(rename = "item.status_changed")]
    ItemStatusChanged,
    #[serde(rename = "item.superseded")]
    ItemSuperseded,
    #[serde(rename = "item.promoted_to_system")]
    ItemPromotedToSystem,
    #[serde(rename = "item.demoted_from_system")]
    ItemDemotedFromSystem,
    #[serde(rename = "item.remark_appended")]
    ItemRemarkAppended,
    #[serde(rename = "item.remark_removed")]
    ItemRemarkRemoved,
}

impl NotebookEventType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::NotebookCreated => "notebook.created",
            Self::NotebookUpdated => "notebook.updated",
            Self::ItemAppended => "item.appended",
            Self::ItemStatusChanged => "item.status_changed",
            Self::ItemSuperseded => "item.superseded",
            Self::ItemPromotedToSystem => "item.promoted_to_system",
            Self::ItemDemotedFromSystem => "item.demoted_from_system",
            Self::ItemRemarkAppended => "item.remark_appended",
            Self::ItemRemarkRemoved => "item.remark_removed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    AllActive,
    Latest,
    Title,
    Tags,
    Items,
}

impl ReadMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AllActive => "all_active",
            Self::Latest => "latest",
            Self::Title => "title",
            Self::Tags => "tags",
            Self::Items => "items",
        }
    }
}

// ============================================================ domain structs

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub r#type: String, // "session_message" | "tool_result" | "file" | "manual" | "system"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notebook {
    pub id: String,
    pub kind: NotebookKind,
    pub title: String,
    pub description: String,
    pub status: NotebookStatus,
    pub entry_count: i64,
    pub active_entry_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_updated_at: Option<String>,
    pub revision: i64,
    pub version: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotebookItem {
    pub item_id: String,
    pub notebook_id: String,
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    pub write_reason: WriteReason,
    pub confidence: Confidence,
    pub status: NotebookItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub item_revision: i64,
    pub content_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotebookItemRemark {
    pub remark_id: String,
    pub item_id: String,
    pub notebook_id: String,
    pub remark_type: String,
    pub content: String,
    pub status: NotebookItemRemarkStatus,
    pub created_at: String,
    pub updated_at: String,
}

// ============================================================ input / output

#[derive(Clone, Debug)]
pub struct ListNotebooksInput {
    pub include_archived: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct NotebookRegistryEntry {
    pub id: String,
    pub kind: NotebookKind,
    pub title: String,
    pub description: String,
    pub entry_count: i64,
    pub active_entry_count: i64,
    pub latest_title: Option<String>,
    pub latest_updated_at: Option<String>,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct CreateOrUpdateNotebookInput {
    pub notebook_id: String,
    pub kind: Option<NotebookKind>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CreateOrUpdateNotebookResult {
    pub notebook: Notebook,
    pub created: bool,
}

#[derive(Clone, Debug)]
pub struct ReadNotebookInput {
    pub session_id: Option<String>,
    pub notebook_id: String,
    pub tags: Option<Vec<String>>,
    pub title: Option<String>,
    pub latest_n: Option<usize>,
    pub item_ids: Option<Vec<String>>,
    pub since_version: Option<String>,
    pub include_status: Option<Vec<NotebookItemStatus>>,
    pub include_superseded: bool,
    pub max_items: Option<usize>,
    pub max_bytes: Option<usize>,
    pub allow_unchanged: bool,
}

impl Default for ReadNotebookInput {
    fn default() -> Self {
        Self {
            session_id: None,
            notebook_id: String::new(),
            tags: None,
            title: None,
            latest_n: None,
            item_ids: None,
            since_version: None,
            include_status: None,
            include_superseded: false,
            max_items: None,
            max_bytes: None,
            allow_unchanged: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ReadEntry {
    pub item_id: String,
    pub title: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    pub confidence: Confidence,
    pub status: NotebookItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_tags: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status")]
pub enum NotebookReadResult {
    #[serde(rename = "ok")]
    Ok {
        notebook_id: String,
        version: String,
        revision: i64,
        read_scope_hash: String,
        mode: ReadMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        matched_tags: Option<Vec<String>>,
        entries: Vec<ReadEntry>,
        truncated: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        truncated_inputs: Vec<String>,
    },
    #[serde(rename = "unchanged")]
    Unchanged {
        notebook_id: String,
        version: String,
        revision: i64,
        read_scope_hash: String,
        instruction: String,
    },
}

pub const UNCHANGED_INSTRUCTION: &str = "This notebook range has not changed since it was last read in this session. Use the earlier notebook content already present in the conversation history. Do not read it again unless the notebook changes or the user explicitly asks.";

#[derive(Clone, Debug)]
pub struct AppendNoteInput {
    pub session_id: Option<String>,
    pub notebook_id: String,
    pub title: String,
    pub content: String,
    pub source_excerpt: Option<String>,
    pub source_ref: Option<SourceRef>,
    pub source_session_id: Option<String>,
    pub write_reason: WriteReason,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub confidence: Option<Confidence>,
    pub tags: Vec<String>,
    pub detect_conflicts: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PossibleConflict {
    pub item_id: String,
    pub title: String,
    pub reason: ConflictReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_tags: Option<Vec<String>>,
    pub status: NotebookItemStatus,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictReason {
    SameTitle,
    NearTitle,
    TagOverlap,
    ActiveOverlap,
}

#[derive(Clone, Debug, Serialize)]
pub struct AppendNoteResult {
    pub status: &'static str,
    pub item_id: String,
    pub notebook_id: String,
    pub version: String,
    pub revision: i64,
    pub possible_conflicts: Vec<PossibleConflict>,
}

#[derive(Clone, Debug)]
pub struct MarkNoteStatusInput {
    pub session_id: Option<String>,
    pub item_id: String,
    pub status: NotebookItemStatus,
    pub reason: String,
    pub superseded_by: Option<String>,
    pub expected_item_revision: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MarkNoteStatusResult {
    pub status: &'static str,
    pub item_id: String,
    pub notebook_id: String,
    pub notebook_version: String,
    pub notebook_revision: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotebookItemRemarkStatus {
    Active,
    Deleted,
}

impl NotebookItemRemarkStatus {
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "active" => Self::Active,
            "deleted" => Self::Deleted,
            other => {
                return Err(NotebookError::Storage(format!(
                    "bad item remark status: {other}"
                )))
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct ListItemRemarksInput {
    pub item_id: String,
    pub remark_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppendItemRemarkInput {
    pub session_id: Option<String>,
    pub item_id: String,
    pub remark_type: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AppendItemRemarkResult {
    pub status: &'static str,
    pub remark_id: String,
    pub item_id: String,
    pub notebook_id: String,
    pub notebook_version: String,
    pub notebook_revision: i64,
}

#[derive(Clone, Debug)]
pub struct RemoveItemRemarkInput {
    pub session_id: Option<String>,
    pub item_id: String,
    pub remark_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RemoveItemRemarkResult {
    pub status: &'static str,
    pub remark_id: String,
    pub item_id: String,
    pub notebook_id: String,
    pub notebook_version: String,
    pub notebook_revision: i64,
}

#[derive(Clone, Debug)]
pub struct PromoteToSystemInput {
    pub item_id: String,
    pub reason: String,
    pub replace_item_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status")]
pub enum PromoteToSystemResult {
    #[serde(rename = "ok")]
    Ok {
        system_notebook_id: &'static str,
        active_system_count: usize,
        version: String,
    },
    #[serde(rename = "limit_exceeded")]
    LimitExceeded {
        system_notebook_id: &'static str,
        active_system_count: usize,
    },
}

#[derive(Clone, Debug)]
pub struct BuildRegistryContextInput {
    pub max_notebooks: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RegistryContext {
    pub block_type: &'static str,
    pub text: String,
    pub notebooks: Vec<NotebookRegistryEntry>,
}

#[derive(Clone, Debug)]
pub struct BuildSystemContextInput {
    pub max_items: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemContextItem {
    pub item_id: String,
    pub title: String,
    pub content: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemContext {
    pub block_type: &'static str,
    pub text: String,
    pub items: Vec<SystemContextItem>,
}

#[derive(Clone, Debug)]
pub struct BuildHintsInput {
    pub session_id: String,
    pub topic_tags: Option<Vec<String>>,
    pub candidate_notebook_ids: Option<Vec<String>>,
    pub max_hints: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HintReason {
    TopicRelevance,
    CrossSessionUpdate,
    NearTitleUpdate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HintSuppressionReason {
    AlreadyReadUnchanged,
    TooManyHints,
    NotRelevant,
}

#[derive(Clone, Debug, Serialize)]
pub struct NotebookHint {
    pub notebook_id: String,
    pub reason: HintReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_tags: Option<Vec<String>>,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SuppressedHint {
    pub notebook_id: String,
    pub reason: HintSuppressionReason,
}

#[derive(Clone, Debug, Serialize)]
pub struct HintsContext {
    pub block_type: &'static str,
    pub hints: Vec<NotebookHint>,
    pub suppressed: Vec<SuppressedHint>,
}

// ============================================================ main type

#[derive(Clone, Debug)]
pub struct AgentNotebookConfig {
    pub root: PathBuf,
}

impl AgentNotebookConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

/// File-backed Agent Notebook store.
///
/// Thread-safe: an internal `Mutex<Connection>` serializes writes. The
/// connection is opened with WAL journaling so multiple readers can proceed
/// in parallel, but for simplicity all access goes through the same handle.
pub struct AgentNotebook {
    cfg: AgentNotebookConfig,
    conn: Mutex<Connection>,
}

impl AgentNotebook {
    pub fn open(cfg: AgentNotebookConfig) -> Result<Self> {
        std::fs::create_dir_all(&cfg.root)?;
        let db_path = cfg.root.join("notebook.sqlite");
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        ensure_schema(&conn)?;
        Ok(Self {
            cfg,
            conn: Mutex::new(conn),
        })
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    // -------------------------------------------------- list_notebooks (§5.1)

    pub fn list_notebooks(&self, input: ListNotebooksInput) -> Result<Vec<NotebookRegistryEntry>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, title, description, entry_count, active_entry_count,
                    latest_title, latest_updated_at, version, status
             FROM notebooks
             WHERE (? OR status = 'active')
             ORDER BY id ASC",
        )?;
        let include_archived = if input.include_archived { 1 } else { 0 };
        let rows = stmt.query_map(params![include_archived], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, title, description, ec, aec, lt, lu, v, status) = row?;
            // Spec §11.2: registry never exposes deleted notebooks.
            if status == "deleted" {
                continue;
            }
            out.push(NotebookRegistryEntry {
                id,
                kind: NotebookKind::from_str(&kind)?,
                title,
                description,
                entry_count: ec,
                active_entry_count: aec,
                latest_title: lt,
                latest_updated_at: lu,
                version: v,
            });
        }
        Ok(out)
    }

    // ------------------------------------- create_or_update_notebook (§5.2)

    pub fn create_or_update_notebook(
        &self,
        input: CreateOrUpdateNotebookInput,
    ) -> Result<CreateOrUpdateNotebookResult> {
        let notebook_id = validate_notebook_id(&input.notebook_id)?;
        if let Some(t) = &input.title {
            if t.trim().is_empty() {
                return Err(NotebookError::InvalidInput("title is empty".into()));
            }
        }
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = now_iso8601();

        let existing = load_notebook(&tx, &notebook_id)?;
        let (notebook, created) = match existing {
            None => {
                let kind = input.kind.unwrap_or(default_kind_for_id(&notebook_id));
                let title = input.title.clone().unwrap_or_else(|| notebook_id.clone());
                let description = input.description.clone().unwrap_or_default();
                let revision: i64 = 1;
                let event_id = new_event_id();
                let version = make_version(revision, &event_id);
                tx.execute(
                    "INSERT INTO notebooks(
                        id, kind, title, description, status,
                        revision, version, latest_item_id, latest_title, latest_updated_at,
                        entry_count, active_entry_count, created_at, updated_at, archived_at
                     ) VALUES (?, ?, ?, ?, 'active', ?, ?, NULL, NULL, NULL, 0, 0, ?, ?, NULL)",
                    params![
                        notebook_id,
                        kind.as_str(),
                        title,
                        description,
                        revision,
                        version,
                        now,
                        now
                    ],
                )?;
                let seq = next_event_seq(&tx)?;
                insert_event(
                    &tx,
                    &EventRow {
                        event_id: event_id.clone(),
                        seq,
                        event_type: NotebookEventType::NotebookCreated,
                        notebook_id: notebook_id.clone(),
                        item_id: None,
                        actor_session_id: None,
                        title: Some(title.clone()),
                        summary: Some(format!("created notebook {notebook_id}")),
                        tags: None,
                        notebook_version: version.clone(),
                        notebook_revision: revision,
                        created_at: now.clone(),
                    },
                )?;
                let nb = load_notebook(&tx, &notebook_id)?.expect("notebook just inserted");
                (nb, true)
            }
            Some(mut nb) => {
                let mut changed = false;
                if let Some(k) = input.kind {
                    if k != nb.kind {
                        nb.kind = k;
                        changed = true;
                    }
                }
                if let Some(t) = input.title {
                    if t != nb.title {
                        nb.title = t;
                        changed = true;
                    }
                }
                if let Some(d) = input.description {
                    if d != nb.description {
                        nb.description = d;
                        changed = true;
                    }
                }
                if changed {
                    nb.revision += 1;
                    let event_id = new_event_id();
                    nb.version = make_version(nb.revision, &event_id);
                    nb.updated_at = now.clone();
                    tx.execute(
                        "UPDATE notebooks SET kind = ?, title = ?, description = ?,
                                              revision = ?, version = ?, updated_at = ?
                         WHERE id = ?",
                        params![
                            nb.kind.as_str(),
                            nb.title,
                            nb.description,
                            nb.revision,
                            nb.version,
                            nb.updated_at,
                            notebook_id
                        ],
                    )?;
                    let seq = next_event_seq(&tx)?;
                    insert_event(
                        &tx,
                        &EventRow {
                            event_id,
                            seq,
                            event_type: NotebookEventType::NotebookUpdated,
                            notebook_id: notebook_id.clone(),
                            item_id: None,
                            actor_session_id: None,
                            title: Some(nb.title.clone()),
                            summary: Some("notebook metadata updated".to_string()),
                            tags: None,
                            notebook_version: nb.version.clone(),
                            notebook_revision: nb.revision,
                            created_at: now.clone(),
                        },
                    )?;
                }
                (nb, false)
            }
        };

        tx.commit()?;
        Ok(CreateOrUpdateNotebookResult { notebook, created })
    }

    // -------------------------------------------------- read_notebook (§5.3)

    pub fn read_notebook(&self, input: ReadNotebookInput) -> Result<NotebookReadResult> {
        let notebook_id = validate_notebook_id(&input.notebook_id)?;

        // Resolve include_status. Default: active only. include_superseded sugar
        // expands to active+superseded.
        let mut include_status: HashSet<NotebookItemStatus> = input
            .include_status
            .clone()
            .map(|v| v.into_iter().collect())
            .unwrap_or_else(|| [NotebookItemStatus::Active].into_iter().collect());
        if input.include_superseded {
            include_status.insert(NotebookItemStatus::Active);
            include_status.insert(NotebookItemStatus::Superseded);
        }
        if include_status.is_empty() {
            include_status.insert(NotebookItemStatus::Active);
        }

        // Decide recall mode per §5.3.1.
        let mut truncated_inputs = Vec::new();
        let raw_tags = input.tags.clone().unwrap_or_default();
        let normalized_tags = normalize_tags(&raw_tags)?;
        let star_or_empty = raw_tags.is_empty()
            || raw_tags.iter().any(|t| t.trim() == "*") && normalized_tags.is_empty();
        // tags is empty after normalization OR contains only "*" → no filter.
        let tags_effective: Vec<String> = if star_or_empty {
            Vec::new()
        } else {
            normalized_tags.clone()
        };

        let mode: ReadMode;
        let max_items_default = DEFAULT_MAX_ITEMS;
        let max_items = input.max_items.unwrap_or(max_items_default);
        let max_bytes = input.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);

        if let Some(ids) = &input.item_ids {
            if !ids.is_empty() {
                mode = ReadMode::Items;
                if input.title.is_some() {
                    truncated_inputs.push("title".into());
                }
                if !tags_effective.is_empty() {
                    truncated_inputs.push("tags".into());
                }
                if input.latest_n.is_some() {
                    truncated_inputs.push("latest_n".into());
                }
            } else if input.title.is_some() {
                mode = ReadMode::Title;
            } else if !tags_effective.is_empty() {
                mode = ReadMode::Tags;
            } else if input.latest_n.is_some() {
                mode = ReadMode::Latest;
            } else {
                mode = ReadMode::AllActive;
            }
        } else if input.title.is_some() {
            mode = ReadMode::Title;
            if !tags_effective.is_empty() {
                truncated_inputs.push("tags".into());
            }
            if input.latest_n.is_some() {
                truncated_inputs.push("latest_n".into());
            }
        } else if !tags_effective.is_empty() {
            mode = ReadMode::Tags;
            // latest_n is allowed alongside tags — it just caps max_items.
        } else if input.latest_n.is_some() {
            mode = ReadMode::Latest;
        } else {
            mode = ReadMode::AllActive;
        }

        // Effective max_items: tags+latest_n => min(max_items, latest_n);
        // pure latest => latest_n is the cap.
        let mut effective_max_items = max_items;
        if let Some(n) = input.latest_n {
            effective_max_items = effective_max_items.min(n);
            if mode == ReadMode::Latest {
                effective_max_items = effective_max_items.min(n);
            }
        }
        if effective_max_items == 0 {
            effective_max_items = 1;
        }

        // Build read_scope_hash. Tag set is sorted/deduped during normalization.
        let read_scope_value = build_read_scope_value(
            mode,
            &tags_effective,
            input.title.as_deref(),
            input.latest_n,
            input.item_ids.as_deref(),
            &include_status,
        );
        let read_scope_hash = blake3_hex(canonical_json(&read_scope_value).as_bytes());

        let conn = self.lock_conn()?;
        let notebook = load_notebook(&conn, &notebook_id)?
            .ok_or_else(|| NotebookError::NotFound(format!("notebook {notebook_id}")))?;

        // Unchanged short-circuit (§5.3.3).
        if input.allow_unchanged {
            if let Some(sid) = input.session_id.as_deref() {
                // Direct hit on same scope.
                if let Some(prev_version) =
                    fetch_read_cache_version(&conn, sid, &notebook_id, &read_scope_hash)?
                {
                    if prev_version == notebook.version {
                        return Ok(NotebookReadResult::Unchanged {
                            notebook_id: notebook_id.clone(),
                            version: notebook.version.clone(),
                            revision: notebook.revision,
                            read_scope_hash,
                            instruction: UNCHANGED_INSTRUCTION.to_string(),
                        });
                    }
                }
                // since_version pin per §5.3.3 (3).
                if let Some(sv) = input.since_version.as_deref() {
                    if sv == notebook.version
                        && fetch_read_cache_version(&conn, sid, &notebook_id, &read_scope_hash)?
                            .is_some()
                    {
                        return Ok(NotebookReadResult::Unchanged {
                            notebook_id: notebook_id.clone(),
                            version: notebook.version.clone(),
                            revision: notebook.revision,
                            read_scope_hash,
                            instruction: UNCHANGED_INSTRUCTION.to_string(),
                        });
                    }
                }
                // If session previously read all_active on this notebook+version,
                // any narrower scope is already covered.
                if mode != ReadMode::AllActive
                    && session_has_all_active_at(&conn, sid, &notebook_id, &notebook.version)?
                {
                    return Ok(NotebookReadResult::Unchanged {
                        notebook_id: notebook_id.clone(),
                        version: notebook.version.clone(),
                        revision: notebook.revision,
                        read_scope_hash,
                        instruction: UNCHANGED_INSTRUCTION.to_string(),
                    });
                }
            }
        }

        // Resolve candidate item_ids per mode.
        let mut candidate_ids: Vec<String> = match mode {
            ReadMode::Items => input
                .item_ids
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            ReadMode::Title => {
                let needle = input.title.as_deref().unwrap_or("");
                title_candidates(&conn, &notebook_id, needle, &include_status)?
            }
            ReadMode::Tags => {
                tag_candidates(&conn, &notebook_id, &tags_effective, &include_status)?
            }
            ReadMode::Latest | ReadMode::AllActive => {
                all_candidates(&conn, &notebook_id, &include_status)?
            }
        };

        // Fetch the actual item rows.
        let items = fetch_items_by_ids(&conn, &candidate_ids)?;
        let mut items_map: BTreeMap<String, NotebookItem> = BTreeMap::new();
        for it in items {
            items_map.insert(it.item_id.clone(), it);
        }

        // For Items mode we keep input order; for everything else, apply the
        // canonical (updated_at DESC, created_at DESC, item_id ASC) sort (§5.3.4).
        let now_iso = now_iso8601();
        let mut sorted: Vec<NotebookItem> = candidate_ids
            .drain(..)
            .filter_map(|id| items_map.remove(&id))
            .filter(|it| !is_expired(it, &now_iso))
            .filter(|it| include_status.contains(&it.status))
            .collect();
        if mode != ReadMode::Items {
            sorted.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| b.created_at.cmp(&a.created_at))
                    .then_with(|| a.item_id.cmp(&b.item_id))
            });
        }

        // Compute matched_tags for tags mode.
        let lower_filter: Vec<String> = tags_effective.clone();
        let mut union_matched: BTreeMap<String, ()> = BTreeMap::new();

        // Apply max_items / max_bytes truncation.
        let mut truncated = false;
        let mut entries: Vec<ReadEntry> = Vec::new();
        let mut bytes_used = 0usize;
        for it in sorted.into_iter() {
            if entries.len() >= effective_max_items {
                truncated = true;
                break;
            }
            let mut per_entry_matched: Option<Vec<String>> = None;
            if mode == ReadMode::Tags {
                let item_tags: HashSet<&str> = it.tags.iter().map(|s| s.as_str()).collect();
                let mut m: Vec<String> = lower_filter
                    .iter()
                    .filter(|t| item_tags.contains(t.as_str()))
                    .cloned()
                    .collect();
                m.sort();
                m.dedup();
                for tag in &m {
                    union_matched.insert(tag.clone(), ());
                }
                per_entry_matched = Some(m);
            }
            let size_estimate = it.content.len() + it.title.len();
            if !entries.is_empty() && bytes_used.saturating_add(size_estimate) > max_bytes {
                truncated = true;
                break;
            }
            bytes_used = bytes_used.saturating_add(size_estimate);
            entries.push(ReadEntry {
                item_id: it.item_id,
                title: it.title,
                content: it.content,
                source_excerpt: it.source_excerpt,
                created_at: it.created_at,
                updated_at: it.updated_at,
                source_session_id: it.source_session_id,
                confidence: it.confidence,
                status: it.status,
                valid_from: it.valid_from,
                valid_until: it.valid_until,
                tags: it.tags,
                matched_tags: per_entry_matched,
            });
        }

        let matched_tags_total = if mode == ReadMode::Tags {
            Some(union_matched.into_keys().collect())
        } else {
            None
        };

        // Update session read cache (§5.3.3 #6).
        if let Some(sid) = input.session_id.as_deref() {
            let returned: Vec<String> = entries.iter().map(|e| e.item_id.clone()).collect();
            upsert_read_cache(
                &conn,
                sid,
                &notebook_id,
                &read_scope_hash,
                mode,
                &read_scope_value,
                &notebook.version,
                notebook.revision,
                &returned,
                input.max_bytes,
                &now_iso,
            )?;
        }

        Ok(NotebookReadResult::Ok {
            notebook_id,
            version: notebook.version,
            revision: notebook.revision,
            read_scope_hash,
            mode,
            matched_tags: matched_tags_total,
            entries,
            truncated,
            truncated_inputs,
        })
    }

    // -------------------------------------------------- append_note (§5.4)

    pub fn append_note(&self, input: AppendNoteInput) -> Result<AppendNoteResult> {
        let notebook_id = validate_notebook_id(&input.notebook_id)?;
        let title = input.title.trim();
        if title.is_empty() {
            return Err(NotebookError::InvalidInput("title is empty".into()));
        }
        if input.content.is_empty() {
            return Err(NotebookError::InvalidInput("content is empty".into()));
        }

        // Tag normalization (§3.6). Errors on any malformed tag.
        let tags = normalize_tags(&input.tags)?;
        if tags.len() > MAX_TAGS_PER_ITEM {
            return Err(NotebookError::InvalidInput(format!(
                "too many tags: {} (max {})",
                tags.len(),
                MAX_TAGS_PER_ITEM
            )));
        }

        let confidence = input.confidence.unwrap_or(Confidence::Medium);
        let source_ref_json = match &input.source_ref {
            Some(s) => Some(serde_json::to_string(s)?),
            None => None,
        };

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = now_iso8601();

        // Auto-create notebook if missing (mirrors the spec's append-first
        // ergonomics; create_or_update_notebook stays the canonical creator).
        let mut notebook = match load_notebook(&tx, &notebook_id)? {
            Some(nb) => nb,
            None => {
                let kind = default_kind_for_id(&notebook_id);
                let event_id = new_event_id();
                let revision: i64 = 1;
                let version = make_version(revision, &event_id);
                tx.execute(
                    "INSERT INTO notebooks(
                        id, kind, title, description, status,
                        revision, version, latest_item_id, latest_title, latest_updated_at,
                        entry_count, active_entry_count, created_at, updated_at, archived_at
                     ) VALUES (?, ?, ?, '', 'active', ?, ?, NULL, NULL, NULL, 0, 0, ?, ?, NULL)",
                    params![
                        notebook_id,
                        kind.as_str(),
                        notebook_id.clone(),
                        revision,
                        version,
                        now,
                        now
                    ],
                )?;
                let seq = next_event_seq(&tx)?;
                insert_event(
                    &tx,
                    &EventRow {
                        event_id: event_id.clone(),
                        seq,
                        event_type: NotebookEventType::NotebookCreated,
                        notebook_id: notebook_id.clone(),
                        item_id: None,
                        actor_session_id: None,
                        title: Some(notebook_id.clone()),
                        summary: Some(format!("auto-created notebook {notebook_id}")),
                        tags: None,
                        notebook_version: version.clone(),
                        notebook_revision: revision,
                        created_at: now.clone(),
                    },
                )?;
                load_notebook(&tx, &notebook_id)?.expect("notebook just inserted")
            }
        };

        // Detect conflicts before insertion (§5.4 #7-9).
        let mut conflicts: Vec<PossibleConflict> = Vec::new();
        if input.detect_conflicts {
            conflicts = detect_conflicts(
                &tx,
                &notebook_id,
                title,
                &tags,
                input.valid_from.as_deref(),
                input.valid_until.as_deref(),
            )?;
        }

        // Insert new item.
        let item_id = new_item_id();
        let content_hash = blake3_hex(input.content.as_bytes());
        tx.execute(
            "INSERT INTO notebook_items(
                item_id, notebook_id, title, content,
                source_excerpt, source_ref, source_session_id,
                write_reason, confidence, status, valid_from, valid_until,
                created_at, updated_at, item_revision, content_hash
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?, 1, ?)",
            params![
                item_id,
                notebook_id,
                title,
                input.content,
                input.source_excerpt,
                source_ref_json,
                input.source_session_id,
                input.write_reason.as_str(),
                confidence.as_str(),
                input.valid_from,
                input.valid_until,
                now,
                now,
                content_hash
            ],
        )?;

        // Tags.
        for t in &tags {
            tx.execute(
                "INSERT OR IGNORE INTO notebook_item_tags(item_id, tag) VALUES (?, ?)",
                params![item_id, t],
            )?;
        }

        // Notebook stats + version bump.
        notebook.revision += 1;
        notebook.entry_count += 1;
        notebook.active_entry_count += 1;
        notebook.latest_item_id = Some(item_id.clone());
        notebook.latest_title = Some(title.to_string());
        notebook.latest_updated_at = Some(now.clone());
        let event_id = new_event_id();
        notebook.version = make_version(notebook.revision, &event_id);
        notebook.updated_at = now.clone();
        tx.execute(
            "UPDATE notebooks SET revision = ?, version = ?, latest_item_id = ?,
                                  latest_title = ?, latest_updated_at = ?,
                                  entry_count = ?, active_entry_count = ?, updated_at = ?
             WHERE id = ?",
            params![
                notebook.revision,
                notebook.version,
                notebook.latest_item_id,
                notebook.latest_title,
                notebook.latest_updated_at,
                notebook.entry_count,
                notebook.active_entry_count,
                notebook.updated_at,
                notebook_id
            ],
        )?;

        let seq = next_event_seq(&tx)?;
        insert_event(
            &tx,
            &EventRow {
                event_id,
                seq,
                event_type: NotebookEventType::ItemAppended,
                notebook_id: notebook_id.clone(),
                item_id: Some(item_id.clone()),
                actor_session_id: input.session_id.clone(),
                title: Some(title.to_string()),
                summary: Some(short_summary(&input.content)),
                tags: Some(tags.clone()),
                notebook_version: notebook.version.clone(),
                notebook_revision: notebook.revision,
                created_at: now.clone(),
            },
        )?;
        tx.commit()?;
        Ok(AppendNoteResult {
            status: "ok",
            item_id,
            notebook_id,
            version: notebook.version,
            revision: notebook.revision,
            possible_conflicts: conflicts,
        })
    }

    // -------------------------------------------------- mark_note_status (§5.5)

    pub fn mark_note_status(&self, input: MarkNoteStatusInput) -> Result<MarkNoteStatusResult> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = now_iso8601();

        let row: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT notebook_id, status, item_revision FROM notebook_items
                 WHERE item_id = ?",
                params![input.item_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (notebook_id, prev_status_s, prev_rev) =
            row.ok_or_else(|| NotebookError::NotFound(format!("notebook item {}", input.item_id)))?;
        let prev_status = NotebookItemStatus::from_str(&prev_status_s)?;

        if let Some(expected) = input.expected_item_revision {
            if expected != prev_rev {
                return Err(NotebookError::VersionConflict(format!(
                    "expected item_revision {expected}, got {prev_rev}"
                )));
            }
        }

        if input.status == prev_status {
            // No-op but still bump item revision so callers can chain optimistic
            // locks; keep notebook version stable to avoid noise.
            tx.execute(
                "UPDATE notebook_items SET item_revision = item_revision + 1, updated_at = ?
                 WHERE item_id = ?",
                params![now, input.item_id],
            )?;
            let nb = load_notebook(&tx, &notebook_id)?
                .ok_or_else(|| NotebookError::Storage("notebook missing".into()))?;
            tx.commit()?;
            return Ok(MarkNoteStatusResult {
                status: "ok",
                item_id: input.item_id,
                notebook_id,
                notebook_version: nb.version,
                notebook_revision: nb.revision,
            });
        }

        // Apply the new status.
        tx.execute(
            "UPDATE notebook_items SET status = ?, updated_at = ?,
                                       item_revision = item_revision + 1
             WHERE item_id = ?",
            params![input.status.as_str(), now, input.item_id],
        )?;

        // Adjust notebook active counts.
        let mut nb = load_notebook(&tx, &notebook_id)?
            .ok_or_else(|| NotebookError::Storage("notebook missing".into()))?;
        let was_active = prev_status == NotebookItemStatus::Active;
        let now_active = input.status == NotebookItemStatus::Active;
        if was_active && !now_active {
            nb.active_entry_count -= 1;
        } else if !was_active && now_active {
            nb.active_entry_count += 1;
        }
        if input.status == NotebookItemStatus::Deleted && prev_status != NotebookItemStatus::Deleted
        {
            nb.entry_count -= 1;
        } else if prev_status == NotebookItemStatus::Deleted
            && input.status != NotebookItemStatus::Deleted
        {
            nb.entry_count += 1;
        }
        nb.revision += 1;
        let event_id = new_event_id();
        nb.version = make_version(nb.revision, &event_id);
        nb.updated_at = now.clone();
        tx.execute(
            "UPDATE notebooks SET revision = ?, version = ?, updated_at = ?,
                                  active_entry_count = ?, entry_count = ?
             WHERE id = ?",
            params![
                nb.revision,
                nb.version,
                nb.updated_at,
                nb.active_entry_count,
                nb.entry_count,
                notebook_id
            ],
        )?;

        // Supersede edge + cascading event.
        let mut event_type = NotebookEventType::ItemStatusChanged;
        if input.status == NotebookItemStatus::Superseded {
            if let Some(by) = &input.superseded_by {
                tx.execute(
                    "INSERT OR REPLACE INTO notebook_item_edges(
                        from_item_id, to_item_id, edge_type, reason, created_at, created_by
                     ) VALUES (?, ?, 'supersedes', ?, ?, ?)",
                    params![by, input.item_id, input.reason, now, ""],
                )?;
            }
            event_type = NotebookEventType::ItemSuperseded;
        }

        let seq = next_event_seq(&tx)?;
        insert_event(
            &tx,
            &EventRow {
                event_id,
                seq,
                event_type,
                notebook_id: notebook_id.clone(),
                item_id: Some(input.item_id.clone()),
                actor_session_id: input.session_id.clone(),
                title: None,
                summary: Some(format!(
                    "{} -> {}: {}",
                    prev_status.as_str(),
                    input.status.as_str(),
                    short_summary(&input.reason)
                )),
                tags: None,
                notebook_version: nb.version.clone(),
                notebook_revision: nb.revision,
                created_at: now.clone(),
            },
        )?;

        tx.commit()?;
        Ok(MarkNoteStatusResult {
            status: "ok",
            item_id: input.item_id,
            notebook_id,
            notebook_version: nb.version,
            notebook_revision: nb.revision,
        })
    }

    pub fn list_item_remarks(
        &self,
        input: ListItemRemarksInput,
    ) -> Result<Vec<NotebookItemRemark>> {
        let item_id = validate_item_id(&input.item_id)?;
        let remark_type = input
            .remark_type
            .as_deref()
            .map(normalize_remark_type)
            .transpose()?;
        let conn = self.lock_conn()?;
        let notebook_id = item_notebook_id(&conn, &item_id)?
            .ok_or_else(|| NotebookError::NotFound(format!("notebook item {item_id}")))?;
        let mut query = String::from(
            "SELECT remark_id, remark_type, content, status,
                    created_at, updated_at
             FROM notebook_item_remarks
             WHERE item_id = ? AND status = 'active'",
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&item_id];
        if let Some(remark_type) = &remark_type {
            query.push_str(" AND remark_type = ?");
            params_vec.push(remark_type);
        }
        query.push_str(" ORDER BY created_at ASC, remark_id ASC");
        let mut stmt = conn.prepare(&query)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (remark_id, remark_type, content, status, created_at, updated_at) = row?;
            out.push(NotebookItemRemark {
                remark_id,
                item_id: item_id.clone(),
                notebook_id: notebook_id.clone(),
                remark_type,
                content,
                status: NotebookItemRemarkStatus::from_str(&status)?,
                created_at,
                updated_at,
            });
        }
        Ok(out)
    }

    pub fn append_item_remark(
        &self,
        input: AppendItemRemarkInput,
    ) -> Result<AppendItemRemarkResult> {
        let item_id = validate_item_id(&input.item_id)?;
        let remark_type = normalize_remark_type(&input.remark_type)?;
        if input.content.trim().is_empty() {
            return Err(NotebookError::InvalidInput(
                "remark content is empty".into(),
            ));
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = now_iso8601();
        let (notebook_id, item_title, item_status) = load_item_remark_target(&tx, &item_id)?
            .ok_or_else(|| NotebookError::NotFound(format!("notebook item {item_id}")))?;
        if item_status == NotebookItemStatus::Deleted {
            return Err(NotebookError::InvalidInput(
                "cannot append remark to deleted item".into(),
            ));
        }

        let remark_id = new_remark_id();
        tx.execute(
            "INSERT INTO notebook_item_remarks(
                remark_id, item_id, remark_type, content, status, created_at, updated_at
             ) VALUES (?, ?, ?, ?, 'active', ?, ?)",
            params![remark_id, item_id, remark_type, input.content, now, now],
        )?;
        tx.execute(
            "UPDATE notebook_items SET updated_at = ?, item_revision = item_revision + 1
             WHERE item_id = ?",
            params![now, item_id],
        )?;
        let event_id = new_event_id();
        let notebook_version = bump_notebook_version(&tx, &notebook_id, &now, Some(&event_id))?;
        let notebook_revision = notebook_revision(&tx, &notebook_id)?;
        let tags = fetch_item_tags(&tx, &item_id)?;
        let seq = next_event_seq(&tx)?;
        insert_event(
            &tx,
            &EventRow {
                event_id,
                seq,
                event_type: NotebookEventType::ItemRemarkAppended,
                notebook_id: notebook_id.clone(),
                item_id: Some(item_id.clone()),
                actor_session_id: input.session_id.clone(),
                title: Some(item_title),
                summary: Some(format!("{remark_type}: {}", short_summary(&input.content))),
                tags: Some(tags),
                notebook_version: notebook_version.clone(),
                notebook_revision,
                created_at: now.clone(),
            },
        )?;
        tx.commit()?;
        Ok(AppendItemRemarkResult {
            status: "ok",
            remark_id,
            item_id,
            notebook_id,
            notebook_version,
            notebook_revision,
        })
    }

    pub fn remove_item_remark(
        &self,
        input: RemoveItemRemarkInput,
    ) -> Result<RemoveItemRemarkResult> {
        let item_id = validate_item_id(&input.item_id)?;
        let remark_id = validate_remark_id(&input.remark_id)?;
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = now_iso8601();
        let (notebook_id, item_title, remark_type) =
            load_active_remark_target(&tx, &item_id, &remark_id)?.ok_or_else(|| {
                NotebookError::NotFound(format!("remark {remark_id} on notebook item {item_id}"))
            })?;
        tx.execute(
            "UPDATE notebook_item_remarks SET status = 'deleted', updated_at = ?
             WHERE remark_id = ? AND item_id = ?",
            params![now, remark_id, item_id],
        )?;
        tx.execute(
            "UPDATE notebook_items SET updated_at = ?, item_revision = item_revision + 1
             WHERE item_id = ?",
            params![now, item_id],
        )?;
        let event_id = new_event_id();
        let notebook_version = bump_notebook_version(&tx, &notebook_id, &now, Some(&event_id))?;
        let notebook_revision = notebook_revision(&tx, &notebook_id)?;
        let tags = fetch_item_tags(&tx, &item_id)?;
        let seq = next_event_seq(&tx)?;
        insert_event(
            &tx,
            &EventRow {
                event_id,
                seq,
                event_type: NotebookEventType::ItemRemarkRemoved,
                notebook_id: notebook_id.clone(),
                item_id: Some(item_id.clone()),
                actor_session_id: input.session_id.clone(),
                title: Some(item_title),
                summary: Some(format!("removed remark {remark_type}")),
                tags: Some(tags),
                notebook_version: notebook_version.clone(),
                notebook_revision,
                created_at: now.clone(),
            },
        )?;
        tx.commit()?;
        Ok(RemoveItemRemarkResult {
            status: "ok",
            remark_id,
            item_id,
            notebook_id,
            notebook_version,
            notebook_revision,
        })
    }

    // --------------------------------- promote_to_system_notebook (§5.6)

    pub fn promote_to_system_notebook(
        &self,
        input: PromoteToSystemInput,
    ) -> Result<PromoteToSystemResult> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;
        let now = now_iso8601();

        // Fetch source item.
        let src: Option<(String, String, String, String, Option<String>)> = tx
            .query_row(
                "SELECT notebook_id, status, confidence, title, valid_until
                 FROM notebook_items
                 WHERE item_id = ?",
                params![input.item_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        let (src_nb_id, status, confidence, title, valid_until) =
            src.ok_or_else(|| NotebookError::NotFound(format!("item {}", input.item_id)))?;
        let status = NotebookItemStatus::from_str(&status)?;
        let confidence = Confidence::from_str(&confidence)?;
        if status != NotebookItemStatus::Active {
            return Err(NotebookError::InvalidInput(
                "only active items may be promoted".into(),
            ));
        }
        if confidence != Confidence::High {
            return Err(NotebookError::InvalidInput(
                "only high-confidence items may be promoted".into(),
            ));
        }
        if let Some(vu) = &valid_until {
            if vu.as_str() <= now.as_str() {
                return Err(NotebookError::InvalidInput(
                    "expired items may not be promoted".into(),
                ));
            }
        }

        // Ensure system notebook exists.
        ensure_system_notebook(&tx, &now)?;

        // Compute active count after the prospective promotion.
        let active_in_system: usize = tx.query_row(
            "SELECT COUNT(*) FROM notebook_items
             WHERE notebook_id = ? AND status = 'active'",
            params![SYSTEM_NOTEBOOK_ID],
            |r| r.get::<_, i64>(0),
        )? as usize;

        // If replace_item_id given, soft-demote it first (frees a slot).
        if let Some(rep_id) = &input.replace_item_id {
            tx.execute(
                "UPDATE notebook_items SET status = 'superseded', updated_at = ?,
                                            item_revision = item_revision + 1
                 WHERE item_id = ? AND notebook_id = ?",
                params![now, rep_id, SYSTEM_NOTEBOOK_ID],
            )?;
            // The replaced count is one less active in system; recompute below.
        }
        let active_after_replace: usize = tx.query_row(
            "SELECT COUNT(*) FROM notebook_items
             WHERE notebook_id = ? AND status = 'active'",
            params![SYSTEM_NOTEBOOK_ID],
            |r| r.get::<_, i64>(0),
        )? as usize;

        if active_after_replace >= SYSTEM_NOTEBOOK_MAX_ACTIVE {
            // Rollback potential replacement by rolling back the whole tx.
            tx.rollback()?;
            return Ok(PromoteToSystemResult::LimitExceeded {
                system_notebook_id: SYSTEM_NOTEBOOK_ID,
                active_system_count: active_in_system,
            });
        }

        // Move item to system notebook. We update notebook_id on the existing
        // item to preserve audit history (created_at, actor, source_ref…).
        tx.execute(
            "UPDATE notebook_items SET notebook_id = ?, updated_at = ?,
                                        item_revision = item_revision + 1
             WHERE item_id = ?",
            params![SYSTEM_NOTEBOOK_ID, now, input.item_id],
        )?;

        // Bump source notebook stats.
        adjust_notebook_counts(&tx, &src_nb_id, -1, -1)?;
        bump_notebook_version(&tx, &src_nb_id, &now, None)?;

        // Bump system notebook stats.
        adjust_notebook_counts(&tx, SYSTEM_NOTEBOOK_ID, 1, 1)?;
        let sys_event_id = new_event_id();
        let sys_nb_version =
            bump_notebook_version(&tx, SYSTEM_NOTEBOOK_ID, &now, Some(&sys_event_id))?;

        // Promote event under the system notebook.
        let seq = next_event_seq(&tx)?;
        insert_event(
            &tx,
            &EventRow {
                event_id: sys_event_id,
                seq,
                event_type: NotebookEventType::ItemPromotedToSystem,
                notebook_id: SYSTEM_NOTEBOOK_ID.to_string(),
                item_id: Some(input.item_id.clone()),
                actor_session_id: None,
                title: Some(title.clone()),
                summary: Some(short_summary(&input.reason)),
                tags: None,
                notebook_version: sys_nb_version.clone(),
                notebook_revision: 0, // filled by helper, see below
                created_at: now.clone(),
            },
        )?;

        // Recompute final active count for the return value.
        let final_count: usize = tx.query_row(
            "SELECT COUNT(*) FROM notebook_items
             WHERE notebook_id = ? AND status = 'active'",
            params![SYSTEM_NOTEBOOK_ID],
            |r| r.get::<_, i64>(0),
        )? as usize;
        tx.commit()?;

        Ok(PromoteToSystemResult::Ok {
            system_notebook_id: SYSTEM_NOTEBOOK_ID,
            active_system_count: final_count,
            version: sys_nb_version,
        })
    }

    // ----------------------------- build_notebook_registry_context (§5.7)

    pub fn build_notebook_registry_context(
        &self,
        input: BuildRegistryContextInput,
    ) -> Result<RegistryContext> {
        let mut notebooks = self.list_notebooks(ListNotebooksInput {
            include_archived: false,
        })?;
        let cap = input.max_notebooks.unwrap_or(50);
        if notebooks.len() > cap {
            notebooks.truncate(cap);
        }
        let mut text = String::new();
        text.push_str("Available notebooks:\n");
        for nb in &notebooks {
            text.push_str(&format!(
                "- {}: {}, {} active entries{}{}, version {}\n",
                nb.id,
                if nb.description.is_empty() {
                    nb.title.as_str()
                } else {
                    nb.description.as_str()
                },
                nb.active_entry_count,
                nb.latest_updated_at
                    .as_deref()
                    .map(|t| format!(", last updated {}", t))
                    .unwrap_or_default(),
                nb.latest_title
                    .as_deref()
                    .map(|t| format!(", latest: \"{}\"", t))
                    .unwrap_or_default(),
                nb.version,
            ));
        }
        text.push_str(
            "Use notebook contents only when relevant. Do not read a notebook repeatedly if it has not changed since the last read in this session.\n",
        );
        Ok(RegistryContext {
            block_type: "notebook_registry",
            text,
            notebooks,
        })
    }

    pub fn build_notebook_list_text(&self) -> Result<String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, active_entry_count, updated_at
             FROM notebooks
             WHERE status = 'active'
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;

        let mut notebooks = Vec::new();
        for row in rows {
            notebooks.push(row?);
        }

        if notebooks.is_empty() {
            return Ok("Available notebooks: 0 total.\n".to_string());
        }

        let mut text = format!("Available notebooks: {} total.\n", notebooks.len());
        for (id, title, description, active_entry_count, updated_at) in notebooks {
            let label = if description.is_empty() {
                title.as_str()
            } else {
                description.as_str()
            };
            text.push_str(&format!(
                "- {}: {}, {} records, last modified {}\n",
                id, label, active_entry_count, updated_at
            ));
        }
        Ok(text)
    }

    pub fn build_recent_items_text(
        &self,
        max_items: usize,
        since_access_at_secs: u64,
    ) -> Result<String> {
        self.build_recent_items_text_inner(max_items, since_access_at_secs, false)
    }

    pub fn build_recent_items_text_for_self_check(
        &self,
        max_items: usize,
        since_access_at_secs: u64,
    ) -> Result<String> {
        self.build_recent_items_text_inner(max_items, since_access_at_secs, true)
    }

    /// Max `updated_at` (unix seconds) across all active items in active
    /// notebooks, or 0 when there are none. Cheap single-row query used by the
    /// Self-Check wakeup gate to detect "the Notebook changed" without
    /// rendering the full prompt or invoking the LLM. Note that appending a
    /// remark *does* bump the item's `updated_at`, so the gate compares against
    /// a watermark it advances after each round (see agent_session.rs) rather
    /// than treating any nonzero value as a change.
    pub fn latest_active_item_update_secs(&self) -> Result<u64> {
        let conn = self.lock_conn()?;
        let max_updated_at: Option<String> = conn.query_row(
            "SELECT MAX(i.updated_at)
             FROM notebook_items i
             JOIN notebooks n
               ON n.id = i.notebook_id
             WHERE i.status = 'active'
               AND n.status = 'active'",
            [],
            |r| r.get::<_, Option<String>>(0),
        )?;
        let secs = max_updated_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp().max(0) as u64)
            .unwrap_or(0);
        Ok(secs)
    }

    fn build_recent_items_text_inner(
        &self,
        max_items: usize,
        since_access_at_secs: u64,
        include_unreviewed: bool,
    ) -> Result<String> {
        let max_items = max_items.max(1);
        let conn = self.lock_conn()?;
        let now = now_iso8601();
        let since = since_access_at_secs_to_iso8601(since_access_at_secs);
        let mut stmt = conn.prepare(
            "SELECT i.item_id, i.status, i.updated_at, i.source_session_id, i.content
             FROM notebook_items i
             JOIN notebooks n
               ON n.id = i.notebook_id
             WHERE i.status = 'active'
               AND n.status = 'active'
               AND (i.valid_until IS NULL OR i.valid_until >= ?)
               AND (
                    ? = ''
                    OR i.updated_at >= ?
                    OR EXISTS (
                        SELECT 1
                        FROM notebook_item_remarks r
                        WHERE r.item_id = i.item_id
                          AND r.status = 'active'
                          AND r.remark_type = 'self_check'
                          AND r.content LIKE '%keep_observing%'
                    )
                    OR (
                        ? != 0
                        AND NOT EXISTS (
                            SELECT 1
                            FROM notebook_item_remarks r
                            WHERE r.item_id = i.item_id
                              AND r.status = 'active'
                              AND r.remark_type = 'self_check'
                        )
                    )
               )
             ORDER BY i.updated_at DESC, i.created_at DESC, i.item_id ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(
            params![
                now,
                since,
                since,
                if include_unreviewed { 1_i64 } else { 0_i64 },
                max_items as i64
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }

        if items.is_empty() {
            return Ok("Recent notebook items: none.\n".to_string());
        }

        let mut text = format!("Recent notebook items: latest {}.\n", items.len());
        for (item_id, status, updated_at, source_session_id, content) in items {
            text.push_str(&format!(
                "{} {} {} {}\n{}\n",
                item_id,
                status,
                updated_at,
                source_session_id.as_deref().unwrap_or("-"),
                content
            ));
        }
        Ok(text)
    }

    // ----------------------------- build_system_notebook_context (§5.8)

    pub fn build_system_notebook_context(
        &self,
        input: BuildSystemContextInput,
    ) -> Result<SystemContext> {
        let max = input.max_items.unwrap_or(SYSTEM_NOTEBOOK_MAX_ACTIVE);
        let conn = self.lock_conn()?;
        let now = now_iso8601();
        let mut stmt = conn.prepare(
            "SELECT item_id, title, content, updated_at, valid_until, confidence
             FROM notebook_items
             WHERE notebook_id = ? AND status = 'active'
             ORDER BY updated_at DESC, created_at DESC, item_id ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![SYSTEM_NOTEBOOK_ID, max as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut items = Vec::new();
        for row in rows {
            let (id, title, content, upd, valid_until, confidence) = row?;
            // §5.8 #4: skip low confidence and expired.
            let confidence = Confidence::from_str(&confidence)?;
            if confidence == Confidence::Low {
                continue;
            }
            if let Some(vu) = &valid_until {
                if vu.as_str() <= now.as_str() {
                    continue;
                }
            }
            items.push(SystemContextItem {
                item_id: id,
                title,
                content,
                updated_at: upd,
            });
        }
        let mut text = String::new();
        if items.is_empty() {
            text.push_str("(no system notebook items)\n");
        } else {
            text.push_str("System facts:\n");
            for it in &items {
                text.push_str(&format!("- {}: {}\n", it.title, it.content));
            }
        }
        Ok(SystemContext {
            block_type: "system_notebook",
            text,
            items,
        })
    }

    // -------------------------------- build_notebook_hints (§5.9)

    pub fn build_notebook_hints(&self, input: BuildHintsInput) -> Result<HintsContext> {
        let topic_tags = match &input.topic_tags {
            Some(t) => normalize_tags(t)?,
            None => Vec::new(),
        };
        let max_hints = input.max_hints.unwrap_or(DEFAULT_MAX_HINTS);

        let conn = self.lock_conn()?;
        let mut hints: Vec<NotebookHint> = Vec::new();
        let mut suppressed: Vec<SuppressedHint> = Vec::new();
        let mut seen_notebooks: HashSet<String> = HashSet::new();

        // 1) Cross-session update hints from event log (§5.9 #5, §7).
        let watermark_seq = fetch_watermark_seq(&conn, &input.session_id)?;
        let mut event_stmt = conn.prepare(
            "SELECT e.seq, e.event_id, e.notebook_id, e.event_type, e.title, e.tags,
                    e.notebook_version, e.created_at, e.actor_session_id, n.version
             FROM notebook_events e
             JOIN notebooks n ON n.id = e.notebook_id
             WHERE e.seq > ?
               AND (e.actor_session_id IS NULL OR e.actor_session_id != ?)
             ORDER BY e.seq ASC",
        )?;
        let rows = event_stmt.query_map(params![watermark_seq, input.session_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, String>(9)?,
            ))
        })?;
        let mut latest_seq = watermark_seq;
        let mut per_notebook: HashMap<
            String,
            (String, Option<String>, Option<Vec<String>>, String),
        > = HashMap::new();
        for row in rows {
            let (seq, _ev_id, nb_id, _ty, title, tags_s, _nv, created_at, _actor, current_v) = row?;
            latest_seq = latest_seq.max(seq);
            let tags = match tags_s {
                Some(s) if !s.is_empty() => serde_json::from_str::<Vec<String>>(&s).ok(),
                _ => None,
            };
            per_notebook.insert(
                nb_id,
                (title.unwrap_or_default(), Some(created_at), tags, current_v),
            );
        }
        for (nb_id, (title, upd, tags, version)) in per_notebook.drain() {
            if has_session_read(&conn, &input.session_id, &nb_id)? {
                if session_has_unchanged_for_notebook(&conn, &input.session_id, &nb_id, &version)? {
                    suppressed.push(SuppressedHint {
                        notebook_id: nb_id,
                        reason: HintSuppressionReason::AlreadyReadUnchanged,
                    });
                    continue;
                }
                // Session has touched this notebook → cross-session hint applies.
                if hints.len() >= max_hints {
                    suppressed.push(SuppressedHint {
                        notebook_id: nb_id,
                        reason: HintSuppressionReason::TooManyHints,
                    });
                    continue;
                }
                let matched = if !topic_tags.is_empty() {
                    if let Some(t) = &tags {
                        let set: HashSet<&str> = t.iter().map(|s| s.as_str()).collect();
                        let m: Vec<String> = topic_tags
                            .iter()
                            .filter(|tt| set.contains(tt.as_str()))
                            .cloned()
                            .collect();
                        if m.is_empty() {
                            None
                        } else {
                            Some(m)
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                let text = format!(
                    "Notebook update since your last turn: {} was updated{}{}",
                    nb_id,
                    upd.as_deref()
                        .map(|t| format!(" at {}", t))
                        .unwrap_or_default(),
                    matched
                        .as_ref()
                        .map(|m| format!(" (tag overlap: {})", m.join(",")))
                        .unwrap_or_default(),
                );
                hints.push(NotebookHint {
                    notebook_id: nb_id.clone(),
                    reason: HintReason::CrossSessionUpdate,
                    title: if title.is_empty() { None } else { Some(title) },
                    updated_at: upd,
                    version,
                    matched_tags: matched,
                    text,
                });
                seen_notebooks.insert(nb_id);
            }
        }

        // 2) Topic relevance hints (§5.9 #4). Only when topic_tags is non-empty.
        if !topic_tags.is_empty() && hints.len() < max_hints {
            let candidates = topic_relevance_candidates(
                &conn,
                &topic_tags,
                input.candidate_notebook_ids.as_deref(),
            )?;
            for (nb_id, latest_title, latest_upd, matched, version) in candidates {
                if hints.len() >= max_hints {
                    suppressed.push(SuppressedHint {
                        notebook_id: nb_id,
                        reason: HintSuppressionReason::TooManyHints,
                    });
                    continue;
                }
                if seen_notebooks.contains(&nb_id) {
                    continue;
                }
                // Suppress already-read & unchanged (§5.9 #3).
                if session_has_unchanged_for_notebook(&conn, &input.session_id, &nb_id, &version)? {
                    suppressed.push(SuppressedHint {
                        notebook_id: nb_id,
                        reason: HintSuppressionReason::AlreadyReadUnchanged,
                    });
                    continue;
                }
                let text = format!(
                    "A notebook may contain relevant information about this topic. Read {} if needed.",
                    nb_id
                );
                hints.push(NotebookHint {
                    notebook_id: nb_id.clone(),
                    reason: HintReason::TopicRelevance,
                    title: latest_title,
                    updated_at: latest_upd,
                    version,
                    matched_tags: Some(matched),
                    text,
                });
                seen_notebooks.insert(nb_id);
            }
        }

        // 3) Advance watermark so we don't repeat the same events next turn.
        if latest_seq > watermark_seq {
            upsert_watermark(&conn, &input.session_id, latest_seq)?;
        }

        Ok(HintsContext {
            block_type: "notebook_hints",
            hints,
            suppressed,
        })
    }

    // ---------------------------------------------------- helpers

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| NotebookError::Storage(format!("connection lock poisoned: {e}")))
    }
}

// ============================================================ free helpers

fn ensure_schema(conn: &Connection) -> Result<()> {
    if table_has_column(conn, "notebooks", "owner_user_id")?
        || table_has_column(conn, "notebook_items", "actor_kind")?
        || table_has_column(conn, "notebook_items", "metadata")?
    {
        conn.execute_batch(
            "DROP TABLE IF EXISTS session_watermarks;
             DROP TABLE IF EXISTS session_reads;
             DROP TABLE IF EXISTS notebook_events;
             DROP TABLE IF EXISTS notebook_item_remarks;
             DROP TABLE IF EXISTS notebook_item_edges;
             DROP TABLE IF EXISTS notebook_item_tags;
             DROP TABLE IF EXISTS notebook_items;
             DROP TABLE IF EXISTS notebooks;",
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notebooks (
            id                 TEXT NOT NULL,
            kind               TEXT NOT NULL,
            title              TEXT NOT NULL,
            description        TEXT NOT NULL DEFAULT '',
            status             TEXT NOT NULL DEFAULT 'active',
            revision           INTEGER NOT NULL DEFAULT 0,
            version            TEXT NOT NULL,
            latest_item_id     TEXT,
            latest_title       TEXT,
            latest_updated_at  TEXT,
            entry_count        INTEGER NOT NULL DEFAULT 0,
            active_entry_count INTEGER NOT NULL DEFAULT 0,
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            archived_at        TEXT,
            PRIMARY KEY(id)
         );
         CREATE TABLE IF NOT EXISTS notebook_items (
            item_id            TEXT PRIMARY KEY,
            notebook_id        TEXT NOT NULL,
            title              TEXT NOT NULL,
            content            TEXT NOT NULL,
            source_excerpt     TEXT,
            source_ref         TEXT,
            source_session_id  TEXT,
            write_reason       TEXT NOT NULL,
            confidence         TEXT NOT NULL DEFAULT 'medium',
            status             TEXT NOT NULL DEFAULT 'active',
            valid_from         TEXT,
            valid_until        TEXT,
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            item_revision      INTEGER NOT NULL DEFAULT 1,
            content_hash       TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_items_nb_status_upd
            ON notebook_items(notebook_id, status, updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_items_title
            ON notebook_items(notebook_id, title);
         CREATE TABLE IF NOT EXISTS notebook_item_tags (
            item_id TEXT NOT NULL,
            tag     TEXT NOT NULL,
            PRIMARY KEY(item_id, tag)
         );
         CREATE INDEX IF NOT EXISTS idx_item_tags_tag ON notebook_item_tags(tag);
         CREATE TABLE IF NOT EXISTS notebook_item_edges (
            from_item_id TEXT NOT NULL,
            to_item_id   TEXT NOT NULL,
            edge_type    TEXT NOT NULL,
            reason       TEXT,
            created_at   TEXT NOT NULL,
            created_by   TEXT,
            PRIMARY KEY(from_item_id, to_item_id, edge_type)
         );
         CREATE TABLE IF NOT EXISTS notebook_item_remarks (
            remark_id       TEXT PRIMARY KEY,
            item_id         TEXT NOT NULL,
            remark_type     TEXT NOT NULL,
            content         TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'active',
            created_at      TEXT NOT NULL,
            updated_at      TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_item_remarks_item_type_status
            ON notebook_item_remarks(item_id, remark_type, status, created_at);
         CREATE TABLE IF NOT EXISTS notebook_events (
            event_id          TEXT PRIMARY KEY,
            seq               INTEGER NOT NULL,
            event_type        TEXT NOT NULL,
            notebook_id       TEXT NOT NULL,
            item_id           TEXT,
            actor_session_id  TEXT,
            title             TEXT,
            summary           TEXT,
            tags              TEXT,
            notebook_version  TEXT NOT NULL,
            notebook_revision INTEGER NOT NULL,
            created_at        TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_events_seq ON notebook_events(seq);
         CREATE INDEX IF NOT EXISTS idx_events_nb_seq
            ON notebook_events(notebook_id, seq);
         CREATE TABLE IF NOT EXISTS session_reads (
            session_id        TEXT NOT NULL,
            notebook_id       TEXT NOT NULL,
            read_scope_hash   TEXT NOT NULL,
            mode              TEXT NOT NULL,
            read_scope        TEXT NOT NULL,
            notebook_version  TEXT NOT NULL,
            notebook_revision INTEGER NOT NULL,
            returned_item_ids TEXT NOT NULL,
            max_bytes         INTEGER,
            read_at           TEXT NOT NULL,
            PRIMARY KEY(session_id, notebook_id, read_scope_hash)
         );
         CREATE INDEX IF NOT EXISTS idx_session_reads_nb
            ON session_reads(session_id, notebook_id);
         CREATE TABLE IF NOT EXISTS session_watermarks (
            session_id     TEXT NOT NULL,
            last_seen_seq  INTEGER NOT NULL DEFAULT 0,
            last_seen_at   TEXT NOT NULL,
            PRIMARY KEY(session_id)
         );",
    )?;
    Ok(())
}

fn table_has_column(conn: &Connection, table_name: &str, column_name: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for row in rows {
        if row? == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_notebook_id(id: &str) -> Result<String> {
    let t = id.trim();
    if t.is_empty() {
        return Err(NotebookError::InvalidInput("notebook_id is empty".into()));
    }
    if t.len() > 200 {
        return Err(NotebookError::InvalidInput("notebook_id too long".into()));
    }
    if t.contains('\0') || t.chars().any(|c| c.is_control()) {
        return Err(NotebookError::InvalidInput(
            "notebook_id has control chars".into(),
        ));
    }
    Ok(t.to_string())
}

fn validate_item_id(id: &str) -> Result<String> {
    let t = id.trim();
    if t.is_empty() {
        return Err(NotebookError::InvalidInput("item_id is empty".into()));
    }
    if t.len() > 128 {
        return Err(NotebookError::InvalidInput("item_id too long".into()));
    }
    if t.contains('\0') || t.chars().any(|c| c.is_control()) {
        return Err(NotebookError::InvalidInput(
            "item_id has control chars".into(),
        ));
    }
    Ok(t.to_string())
}

fn validate_remark_id(id: &str) -> Result<String> {
    let t = id.trim();
    if t.is_empty() {
        return Err(NotebookError::InvalidInput("remark_id is empty".into()));
    }
    if t.len() > 128 {
        return Err(NotebookError::InvalidInput("remark_id too long".into()));
    }
    if t.contains('\0') || t.chars().any(|c| c.is_control()) {
        return Err(NotebookError::InvalidInput(
            "remark_id has control chars".into(),
        ));
    }
    Ok(t.to_string())
}

fn normalize_remark_type(raw: &str) -> Result<String> {
    let value = collapse_whitespace(raw.trim());
    if value.is_empty() {
        return Err(NotebookError::InvalidInput("remark_type is empty".into()));
    }
    if value.len() > 64 {
        return Err(NotebookError::InvalidInput("remark_type too long".into()));
    }
    if value.contains('\0') || value.chars().any(|c| c.is_control()) {
        return Err(NotebookError::InvalidInput(
            "remark_type has control chars".into(),
        ));
    }
    Ok(value)
}

fn default_kind_for_id(id: &str) -> NotebookKind {
    if id == SYSTEM_NOTEBOOK_ID {
        NotebookKind::System
    } else if id.starts_with("projects/") {
        NotebookKind::Project
    } else if id.starts_with("agent/") {
        NotebookKind::Agent
    } else {
        NotebookKind::Normal
    }
}

/// Normalize and validate a list of tags per §3.6.
///
/// Rules:
/// 1. trim leading/trailing whitespace,
/// 2. collapse internal whitespace runs to a single space,
/// 3. lowercase,
/// 4. only ASCII [A-Za-z0-9 -], at least one alphanumeric,
/// 5. UTF-8 byte length 2–32,
/// 6. de-dupe and sort (tag order is irrelevant per §2.6, so a canonical
///    sorted form gives us a stable read_scope_hash).
///
/// `"*"` placeholders are dropped (the caller treats `[]` and `["*"]` as
/// "no tag filter").
pub fn normalize_tags(raw: &[String]) -> Result<Vec<String>> {
    let mut set: BTreeMap<String, ()> = BTreeMap::new();
    for t in raw {
        let collapsed = collapse_whitespace(t.trim());
        if collapsed.is_empty() {
            continue;
        }
        if collapsed == "*" {
            // Sentinel; the read paths treat it as "no filter".
            continue;
        }
        let lower = collapsed.to_lowercase();
        validate_tag(&lower)?;
        set.insert(lower, ());
    }
    Ok(set.into_keys().collect())
}

fn validate_tag(tag: &str) -> Result<()> {
    let len = tag.as_bytes().len();
    if len < MIN_TAG_BYTES || len > MAX_TAG_BYTES {
        return Err(NotebookError::InvalidTag(format!(
            "tag length must be {}-{} bytes: {:?}",
            MIN_TAG_BYTES, MAX_TAG_BYTES, tag
        )));
    }
    let mut has_alnum = false;
    for c in tag.chars() {
        let ok = matches!(c, 'a'..='z' | '0'..='9' | ' ' | '-');
        if !ok {
            return Err(NotebookError::InvalidTag(format!(
                "tag has forbidden character {:?}: {:?}",
                c, tag
            )));
        }
        if c.is_ascii_alphanumeric() {
            has_alnum = true;
        }
    }
    if !has_alnum {
        return Err(NotebookError::InvalidTag(format!(
            "tag must contain at least one alphanumeric: {:?}",
            tag
        )));
    }
    Ok(())
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn since_access_at_secs_to_iso8601(secs: u64) -> String {
    if secs == 0 {
        return String::new();
    }
    DateTime::<Utc>::from_timestamp(secs.min(i64::MAX as u64) as i64, 0)
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| "9999-12-31T23:59:59Z".to_string())
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn new_item_id() -> String {
    format!("itm_{}", random_hex(12))
}

fn new_event_id() -> String {
    format!("evt_{}", random_hex(12))
}

fn new_remark_id() -> String {
    format!("rmk_{}", random_hex(12))
}

fn random_hex(n_bytes: usize) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&pid.to_le_bytes());
    hasher.update(&c.to_le_bytes());
    let hex = hasher.finalize().to_hex().to_string();
    hex[..n_bytes * 2].to_string()
}

fn make_version(revision: i64, event_id: &str) -> String {
    let h = blake3::hash(event_id.as_bytes()).to_hex().to_string();
    format!("n_{}_{}", revision, &h[..8])
}

fn short_summary(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.len() <= 200 {
        line.to_string()
    } else {
        let mut end = 200;
        while end > 0 && !line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &line[..end])
    }
}

fn canonical_json(v: &Json) -> String {
    // Re-serialize with sorted keys for hash stability.
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
        Json::Array(arr) => Json::Array(arr.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

fn build_read_scope_value(
    mode: ReadMode,
    tags: &[String],
    title: Option<&str>,
    latest_n: Option<usize>,
    item_ids: Option<&[String]>,
    include_status: &HashSet<NotebookItemStatus>,
) -> Json {
    let mut statuses: Vec<&'static str> = include_status.iter().map(|s| s.as_str()).collect();
    statuses.sort();
    let mut obj = serde_json::Map::new();
    obj.insert("mode".into(), Json::from(mode.as_str()));
    obj.insert(
        "include_status".into(),
        Json::Array(statuses.into_iter().map(Json::from).collect()),
    );
    match mode {
        ReadMode::Tags => {
            obj.insert(
                "tags".into(),
                Json::Array(tags.iter().cloned().map(Json::from).collect()),
            );
        }
        ReadMode::Title => {
            obj.insert("title".into(), Json::from(title.unwrap_or("")));
        }
        ReadMode::Latest => {
            obj.insert("latest_n".into(), Json::from(latest_n.unwrap_or(0) as u64));
        }
        ReadMode::Items => {
            let mut ids: Vec<String> = item_ids.unwrap_or(&[]).to_vec();
            ids.sort();
            obj.insert(
                "item_ids".into(),
                Json::Array(ids.into_iter().map(Json::from).collect()),
            );
        }
        ReadMode::AllActive => {}
    }
    Json::Object(obj)
}

#[derive(Clone, Debug)]
struct EventRow {
    event_id: String,
    seq: i64,
    event_type: NotebookEventType,
    notebook_id: String,
    item_id: Option<String>,
    actor_session_id: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    tags: Option<Vec<String>>,
    notebook_version: String,
    notebook_revision: i64,
    created_at: String,
}

fn insert_event(conn: &Connection, e: &EventRow) -> Result<()> {
    let tags_s = match &e.tags {
        Some(t) => Some(serde_json::to_string(t)?),
        None => None,
    };
    conn.execute(
        "INSERT INTO notebook_events(
            event_id, seq, event_type, notebook_id,
            item_id, actor_session_id, title, summary, tags,
            notebook_version, notebook_revision, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            e.event_id,
            e.seq,
            e.event_type.as_str(),
            e.notebook_id,
            e.item_id,
            e.actor_session_id,
            e.title,
            e.summary,
            tags_s,
            e.notebook_version,
            e.notebook_revision,
            e.created_at
        ],
    )?;
    Ok(())
}

fn next_event_seq(conn: &Connection) -> Result<i64> {
    let v: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM notebook_events",
        [],
        |r| r.get(0),
    )?;
    Ok(v)
}

fn load_notebook(conn: &Connection, notebook_id: &str) -> Result<Option<Notebook>> {
    let row = conn
        .query_row(
            "SELECT id, kind, title, description, status, entry_count, active_entry_count,
                    latest_item_id, latest_title, latest_updated_at, revision, version,
                    created_at, updated_at, archived_at
             FROM notebooks
             WHERE id = ?",
            params![notebook_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
                    r.get::<_, String>(13)?,
                    r.get::<_, Option<String>>(14)?,
                ))
            },
        )
        .optional()?;
    let Some((
        id,
        kind,
        title,
        description,
        status,
        entry_count,
        active_entry_count,
        latest_item_id,
        latest_title,
        latest_updated_at,
        revision,
        version,
        created_at,
        updated_at,
        archived_at,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(Notebook {
        id,
        kind: NotebookKind::from_str(&kind)?,
        title,
        description,
        status: NotebookStatus::from_str(&status)?,
        entry_count,
        active_entry_count,
        latest_item_id,
        latest_title,
        latest_updated_at,
        revision,
        version,
        created_at,
        updated_at,
        archived_at,
    }))
}

fn detect_conflicts(
    conn: &Connection,
    notebook_id: &str,
    title: &str,
    tags: &[String],
    valid_from: Option<&str>,
    valid_until: Option<&str>,
) -> Result<Vec<PossibleConflict>> {
    let mut out: Vec<PossibleConflict> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // 1) Same / near title.
    let mut stmt = conn.prepare(
        "SELECT item_id, title, status, updated_at FROM notebook_items
         WHERE notebook_id = ? AND status = 'active' AND lower(title) = lower(?)
         LIMIT 8",
    )?;
    let rows = stmt.query_map(params![notebook_id, title], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (id, t, s, upd) = row?;
        seen.insert(id.clone());
        out.push(PossibleConflict {
            item_id: id,
            title: t,
            reason: ConflictReason::SameTitle,
            matched_tags: None,
            status: NotebookItemStatus::from_str(&s)?,
            updated_at: upd,
        });
    }

    // 2) Tag overlap.
    if !tags.is_empty() {
        let placeholders = std::iter::repeat("?")
            .take(tags.len())
            .collect::<Vec<_>>()
            .join(",");
        let q = format!(
            "SELECT i.item_id, i.title, i.status, i.updated_at, GROUP_CONCAT(t.tag, ',')
             FROM notebook_items i
             JOIN notebook_item_tags t ON t.item_id = i.item_id
             WHERE i.notebook_id = ? AND i.status = 'active'
               AND t.tag IN ({})
             GROUP BY i.item_id
             ORDER BY i.updated_at DESC
             LIMIT 8",
            placeholders
        );
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        params_vec.push(&notebook_id);
        for t in tags {
            params_vec.push(t);
        }
        let mut stmt = conn.prepare(&q)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        for row in rows {
            let (id, t, s, upd, tag_list) = row?;
            if seen.contains(&id) {
                continue;
            }
            let mut matched: Vec<String> = tag_list
                .unwrap_or_default()
                .split(',')
                .filter(|t| tags.iter().any(|tt| tt == t))
                .map(|s| s.to_string())
                .collect();
            matched.sort();
            matched.dedup();
            seen.insert(id.clone());
            out.push(PossibleConflict {
                item_id: id,
                title: t,
                reason: ConflictReason::TagOverlap,
                matched_tags: Some(matched),
                status: NotebookItemStatus::from_str(&s)?,
                updated_at: upd,
            });
        }
    }

    // 3) Active validity overlap.
    if valid_from.is_some() || valid_until.is_some() {
        let new_from = valid_from.unwrap_or("0000-01-01");
        let new_until = valid_until.unwrap_or("9999-12-31");
        let mut stmt = conn.prepare(
            "SELECT item_id, title, status, updated_at, valid_from, valid_until
             FROM notebook_items
             WHERE notebook_id = ? AND status = 'active'
               AND (valid_from IS NOT NULL OR valid_until IS NOT NULL)
             LIMIT 16",
        )?;
        let rows = stmt.query_map(params![notebook_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in rows {
            let (id, t, s, upd, vf, vu) = row?;
            if seen.contains(&id) {
                continue;
            }
            let exist_from = vf.as_deref().unwrap_or("0000-01-01");
            let exist_until = vu.as_deref().unwrap_or("9999-12-31");
            if exist_from <= new_until && new_from <= exist_until {
                seen.insert(id.clone());
                out.push(PossibleConflict {
                    item_id: id,
                    title: t,
                    reason: ConflictReason::ActiveOverlap,
                    matched_tags: None,
                    status: NotebookItemStatus::from_str(&s)?,
                    updated_at: upd,
                });
            }
        }
    }

    Ok(out)
}

fn title_candidates(
    conn: &Connection,
    notebook_id: &str,
    needle: &str,
    include_status: &HashSet<NotebookItemStatus>,
) -> Result<Vec<String>> {
    let status_filter = build_status_filter(include_status);
    let q = format!(
        "SELECT item_id FROM notebook_items
         WHERE notebook_id = ? AND lower(title) = lower(?) AND status IN ({})",
        status_filter
    );
    let mut stmt = conn.prepare(&q)?;
    let mut ids = Vec::new();
    let rows = stmt.query_map(params![notebook_id, needle], |r| r.get::<_, String>(0))?;
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn tag_candidates(
    conn: &Connection,
    notebook_id: &str,
    tags: &[String],
    include_status: &HashSet<NotebookItemStatus>,
) -> Result<Vec<String>> {
    if tags.is_empty() {
        return all_candidates(conn, notebook_id, include_status);
    }
    let placeholders = std::iter::repeat("?")
        .take(tags.len())
        .collect::<Vec<_>>()
        .join(",");
    let status_filter = build_status_filter(include_status);
    let q = format!(
        "SELECT i.item_id FROM notebook_items i
         JOIN notebook_item_tags t ON t.item_id = i.item_id
         WHERE i.notebook_id = ? AND i.status IN ({})
           AND t.tag IN ({})
         GROUP BY i.item_id",
        status_filter, placeholders
    );
    let mut stmt = conn.prepare(&q)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
    params_vec.push(&notebook_id);
    for t in tags {
        params_vec.push(t);
    }
    let rows = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
        r.get::<_, String>(0)
    })?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn all_candidates(
    conn: &Connection,
    notebook_id: &str,
    include_status: &HashSet<NotebookItemStatus>,
) -> Result<Vec<String>> {
    let status_filter = build_status_filter(include_status);
    let q = format!(
        "SELECT item_id FROM notebook_items
         WHERE notebook_id = ? AND status IN ({})",
        status_filter
    );
    let mut stmt = conn.prepare(&q)?;
    let rows = stmt.query_map(params![notebook_id], |r| r.get::<_, String>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn build_status_filter(include_status: &HashSet<NotebookItemStatus>) -> String {
    let parts: Vec<String> = include_status
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect();
    if parts.is_empty() {
        "'active'".to_string()
    } else {
        parts.join(",")
    }
}

fn fetch_items_by_ids(conn: &Connection, ids: &[String]) -> Result<Vec<NotebookItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let q = format!(
        "SELECT i.item_id, i.notebook_id, i.title, i.content, i.source_excerpt, i.source_ref,
                i.source_session_id, i.write_reason, i.confidence,
                i.status, i.valid_from, i.valid_until, i.created_at, i.updated_at,
                i.item_revision, i.content_hash,
                COALESCE((SELECT GROUP_CONCAT(tag, ',') FROM notebook_item_tags
                          WHERE item_id = i.item_id ORDER BY tag), '')
         FROM notebook_items i
         WHERE i.item_id IN ({})",
        placeholders
    );
    let mut stmt = conn.prepare(&q)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
    for id in ids {
        params_vec.push(id);
    }
    let rows = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
            r.get::<_, Option<String>>(10)?,
            r.get::<_, Option<String>>(11)?,
            r.get::<_, String>(12)?,
            r.get::<_, String>(13)?,
            r.get::<_, i64>(14)?,
            r.get::<_, String>(15)?,
            r.get::<_, String>(16)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            item_id,
            notebook_id,
            title,
            content,
            source_excerpt,
            source_ref_s,
            source_session_id,
            write_reason,
            confidence,
            status,
            valid_from,
            valid_until,
            created_at,
            updated_at,
            item_revision,
            content_hash,
            tags_concat,
        ) = row?;
        let source_ref = match source_ref_s {
            Some(s) if !s.is_empty() => Some(serde_json::from_str::<SourceRef>(&s)?),
            _ => None,
        };
        let tags: Vec<String> = if tags_concat.is_empty() {
            Vec::new()
        } else {
            tags_concat.split(',').map(|s| s.to_string()).collect()
        };
        out.push(NotebookItem {
            item_id,
            notebook_id,
            title,
            content,
            source_excerpt,
            source_ref,
            source_session_id,
            write_reason: WriteReason::from_str(&write_reason)?,
            confidence: Confidence::from_str(&confidence)?,
            status: NotebookItemStatus::from_str(&status)?,
            valid_from,
            valid_until,
            tags,
            created_at,
            updated_at,
            item_revision,
            content_hash,
        });
    }
    Ok(out)
}

fn item_notebook_id(conn: &Connection, item_id: &str) -> Result<Option<String>> {
    let notebook_id = conn
        .query_row(
            "SELECT notebook_id FROM notebook_items
             WHERE item_id = ?",
            params![item_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    Ok(notebook_id)
}

fn load_item_remark_target(
    conn: &Connection,
    item_id: &str,
) -> Result<Option<(String, String, NotebookItemStatus)>> {
    let row = conn
        .query_row(
            "SELECT notebook_id, title, status FROM notebook_items
             WHERE item_id = ?",
            params![item_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((notebook_id, title, status)) = row else {
        return Ok(None);
    };
    Ok(Some((
        notebook_id,
        title,
        NotebookItemStatus::from_str(&status)?,
    )))
}

fn load_active_remark_target(
    conn: &Connection,
    item_id: &str,
    remark_id: &str,
) -> Result<Option<(String, String, String)>> {
    let row = conn
        .query_row(
            "SELECT i.notebook_id, i.title, r.remark_type
             FROM notebook_item_remarks r
             JOIN notebook_items i
               ON i.item_id = r.item_id
             WHERE r.remark_id = ? AND r.item_id = ?
               AND r.status = 'active'",
            params![remark_id, item_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(row)
}

fn notebook_revision(conn: &Connection, notebook_id: &str) -> Result<i64> {
    let revision = conn.query_row(
        "SELECT revision FROM notebooks
         WHERE id = ?",
        params![notebook_id],
        |r| r.get::<_, i64>(0),
    )?;
    Ok(revision)
}

fn fetch_item_tags(conn: &Connection, item_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT tag FROM notebook_item_tags
         WHERE item_id = ?
         ORDER BY tag ASC",
    )?;
    let rows = stmt.query_map(params![item_id], |r| r.get::<_, String>(0))?;
    let mut tags = Vec::new();
    for row in rows {
        tags.push(row?);
    }
    Ok(tags)
}

fn is_expired(item: &NotebookItem, now: &str) -> bool {
    matches!(&item.valid_until, Some(v) if v.as_str() <= now)
}

fn upsert_read_cache(
    conn: &Connection,
    session_id: &str,
    notebook_id: &str,
    read_scope_hash: &str,
    mode: ReadMode,
    read_scope_value: &Json,
    notebook_version: &str,
    notebook_revision: i64,
    returned_ids: &[String],
    max_bytes: Option<usize>,
    read_at: &str,
) -> Result<()> {
    let scope_s = serde_json::to_string(read_scope_value)?;
    let ids_s = serde_json::to_string(returned_ids)?;
    conn.execute(
        "INSERT INTO session_reads(
            session_id, notebook_id, read_scope_hash,
            mode, read_scope, notebook_version, notebook_revision, returned_item_ids,
            max_bytes, read_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(session_id, notebook_id, read_scope_hash)
         DO UPDATE SET mode = excluded.mode, read_scope = excluded.read_scope,
                       notebook_version = excluded.notebook_version,
                       notebook_revision = excluded.notebook_revision,
                       returned_item_ids = excluded.returned_item_ids,
                       max_bytes = excluded.max_bytes,
                       read_at = excluded.read_at",
        params![
            session_id,
            notebook_id,
            read_scope_hash,
            mode.as_str(),
            scope_s,
            notebook_version,
            notebook_revision,
            ids_s,
            max_bytes.map(|n| n as i64),
            read_at
        ],
    )?;
    Ok(())
}

fn fetch_read_cache_version(
    conn: &Connection,
    session_id: &str,
    notebook_id: &str,
    read_scope_hash: &str,
) -> Result<Option<String>> {
    let v = conn
        .query_row(
            "SELECT notebook_version FROM session_reads
             WHERE session_id = ? AND notebook_id = ? AND read_scope_hash = ?",
            params![session_id, notebook_id, read_scope_hash],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    Ok(v)
}

fn session_has_all_active_at(
    conn: &Connection,
    session_id: &str,
    notebook_id: &str,
    notebook_version: &str,
) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_reads
         WHERE session_id = ? AND notebook_id = ? AND mode = 'all_active' AND notebook_version = ?",
        params![session_id, notebook_id, notebook_version],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn has_session_read(conn: &Connection, session_id: &str, notebook_id: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_reads
         WHERE session_id = ? AND notebook_id = ?",
        params![session_id, notebook_id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn session_has_unchanged_for_notebook(
    conn: &Connection,
    session_id: &str,
    notebook_id: &str,
    current_version: &str,
) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_reads
         WHERE session_id = ? AND notebook_id = ? AND notebook_version = ?",
        params![session_id, notebook_id, current_version],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

fn fetch_watermark_seq(conn: &Connection, session_id: &str) -> Result<i64> {
    let v = conn
        .query_row(
            "SELECT last_seen_seq FROM session_watermarks
             WHERE session_id = ?",
            params![session_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    Ok(v.unwrap_or(0))
}

fn upsert_watermark(conn: &Connection, session_id: &str, seq: i64) -> Result<()> {
    let now = now_iso8601();
    conn.execute(
        "INSERT INTO session_watermarks(
            session_id, last_seen_seq, last_seen_at
         ) VALUES (?, ?, ?)
         ON CONFLICT(session_id) DO UPDATE SET
            last_seen_seq = MAX(excluded.last_seen_seq, session_watermarks.last_seen_seq),
            last_seen_at = excluded.last_seen_at",
        params![session_id, seq, now],
    )?;
    Ok(())
}

fn adjust_notebook_counts(
    conn: &Connection,
    notebook_id: &str,
    delta_entry: i64,
    delta_active: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE notebooks SET entry_count = entry_count + ?, active_entry_count = active_entry_count + ?
         WHERE id = ?",
        params![delta_entry, delta_active, notebook_id],
    )?;
    Ok(())
}

fn bump_notebook_version(
    conn: &Connection,
    notebook_id: &str,
    now: &str,
    event_id: Option<&str>,
) -> Result<String> {
    let revision: i64 = conn.query_row(
        "SELECT revision FROM notebooks
         WHERE id = ?",
        params![notebook_id],
        |r| r.get(0),
    )?;
    let new_rev = revision + 1;
    let eid_owned;
    let eid = match event_id {
        Some(id) => id,
        None => {
            eid_owned = new_event_id();
            eid_owned.as_str()
        }
    };
    let version = make_version(new_rev, eid);
    conn.execute(
        "UPDATE notebooks SET revision = ?, version = ?, updated_at = ?
         WHERE id = ?",
        params![new_rev, version, now, notebook_id],
    )?;
    Ok(version)
}

fn ensure_system_notebook(conn: &Connection, now: &str) -> Result<()> {
    if load_notebook(conn, SYSTEM_NOTEBOOK_ID)?.is_some() {
        return Ok(());
    }
    let event_id = new_event_id();
    let revision: i64 = 1;
    let version = make_version(revision, &event_id);
    conn.execute(
        "INSERT INTO notebooks(
            id, kind, title, description, status,
            revision, version, latest_item_id, latest_title, latest_updated_at,
            entry_count, active_entry_count, created_at, updated_at, archived_at
         ) VALUES (?, 'system', 'System Notebook', 'High-confidence facts injected into the system prompt.',
                   'active', ?, ?, NULL, NULL, NULL, 0, 0, ?, ?, NULL)",
        params![SYSTEM_NOTEBOOK_ID, revision, version, now, now],
    )?;
    let seq = next_event_seq(conn)?;
    insert_event(
        conn,
        &EventRow {
            event_id,
            seq,
            event_type: NotebookEventType::NotebookCreated,
            notebook_id: SYSTEM_NOTEBOOK_ID.to_string(),
            item_id: None,
            actor_session_id: None,
            title: Some("System Notebook".into()),
            summary: Some("system notebook bootstrapped".into()),
            tags: None,
            notebook_version: version,
            notebook_revision: revision,
            created_at: now.to_string(),
        },
    )?;
    Ok(())
}

fn topic_relevance_candidates(
    conn: &Connection,
    topic_tags: &[String],
    candidate_notebook_ids: Option<&[String]>,
) -> Result<Vec<(String, Option<String>, Option<String>, Vec<String>, String)>> {
    if topic_tags.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat("?")
        .take(topic_tags.len())
        .collect::<Vec<_>>()
        .join(",");
    let mut where_extra = String::new();
    if let Some(ids) = candidate_notebook_ids {
        if !ids.is_empty() {
            let ph = std::iter::repeat("?")
                .take(ids.len())
                .collect::<Vec<_>>()
                .join(",");
            where_extra = format!(" AND i.notebook_id IN ({})", ph);
        }
    }
    let q = format!(
        "SELECT i.notebook_id, i.title, i.updated_at,
                (SELECT GROUP_CONCAT(t2.tag, ',') FROM notebook_item_tags t2
                 WHERE t2.item_id = i.item_id AND t2.tag IN ({tags})) AS matched,
                n.version
         FROM notebook_items i
         JOIN notebook_item_tags t ON t.item_id = i.item_id
         JOIN notebooks n ON n.id = i.notebook_id
         WHERE i.status = 'active'
           AND t.tag IN ({tags}) {where_extra}
         GROUP BY i.item_id
         ORDER BY i.updated_at DESC, i.item_id ASC
         LIMIT 32",
        tags = placeholders,
        where_extra = where_extra
    );
    let mut stmt = conn.prepare(&q)?;
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = Vec::new();
    // First `tags` placeholders (subquery).
    for t in topic_tags {
        params_vec.push(t);
    }
    // Second `tags` placeholders (filter).
    for t in topic_tags {
        params_vec.push(t);
    }
    if let Some(ids) = candidate_notebook_ids {
        for id in ids {
            params_vec.push(id);
        }
    }
    let rows = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    // Group by notebook_id; keep newest entry per notebook.
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, Option<String>, Option<String>, Vec<String>, String)> = Vec::new();
    for row in rows {
        let (nb_id, title, upd, matched_s, version) = row?;
        if !seen.insert(nb_id.clone()) {
            continue;
        }
        let matched: Vec<String> = matched_s
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        out.push((nb_id, title, upd, matched, version));
    }
    Ok(out)
}

// ============================================================ tests

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_tmp() -> (TempDir, AgentNotebook) {
        let tmp = TempDir::new().unwrap();
        let n = AgentNotebook::open(AgentNotebookConfig::new(tmp.path())).unwrap();
        (tmp, n)
    }
    fn append(n: &AgentNotebook, nb: &str, title: &str, content: &str, tags: &[&str]) -> String {
        let res = n
            .append_note(AppendNoteInput {
                session_id: Some("sess-A".into()),
                notebook_id: nb.into(),
                title: title.into(),
                content: content.into(),
                source_excerpt: None,
                source_ref: None,
                source_session_id: Some("sess-A".into()),
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: Some(Confidence::High),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                detect_conflicts: true,
            })
            .unwrap();
        res.item_id
    }

    #[test]
    fn latest_active_item_update_secs_tracks_items_and_remark_bumps() {
        let (_t, n) = open_tmp();
        // Empty notebook → 0.
        assert_eq!(n.latest_active_item_update_secs().unwrap(), 0);

        // Appending an item yields a positive watermark.
        let item_id = append(&n, "user/plans", "drink water", "every 30m", &["health"]);
        let after_item = n.latest_active_item_update_secs().unwrap();
        assert!(after_item > 0, "watermark should be set after first item");

        // A self_check remark bumps the item's updated_at, so the watermark is
        // >= the item's — this is exactly why the gate compares against a
        // post-round watermark instead of any nonzero value.
        n.append_item_remark(AppendItemRemarkInput {
            session_id: Some("sess-A".into()),
            item_id: item_id.clone(),
            remark_type: "self_check".into(),
            content: "decision=keep_observing".into(),
        })
        .unwrap();
        let after_remark = n.latest_active_item_update_secs().unwrap();
        assert!(
            after_remark >= after_item,
            "remark append must not lower the watermark"
        );
    }

    #[test]
    fn tag_normalization_rejects_bad_chars_and_collapses_whitespace() {
        let ok = normalize_tags(&[
            "  Phone   Case ".to_string(),
            "phone case".to_string(),
            "*".to_string(),
        ])
        .unwrap();
        assert_eq!(ok, vec!["phone case".to_string()]); // dedup + drop "*"
        assert!(normalize_tags(&["bad\"tag".to_string()]).is_err());
        assert!(normalize_tags(&["中文".to_string()]).is_err());
        assert!(normalize_tags(&["x".to_string()]).is_err()); // too short
    }

    #[test]
    fn append_creates_notebook_and_bumps_version() {
        let (_t, n) = open_tmp();
        let r1 = n
            .append_note(AppendNoteInput {
                session_id: None,
                notebook_id: "user/preferences".into(),
                title: "concise replies".into(),
                content: "user prefers terse output".into(),
                source_excerpt: Some("user said \"be concise\"".into()),
                source_ref: None,
                source_session_id: None,
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: Some(Confidence::High),
                tags: vec!["reply-style".into(), "tone".into()],
                detect_conflicts: true,
            })
            .unwrap();
        assert_eq!(r1.notebook_id, "user/preferences");
        assert!(r1.version.starts_with("n_2_")); // create + append → revision 2
        let v1 = r1.version.clone();
        let r2 = n
            .append_note(AppendNoteInput {
                session_id: None,
                notebook_id: "user/preferences".into(),
                title: "use markdown".into(),
                content: "render lists as markdown".into(),
                source_excerpt: None,
                source_ref: None,
                source_session_id: None,
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: Some(Confidence::Medium),
                tags: vec!["formatting".into()],
                detect_conflicts: true,
            })
            .unwrap();
        assert_ne!(v1, r2.version);
        assert!(r2.version.starts_with("n_3_"));
        let regs = n
            .list_notebooks(ListNotebooksInput {
                include_archived: false,
            })
            .unwrap();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].active_entry_count, 2);
        assert_eq!(regs[0].entry_count, 2);
        assert_eq!(regs[0].latest_title.as_deref(), Some("use markdown"));
    }

    #[test]
    fn append_rejects_invalid_tag() {
        let (_t, n) = open_tmp();
        let err = n
            .append_note(AppendNoteInput {
                session_id: None,
                notebook_id: "user/preferences".into(),
                title: "bad".into(),
                content: "x".into(),
                source_excerpt: None,
                source_ref: None,
                source_session_id: None,
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: None,
                tags: vec!["bad\"tag".into()],
                detect_conflicts: false,
            })
            .unwrap_err();
        assert_eq!(err.code(), "invalid_tag");
    }

    #[test]
    fn read_returns_items_in_updated_at_desc_order() {
        let (_t, n) = open_tmp();
        let _ = append(&n, "projects/p", "a", "first", &["alpha"]);
        std::thread::sleep(std::time::Duration::from_secs(1));
        let _ = append(&n, "projects/p", "b", "second", &["beta"]);
        std::thread::sleep(std::time::Duration::from_secs(1));
        let _ = append(&n, "projects/p", "c", "third", &["alpha", "beta"]);
        let res = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("sess-1".into()),
                notebook_id: "projects/p".into(),
                tags: None,
                title: None,
                latest_n: None,
                item_ids: None,
                since_version: None,
                include_status: None,
                include_superseded: false,
                max_items: Some(10),
                max_bytes: Some(12000),
                allow_unchanged: true,
            })
            .unwrap();
        let NotebookReadResult::Ok { entries, .. } = res else {
            panic!("expected ok");
        };
        assert_eq!(entries.len(), 3);
        let titles: Vec<&str> = entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["c", "b", "a"]);

        // Tag mode must use the same sort key; matching multiple tags does not
        // bubble c to a different position vs single-tag matches.
        let res = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("sess-2".into()),
                notebook_id: "projects/p".into(),
                tags: Some(vec!["alpha".into(), "beta".into()]),
                title: None,
                latest_n: None,
                item_ids: None,
                since_version: None,
                include_status: None,
                include_superseded: false,
                max_items: Some(10),
                max_bytes: Some(12000),
                allow_unchanged: true,
            })
            .unwrap();
        let NotebookReadResult::Ok {
            entries,
            mode,
            matched_tags,
            ..
        } = res
        else {
            panic!("expected ok");
        };
        assert_eq!(mode, ReadMode::Tags);
        let titles: Vec<&str> = entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["c", "b", "a"]);
        let mut mt = matched_tags.unwrap();
        mt.sort();
        assert_eq!(mt, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn read_unchanged_returns_after_same_scope_and_version() {
        let (_t, n) = open_tmp();
        append(&n, "user/preferences", "a", "one", &["focus"]);
        let r1 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: None,
                title: None,
                latest_n: None,
                item_ids: None,
                since_version: None,
                include_status: None,
                include_superseded: false,
                max_items: None,
                max_bytes: None,
                allow_unchanged: true,
            })
            .unwrap();
        let NotebookReadResult::Ok { .. } = r1 else {
            panic!("expected ok");
        };
        let r2 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: None,
                title: None,
                latest_n: None,
                item_ids: None,
                since_version: None,
                include_status: None,
                include_superseded: false,
                max_items: None,
                max_bytes: None,
                allow_unchanged: true,
            })
            .unwrap();
        assert!(matches!(r2, NotebookReadResult::Unchanged { .. }));
    }

    #[test]
    fn read_scope_hash_is_tag_order_independent() {
        let (_t, n) = open_tmp();
        append(&n, "user/preferences", "a", "x", &["focus", "tone"]);
        let r1 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: Some(vec!["focus".into(), "tone".into()]),
                ..Default::default()
            })
            .unwrap();
        let r2 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: Some(vec!["tone".into(), "focus".into()]),
                ..Default::default()
            })
            .unwrap();
        let h1 = match r1 {
            NotebookReadResult::Ok {
                read_scope_hash, ..
            } => read_scope_hash,
            NotebookReadResult::Unchanged {
                read_scope_hash, ..
            } => read_scope_hash,
        };
        let h2 = match r2 {
            NotebookReadResult::Ok {
                read_scope_hash, ..
            } => read_scope_hash,
            NotebookReadResult::Unchanged {
                read_scope_hash, ..
            } => read_scope_hash,
        };
        assert_eq!(h1, h2);

        let r3 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: Some(vec!["tone".into()]),
                ..Default::default()
            })
            .unwrap();
        let h3 = match r3 {
            NotebookReadResult::Ok {
                read_scope_hash, ..
            } => read_scope_hash,
            NotebookReadResult::Unchanged {
                read_scope_hash, ..
            } => read_scope_hash,
        };
        assert_ne!(h1, h3);
    }

    #[test]
    fn read_all_active_covers_subsequent_tag_reads() {
        let (_t, n) = open_tmp();
        append(&n, "user/preferences", "a", "x", &["focus"]);
        let r1 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: None, // all_active
                ..Default::default()
            })
            .unwrap();
        let NotebookReadResult::Ok { .. } = r1 else {
            panic!("expected ok");
        };
        let r2 = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                tags: Some(vec!["focus".into()]),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(r2, NotebookReadResult::Unchanged { .. }));
    }

    #[test]
    fn mark_stale_hides_from_default_read() {
        let (_t, n) = open_tmp();
        let id = append(&n, "user/preferences", "old habit", "x", &["focus"]);
        n.mark_note_status(MarkNoteStatusInput {
            session_id: None,
            item_id: id.clone(),
            status: NotebookItemStatus::Stale,
            reason: "no longer applies".into(),
            superseded_by: None,
            expected_item_revision: None,
        })
        .unwrap();
        let r = n
            .read_notebook(ReadNotebookInput {
                session_id: None,
                notebook_id: "user/preferences".into(),
                tags: None,
                title: None,
                latest_n: None,
                item_ids: None,
                since_version: None,
                include_status: None,
                include_superseded: false,
                max_items: None,
                max_bytes: None,
                allow_unchanged: true,
            })
            .unwrap();
        let NotebookReadResult::Ok { entries, .. } = r else {
            panic!("expected ok");
        };
        assert!(entries.is_empty(), "stale item must be hidden by default");
    }

    #[test]
    fn supersede_edge_created_and_old_item_hidden() {
        let (_t, n) = open_tmp();
        let old_id = append(&n, "user/preferences", "concise", "v1", &["tone"]);
        let new_id = append(&n, "user/preferences", "concise v2", "v2", &["tone"]);
        n.mark_note_status(MarkNoteStatusInput {
            session_id: None,
            item_id: old_id.clone(),
            status: NotebookItemStatus::Superseded,
            reason: "replaced".into(),
            superseded_by: Some(new_id.clone()),
            expected_item_revision: None,
        })
        .unwrap();
        let r = n
            .read_notebook(ReadNotebookInput {
                session_id: None,
                notebook_id: "user/preferences".into(),
                ..Default::default()
            })
            .unwrap();
        let NotebookReadResult::Ok { entries, .. } = r else {
            panic!("expected ok");
        };
        let ids: Vec<&str> = entries.iter().map(|e| e.item_id.as_str()).collect();
        assert!(ids.contains(&new_id.as_str()));
        assert!(!ids.contains(&old_id.as_str()));
    }

    #[test]
    fn tag_overlap_conflict_returned() {
        let (_t, n) = open_tmp();
        append(&n, "user/preferences", "a", "x", &["focus", "tone"]);
        let r = n
            .append_note(AppendNoteInput {
                session_id: None,
                notebook_id: "user/preferences".into(),
                title: "b".into(),
                content: "y".into(),
                source_excerpt: None,
                source_ref: None,
                source_session_id: None,
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: None,
                tags: vec!["tone".into()],
                detect_conflicts: true,
            })
            .unwrap();
        assert!(r
            .possible_conflicts
            .iter()
            .any(|c| c.reason == ConflictReason::TagOverlap
                && c.matched_tags
                    .as_ref()
                    .is_some_and(|m| m.contains(&"tone".to_string()))));
    }

    #[test]
    fn item_remarks_append_list_filter_and_remove() {
        let (_t, n) = open_tmp();
        let item_id = append(&n, "user/preferences", "a", "x", &["focus"]);
        let first = n
            .append_item_remark(AppendItemRemarkInput {
                session_id: Some("sess-A".into()),
                item_id: item_id.clone(),
                remark_type: "red".into(),
                content: "needs confirmation".into(),
            })
            .unwrap();
        let second = n
            .append_item_remark(AppendItemRemarkInput {
                session_id: Some("sess-A".into()),
                item_id: item_id.clone(),
                remark_type: "blue".into(),
                content: "owner supplied".into(),
            })
            .unwrap();
        assert_eq!(first.item_id, item_id);
        assert_ne!(first.notebook_version, second.notebook_version);

        let all = n
            .list_item_remarks(ListItemRemarksInput {
                item_id: item_id.clone(),
                remark_type: None,
            })
            .unwrap();
        assert_eq!(all.len(), 2);

        let red = n
            .list_item_remarks(ListItemRemarksInput {
                item_id: item_id.clone(),
                remark_type: Some("red".into()),
            })
            .unwrap();
        assert_eq!(red.len(), 1);
        assert_eq!(red[0].remark_id, first.remark_id);
        assert_eq!(red[0].content, "needs confirmation");

        let removed = n
            .remove_item_remark(RemoveItemRemarkInput {
                session_id: Some("sess-A".into()),
                item_id: item_id.clone(),
                remark_id: first.remark_id.clone(),
            })
            .unwrap();
        assert_eq!(removed.remark_id, first.remark_id);

        let after = n
            .list_item_remarks(ListItemRemarksInput {
                item_id,
                remark_type: None,
            })
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].remark_id, second.remark_id);
    }

    #[test]
    fn system_notebook_enforces_active_ceiling() {
        let (_t, n) = open_tmp();
        // Fill SYSTEM_NOTEBOOK_MAX_ACTIVE items and promote them all.
        for i in 0..SYSTEM_NOTEBOOK_MAX_ACTIVE {
            let id = append(
                &n,
                "user/preferences",
                &format!("fact {}", i),
                "content",
                &["focus"],
            );
            let r = n
                .promote_to_system_notebook(PromoteToSystemInput {
                    item_id: id,
                    reason: format!("promote {}", i),
                    replace_item_id: None,
                })
                .unwrap();
            assert!(matches!(r, PromoteToSystemResult::Ok { .. }));
        }
        // One more should fail with limit_exceeded.
        let extra = append(&n, "user/preferences", "extra", "x", &["focus"]);
        let r = n
            .promote_to_system_notebook(PromoteToSystemInput {
                item_id: extra,
                reason: "over the cap".into(),
                replace_item_id: None,
            })
            .unwrap();
        assert!(matches!(r, PromoteToSystemResult::LimitExceeded { .. }));
    }

    #[test]
    fn cross_session_update_hint_after_remote_append() {
        let (_t, n) = open_tmp();
        // Session A reads the notebook (creates the read cache row).
        append(&n, "projects/p", "first", "content", &["aa"]);
        let _ = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("A".into()),
                notebook_id: "projects/p".into(),
                ..Default::default()
            })
            .unwrap();
        // Session B writes.
        let _ = n
            .append_note(AppendNoteInput {
                session_id: Some("B".into()),
                notebook_id: "projects/p".into(),
                title: "from B".into(),
                content: "later".into(),
                source_excerpt: None,
                source_ref: None,
                source_session_id: Some("B".into()),
                write_reason: WriteReason::UserExplicit,
                valid_from: None,
                valid_until: None,
                confidence: None,
                tags: vec!["aa".into()],
                detect_conflicts: false,
            })
            .unwrap();
        let h = n
            .build_notebook_hints(BuildHintsInput {
                session_id: "A".into(),
                topic_tags: None,
                candidate_notebook_ids: None,
                max_hints: Some(3),
            })
            .unwrap();
        assert!(h
            .hints
            .iter()
            .any(|x| x.notebook_id == "projects/p" && x.reason == HintReason::CrossSessionUpdate));
        // Watermark advanced — next call should produce no fresh hint.
        let h2 = n
            .build_notebook_hints(BuildHintsInput {
                session_id: "A".into(),
                topic_tags: None,
                candidate_notebook_ids: None,
                max_hints: Some(3),
            })
            .unwrap();
        assert!(!h2
            .hints
            .iter()
            .any(|x| x.notebook_id == "projects/p" && x.reason == HintReason::CrossSessionUpdate));
    }

    #[test]
    fn topic_hint_suppressed_for_already_read_unchanged_notebook() {
        let (_t, n) = open_tmp();
        append(&n, "user/preferences", "a", "x", &["focus"]);
        let _ = n
            .read_notebook(ReadNotebookInput {
                session_id: Some("S".into()),
                notebook_id: "user/preferences".into(),
                ..Default::default()
            })
            .unwrap();
        let h = n
            .build_notebook_hints(BuildHintsInput {
                session_id: "S".into(),
                topic_tags: Some(vec!["focus".into()]),
                candidate_notebook_ids: None,
                max_hints: Some(3),
            })
            .unwrap();
        assert!(h.hints.is_empty());
        assert!(h
            .suppressed
            .iter()
            .any(|s| s.reason == HintSuppressionReason::AlreadyReadUnchanged));
    }

    #[test]
    fn registry_text_excludes_content() {
        let (_t, n) = open_tmp();
        append(&n, "user/preferences", "secret", "DO NOT LEAK", &["xx"]);
        let ctx = n
            .build_notebook_registry_context(BuildRegistryContextInput {
                max_notebooks: None,
            })
            .unwrap();
        assert!(!ctx.text.contains("DO NOT LEAK"));
        assert!(ctx.text.contains("user/preferences"));
    }

    #[test]
    fn prompt_notebook_texts_include_registry_and_recent_item_content() {
        let (_t, n) = open_tmp();
        append(
            &n,
            "user/preferences",
            "tone",
            "direct content A",
            &["tone"],
        );
        append(&n, "projects/demo", "scope", "direct content B", &["scope"]);

        let list_text = n.build_notebook_list_text().unwrap();
        assert!(list_text.contains("Available notebooks: 2 total."));
        assert!(list_text.contains("user/preferences"));
        assert!(list_text.contains("projects/demo"));
        assert!(list_text.contains("1 records"));
        assert!(list_text.contains("last modified"));
        assert!(!list_text.contains("direct content A"));

        let recent_text = n.build_recent_items_text(2, 0).unwrap();
        assert!(recent_text.contains("Recent notebook items: latest 2."));
        assert!(recent_text.contains("active"));
        assert!(recent_text.contains("sess-A"));
        assert!(recent_text.contains("direct content A"));
        assert!(recent_text.contains("direct content B"));
        assert!(!recent_text.contains("tone"));
        assert!(!recent_text.contains("scope"));
    }

    #[test]
    fn recent_items_text_respects_since_window_except_keep_observing() {
        let (_t, n) = open_tmp();
        let old_id = append(&n, "user/preferences", "old action", "content", &["action"]);
        let observing_id = append(
            &n,
            "user/preferences",
            "observing action",
            "content",
            &["action"],
        );
        n.append_item_remark(AppendItemRemarkInput {
            session_id: Some("sess-A".into()),
            item_id: observing_id.clone(),
            remark_type: "self_check".into(),
            content: "decision=keep_observing reason=missing context".into(),
        })
        .unwrap();

        let future_since = 4_102_444_800;
        let recent_text = n.build_recent_items_text(8, future_since).unwrap();
        assert!(recent_text.contains(&observing_id));
        assert!(recent_text.contains("content"));
        assert!(!recent_text.contains(&old_id));
        assert!(!recent_text.contains("observing action"));
        assert!(!recent_text.contains("old action"));
    }

    #[test]
    fn self_check_recent_items_include_unreviewed_outside_since_window() {
        let (_t, n) = open_tmp();
        let pending_id = append(
            &n,
            "user/preferences",
            "pending exercise reminder",
            "content",
            &["action"],
        );
        let reviewed_id = append(
            &n,
            "user/preferences",
            "finished email note",
            "content",
            &["action"],
        );
        n.append_item_remark(AppendItemRemarkInput {
            session_id: Some("sess-A".into()),
            item_id: reviewed_id,
            remark_type: "self_check".into(),
            content: "decision=mark_no_action reason=reviewed".into(),
        })
        .unwrap();

        let future_since = 4_102_444_800;
        let recent_text = n
            .build_recent_items_text_for_self_check(8, future_since)
            .unwrap();
        assert!(recent_text.contains(&pending_id));
        assert!(recent_text.contains("content"));
        assert!(!recent_text.contains("pending exercise reminder"));
        assert!(!recent_text.contains("finished email note"));

        let normal_recent_text = n.build_recent_items_text(8, future_since).unwrap();
        assert!(!normal_recent_text.contains(&pending_id));
        assert!(!normal_recent_text.contains("pending exercise reminder"));
    }

    #[test]
    fn list_filters_archived_by_default() {
        let (_t, n) = open_tmp();
        let _ = n
            .create_or_update_notebook(CreateOrUpdateNotebookInput {
                notebook_id: "user/profile".into(),
                kind: None,
                title: Some("profile".into()),
                description: Some("user profile".into()),
            })
            .unwrap();
        let regs = n
            .list_notebooks(ListNotebooksInput {
                include_archived: false,
            })
            .unwrap();
        assert_eq!(regs.len(), 1);
    }
}
