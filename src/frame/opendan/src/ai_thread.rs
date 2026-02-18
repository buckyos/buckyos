use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

const DEFAULT_DB_REL_PATH: &str = "environment/memory/threads.db";
const DEFAULT_INFERENCE_MAX_CANDIDATES: usize = 64;
const DEFAULT_THREAD_CONFIDENCE: f64 = 0.6;
const DEFAULT_RECORD_CONFIDENCE: f64 = 0.7;
const RECENT_ACTIVITY_WINDOW_MS: i64 = 30 * 60 * 1000;
const MAX_TEXT_FIELD_LEN: usize = 1024;

#[derive(thiserror::Error, Debug)]
pub enum AIThreadError {
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("invalid args: {0}")]
    InvalidArgs(String),
    #[error("io error on `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct AIThreadConfig {
    pub agent_root: PathBuf,
    pub db_rel_path: PathBuf,
    pub inference_max_candidates: usize,
}

impl AIThreadConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_root: agent_root.into(),
            db_rel_path: PathBuf::from(DEFAULT_DB_REL_PATH),
            inference_max_candidates: DEFAULT_INFERENCE_MAX_CANDIDATES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AIThreadStore {
    db_path: PathBuf,
    inference_max_candidates: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    User,
    Tool,
    Workspace,
    Owner,
    System,
    OtherAgent,
}

impl Default for SourceType {
    fn default() -> Self {
        Self::System
    }
}

impl SourceType {
    fn as_str(&self) -> &'static str {
        match self {
            SourceType::User => "user",
            SourceType::Tool => "tool",
            SourceType::Workspace => "workspace",
            SourceType::Owner => "owner",
            SourceType::System => "system",
            SourceType::OtherAgent => "other_agent",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Active,
    Archived,
    Deleted,
}

impl Default for ThreadStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl ThreadStatus {
    fn as_str(&self) -> &'static str {
        match self {
            ThreadStatus::Active => "active",
            ThreadStatus::Archived => "archived",
            ThreadStatus::Deleted => "deleted",
        }
    }

    fn from_db(raw: &str) -> Self {
        match raw {
            "active" => ThreadStatus::Active,
            "archived" => ThreadStatus::Archived,
            "deleted" => ThreadStatus::Deleted,
            _ => ThreadStatus::Active,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Trusted,
    Untrusted,
}

impl Default for TrustLevel {
    fn default() -> Self {
        Self::Untrusted
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Todo,
    Doing,
    Waiting,
    Done,
}

impl Default for TodoStatus {
    fn default() -> Self {
        Self::Todo
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoOwner {
    Agent,
    User,
    External,
}

impl Default for TodoOwner {
    fn default() -> Self {
        Self::Agent
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceSnapshot {
    pub summary: String,
    pub recent_changes: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Provenance {
    pub thread_id: String,
    pub session_id: String,
    pub event_id: String,
    pub source_type: SourceType,
    pub source_ref: String,
    pub ts: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThreadMeta {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub summary: String,
    pub status: ThreadStatus,
    pub tags: Vec<String>,
    pub entities: Vec<String>,
    pub created_ts: i64,
    pub last_activity_ts: i64,
    pub confidence: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThreadState {
    pub working_memory: Vec<MemoryItem>,
    pub todo: Vec<TodoItem>,
    pub facts: Vec<FactRecord>,
    pub worklog: Vec<LogEntry>,
    pub artifacts: Vec<ArtifactRecord>,
    pub last_workspace_snapshot: Option<WorkspaceSnapshot>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Thread {
    pub meta: ThreadMeta,
    pub state: ThreadState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemoryItem {
    pub content: String,
    pub source: String,
    pub ts: i64,
    pub confidence: f64,
    pub trust: TrustLevel,
    pub provenance: Provenance,
    pub tombstone: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TodoItem {
    pub title: String,
    pub next_action: String,
    pub blocked_by: Vec<String>,
    pub status: TodoStatus,
    pub owner: TodoOwner,
    pub provenance: Provenance,
    pub tombstone: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FactRecord {
    pub subject: String,
    pub predicate: String,
    pub obj: String,
    pub confidence: f64,
    pub trust: TrustLevel,
    pub source: String,
    pub provenance: Provenance,
    pub tombstone: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LogEntry {
    pub text: String,
    pub provenance: Provenance,
    pub tombstone: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ArtifactRecord {
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: Json,
    pub provenance: Provenance,
    pub confidence: f64,
    pub tombstone: bool,
}

#[derive(Clone, Debug)]
pub enum ThreadWrite {
    Memory(MemoryItem),
    Todo(TodoItem),
    Fact(FactRecord),
    Log(LogEntry),
    Artifact(ArtifactRecord),
}

impl ThreadWrite {
    fn provenance(&self) -> &Provenance {
        match self {
            ThreadWrite::Memory(item) => &item.provenance,
            ThreadWrite::Todo(item) => &item.provenance,
            ThreadWrite::Fact(item) => &item.provenance,
            ThreadWrite::Log(item) => &item.provenance,
            ThreadWrite::Artifact(item) => &item.provenance,
        }
    }

    fn record_type(&self) -> &'static str {
        match self {
            ThreadWrite::Memory(_) => "memory",
            ThreadWrite::Todo(_) => "todo",
            ThreadWrite::Fact(_) => "fact",
            ThreadWrite::Log(_) => "log",
            ThreadWrite::Artifact(_) => "artifact",
        }
    }

    fn confidence(&self) -> f64 {
        match self {
            ThreadWrite::Memory(item) => item.confidence,
            ThreadWrite::Todo(_) => DEFAULT_RECORD_CONFIDENCE,
            ThreadWrite::Fact(item) => item.confidence,
            ThreadWrite::Log(_) => DEFAULT_RECORD_CONFIDENCE,
            ThreadWrite::Artifact(item) => item.confidence,
        }
    }

    fn serialize_payload(&self) -> Result<String, AIThreadError> {
        match self {
            ThreadWrite::Memory(item) => Ok(serde_json::to_string(item)?),
            ThreadWrite::Todo(item) => Ok(serde_json::to_string(item)?),
            ThreadWrite::Fact(item) => Ok(serde_json::to_string(item)?),
            ThreadWrite::Log(item) => Ok(serde_json::to_string(item)?),
            ThreadWrite::Artifact(item) => Ok(serde_json::to_string(item)?),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ThreadResolveInput {
    pub owner_agent: String,
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub event_id: Option<String>,
    pub source_type: SourceType,
    pub source_ref: Option<String>,
    pub title_hint: Option<String>,
    pub summary_hint: Option<String>,
    pub tags_hint: Vec<String>,
    pub entities_hint: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThreadResolveResult {
    pub thread_id: String,
    pub session_id: String,
    pub inferred: bool,
    pub reason: String,
    pub confidence: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ThreadMetaPatch {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: Option<ThreadStatus>,
    pub tags: Option<Vec<String>>,
    pub entities: Option<Vec<String>>,
    pub confidence: Option<f64>,
    pub last_activity_ts: Option<i64>,
}

impl AIThreadStore {
    pub fn new(mut cfg: AIThreadConfig) -> Result<Self, AIThreadError> {
        if cfg.inference_max_candidates == 0 {
            cfg.inference_max_candidates = DEFAULT_INFERENCE_MAX_CANDIDATES;
        }

        let agent_root = normalize_root(&cfg.agent_root)?;
        cfg.agent_root = agent_root.clone();
        let db_path = resolve_relative_path(&agent_root, &cfg.db_rel_path)?;
        ensure_parent_dir(&db_path)?;
        let store = Self {
            db_path,
            inference_max_candidates: cfg.inference_max_candidates,
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn open(db_path: impl Into<PathBuf>) -> Result<Self, AIThreadError> {
        let db_path = db_path.into();
        ensure_parent_dir(&db_path)?;
        let store = Self {
            db_path,
            inference_max_candidates: DEFAULT_INFERENCE_MAX_CANDIDATES,
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn resolve_thread(
        &self,
        input: ThreadResolveInput,
    ) -> Result<ThreadResolveResult, AIThreadError> {
        let owner_agent = sanitize_required_text("owner_agent", &input.owner_agent)?;
        let explicit_thread_id = sanitize_optional_text(input.thread_id.as_deref())?;
        let session_id_hint = sanitize_optional_text(input.session_id.as_deref())?;
        let source_ref = sanitize_optional_text(input.source_ref.as_deref())?;
        let title_hint = sanitize_optional_text(input.title_hint.as_deref())?;
        let summary_hint = sanitize_optional_text(input.summary_hint.as_deref())?;
        let tags_hint = normalize_text_list(input.tags_hint);
        let entities_hint = normalize_text_list(input.entities_hint);
        let now = now_ms();

        if let Some(thread_id) = explicit_thread_id {
            let session_id = session_id_hint.unwrap_or_else(|| thread_id.clone());
            return self.run_db("resolve explicit thread", |conn| {
                ensure_thread_row(
                    conn,
                    &owner_agent,
                    &thread_id,
                    &session_id,
                    title_hint.as_deref(),
                    summary_hint.as_deref(),
                    &tags_hint,
                    &entities_hint,
                    DEFAULT_THREAD_CONFIDENCE.max(1.0),
                    source_ref.as_deref(),
                    now,
                )?;
                touch_thread(
                    conn,
                    &owner_agent,
                    &thread_id,
                    Some(&session_id),
                    source_ref.as_deref(),
                    now,
                )?;
                Ok(ThreadResolveResult {
                    thread_id,
                    session_id,
                    inferred: false,
                    reason: "explicit_thread_id".to_string(),
                    confidence: 1.0,
                })
            });
        }

        self.run_db("resolve inferred thread", |conn| {
            if let Some(session_id) = session_id_hint.as_deref() {
                if let Some(found) = find_by_session(conn, &owner_agent, session_id)? {
                    touch_thread(
                        conn,
                        &owner_agent,
                        &found.thread_id,
                        Some(found.session_id.as_str()),
                        source_ref.as_deref(),
                        now,
                    )?;
                    return Ok(ThreadResolveResult {
                        thread_id: found.thread_id,
                        session_id: found.session_id,
                        inferred: true,
                        reason: "session_match".to_string(),
                        confidence: found.confidence,
                    });
                }
            }

            if let Some(source_ref) = source_ref.as_deref() {
                if let Some(found) = find_by_source_ref(conn, &owner_agent, source_ref)? {
                    touch_thread(
                        conn,
                        &owner_agent,
                        &found.thread_id,
                        Some(found.session_id.as_str()),
                        Some(source_ref),
                        now,
                    )?;
                    return Ok(ThreadResolveResult {
                        thread_id: found.thread_id,
                        session_id: found.session_id,
                        inferred: true,
                        reason: "source_ref_match".to_string(),
                        confidence: clamp_confidence(found.confidence.max(0.8)),
                    });
                }
            }

            if let Some(found) = find_by_hints(
                conn,
                &owner_agent,
                &tags_hint,
                &entities_hint,
                self.inference_max_candidates,
            )? {
                touch_thread(
                    conn,
                    &owner_agent,
                    &found.thread_id,
                    Some(found.session_id.as_str()),
                    source_ref.as_deref(),
                    now,
                )?;
                return Ok(ThreadResolveResult {
                    thread_id: found.thread_id,
                    session_id: found.session_id,
                    inferred: true,
                    reason: "tags_entities_match".to_string(),
                    confidence: clamp_confidence(found.confidence),
                });
            }

            if !matches!(input.source_type, SourceType::User | SourceType::Owner) {
                if let Some(found) = find_recent_active(conn, &owner_agent, now)? {
                    touch_thread(
                        conn,
                        &owner_agent,
                        &found.thread_id,
                        Some(found.session_id.as_str()),
                        source_ref.as_deref(),
                        now,
                    )?;
                    return Ok(ThreadResolveResult {
                        thread_id: found.thread_id,
                        session_id: found.session_id,
                        inferred: true,
                        reason: "recent_active_match".to_string(),
                        confidence: clamp_confidence(found.confidence.max(0.7)),
                    });
                }
            }

            let thread_id = generate_id("thread");
            let session_id = session_id_hint.unwrap_or_else(|| thread_id.clone());
            ensure_thread_row(
                conn,
                &owner_agent,
                &thread_id,
                &session_id,
                title_hint.as_deref(),
                summary_hint.as_deref(),
                &tags_hint,
                &entities_hint,
                DEFAULT_THREAD_CONFIDENCE,
                source_ref.as_deref(),
                now,
            )?;
            Ok(ThreadResolveResult {
                thread_id,
                session_id,
                inferred: true,
                reason: format!("created_new:{}", input.source_type.as_str()),
                confidence: DEFAULT_THREAD_CONFIDENCE,
            })
        })
    }

    pub fn get_thread(
        &self,
        owner_agent: &str,
        thread_id: &str,
        include_deleted: bool,
        include_tombstone: bool,
    ) -> Result<Option<Thread>, AIThreadError> {
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let thread_id = sanitize_required_text("thread_id", thread_id)?;
        self.run_db("get thread", |conn| {
            let mut sql = String::from(
                "SELECT
                    thread_id, session_id, title, summary, status, tags_json, entities_json,
                    created_ts, last_activity_ts, confidence
                 FROM threads
                 WHERE owner_agent = ?1 AND thread_id = ?2",
            );
            if !include_deleted {
                sql.push_str(" AND status != 'deleted'");
            }
            sql.push_str(" LIMIT 1");

            let row = conn
                .query_row(&sql, params![owner_agent, thread_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, f64>(9)?,
                    ))
                })
                .optional()?;

            let Some((
                id,
                session_id,
                title,
                summary,
                status,
                tags_json,
                entities_json,
                created_ts,
                last_activity_ts,
                confidence,
            )) = row
            else {
                return Ok(None);
            };

            let tags = decode_text_list(&tags_json)?;
            let entities = decode_text_list(&entities_json)?;
            let meta = ThreadMeta {
                id,
                session_id,
                title,
                summary,
                status: ThreadStatus::from_db(&status),
                tags,
                entities,
                created_ts,
                last_activity_ts,
                confidence: clamp_confidence(confidence),
            };

            let mut state = ThreadState::default();
            let mut records_sql = String::from(
                "SELECT record_type, payload_json, confidence, tombstone
                 FROM thread_records
                 WHERE owner_agent = ?1 AND thread_id = ?2",
            );
            if !include_tombstone {
                records_sql.push_str(" AND tombstone = 0");
            }
            records_sql.push_str(" ORDER BY created_ts ASC");
            let mut stmt = conn.prepare(&records_sql)?;
            let rows = stmt.query_map(params![owner_agent, thread_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            })?;

            for row in rows {
                let (record_type, payload_json, confidence, tombstone) = row?;
                match record_type.as_str() {
                    "memory" => {
                        let mut item: MemoryItem = serde_json::from_str(&payload_json)?;
                        item.confidence = clamp_confidence(confidence);
                        item.tombstone = tombstone;
                        state.working_memory.push(item);
                    }
                    "todo" => {
                        let mut item: TodoItem = serde_json::from_str(&payload_json)?;
                        item.tombstone = tombstone;
                        state.todo.push(item);
                    }
                    "fact" => {
                        let mut item: FactRecord = serde_json::from_str(&payload_json)?;
                        item.confidence = clamp_confidence(confidence);
                        item.tombstone = tombstone;
                        state.facts.push(item);
                    }
                    "log" => {
                        let mut item: LogEntry = serde_json::from_str(&payload_json)?;
                        item.tombstone = tombstone;
                        state.worklog.push(item);
                    }
                    "artifact" => {
                        let mut item: ArtifactRecord = serde_json::from_str(&payload_json)?;
                        item.confidence = clamp_confidence(confidence);
                        item.tombstone = tombstone;
                        if item.kind == "workspace_observation" {
                            if let Ok(snapshot) =
                                serde_json::from_value::<WorkspaceSnapshot>(item.payload.clone())
                            {
                                state.last_workspace_snapshot = Some(snapshot);
                            }
                        }
                        state.artifacts.push(item);
                    }
                    _ => {}
                }
            }

            Ok(Some(Thread { meta, state }))
        })
    }

    pub fn update_thread_meta(
        &self,
        owner_agent: &str,
        thread_id: &str,
        patch: ThreadMetaPatch,
    ) -> Result<Option<ThreadMeta>, AIThreadError> {
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let thread_id = sanitize_required_text("thread_id", thread_id)?;
        self.run_db("update thread meta", |conn| {
            let row = conn
                .query_row(
                    "SELECT
                        session_id, title, summary, status, tags_json, entities_json,
                        created_ts, last_activity_ts, confidence
                     FROM threads
                     WHERE owner_agent = ?1 AND thread_id = ?2
                     LIMIT 1",
                    params![owner_agent, thread_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, f64>(8)?,
                        ))
                    },
                )
                .optional()?;

            let Some((
                session_id,
                old_title,
                old_summary,
                old_status,
                old_tags_json,
                old_entities_json,
                created_ts,
                old_last_activity_ts,
                old_confidence,
            )) = row
            else {
                return Ok(None);
            };

            let title = patch
                .title
                .as_deref()
                .map(sanitize_required_text_inplace)
                .transpose()?
                .unwrap_or(old_title);
            let summary = patch
                .summary
                .as_deref()
                .map(sanitize_required_text_inplace)
                .transpose()?
                .unwrap_or(old_summary);
            let status = patch
                .status
                .as_ref()
                .map(|v| v.as_str().to_string())
                .unwrap_or(old_status);
            let tags = patch
                .tags
                .map(normalize_text_list)
                .unwrap_or_else(|| decode_text_list(&old_tags_json).unwrap_or_default());
            let entities = patch
                .entities
                .map(normalize_text_list)
                .unwrap_or_else(|| decode_text_list(&old_entities_json).unwrap_or_default());
            let confidence = clamp_confidence(patch.confidence.unwrap_or(old_confidence));
            let last_activity_ts = patch.last_activity_ts.unwrap_or(old_last_activity_ts);

            let tags_json = serde_json::to_string(&tags)?;
            let entities_json = serde_json::to_string(&entities)?;

            conn.execute(
                "UPDATE threads
                 SET title = ?1, summary = ?2, status = ?3, tags_json = ?4, entities_json = ?5,
                     confidence = ?6, last_activity_ts = ?7
                 WHERE owner_agent = ?8 AND thread_id = ?9",
                params![
                    title,
                    summary,
                    status,
                    tags_json,
                    entities_json,
                    confidence,
                    last_activity_ts,
                    owner_agent,
                    thread_id
                ],
            )?;

            Ok(Some(ThreadMeta {
                id: thread_id.to_string(),
                session_id,
                title,
                summary,
                status: ThreadStatus::from_db(&status),
                tags,
                entities,
                created_ts,
                last_activity_ts,
                confidence,
            }))
        })
    }

    pub fn append_write(
        &self,
        owner_agent: &str,
        write: ThreadWrite,
    ) -> Result<String, AIThreadError> {
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let provenance = write.provenance();
        let thread_id = sanitize_required_text("provenance.thread_id", &provenance.thread_id)?;
        let session_id = sanitize_optional_text(Some(&provenance.session_id))?
            .unwrap_or_else(|| thread_id.clone());
        let source_ref = sanitize_optional_text(Some(&provenance.source_ref))?;
        let payload_json = write.serialize_payload()?;
        let record_type = write.record_type();
        let confidence = clamp_confidence(write.confidence());
        let now = now_ms();
        let record_id = generate_id("rec");
        let event_id = sanitize_optional_text(Some(&provenance.event_id))?;

        self.run_db("append thread write", |conn| {
            ensure_thread_row(
                conn,
                &owner_agent,
                &thread_id,
                &session_id,
                None,
                None,
                &[],
                &[],
                DEFAULT_THREAD_CONFIDENCE,
                source_ref.as_deref(),
                now,
            )?;
            conn.execute(
                "INSERT INTO thread_records (
                    owner_agent, thread_id, record_id, record_type, payload_json, confidence,
                    trust, tombstone, created_ts, updated_ts, provenance_json, source_ref, event_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, ?7, ?8, ?9, ?10, ?11)",
                params![
                    owner_agent,
                    thread_id,
                    record_id,
                    record_type,
                    payload_json,
                    confidence,
                    now,
                    now,
                    serde_json::to_string(provenance)?,
                    source_ref,
                    event_id
                ],
            )?;
            touch_thread(
                conn,
                &owner_agent,
                &thread_id,
                Some(&session_id),
                source_ref.as_deref(),
                now,
            )?;
            Ok(record_id.clone())
        })
    }

    pub fn delete_thread(&self, owner_agent: &str, thread_id: &str) -> Result<bool, AIThreadError> {
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let thread_id = sanitize_required_text("thread_id", thread_id)?;
        self.run_db("delete thread", |conn| {
            let changed = conn.execute(
                "UPDATE threads
                 SET status = 'deleted', deleted_ts = ?1, last_activity_ts = ?2
                 WHERE owner_agent = ?3 AND thread_id = ?4 AND status != 'deleted'",
                params![now_ms(), now_ms(), owner_agent, thread_id],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn deweight_thread(
        &self,
        owner_agent: &str,
        thread_id: &str,
        delta: f64,
    ) -> Result<Option<f64>, AIThreadError> {
        if delta <= 0.0 {
            return Err(AIThreadError::InvalidArgs("delta must be > 0".to_string()));
        }
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let thread_id = sanitize_required_text("thread_id", thread_id)?;
        self.run_db("deweight thread", |conn| {
            let changed = conn.execute(
                "UPDATE threads
                 SET confidence = MAX(0.0, confidence - ?1), last_activity_ts = ?2
                 WHERE owner_agent = ?3 AND thread_id = ?4",
                params![delta, now_ms(), owner_agent, thread_id],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            let confidence: f64 = conn.query_row(
                "SELECT confidence FROM threads WHERE owner_agent = ?1 AND thread_id = ?2",
                params![owner_agent, thread_id],
                |row| row.get(0),
            )?;
            Ok(Some(clamp_confidence(confidence)))
        })
    }

    pub fn tombstone_record(
        &self,
        owner_agent: &str,
        record_id: &str,
    ) -> Result<bool, AIThreadError> {
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let record_id = sanitize_required_text("record_id", record_id)?;
        self.run_db("tombstone record", |conn| {
            let changed = conn.execute(
                "UPDATE thread_records
                 SET tombstone = 1, updated_ts = ?1
                 WHERE owner_agent = ?2 AND record_id = ?3",
                params![now_ms(), owner_agent, record_id],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn deweight_record(
        &self,
        owner_agent: &str,
        record_id: &str,
        delta: f64,
    ) -> Result<Option<f64>, AIThreadError> {
        if delta <= 0.0 {
            return Err(AIThreadError::InvalidArgs("delta must be > 0".to_string()));
        }
        let owner_agent = sanitize_required_text("owner_agent", owner_agent)?;
        let record_id = sanitize_required_text("record_id", record_id)?;
        self.run_db("deweight record", |conn| {
            let changed = conn.execute(
                "UPDATE thread_records
                 SET confidence = MAX(0.0, confidence - ?1), updated_ts = ?2
                 WHERE owner_agent = ?3 AND record_id = ?4",
                params![delta, now_ms(), owner_agent, record_id],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            let confidence: f64 = conn.query_row(
                "SELECT confidence FROM thread_records WHERE owner_agent = ?1 AND record_id = ?2",
                params![owner_agent, record_id],
                |row| row.get(0),
            )?;
            Ok(Some(clamp_confidence(confidence)))
        })
    }

    fn init_schema(&self) -> Result<(), AIThreadError> {
        self.run_db("init schema", |conn| ensure_schema(conn))
    }

    fn run_db<T, F>(&self, _op: &str, f: F) -> Result<T, AIThreadError>
    where
        F: FnOnce(&Connection) -> Result<T, AIThreadError>,
    {
        let conn = Connection::open(&self.db_path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        f(&conn)
    }
}

#[derive(Clone, Debug, Default)]
struct ThreadCandidate {
    thread_id: String,
    session_id: String,
    confidence: f64,
}

fn ensure_schema(conn: &Connection) -> Result<(), AIThreadError> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS threads (
    owner_agent TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
    tags_json TEXT NOT NULL DEFAULT '[]',
    entities_json TEXT NOT NULL DEFAULT '[]',
    confidence REAL NOT NULL DEFAULT 0.6,
    created_ts INTEGER NOT NULL DEFAULT 0,
    last_activity_ts INTEGER NOT NULL DEFAULT 0,
    last_source_ref TEXT,
    deleted_ts INTEGER,
    PRIMARY KEY(owner_agent, thread_id)
);

CREATE INDEX IF NOT EXISTS idx_threads_owner_activity
ON threads(owner_agent, last_activity_ts DESC);

CREATE INDEX IF NOT EXISTS idx_threads_owner_status_activity
ON threads(owner_agent, status, last_activity_ts DESC);

CREATE INDEX IF NOT EXISTS idx_threads_owner_session
ON threads(owner_agent, session_id);

CREATE INDEX IF NOT EXISTS idx_threads_owner_source
ON threads(owner_agent, last_source_ref, last_activity_ts DESC);

CREATE TABLE IF NOT EXISTS thread_records (
    owner_agent TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    record_id TEXT PRIMARY KEY,
    record_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.7,
    trust TEXT,
    tombstone INTEGER NOT NULL DEFAULT 0,
    created_ts INTEGER NOT NULL DEFAULT 0,
    updated_ts INTEGER NOT NULL DEFAULT 0,
    provenance_json TEXT NOT NULL,
    source_ref TEXT,
    event_id TEXT,
    FOREIGN KEY(owner_agent, thread_id)
        REFERENCES threads(owner_agent, thread_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_records_owner_thread_ts
ON thread_records(owner_agent, thread_id, created_ts DESC);

CREATE INDEX IF NOT EXISTS idx_thread_records_owner_source
ON thread_records(owner_agent, source_ref, created_ts DESC);

CREATE INDEX IF NOT EXISTS idx_thread_records_owner_event
ON thread_records(owner_agent, event_id, created_ts DESC);
"#,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ensure_thread_row(
    conn: &Connection,
    owner_agent: &str,
    thread_id: &str,
    session_id: &str,
    title: Option<&str>,
    summary: Option<&str>,
    tags: &[String],
    entities: &[String],
    confidence: f64,
    source_ref: Option<&str>,
    ts: i64,
) -> Result<(), AIThreadError> {
    let title = title.unwrap_or_default();
    let summary = summary.unwrap_or_default();
    let tags_json = serde_json::to_string(tags)?;
    let entities_json = serde_json::to_string(entities)?;
    conn.execute(
        "INSERT OR IGNORE INTO threads (
            owner_agent, thread_id, session_id, title, summary, status, tags_json,
            entities_json, confidence, created_ts, last_activity_ts, last_source_ref
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            owner_agent,
            thread_id,
            session_id,
            title,
            summary,
            tags_json,
            entities_json,
            clamp_confidence(confidence),
            ts,
            ts,
            source_ref
        ],
    )?;

    conn.execute(
        "UPDATE threads
         SET
            session_id = CASE WHEN ?1 != '' THEN ?1 ELSE session_id END,
            title = CASE WHEN title = '' AND ?2 != '' THEN ?2 ELSE title END,
            summary = CASE WHEN summary = '' AND ?3 != '' THEN ?3 ELSE summary END,
            last_activity_ts = MAX(last_activity_ts, ?4),
            last_source_ref = COALESCE(?5, last_source_ref),
            status = CASE WHEN status = 'deleted' THEN 'active' ELSE status END
         WHERE owner_agent = ?6 AND thread_id = ?7",
        params![
            session_id,
            title,
            summary,
            ts,
            source_ref,
            owner_agent,
            thread_id
        ],
    )?;
    Ok(())
}

fn touch_thread(
    conn: &Connection,
    owner_agent: &str,
    thread_id: &str,
    session_id: Option<&str>,
    source_ref: Option<&str>,
    ts: i64,
) -> Result<(), AIThreadError> {
    conn.execute(
        "UPDATE threads
         SET
            session_id = COALESCE(NULLIF(?1, ''), session_id),
            last_activity_ts = MAX(last_activity_ts, ?2),
            last_source_ref = COALESCE(?3, last_source_ref)
         WHERE owner_agent = ?4 AND thread_id = ?5",
        params![session_id, ts, source_ref, owner_agent, thread_id],
    )?;
    Ok(())
}

fn find_by_session(
    conn: &Connection,
    owner_agent: &str,
    session_id: &str,
) -> Result<Option<ThreadCandidate>, AIThreadError> {
    conn.query_row(
        "SELECT thread_id, session_id, confidence
         FROM threads
         WHERE owner_agent = ?1 AND session_id = ?2 AND status != 'deleted'
         ORDER BY last_activity_ts DESC
         LIMIT 1",
        params![owner_agent, session_id],
        |row| {
            Ok(ThreadCandidate {
                thread_id: row.get(0)?,
                session_id: row.get(1)?,
                confidence: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(AIThreadError::from)
}

fn find_by_source_ref(
    conn: &Connection,
    owner_agent: &str,
    source_ref: &str,
) -> Result<Option<ThreadCandidate>, AIThreadError> {
    conn.query_row(
        "SELECT thread_id, session_id, confidence
         FROM threads
         WHERE owner_agent = ?1 AND last_source_ref = ?2 AND status != 'deleted'
         ORDER BY last_activity_ts DESC
         LIMIT 1",
        params![owner_agent, source_ref],
        |row| {
            Ok(ThreadCandidate {
                thread_id: row.get(0)?,
                session_id: row.get(1)?,
                confidence: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(AIThreadError::from)
}

fn find_by_hints(
    conn: &Connection,
    owner_agent: &str,
    tags_hint: &[String],
    entities_hint: &[String],
    limit: usize,
) -> Result<Option<ThreadCandidate>, AIThreadError> {
    if tags_hint.is_empty() && entities_hint.is_empty() {
        return Ok(None);
    }

    let tags_set = tags_hint.iter().cloned().collect::<HashSet<_>>();
    let entities_set = entities_hint.iter().cloned().collect::<HashSet<_>>();
    let limit = i64::try_from(limit.max(1)).map_err(|_| {
        AIThreadError::InvalidArgs("inference candidate limit too large".to_string())
    })?;

    let mut stmt = conn.prepare(
        "SELECT thread_id, session_id, confidence, tags_json, entities_json
         FROM threads
         WHERE owner_agent = ?1 AND status != 'deleted'
         ORDER BY last_activity_ts DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![owner_agent, limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    let mut best: Option<ThreadCandidate> = None;
    let mut best_score = 0_usize;
    for row in rows {
        let (thread_id, session_id, confidence, tags_json, entities_json) = row?;
        let tags = decode_text_list(&tags_json)?;
        let entities = decode_text_list(&entities_json)?;
        let tag_score = tags.iter().filter(|v| tags_set.contains(*v)).count();
        let entity_score = entities
            .iter()
            .filter(|v| entities_set.contains(*v))
            .count();
        let score = tag_score + entity_score;
        if score == 0 || score < best_score {
            continue;
        }
        let scored_confidence = clamp_confidence(confidence + 0.05_f64 * score as f64);
        best_score = score;
        best = Some(ThreadCandidate {
            thread_id,
            session_id,
            confidence: scored_confidence,
        });
    }
    Ok(best)
}

fn find_recent_active(
    conn: &Connection,
    owner_agent: &str,
    now: i64,
) -> Result<Option<ThreadCandidate>, AIThreadError> {
    conn.query_row(
        "SELECT thread_id, session_id, confidence
         FROM threads
         WHERE owner_agent = ?1 AND status = 'active' AND last_activity_ts >= ?2
         ORDER BY last_activity_ts DESC
         LIMIT 1",
        params![owner_agent, now.saturating_sub(RECENT_ACTIVITY_WINDOW_MS)],
        |row| {
            Ok(ThreadCandidate {
                thread_id: row.get(0)?,
                session_id: row.get(1)?,
                confidence: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(AIThreadError::from)
}

fn normalize_root(root: &Path) -> Result<PathBuf, AIThreadError> {
    if root.as_os_str().is_empty() {
        return Err(AIThreadError::InvalidConfig(
            "agent_root cannot be empty".to_string(),
        ));
    }
    std::fs::create_dir_all(root).map_err(|source| AIThreadError::Io {
        path: root.display().to_string(),
        source,
    })?;
    std::fs::canonicalize(root).map_err(|source| AIThreadError::Io {
        path: root.display().to_string(),
        source,
    })
}

fn resolve_relative_path(root: &Path, rel_path: &Path) -> Result<PathBuf, AIThreadError> {
    if rel_path.is_absolute() {
        return Err(AIThreadError::InvalidConfig(format!(
            "db_rel_path `{}` must be relative",
            rel_path.display()
        )));
    }
    for component in rel_path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(AIThreadError::InvalidConfig(format!(
                "db_rel_path `{}` cannot contain `..`",
                rel_path.display()
            )));
        }
    }
    Ok(root.join(rel_path))
}

fn ensure_parent_dir(path: &Path) -> Result<(), AIThreadError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|source| AIThreadError::Io {
        path: parent.display().to_string(),
        source,
    })?;
    Ok(())
}

fn sanitize_required_text(name: &str, raw: &str) -> Result<String, AIThreadError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AIThreadError::InvalidArgs(format!(
            "`{name}` cannot be empty"
        )));
    }
    if trimmed.len() > MAX_TEXT_FIELD_LEN {
        return Err(AIThreadError::InvalidArgs(format!(
            "`{name}` length exceeds {MAX_TEXT_FIELD_LEN}"
        )));
    }
    Ok(trimmed.to_string())
}

fn sanitize_required_text_inplace(raw: &str) -> Result<String, AIThreadError> {
    sanitize_required_text("meta field", raw)
}

fn sanitize_optional_text(raw: Option<&str>) -> Result<Option<String>, AIThreadError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_TEXT_FIELD_LEN {
        return Err(AIThreadError::InvalidArgs(format!(
            "text length exceeds {MAX_TEXT_FIELD_LEN}"
        )));
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_text_list(items: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();
    for item in items {
        let normalized = item.trim().to_string();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn decode_text_list(raw: &str) -> Result<Vec<String>, AIThreadError> {
    let values = serde_json::from_str::<Vec<String>>(raw).unwrap_or_default();
    Ok(normalize_text_list(values))
}

fn clamp_confidence(value: f64) -> f64 {
    if !value.is_finite() {
        return DEFAULT_THREAD_CONFIDENCE;
    }
    value.clamp(0.0, 1.0)
}

fn now_ms() -> i64 {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(dur.as_millis()).unwrap_or(i64::MAX)
}

fn generate_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn resolve_thread_prefers_explicit_thread_id() {
        let tmp = tempdir().expect("create tmpdir");
        let store = AIThreadStore::open(tmp.path().join("threads.db")).expect("open store");

        store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-a".to_string()),
                source_type: SourceType::User,
                source_ref: Some("msg:a".to_string()),
                ..Default::default()
            })
            .expect("create thread-a");

        store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-b".to_string()),
                source_type: SourceType::User,
                source_ref: Some("msg:b".to_string()),
                ..Default::default()
            })
            .expect("create thread-b");

        let resolved = store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-a".to_string()),
                source_type: SourceType::Tool,
                source_ref: Some("msg:b".to_string()),
                ..Default::default()
            })
            .expect("resolve explicit");

        assert_eq!(resolved.thread_id, "thread-a");
        assert!(!resolved.inferred);
        assert_eq!(resolved.reason, "explicit_thread_id");
    }

    #[test]
    fn resolve_thread_infers_from_source_ref() {
        let tmp = tempdir().expect("create tmpdir");
        let store = AIThreadStore::open(tmp.path().join("threads.db")).expect("open store");
        store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-source".to_string()),
                source_type: SourceType::Tool,
                source_ref: Some("tool:web.search#123".to_string()),
                ..Default::default()
            })
            .expect("create source thread");

        let inferred = store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                source_type: SourceType::Tool,
                source_ref: Some("tool:web.search#123".to_string()),
                ..Default::default()
            })
            .expect("infer by source_ref");

        assert_eq!(inferred.thread_id, "thread-source");
        assert!(inferred.inferred);
        assert_eq!(inferred.reason, "source_ref_match");
    }

    #[test]
    fn write_with_provenance_and_query_thread() {
        let tmp = tempdir().expect("create tmpdir");
        let store = AIThreadStore::open(tmp.path().join("threads.db")).expect("open store");
        store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-state".to_string()),
                session_id: Some("session-state".to_string()),
                source_type: SourceType::User,
                ..Default::default()
            })
            .expect("create thread");

        let memory = MemoryItem {
            content: "remember owner asks concise outputs".to_string(),
            source: "user-msg".to_string(),
            ts: now_ms(),
            confidence: 0.9,
            trust: TrustLevel::Untrusted,
            provenance: Provenance {
                thread_id: "thread-state".to_string(),
                session_id: "session-state".to_string(),
                event_id: "evt-1".to_string(),
                source_type: SourceType::User,
                source_ref: "msg:center#1".to_string(),
                ts: now_ms(),
            },
            tombstone: false,
        };

        let record_id = store
            .append_write("did:example:agent", ThreadWrite::Memory(memory))
            .expect("append memory");
        assert!(record_id.starts_with("rec-"));

        let updated = store
            .update_thread_meta(
                "did:example:agent",
                "thread-state",
                ThreadMetaPatch {
                    title: Some("Owner Preferences".to_string()),
                    summary: Some("Track user style and constraints".to_string()),
                    tags: Some(vec!["preferences".to_string(), "owner".to_string()]),
                    entities: Some(vec!["owner".to_string(), "style".to_string()]),
                    confidence: Some(0.88),
                    ..Default::default()
                },
            )
            .expect("update meta")
            .expect("meta exists");
        assert_eq!(updated.title, "Owner Preferences");
        assert_eq!(updated.tags.len(), 2);

        let thread = store
            .get_thread("did:example:agent", "thread-state", false, false)
            .expect("get thread")
            .expect("thread should exist");

        assert_eq!(thread.meta.id, "thread-state");
        assert_eq!(thread.meta.session_id, "session-state");
        assert_eq!(thread.state.working_memory.len(), 1);
        assert_eq!(
            thread.state.working_memory[0].provenance.source_ref,
            "msg:center#1"
        );
    }

    #[test]
    fn delete_and_deweight_thread_and_record() {
        let tmp = tempdir().expect("create tmpdir");
        let store = AIThreadStore::open(tmp.path().join("threads.db")).expect("open store");
        store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-cleanup".to_string()),
                source_type: SourceType::System,
                ..Default::default()
            })
            .expect("create thread");

        let fact = FactRecord {
            subject: "repo".to_string(),
            predicate: "has_branch".to_string(),
            obj: "feat/thread-db".to_string(),
            confidence: 0.9,
            trust: TrustLevel::Trusted,
            source: "workspace-scan".to_string(),
            provenance: Provenance {
                thread_id: "thread-cleanup".to_string(),
                session_id: "thread-cleanup".to_string(),
                event_id: "evt-2".to_string(),
                source_type: SourceType::Workspace,
                source_ref: "workspace:git@1".to_string(),
                ts: now_ms(),
            },
            tombstone: false,
        };
        let record_id = store
            .append_write("did:example:agent", ThreadWrite::Fact(fact))
            .expect("append fact");

        let new_thread_conf = store
            .deweight_thread("did:example:agent", "thread-cleanup", 0.4)
            .expect("deweight thread")
            .expect("thread should exist");
        assert!(new_thread_conf <= 0.6);

        let new_record_conf = store
            .deweight_record("did:example:agent", &record_id, 0.5)
            .expect("deweight record")
            .expect("record should exist");
        assert!(new_record_conf <= 0.4);

        assert!(store
            .tombstone_record("did:example:agent", &record_id)
            .expect("tombstone record"));

        let no_tombstone = store
            .get_thread("did:example:agent", "thread-cleanup", false, false)
            .expect("get thread")
            .expect("thread exists");
        assert!(no_tombstone.state.facts.is_empty());

        let with_tombstone = store
            .get_thread("did:example:agent", "thread-cleanup", false, true)
            .expect("get thread")
            .expect("thread exists");
        assert_eq!(with_tombstone.state.facts.len(), 1);
        assert!(with_tombstone.state.facts[0].tombstone);

        assert!(store
            .delete_thread("did:example:agent", "thread-cleanup")
            .expect("delete thread"));
        let hidden = store
            .get_thread("did:example:agent", "thread-cleanup", false, true)
            .expect("query");
        assert!(hidden.is_none());
        let visible_deleted = store
            .get_thread("did:example:agent", "thread-cleanup", true, true)
            .expect("query")
            .expect("deleted thread should be visible");
        assert_eq!(visible_deleted.meta.status, ThreadStatus::Deleted);
    }

    #[test]
    fn artifact_workspace_observation_updates_last_snapshot() {
        let tmp = tempdir().expect("create tmpdir");
        let store = AIThreadStore::open(tmp.path().join("threads.db")).expect("open store");
        store
            .resolve_thread(ThreadResolveInput {
                owner_agent: "did:example:agent".to_string(),
                thread_id: Some("thread-observe".to_string()),
                source_type: SourceType::Workspace,
                ..Default::default()
            })
            .expect("create thread");

        let artifact = ArtifactRecord {
            kind: "workspace_observation".to_string(),
            payload: json!({
                "summary": "workspace has pending changes",
                "recent_changes": ["src/frame/opendan/src/ai_thread.rs"],
                "errors": []
            }),
            provenance: Provenance {
                thread_id: "thread-observe".to_string(),
                session_id: "thread-observe".to_string(),
                event_id: "evt-observe".to_string(),
                source_type: SourceType::Workspace,
                source_ref: "workspace:scan#1".to_string(),
                ts: now_ms(),
            },
            confidence: 0.82,
            tombstone: false,
        };

        store
            .append_write("did:example:agent", ThreadWrite::Artifact(artifact))
            .expect("append artifact");

        let thread = store
            .get_thread("did:example:agent", "thread-observe", false, false)
            .expect("get thread")
            .expect("thread exists");
        let snapshot = thread
            .state
            .last_workspace_snapshot
            .expect("workspace snapshot should be extracted");
        assert!(snapshot.summary.contains("pending changes"));
        assert_eq!(snapshot.recent_changes.len(), 1);
    }
}
