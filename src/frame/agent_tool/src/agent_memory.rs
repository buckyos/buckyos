//! Agent Memory v2.10 — Memory Graph over local canonical JSON + occasion log.
//!
//! The append-only `.meta/occasions.jsonl` log is the replay truth. Canonical
//! JSON files, graph snapshots, path indexes, and SQLite are derived state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};
use fs2::FileExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

pub const SCHEMA_VERSION: &str = "2.10";
pub const PRIMARY_LANGUAGE: &str = "en";
pub const META_DIR: &str = ".meta";
pub const META_JSON: &str = "meta.json";
pub const OCCASIONS_LOG_FILE: &str = "occasions.jsonl";
pub const STATE_FILE: &str = "state.jsonl";
pub const LOCK_FILE: &str = "lock";
pub const SQLITE_FILE: &str = "memory.sqlite";
pub const ARCHIVE_DIR: &str = "archive";
pub const GRAPH_DIR: &str = "graph";
pub const OCCASION_DIR: &str = "occasion";
pub const OBJECT_DIR: &str = "object";
pub const OBSERVATION_DIR: &str = "observation";
pub const ITEM_DIR: &str = "item";
pub const INDEX_DIR: &str = "index";

pub const DEFAULT_MAX_RECORDS: usize = 50;
pub const DEFAULT_MAX_BYTES: usize = 65536;
pub const DEFAULT_BODY_TRUNCATE_BYTES: usize = 4096;
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const SOFT_CONTENT_WARN_BYTES: usize = 256 * 1024;

pub const MAX_SEGMENT_BYTES: usize = 200;
pub const MIN_TAG_BYTES: usize = 2;
pub const MAX_TAG_BYTES: usize = 32;
pub const DEFAULT_FREE_WEIGHT: f64 = 0.5;
pub const DEFAULT_FREE_CONFIDENCE: f64 = 0.5;

const FORBIDDEN_FIRST_SEGMENTS: &[&str] = &[
    META_DIR,
    SQLITE_FILE,
    GRAPH_DIR,
    OCCASION_DIR,
    OBJECT_DIR,
    OBSERVATION_DIR,
    ITEM_DIR,
    INDEX_DIR,
];

const PERCENT_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(0x7f);

#[derive(Debug, Error)]
pub enum AgentMemoryError {
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("lock contention: {0}")]
    LockTimeout(String),
    #[error("corrupted state: {0}")]
    Corrupted(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl AgentMemoryError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::LockTimeout(_) => 2,
            Self::Corrupted(_) => 3,
            _ => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgentMemoryError>;

#[derive(Clone, Debug)]
pub struct AgentMemoryConfig {
    pub root: PathBuf,
    pub lock_timeout: Duration,
}

impl AgentMemoryConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriterInfo {
    pub lang: String,
    pub r#impl: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphMeta {
    pub occasion_log: String,
    pub time_model: String,
    pub state_snapshot: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexMeta {
    pub engine: String,
    pub tokenizer: String,
    pub mechanical_indexes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaJson {
    pub schema_version: String,
    pub primary_language: String,
    pub writer: WriterInfo,
    pub graph: GraphMeta,
    pub index: IndexMeta,
    pub compaction_strategy: String,
    pub initialized_by_occasion: String,
    pub created_at: String,
}

impl MetaJson {
    fn default_now() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            primary_language: PRIMARY_LANGUAGE.to_string(),
            writer: WriterInfo {
                lang: "rust".to_string(),
                r#impl: "agent-memory-rs".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            graph: GraphMeta {
                occasion_log: format!("{META_DIR}/{OCCASIONS_LOG_FILE}"),
                time_model: "dual:occurred_at+noticed_at".to_string(),
                state_snapshot: format!("{META_DIR}/{STATE_FILE}"),
            },
            index: IndexMeta {
                engine: "sqlite-fts5".to_string(),
                tokenizer: "unicode61 remove_diacritics 2".to_string(),
                mechanical_indexes: vec![
                    "by_entity".to_string(),
                    "by_alias".to_string(),
                    "by_pair".to_string(),
                    "by_kind".to_string(),
                    "by_predicate".to_string(),
                    "by_relation".to_string(),
                    "by_weight".to_string(),
                ],
            },
            compaction_strategy: "snapshot".to_string(),
            initialized_by_occasion: "occ_init".to_string(),
            created_at: now_iso8601(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Preamble {
    pub importance: i64,
    pub expired_at: Option<String>,
    pub body: String,
    pub body_offset: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRef {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notebook_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryOccasion {
    pub schema_version: String,
    pub occasion_id: String,
    pub seq: u64,
    pub occurred_at: String,
    pub noticed_at: String,
    pub occasion_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub operations: Vec<GraphOperation>,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GraphOperation {
    UpsertObject(UpsertObjectOp),
    AddObservation(AddObservationOp),
    ReinforceObjectWeight(ReinforceObjectWeightOp),
    UpsertRelation(UpsertRelationOp),
    SetStatus(SetStatusOp),
    SetFree(FlatSetOp),
    RemoveFree(FlatRemoveOp),
    PutItem(PutItemOp),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpsertObjectOp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    pub kind: String,
    pub canonical_name: String,
    #[serde(default)]
    pub aliases: Vec<ObjectAliasInput>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_into: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectAliasInput {
    pub alias: String,
    pub alias_type: String,
    pub confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddObservationOp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    pub confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReinforceObjectWeightOp {
    pub object_id: String,
    pub delta: f64,
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpsertRelationOp {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub write_reason: String,
    #[serde(default)]
    pub replaces: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetStatusOp {
    pub target_kind: String,
    pub target_id: String,
    pub status: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlatSetOp {
    pub key: String,
    pub content: String,
    pub reason: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlatRemoveOp {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PutItemOp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub claim: Value,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub write_reason: String,
    #[serde(default)]
    pub replaces: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryObject {
    pub object_id: String,
    pub kind: String,
    pub canonical_name: String,
    #[serde(default)]
    pub aliases: Vec<ObjectAlias>,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub source_occasion: String,
    pub last_occasion: String,
    pub noticed_at: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_into: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectAlias {
    pub alias: String,
    pub alias_type: String,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub source_occasion: String,
    pub noticed_at: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryObservation {
    pub observation_id: String,
    pub kind: String,
    pub source_occasion: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    pub confidence: f64,
    pub noticed_at: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryItem {
    pub item_id: String,
    pub kind: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub claim: Value,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub source_occasion: String,
    pub noticed_at: String,
    pub write_reason: String,
    pub status: String,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_key: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OccasionAddInput {
    pub occasion_type: String,
    pub summary: String,
    pub occurred_at: Option<String>,
    pub source_ref: Option<SourceRef>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OccasionAddResult {
    pub occasion_id: String,
    pub seq: u64,
}

#[derive(Clone, Debug)]
pub struct LoadOptions {
    pub max_records: usize,
    pub max_bytes: usize,
    pub body_truncate_bytes: usize,
    pub current_time: Option<DateTime<Utc>>,
    pub objects: Vec<String>,
    pub aliases: Vec<String>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            max_records: DEFAULT_MAX_RECORDS,
            max_bytes: DEFAULT_MAX_BYTES,
            body_truncate_bytes: DEFAULT_BODY_TRUNCATE_BYTES,
            current_time: None,
            objects: Vec::new(),
            aliases: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadItem {
    pub item_id: String,
    pub kind: String,
    pub entities: Vec<String>,
    pub weight: f64,
    pub confidence: f64,
    pub source_occasion: String,
    pub noticed_at: String,
    pub evidence: Vec<String>,
    pub matched: Vec<String>,
    pub size: usize,
    pub truncated: bool,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryHintType {
    SessionRaw,
    Event,
    EntityObservation,
    EntityRelation,
    Free,
}

#[derive(Clone, Debug)]
pub struct MemoryHintBudget {
    pub candidate_limit: usize,
    pub keep_limit: usize,
}

impl MemoryHintBudget {
    pub fn new(candidate_limit: usize, keep_limit: usize) -> Self {
        Self {
            candidate_limit,
            keep_limit,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryRecallOptions {
    pub max_hints: usize,
    pub body_truncate_bytes: usize,
    pub session_raw: MemoryHintBudget,
    pub event: MemoryHintBudget,
    pub entity_observation: MemoryHintBudget,
    pub entity_relation: MemoryHintBudget,
    pub free: MemoryHintBudget,
    pub current_time: Option<DateTime<Utc>>,
}

impl Default for MemoryRecallOptions {
    fn default() -> Self {
        Self {
            max_hints: 5,
            body_truncate_bytes: 160,
            session_raw: MemoryHintBudget::new(8, 2),
            event: MemoryHintBudget::new(8, 2),
            entity_observation: MemoryHintBudget::new(8, 2),
            entity_relation: MemoryHintBudget::new(8, 2),
            free: MemoryHintBudget::new(8, 2),
            current_time: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryHint {
    pub hint_type: MemoryHintType,
    pub target_kind: String,
    pub target_id: String,
    pub uri: Option<String>,
    pub hint: String,
    pub reason: String,
    pub matched: Vec<String>,
    pub score: f32,
    pub source_occasion: String,
    pub noticed_at: String,
    pub evidence: Vec<String>,
    pub kind: String,
}

#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    pub ok_keys: usize,
    pub orphan_files: Vec<PathBuf>,
    pub tombstone_residue: Vec<PathBuf>,
    pub missing_content: Vec<String>,
    pub digest_mismatch: Vec<String>,
    pub repaired_index: bool,
}

impl VerifyReport {
    pub fn has_unrecoverable(&self) -> bool {
        !self.missing_content.is_empty() || !self.digest_mismatch.is_empty()
    }

    pub fn is_clean(&self) -> bool {
        self.orphan_files.is_empty()
            && self.tombstone_residue.is_empty()
            && self.missing_content.is_empty()
            && self.digest_mismatch.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
struct MemoryState {
    occasions: BTreeMap<String, MemoryOccasion>,
    objects: BTreeMap<String, MemoryObject>,
    observations: BTreeMap<String, MemoryObservation>,
    items: BTreeMap<String, MemoryItem>,
    free_key_items: BTreeMap<String, String>,
    max_seq: u64,
}

#[derive(Clone)]
pub struct AgentMemory {
    cfg: AgentMemoryConfig,
}

impl AgentMemory {
    pub fn open(cfg: AgentMemoryConfig) -> Result<Self> {
        let m = Self { cfg };
        m.ensure_initialized()?;
        Ok(m)
    }

    pub fn init(cfg: AgentMemoryConfig) -> Result<Self> {
        Self::open(cfg)
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    fn meta_dir(&self) -> PathBuf {
        self.cfg.root.join(META_DIR)
    }
    fn meta_json_path(&self) -> PathBuf {
        self.meta_dir().join(META_JSON)
    }
    fn log_path(&self) -> PathBuf {
        self.meta_dir().join(OCCASIONS_LOG_FILE)
    }
    fn state_path(&self) -> PathBuf {
        self.meta_dir().join(STATE_FILE)
    }
    fn lock_path(&self) -> PathBuf {
        self.meta_dir().join(LOCK_FILE)
    }
    fn sqlite_path(&self) -> PathBuf {
        self.cfg.root.join(SQLITE_FILE)
    }
    fn archive_dir(&self) -> PathBuf {
        self.meta_dir().join(ARCHIVE_DIR)
    }

    fn ensure_initialized(&self) -> Result<()> {
        fs::create_dir_all(&self.cfg.root)?;
        fs::create_dir_all(self.meta_dir())?;
        fs::create_dir_all(self.cfg.root.join(OCCASION_DIR))?;
        fs::create_dir_all(self.cfg.root.join(OBJECT_DIR))?;
        fs::create_dir_all(self.cfg.root.join(OBSERVATION_DIR))?;
        fs::create_dir_all(self.cfg.root.join(ITEM_DIR))?;
        fs::create_dir_all(self.cfg.root.join(INDEX_DIR))?;
        fs::create_dir_all(self.cfg.root.join(GRAPH_DIR).join("indexes"))?;

        let meta_path = self.meta_json_path();
        if !meta_path.exists() {
            let m = MetaJson::default_now();
            let s = serde_json::to_vec_pretty(&m)?;
            atomic_write(&meta_path, &s)?;
        } else {
            let bytes = fs::read(&meta_path)?;
            let m: MetaJson = serde_json::from_slice(&bytes)
                .map_err(|e| AgentMemoryError::Corrupted(format!("meta.json: {}", e)))?;
            if m.primary_language != PRIMARY_LANGUAGE {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported primary_language; v2.10 only supports en (got {})",
                    m.primary_language
                )));
            }
            if schema_major(&m.schema_version) != schema_major(SCHEMA_VERSION) {
                return Err(AgentMemoryError::Invalid(format!(
                    "incompatible schema_version major: {}",
                    m.schema_version
                )));
            }
            if m.schema_version != SCHEMA_VERSION {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported schema_version for writes: {}",
                    m.schema_version
                )));
            }
            if m.graph.time_model != "dual:occurred_at+noticed_at" {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported graph.time_model: {}",
                    m.graph.time_model
                )));
            }
            if m.compaction_strategy != "snapshot" {
                return Err(AgentMemoryError::Invalid(format!(
                    "unsupported compaction_strategy: {}",
                    m.compaction_strategy
                )));
            }
        }

        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .open(self.lock_path())?;

        self.with_writer_lock(|_| {
            let conn = self.open_db()?;
            ensure_schema(&conn)?;
            Ok(())
        })
    }

    pub fn add_occasion(&self, input: OccasionAddInput) -> Result<OccasionAddResult> {
        validate_occasion_type(&input.occasion_type)?;
        validate_summary(&input.summary)?;
        let tags = normalize_tags(&input.tags)?;
        let occurred_at = match input.occurred_at {
            Some(t) => validate_iso8601(&t)?,
            None => now_iso8601(),
        };

        self.with_writer_lock(|_| {
            let state = self.replay_state()?;
            let seq = state.max_seq + 1;
            let occasion_id = format!("occ_{seq:016}");
            let noticed_at = now_iso8601();
            let occasion = self.build_occasion(
                occasion_id.clone(),
                seq,
                "memory.write".to_string(),
                input.occasion_type,
                input.summary,
                occurred_at,
                noticed_at,
                input.source_ref,
                tags,
                Vec::new(),
            )?;
            self.append_occasion(&occasion)?;
            let mut new_state = state;
            apply_occasion_to_state(&occasion, &mut new_state)?;
            self.materialize_state(&new_state)?;
            Ok(OccasionAddResult { occasion_id, seq })
        })
    }

    pub fn set(&self, key: &str, content: &str, reason: &str) -> Result<()> {
        self.set_free(FlatSetOp {
            key: key.to_string(),
            content: content.to_string(),
            reason: reason.to_string(),
            entities: Vec::new(),
            tags: Vec::new(),
            weight: None,
            confidence: None,
        })
    }

    pub fn set_free(&self, mut op: FlatSetOp) -> Result<()> {
        op.key = normalize_key(&op.key)?;
        validate_content(&op.content)?;
        validate_reason(&op.reason)?;
        op.tags = normalize_tags(&op.tags)?;
        validate_probability(op.weight.unwrap_or(DEFAULT_FREE_WEIGHT), "weight")?;
        validate_probability(
            op.confidence.unwrap_or(DEFAULT_FREE_CONFIDENCE),
            "confidence",
        )?;

        self.with_writer_lock(|_| {
            let state = self.replay_state()?;
            let seq = state.max_seq + 1;
            let now = now_iso8601();
            let occasion_id = format!("occ_{seq:016}");
            let operations = vec![GraphOperation::SetFree(op)];
            let occasion = self.build_occasion(
                occasion_id,
                seq,
                "memory.write".to_string(),
                "memory.write".to_string(),
                "Set free memory hint.".to_string(),
                now.clone(),
                now,
                None,
                Vec::new(),
                operations,
            )?;
            self.commit_occasion_locked(state, occasion).map(|_| ())
        })
    }

    pub fn remove(&self, key: &str, reason: Option<&str>) -> Result<()> {
        let key = normalize_key(key)?;
        if let Some(r) = reason {
            validate_reason(r)?;
        }
        self.with_writer_lock(|_| {
            let state = self.replay_state()?;
            let seq = state.max_seq + 1;
            let now = now_iso8601();
            let occasion_id = format!("occ_{seq:016}");
            let operations = vec![GraphOperation::RemoveFree(FlatRemoveOp {
                key,
                reason: reason.map(|s| s.to_string()),
            })];
            let occasion = self.build_occasion(
                occasion_id,
                seq,
                "memory.write".to_string(),
                "memory.write".to_string(),
                "Remove free memory hint.".to_string(),
                now.clone(),
                now,
                None,
                Vec::new(),
                operations,
            )?;
            self.commit_occasion_locked(state, occasion).map(|_| ())
        })
    }

    pub fn observe(&self, occasion_id: &str, op: AddObservationOp) -> Result<String> {
        validate_id(occasion_id, "occ_", "occasion_id")?;
        validate_observation_op(&op)?;
        self.append_operation_to_occasion(occasion_id, GraphOperation::AddObservation(op))
    }

    pub fn upsert_object(&self, occasion_id: &str, op: UpsertObjectOp) -> Result<String> {
        validate_id(occasion_id, "occ_", "occasion_id")?;
        validate_object_op(&op)?;
        self.append_operation_to_occasion(occasion_id, GraphOperation::UpsertObject(op))
    }

    pub fn reinforce_object(
        &self,
        occasion_id: &str,
        op: ReinforceObjectWeightOp,
    ) -> Result<String> {
        validate_id(occasion_id, "occ_", "occasion_id")?;
        validate_id(&op.object_id, "obj_", "object_id")?;
        validate_reason(&op.reason)?;
        self.append_operation_to_occasion(occasion_id, GraphOperation::ReinforceObjectWeight(op))
    }

    pub fn relate(&self, occasion_id: &str, op: UpsertRelationOp) -> Result<String> {
        validate_id(occasion_id, "occ_", "occasion_id")?;
        validate_relation_op(&op)?;
        self.append_operation_to_occasion(occasion_id, GraphOperation::UpsertRelation(op))
    }

    pub fn set_status(&self, occasion_id: &str, op: SetStatusOp) -> Result<String> {
        validate_id(occasion_id, "occ_", "occasion_id")?;
        validate_status_op(&op)?;
        self.append_operation_to_occasion(occasion_id, GraphOperation::SetStatus(op))
    }

    pub fn commit(
        &self,
        occasion_type: &str,
        summary: &str,
        source_ref: Option<SourceRef>,
        tags: Vec<String>,
        operations: Vec<GraphOperation>,
    ) -> Result<OccasionAddResult> {
        validate_occasion_type(occasion_type)?;
        validate_summary(summary)?;
        let tags = normalize_tags(&tags)?;
        self.with_writer_lock(|_| {
            let state = self.replay_state()?;
            let seq = state.max_seq + 1;
            let now = now_iso8601();
            let occasion_id = format!("occ_{seq:016}");
            let occasion = self.build_occasion(
                occasion_id.clone(),
                seq,
                "memory.write".to_string(),
                occasion_type.to_string(),
                summary.to_string(),
                now.clone(),
                now,
                source_ref,
                tags,
                operations,
            )?;
            self.commit_occasion_locked(state, occasion)
                .map(|seq| OccasionAddResult { occasion_id, seq })
        })
    }

    fn append_operation_to_occasion(
        &self,
        occasion_id: &str,
        operation: GraphOperation,
    ) -> Result<String> {
        self.with_writer_lock(|_| {
            let state = self.replay_state()?;
            if !state.occasions.contains_key(occasion_id) {
                return Err(AgentMemoryError::NotFound(occasion_id.to_string()));
            }
            let seq = state.max_seq + 1;
            let now = now_iso8601();
            let wrapper_id = format!("occ_{seq:016}");
            let occasion = self.build_occasion(
                wrapper_id.clone(),
                seq,
                "memory.write".to_string(),
                "curator.action".to_string(),
                format!("Apply graph operation for {occasion_id}."),
                now.clone(),
                now,
                None,
                Vec::new(),
                vec![operation],
            )?;
            self.commit_occasion_locked(state, occasion)?;
            Ok(wrapper_id)
        })
    }

    pub fn get(&self, key: &str) -> Result<String> {
        let key = normalize_key(key)?;
        let state = self.replay_state()?;
        let item_id = state
            .free_key_items
            .get(&key)
            .ok_or_else(|| AgentMemoryError::NotFound(key.clone()))?;
        let item = state
            .items
            .get(item_id)
            .ok_or_else(|| AgentMemoryError::NotFound(key.clone()))?;
        if item.status != "active" {
            return Err(AgentMemoryError::NotFound(key));
        }
        Ok(item
            .claim
            .get("statement")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    pub fn get_item_json(&self, item_id: &str) -> Result<String> {
        validate_id(item_id, "item_", "item_id")?;
        let state = self.replay_state()?;
        let item = state
            .items
            .get(item_id)
            .ok_or_else(|| AgentMemoryError::NotFound(item_id.to_string()))?;
        Ok(serde_json::to_string_pretty(item)?)
    }

    pub fn get_object_json(&self, object_id: &str) -> Result<String> {
        validate_id(object_id, "obj_", "object_id")?;
        let state = self.replay_state()?;
        let object = state
            .objects
            .get(object_id)
            .ok_or_else(|| AgentMemoryError::NotFound(object_id.to_string()))?;
        Ok(serde_json::to_string_pretty(object)?)
    }

    pub fn get_observation_json(&self, observation_id: &str) -> Result<String> {
        validate_id(observation_id, "obs_", "observation_id")?;
        let state = self.replay_state()?;
        let observation = state
            .observations
            .get(observation_id)
            .ok_or_else(|| AgentMemoryError::NotFound(observation_id.to_string()))?;
        Ok(serde_json::to_string_pretty(observation)?)
    }

    pub fn list(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let state = self.replay_state()?;
        let mut keys: Vec<String> = state
            .free_key_items
            .iter()
            .filter_map(|(key, item_id)| {
                let item = state.items.get(item_id)?;
                if item.status != "active" {
                    return None;
                }
                if let Some(prefix) = prefix {
                    if key != prefix && !key.starts_with(prefix) {
                        return None;
                    }
                }
                Some(key.clone())
            })
            .collect();
        keys.sort();
        Ok(keys)
    }

    pub fn list_objects(&self, kind: Option<&str>) -> Result<Vec<String>> {
        let state = self.replay_state()?;
        let mut ids: Vec<String> = state
            .objects
            .values()
            .filter(|o| o.status == "active")
            .filter(|o| kind.map(|k| o.kind == k).unwrap_or(true))
            .map(|o| o.object_id.clone())
            .collect();
        ids.sort();
        Ok(ids)
    }

    pub fn load(&self, tags: &[String], opts: LoadOptions) -> Result<Vec<LoadItem>> {
        let normalized_tags = normalize_tags(tags)?;
        let state = self.replay_state()?;
        let mut objects: BTreeSet<String> = BTreeSet::new();
        for object in &opts.objects {
            validate_id(object, "obj_", "object_id")?;
            objects.insert(resolve_merged_object(&state, object));
        }

        for alias in &opts.aliases {
            let hits = resolve_alias(&state, alias);
            if hits.len() == 1 {
                objects.insert(hits[0].clone());
            }
        }

        let tag_empty = normalized_tags.is_empty();
        let object_empty = objects.is_empty();
        let now = opts.current_time.unwrap_or_else(Utc::now);
        let observation_text = observation_text_by_id(&state);
        let mut ranked = Vec::new();

        for item in state.items.values() {
            if item.status != "active" || item.kind == "salience" {
                continue;
            }
            if !evidence_is_active(&state, item) {
                continue;
            }

            let mut matched = Vec::new();
            let mut structural = 0.0;
            if !object_empty {
                let entity_hits: Vec<String> = item
                    .entities
                    .iter()
                    .filter(|entity| objects.contains(*entity))
                    .cloned()
                    .collect();
                for entity in &entity_hits {
                    matched.push(format!("entity:{entity}"));
                }
                if !entity_hits.is_empty() {
                    structural += 12.0;
                }
                if item.kind == "relation" && item.entities.len() >= 2 {
                    let subject = item.claim.get("subject").and_then(Value::as_str);
                    let object = item.claim.get("object").and_then(Value::as_str);
                    if let (Some(subject), Some(object)) = (subject, object) {
                        if objects.contains(subject) && objects.contains(object) {
                            structural += 18.0;
                            matched.push(format!("pair:{subject}:{object}"));
                        }
                    }
                }
            }

            let mut tag_boost = 0.0;
            if !tag_empty {
                let haystack = item_search_text(item, &observation_text);
                for (idx, tag) in normalized_tags.iter().enumerate() {
                    if phrase_hit(&haystack, tag) || item.tags.iter().any(|t| t == tag) {
                        tag_boost += match idx {
                            0 => 8.0,
                            1 => 4.0,
                            2 => 2.0,
                            _ => 1.0,
                        };
                        matched.push(format!("tag:{tag}"));
                    }
                }
            }

            if !tag_empty && !object_empty && matched.is_empty() {
                continue;
            }
            if !tag_empty && object_empty && tag_boost == 0.0 {
                continue;
            }
            if tag_empty && !object_empty && structural == 0.0 {
                continue;
            }

            let penalty = recency_penalty(&item.noticed_at, now);
            let score =
                structural + tag_boost + item.weight * 10.0 + item.confidence * 6.0 - penalty;
            ranked.push((score, item.weight, item.confidence, item.clone(), matched));
        }

        ranked.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.3.item_id.cmp(&b.3.item_id))
        });

        let mut out = Vec::new();
        let mut bytes_used = 0usize;
        for (_, _, _, item, matched) in ranked {
            if out.len() >= opts.max_records {
                break;
            }
            let summary = claim_summary(&item.claim);
            let (content, truncated) =
                truncate_at_char_boundary(&summary, opts.body_truncate_bytes);
            let size = content.len();
            if bytes_used.saturating_add(size) > opts.max_bytes && !out.is_empty() {
                break;
            }
            bytes_used = bytes_used.saturating_add(size);
            out.push(LoadItem {
                item_id: item.item_id,
                kind: item.kind,
                entities: item.entities,
                weight: item.weight,
                confidence: item.confidence,
                source_occasion: item.source_occasion,
                noticed_at: item.noticed_at,
                evidence: item.evidence,
                matched,
                size,
                truncated,
                content,
            });
        }
        Ok(out)
    }

    pub fn recall_hints(
        &self,
        tags: &[String],
        opts: MemoryRecallOptions,
    ) -> Result<Vec<MemoryHint>> {
        let mut candidates = self.load(
            tags,
            LoadOptions {
                max_records: memory_candidate_limit(&opts),
                max_bytes: DEFAULT_MAX_BYTES,
                body_truncate_bytes: opts.body_truncate_bytes,
                current_time: opts.current_time,
                ..LoadOptions::default()
            },
        )?;
        candidates.sort_by(|a, b| {
            memory_load_score(b)
                .partial_cmp(&memory_load_score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.item_id.cmp(&b.item_id))
        });

        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let class_order = [
            MemoryHintType::SessionRaw,
            MemoryHintType::Event,
            MemoryHintType::EntityObservation,
            MemoryHintType::EntityRelation,
        ];
        for hint_type in class_order {
            let budget = memory_budget_for(&opts, hint_type);
            append_memory_hints_for_type(
                &mut out,
                &mut seen,
                &candidates,
                hint_type,
                budget.candidate_limit,
                budget.keep_limit,
                opts.max_hints,
            );
        }

        let remaining = opts.max_hints.saturating_sub(out.len());
        if remaining > 0 {
            let budget = memory_budget_for(&opts, MemoryHintType::Free);
            append_memory_hints_for_type(
                &mut out,
                &mut seen,
                &candidates,
                MemoryHintType::Free,
                budget.candidate_limit,
                budget.keep_limit.min(remaining),
                opts.max_hints,
            );
        }
        Ok(out)
    }

    pub fn format_load_items(items: &[LoadItem]) -> String {
        let mut out = String::new();
        for item in items {
            out.push_str(&format!("ITEM {}\n", item.item_id));
            out.push_str(&format!("KIND {}\n", item.kind));
            out.push_str(&format!("ENTITIES {}\n", item.entities.join(",")));
            out.push_str(&format!("WEIGHT {:.3}\n", item.weight));
            out.push_str(&format!("CONFIDENCE {:.3}\n", item.confidence));
            out.push_str(&format!("SOURCE_OCCASION {}\n", item.source_occasion));
            out.push_str(&format!("NOTICED_AT {}\n", item.noticed_at));
            out.push_str(&format!("EVIDENCE {}\n", item.evidence.join(",")));
            out.push_str(&format!("MATCHED {}\n", item.matched.join(",")));
            out.push_str(&format!("SIZE {}\n", item.size));
            out.push_str(&format!(
                "TRUNCATED {}\n",
                if item.truncated { 1 } else { 0 }
            ));
            out.push_str("---\n");
            out.push_str(&item.content);
            if !item.content.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("END\n");
        }
        out
    }

    pub fn verify(&self, repair: bool) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        let state = self.replay_state()?;
        report.ok_keys = state.occasions.len()
            + state.objects.len()
            + state.observations.len()
            + state.items.len();

        for occasion in state.occasions.values() {
            if occasion.digest != occasion_digest(occasion)? {
                report.digest_mismatch.push(occasion.occasion_id.clone());
            }
            let path = self.canonical_path(OCCASION_DIR, &occasion.occasion_id);
            if !path.exists() {
                report
                    .missing_content
                    .push(format!("/occasion/{}", occasion.occasion_id));
            }
        }
        for object in state.objects.values() {
            if !state.occasions.contains_key(&object.source_occasion) {
                report.missing_content.push(format!(
                    "object {} source_occasion {}",
                    object.object_id, object.source_occasion
                ));
            }
        }
        for observation in state.observations.values() {
            if !state.occasions.contains_key(&observation.source_occasion) {
                report.missing_content.push(format!(
                    "observation {} source_occasion {}",
                    observation.observation_id, observation.source_occasion
                ));
            }
        }
        for item in state.items.values() {
            if !state.occasions.contains_key(&item.source_occasion) {
                report.missing_content.push(format!(
                    "item {} source_occasion {}",
                    item.item_id, item.source_occasion
                ));
            }
            for obs in &item.evidence {
                if !state.observations.contains_key(obs) {
                    report
                        .missing_content
                        .push(format!("item {} evidence {}", item.item_id, obs));
                }
            }
        }

        let mut expected = HashSet::new();
        for occasion in state.occasions.values() {
            expected.insert(self.canonical_path(OCCASION_DIR, &occasion.occasion_id));
        }
        for object in state.objects.values() {
            expected.insert(self.canonical_path(OBJECT_DIR, &object.object_id));
        }
        for observation in state.observations.values() {
            expected.insert(self.canonical_path(OBSERVATION_DIR, &observation.observation_id));
        }
        for item in state.items.values() {
            expected.insert(self.canonical_path(ITEM_DIR, &item.item_id));
        }
        let mut disk = Vec::new();
        walk_canonical_files(&self.cfg.root, &mut disk)?;
        for path in disk {
            if !expected.contains(&path) {
                report.orphan_files.push(path);
            }
        }

        if repair {
            self.with_writer_lock(|_| {
                self.materialize_state(&state)?;
                Ok(())
            })?;
            report.repaired_index = true;
        }
        if report.has_unrecoverable() && !repair {
            return Err(AgentMemoryError::Corrupted(format!(
                "{} unrecoverable issue(s)",
                report.missing_content.len() + report.digest_mismatch.len()
            )));
        }
        Ok(report)
    }

    pub fn compact(&self) -> Result<()> {
        self.with_writer_lock(|_| {
            let state = self.replay_state()?;
            self.materialize_state(&state)?;

            let mut state_buf = Vec::new();
            for object in state.objects.values() {
                serde_json::to_writer(&mut state_buf, &json!({"type": "object", "value": object}))?;
                state_buf.push(b'\n');
            }
            for observation in state.observations.values() {
                serde_json::to_writer(
                    &mut state_buf,
                    &json!({"type": "observation", "value": observation}),
                )?;
                state_buf.push(b'\n');
            }
            for item in state.items.values() {
                serde_json::to_writer(&mut state_buf, &json!({"type": "item", "value": item}))?;
                state_buf.push(b'\n');
            }
            atomic_write(&self.state_path(), &state_buf)?;

            fs::create_dir_all(self.archive_dir())?;
            let archive_path = self.archive_dir().join(format!(
                "occasions_{}.jsonl",
                Utc::now().format("%Y%m%dT%H%M%SZ")
            ));
            if self.log_path().exists() && fs::metadata(self.log_path())?.len() > 0 {
                fs::rename(self.log_path(), archive_path)?;
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log_path())?;
            Ok(())
        })
    }

    fn build_occasion(
        &self,
        occasion_id: String,
        seq: u64,
        actor_session_id: String,
        occasion_type: String,
        summary: String,
        occurred_at: String,
        noticed_at: String,
        source_ref: Option<SourceRef>,
        tags: Vec<String>,
        operations: Vec<GraphOperation>,
    ) -> Result<MemoryOccasion> {
        let mut occasion = MemoryOccasion {
            schema_version: SCHEMA_VERSION.to_string(),
            occasion_id,
            seq,
            occurred_at,
            noticed_at,
            occasion_type,
            actor_session_id: Some(actor_session_id),
            source_ref,
            summary,
            tags,
            operations,
            digest: String::new(),
        };
        occasion.digest = occasion_digest(&occasion)?;
        Ok(occasion)
    }

    fn commit_occasion_locked(&self, state: MemoryState, occasion: MemoryOccasion) -> Result<u64> {
        let mut new_state = state;
        apply_occasion_to_state(&occasion, &mut new_state)?;
        self.append_occasion(&occasion)?;
        self.materialize_state(&new_state)?;
        Ok(occasion.seq)
    }

    fn append_occasion(&self, occasion: &MemoryOccasion) -> Result<()> {
        let mut line = serde_json::to_vec(occasion)?;
        line.push(b'\n');
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        f.write_all(&line)?;
        f.sync_all()?;
        Ok(())
    }

    fn replay_state(&self) -> Result<MemoryState> {
        let mut occasions = Vec::new();
        for path in self.occasion_log_paths()? {
            read_occasions_jsonl(&path, &mut occasions)?;
        }
        occasions.sort_by(|a, b| a.seq.cmp(&b.seq));
        let mut seen_seq = HashSet::new();
        let mut state = MemoryState::default();
        for occasion in occasions {
            if !seen_seq.insert(occasion.seq) {
                return Err(AgentMemoryError::Corrupted(format!(
                    "duplicate occasion seq {}",
                    occasion.seq
                )));
            }
            if occasion.digest != occasion_digest(&occasion)? {
                return Err(AgentMemoryError::Corrupted(format!(
                    "occasion digest mismatch: {}",
                    occasion.occasion_id
                )));
            }
            apply_occasion_to_state(&occasion, &mut state)?;
        }
        Ok(state)
    }

    fn occasion_log_paths(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        let archive = self.archive_dir();
        if archive.exists() {
            let mut archived = Vec::new();
            for entry in fs::read_dir(archive)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    archived.push(path);
                }
            }
            archived.sort();
            paths.extend(archived);
        }
        if self.log_path().exists() {
            paths.push(self.log_path());
        }
        Ok(paths)
    }

    fn materialize_state(&self, state: &MemoryState) -> Result<()> {
        for occasion in state.occasions.values() {
            self.write_canonical(OCCASION_DIR, &occasion.occasion_id, occasion)?;
        }
        for object in state.objects.values() {
            self.write_canonical(OBJECT_DIR, &object.object_id, object)?;
        }
        for observation in state.observations.values() {
            self.write_canonical(OBSERVATION_DIR, &observation.observation_id, observation)?;
        }
        for item in state.items.values() {
            self.write_canonical(ITEM_DIR, &item.item_id, item)?;
        }
        self.write_graph_snapshots(state)?;
        self.rebuild_path_indexes(state)?;
        self.rebuild_sqlite(state)?;
        Ok(())
    }

    fn write_graph_snapshots(&self, state: &MemoryState) -> Result<()> {
        let graph = self.cfg.root.join(GRAPH_DIR);
        fs::create_dir_all(&graph)?;
        write_jsonl(graph.join("objects.jsonl"), state.objects.values())?;
        write_jsonl(
            graph.join("observations.jsonl"),
            state.observations.values(),
        )?;
        write_jsonl(graph.join("items.jsonl"), state.items.values())?;
        Ok(())
    }

    fn rebuild_path_indexes(&self, state: &MemoryState) -> Result<()> {
        let index_root = self.cfg.root.join(INDEX_DIR);
        let _ = fs::remove_dir_all(&index_root);
        fs::create_dir_all(&index_root)?;

        for object in state.objects.values() {
            touch_index(
                &index_root
                    .join("by_kind")
                    .join(&object.kind)
                    .join(&object.object_id),
            )?;
            for alias in &object.aliases {
                if alias.status == "active" {
                    touch_index(
                        &index_root
                            .join("by_alias")
                            .join(percent_segment(&normalize_alias(&alias.alias)))
                            .join(&object.object_id),
                    )?;
                }
            }
        }
        for observation in state.observations.values() {
            if observation.status != "active" {
                continue;
            }
            for entity in &observation.entities {
                touch_index(
                    &index_root
                        .join("by_entity")
                        .join(entity)
                        .join("obs")
                        .join(&observation.observation_id),
                )?;
            }
        }
        for item in state.items.values() {
            if item.status != "active" || item.kind == "salience" {
                continue;
            }
            for entity in &item.entities {
                touch_index(
                    &index_root
                        .join("by_entity")
                        .join(entity)
                        .join("item")
                        .join(&item.item_id),
                )?;
            }
            touch_index(
                &index_root
                    .join("by_kind")
                    .join(&item.kind)
                    .join(&item.item_id),
            )?;
            if item.kind == "relation" {
                let subject = item
                    .claim
                    .get("subject")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let predicate = item
                    .claim
                    .get("predicate")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let object = item
                    .claim
                    .get("object")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !subject.is_empty() && !object.is_empty() {
                    let pair = normalized_pair(subject, object);
                    touch_index(&index_root.join("by_pair").join(pair).join(&item.item_id))?;
                }
                if !predicate.is_empty() {
                    touch_index(
                        &index_root
                            .join("by_predicate")
                            .join(predicate)
                            .join(&item.item_id),
                    )?;
                }
                if !subject.is_empty() && !predicate.is_empty() && !object.is_empty() {
                    touch_index(
                        &index_root
                            .join("by_relation")
                            .join(subject)
                            .join(predicate)
                            .join(object)
                            .join(&item.item_id),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn rebuild_sqlite(&self, state: &MemoryState) -> Result<()> {
        let _ = fs::remove_file(self.sqlite_path());
        let conn = self.open_db()?;
        ensure_schema(&conn)?;
        let tx = conn.unchecked_transaction()?;

        for object in state.objects.values() {
            tx.execute(
                "INSERT INTO objects(object_id, kind, canonical_name, weight, confidence, status, source_occasion, last_occasion, noticed_at, merged_into)
                 VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    object.object_id,
                    object.kind,
                    object.canonical_name,
                    object.weight,
                    object.confidence,
                    object.status,
                    object.source_occasion,
                    object.last_occasion,
                    object.noticed_at,
                    object.merged_into,
                ],
            )?;
            for alias in &object.aliases {
                tx.execute(
                    "INSERT OR REPLACE INTO aliases(alias_norm, object_id, alias_type, confidence, status, source_occasion)
                     VALUES(?, ?, ?, ?, ?, ?)",
                    params![
                        normalize_alias(&alias.alias),
                        object.object_id,
                        alias.alias_type,
                        alias.confidence,
                        alias.status,
                        alias.source_occasion,
                    ],
                )?;
            }
        }

        for observation in state.observations.values() {
            tx.execute(
                "INSERT INTO observations(observation_id, kind, source_occasion, content, confidence, noticed_at, status)
                 VALUES(?, ?, ?, ?, ?, ?, ?)",
                params![
                    observation.observation_id,
                    observation.kind,
                    observation.source_occasion,
                    observation.content,
                    observation.confidence,
                    observation.noticed_at,
                    observation.status,
                ],
            )?;
        }

        let observation_text = observation_text_by_id(state);
        for item in state.items.values() {
            tx.execute(
                "INSERT INTO items(item_id, kind, claim_json, weight, confidence, source_occasion, noticed_at, status)
                 VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    item.item_id,
                    item.kind,
                    serde_json::to_string(&item.claim)?,
                    item.weight,
                    item.confidence,
                    item.source_occasion,
                    item.noticed_at,
                    item.status,
                ],
            )?;
            for entity in &item.entities {
                tx.execute(
                    "INSERT OR IGNORE INTO item_entities(item_id, object_id) VALUES(?, ?)",
                    params![item.item_id, entity],
                )?;
            }
            for evidence in &item.evidence {
                tx.execute(
                    "INSERT OR IGNORE INTO item_evidence(item_id, observation_id) VALUES(?, ?)",
                    params![item.item_id, evidence],
                )?;
            }
            if item.kind == "relation" {
                tx.execute(
                    "INSERT OR REPLACE INTO relations(item_id, subject, predicate, object) VALUES(?, ?, ?, ?)",
                    params![
                        item.item_id,
                        item.claim.get("subject").and_then(Value::as_str).unwrap_or(""),
                        item.claim.get("predicate").and_then(Value::as_str).unwrap_or(""),
                        item.claim.get("object").and_then(Value::as_str).unwrap_or(""),
                    ],
                )?;
            }
            if item.status == "active" && item.kind != "salience" {
                tx.execute(
                    "INSERT INTO memory_fts(ref_id, ref_type, object_text, predicate_text, claim_text, observation_text)
                     VALUES(?, 'item', ?, ?, ?, ?)",
                    params![
                        item.item_id,
                        item.entities.join(" "),
                        item.claim.get("predicate").and_then(Value::as_str).unwrap_or(""),
                        claim_summary(&item.claim),
                        item.evidence
                            .iter()
                            .filter_map(|id| observation_text.get(id))
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" "),
                    ],
                )?;
            }
        }
        for observation in state.observations.values() {
            if observation.status == "active" {
                tx.execute(
                    "INSERT INTO memory_fts(ref_id, ref_type, object_text, predicate_text, claim_text, observation_text)
                     VALUES(?, 'observation', ?, '', '', ?)",
                    params![
                        observation.observation_id,
                        observation.entities.join(" "),
                        observation.content,
                    ],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    fn canonical_path(&self, dir: &str, id: &str) -> PathBuf {
        self.cfg.root.join(dir).join(percent_segment(id))
    }

    fn write_canonical<T: Serialize>(&self, dir: &str, id: &str, value: &T) -> Result<()> {
        let path = self.canonical_path(dir, id);
        let bytes = serde_json::to_vec_pretty(value)?;
        atomic_write(&path, &bytes)
    }

    fn open_db(&self) -> Result<Connection> {
        let conn = Connection::open_with_flags(
            self.sqlite_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(conn)
    }

    fn with_writer_lock<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&File) -> Result<T>,
    {
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.lock_path())?;
        let deadline = Instant::now() + self.cfg.lock_timeout;
        loop {
            match lock_file.try_lock_exclusive() {
                Ok(()) => break,
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(AgentMemoryError::LockTimeout(format!(
                            "could not acquire {}: {}",
                            self.lock_path().display(),
                            e
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let result = f(&lock_file);
        let _ = FileExt::unlock(&lock_file);
        result
    }
}

pub type Envelope = MemoryOccasion;

fn apply_occasion_to_state(occasion: &MemoryOccasion, state: &mut MemoryState) -> Result<()> {
    state.max_seq = state.max_seq.max(occasion.seq);
    state
        .occasions
        .insert(occasion.occasion_id.clone(), occasion.clone());

    for (idx, op) in occasion.operations.iter().enumerate() {
        match op {
            GraphOperation::SetFree(op) => apply_set_free(occasion, state, idx, op)?,
            GraphOperation::RemoveFree(op) => apply_remove_free(occasion, state, op)?,
            GraphOperation::AddObservation(op) => apply_add_observation(occasion, state, idx, op)?,
            GraphOperation::UpsertObject(op) => apply_upsert_object(occasion, state, idx, op)?,
            GraphOperation::ReinforceObjectWeight(op) => {
                apply_reinforce_object(occasion, state, idx, op)?
            }
            GraphOperation::UpsertRelation(op) => apply_relation(occasion, state, idx, op)?,
            GraphOperation::SetStatus(op) => apply_set_status(occasion, state, op)?,
            GraphOperation::PutItem(op) => apply_put_item(occasion, state, idx, op)?,
        }
    }
    Ok(())
}

fn apply_set_free(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    idx: usize,
    op: &FlatSetOp,
) -> Result<()> {
    let key = normalize_key(&op.key)?;
    let previous = state.free_key_items.get(&key).cloned();
    if let Some(prev) = &previous {
        if let Some(item) = state.items.get_mut(prev) {
            item.status = "superseded".to_string();
            item.noticed_at = occasion.noticed_at.clone();
        }
    }
    let item_id = format!("item_{:016}_{idx:02}", occasion.seq);
    let item = MemoryItem {
        item_id: item_id.clone(),
        kind: "free".to_string(),
        entities: op.entities.clone(),
        claim: json!({
            "type": "free",
            "key": key,
            "statement": op.content,
        }),
        weight: clamp01(op.weight.unwrap_or(DEFAULT_FREE_WEIGHT)),
        confidence: clamp01(op.confidence.unwrap_or(DEFAULT_FREE_CONFIDENCE)),
        evidence: Vec::new(),
        source_occasion: occasion.occasion_id.clone(),
        noticed_at: occasion.noticed_at.clone(),
        write_reason: op.reason.clone(),
        status: "active".to_string(),
        replaces: previous.into_iter().collect(),
        free_key: Some(key.clone()),
        tags: normalize_tags(&op.tags)?,
    };
    state.free_key_items.insert(key, item_id.clone());
    state.items.insert(item_id, item);
    Ok(())
}

fn apply_remove_free(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    op: &FlatRemoveOp,
) -> Result<()> {
    let key = normalize_key(&op.key)?;
    if let Some(item_id) = state.free_key_items.remove(&key) {
        if let Some(item) = state.items.get_mut(&item_id) {
            item.status = "deleted".to_string();
            item.noticed_at = occasion.noticed_at.clone();
        }
    }
    Ok(())
}

fn apply_add_observation(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    idx: usize,
    op: &AddObservationOp,
) -> Result<()> {
    let observation_id = op
        .observation_id
        .clone()
        .unwrap_or_else(|| format!("obs_{:016}_{idx:02}", occasion.seq));
    validate_id(&observation_id, "obs_", "observation_id")?;
    let observation = MemoryObservation {
        observation_id: observation_id.clone(),
        kind: op.kind.clone(),
        source_occasion: occasion.occasion_id.clone(),
        entities: op.entities.clone(),
        content: op.content.clone(),
        source_excerpt: op.source_excerpt.clone(),
        source_ref: op.source_ref.clone(),
        confidence: clamp01(op.confidence),
        noticed_at: occasion.noticed_at.clone(),
        status: "active".to_string(),
    };
    state.observations.insert(observation_id, observation);
    Ok(())
}

fn apply_upsert_object(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    idx: usize,
    op: &UpsertObjectOp,
) -> Result<()> {
    let object_id = op
        .object_id
        .clone()
        .unwrap_or_else(|| format!("obj_{:016}_{idx:02}", occasion.seq));
    validate_id(&object_id, "obj_", "object_id")?;

    if let Some(merge_into) = &op.merge_into {
        if let Some(object) = state.objects.get_mut(&object_id) {
            object.status = "merged".to_string();
            object.merged_into = Some(merge_into.clone());
            object.last_occasion = occasion.occasion_id.clone();
            object.noticed_at = occasion.noticed_at.clone();
            for alias in &mut object.aliases {
                alias.status = "merged".to_string();
                alias.noticed_at = occasion.noticed_at.clone();
            }
        }
        return Ok(());
    }

    for alias in &op.aliases {
        let alias_norm = normalize_alias(&alias.alias);
        let conflict = state.objects.iter().any(|(other_id, other)| {
            other_id != &object_id
                && other.status == "active"
                && other
                    .aliases
                    .iter()
                    .any(|a| a.status == "active" && normalize_alias(&a.alias) == alias_norm)
        });
        if conflict {
            return Err(AgentMemoryError::Invalid(format!(
                "alias `{}` already points to another active object",
                alias.alias
            )));
        }
    }

    let object = state
        .objects
        .entry(object_id.clone())
        .or_insert_with(|| MemoryObject {
            object_id: object_id.clone(),
            kind: op.kind.clone(),
            canonical_name: op.canonical_name.clone(),
            aliases: Vec::new(),
            weight: clamp01(op.weight.unwrap_or(DEFAULT_FREE_WEIGHT)),
            confidence: clamp01(op.confidence),
            evidence: op.evidence.clone(),
            source_occasion: occasion.occasion_id.clone(),
            last_occasion: occasion.occasion_id.clone(),
            noticed_at: occasion.noticed_at.clone(),
            status: "active".to_string(),
            merged_into: None,
        });

    object.kind = op.kind.clone();
    object.canonical_name = op.canonical_name.clone();
    object.weight = clamp01(op.weight.unwrap_or(object.weight));
    object.confidence = object.confidence.max(clamp01(op.confidence));
    object.last_occasion = occasion.occasion_id.clone();
    object.noticed_at = occasion.noticed_at.clone();
    merge_unique(&mut object.evidence, &op.evidence);

    for alias in &op.aliases {
        let alias_norm = normalize_alias(&alias.alias);
        if let Some(existing) = object
            .aliases
            .iter_mut()
            .find(|a| normalize_alias(&a.alias) == alias_norm)
        {
            existing.confidence = existing.confidence.max(clamp01(alias.confidence));
            existing.noticed_at = occasion.noticed_at.clone();
            merge_unique(&mut existing.evidence, &op.evidence);
        } else {
            object.aliases.push(ObjectAlias {
                alias: alias.alias.clone(),
                alias_type: alias.alias_type.clone(),
                confidence: clamp01(alias.confidence),
                evidence: op.evidence.clone(),
                source_occasion: occasion.occasion_id.clone(),
                noticed_at: occasion.noticed_at.clone(),
                status: "active".to_string(),
            });
        }
    }
    Ok(())
}

fn apply_reinforce_object(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    idx: usize,
    op: &ReinforceObjectWeightOp,
) -> Result<()> {
    let object = state
        .objects
        .get_mut(&op.object_id)
        .ok_or_else(|| AgentMemoryError::NotFound(op.object_id.clone()))?;
    object.weight = clamp01(object.weight + op.delta);
    object.last_occasion = occasion.occasion_id.clone();
    object.noticed_at = occasion.noticed_at.clone();
    let item_id = format!("item_{:016}_{idx:02}", occasion.seq);
    state.items.insert(
        item_id.clone(),
        MemoryItem {
            item_id,
            kind: "salience".to_string(),
            entities: vec![op.object_id.clone()],
            claim: json!({
                "type": "salience",
                "object_id": op.object_id,
                "reason": op.reason,
                "delta": op.delta,
            }),
            weight: object.weight,
            confidence: object.confidence,
            evidence: op.evidence.clone(),
            source_occasion: occasion.occasion_id.clone(),
            noticed_at: occasion.noticed_at.clone(),
            write_reason: op.reason.clone(),
            status: "active".to_string(),
            replaces: Vec::new(),
            free_key: None,
            tags: Vec::new(),
        },
    );
    Ok(())
}

fn apply_relation(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    idx: usize,
    op: &UpsertRelationOp,
) -> Result<()> {
    if !state.objects.contains_key(&op.subject) {
        return Err(AgentMemoryError::NotFound(op.subject.clone()));
    }
    if !state.objects.contains_key(&op.object) {
        return Err(AgentMemoryError::NotFound(op.object.clone()));
    }
    for replaced in &op.replaces {
        if let Some(item) = state.items.get_mut(replaced) {
            item.status = "superseded".to_string();
            item.noticed_at = occasion.noticed_at.clone();
        }
    }
    let item_id = format!("item_{:016}_{idx:02}", occasion.seq);
    state.items.insert(
        item_id.clone(),
        MemoryItem {
            item_id,
            kind: "relation".to_string(),
            entities: vec![op.subject.clone(), op.object.clone()],
            claim: json!({
                "type": "relation",
                "subject": op.subject,
                "predicate": op.predicate,
                "object": op.object,
            }),
            weight: clamp01(op.weight),
            confidence: clamp01(op.confidence),
            evidence: op.evidence.clone(),
            source_occasion: occasion.occasion_id.clone(),
            noticed_at: occasion.noticed_at.clone(),
            write_reason: op.write_reason.clone(),
            status: "active".to_string(),
            replaces: op.replaces.clone(),
            free_key: None,
            tags: Vec::new(),
        },
    );
    Ok(())
}

fn apply_put_item(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    idx: usize,
    op: &PutItemOp,
) -> Result<()> {
    for replaced in &op.replaces {
        if let Some(item) = state.items.get_mut(replaced) {
            item.status = "superseded".to_string();
            item.noticed_at = occasion.noticed_at.clone();
        }
    }
    let item_id = op
        .item_id
        .clone()
        .unwrap_or_else(|| format!("item_{:016}_{idx:02}", occasion.seq));
    state.items.insert(
        item_id.clone(),
        MemoryItem {
            item_id,
            kind: op.kind.clone(),
            entities: op.entities.clone(),
            claim: op.claim.clone(),
            weight: clamp01(op.weight),
            confidence: clamp01(op.confidence),
            evidence: op.evidence.clone(),
            source_occasion: occasion.occasion_id.clone(),
            noticed_at: occasion.noticed_at.clone(),
            write_reason: op.write_reason.clone(),
            status: "active".to_string(),
            replaces: op.replaces.clone(),
            free_key: None,
            tags: Vec::new(),
        },
    );
    Ok(())
}

fn apply_set_status(
    occasion: &MemoryOccasion,
    state: &mut MemoryState,
    op: &SetStatusOp,
) -> Result<()> {
    match op.target_kind.as_str() {
        "item" => {
            let item = state
                .items
                .get_mut(&op.target_id)
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            item.status = op.status.clone();
            item.noticed_at = occasion.noticed_at.clone();
            if item.free_key.is_some() && item.status != "active" {
                if let Some(key) = &item.free_key {
                    state.free_key_items.remove(key);
                }
            }
        }
        "object" => {
            let object = state
                .objects
                .get_mut(&op.target_id)
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            object.status = op.status.clone();
            object.last_occasion = occasion.occasion_id.clone();
            object.noticed_at = occasion.noticed_at.clone();
            if op.status == "merged" {
                object.merged_into = op.replaced_by.clone();
            }
        }
        "observation" => {
            let observation = state
                .observations
                .get_mut(&op.target_id)
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            observation.status = op.status.clone();
            observation.noticed_at = occasion.noticed_at.clone();
        }
        "alias" => {
            let (object_id, alias_norm) = op.target_id.split_once(':').ok_or_else(|| {
                AgentMemoryError::Invalid("alias target must be <object_id>:<alias>".into())
            })?;
            let object = state
                .objects
                .get_mut(object_id)
                .ok_or_else(|| AgentMemoryError::NotFound(object_id.to_string()))?;
            let alias = object
                .aliases
                .iter_mut()
                .find(|a| normalize_alias(&a.alias) == normalize_alias(alias_norm))
                .ok_or_else(|| AgentMemoryError::NotFound(op.target_id.clone()))?;
            alias.status = op.status.clone();
            alias.noticed_at = occasion.noticed_at.clone();
        }
        _ => {
            return Err(AgentMemoryError::Invalid(format!(
                "unsupported target_kind {}",
                op.target_kind
            )))
        }
    }
    Ok(())
}

fn schema_major(v: &str) -> &str {
    v.split('.').next().unwrap_or("")
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn random_suffix() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{pid}-{nanos}")
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn occasion_digest(occasion: &MemoryOccasion) -> Result<String> {
    let mut clone = occasion.clone();
    clone.digest.clear();
    let bytes = serde_json::to_vec(&clone)?;
    Ok(format!("blake3:{}", blake3_hex(&bytes)))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        AgentMemoryError::Invalid(format!("path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        "{}.tmp.{}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "memory".to_string()),
        random_suffix()
    ));
    {
        let mut f = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> Result<()> {
    let f = File::open(path)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> Result<()> {
    Ok(())
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS objects (
           object_id        TEXT PRIMARY KEY,
           kind             TEXT NOT NULL,
           canonical_name   TEXT NOT NULL,
           weight           REAL NOT NULL,
           confidence       REAL NOT NULL,
           status           TEXT NOT NULL,
           source_occasion  TEXT NOT NULL,
           last_occasion    TEXT NOT NULL,
           noticed_at       TEXT NOT NULL,
           merged_into      TEXT
         );
         CREATE TABLE IF NOT EXISTS aliases (
           alias_norm       TEXT NOT NULL,
           object_id        TEXT NOT NULL,
           alias_type       TEXT NOT NULL,
           confidence       REAL NOT NULL,
           status           TEXT NOT NULL,
           source_occasion  TEXT NOT NULL,
           PRIMARY KEY(alias_norm, object_id)
         );
         CREATE TABLE IF NOT EXISTS observations (
           observation_id   TEXT PRIMARY KEY,
           kind             TEXT NOT NULL,
           source_occasion  TEXT NOT NULL,
           content          TEXT NOT NULL,
           confidence       REAL NOT NULL,
           noticed_at       TEXT NOT NULL,
           status           TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS items (
           item_id          TEXT PRIMARY KEY,
           kind             TEXT NOT NULL,
           claim_json       TEXT NOT NULL,
           weight           REAL NOT NULL,
           confidence       REAL NOT NULL,
           source_occasion  TEXT NOT NULL,
           noticed_at       TEXT NOT NULL,
           status           TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS item_entities (
           item_id          TEXT NOT NULL,
           object_id        TEXT NOT NULL,
           PRIMARY KEY(item_id, object_id)
         );
         CREATE TABLE IF NOT EXISTS item_evidence (
           item_id          TEXT NOT NULL,
           observation_id   TEXT NOT NULL,
           PRIMARY KEY(item_id, observation_id)
         );
         CREATE TABLE IF NOT EXISTS relations (
           item_id          TEXT PRIMARY KEY,
           subject          TEXT NOT NULL,
           predicate        TEXT NOT NULL,
           object           TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_objects_kind ON objects(kind);
         CREATE INDEX IF NOT EXISTS idx_objects_status ON objects(status);
         CREATE INDEX IF NOT EXISTS idx_aliases_alias ON aliases(alias_norm);
         CREATE INDEX IF NOT EXISTS idx_items_kind ON items(kind);
         CREATE INDEX IF NOT EXISTS idx_items_status ON items(status);
         CREATE INDEX IF NOT EXISTS idx_item_entities_object ON item_entities(object_id);
         CREATE INDEX IF NOT EXISTS idx_relations_pair ON relations(subject, object);
         CREATE INDEX IF NOT EXISTS idx_relations_predicate ON relations(predicate);
         CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
           ref_id UNINDEXED,
           ref_type UNINDEXED,
           object_text,
           predicate_text,
           claim_text,
           observation_text,
           tokenize = 'unicode61 remove_diacritics 2'
         );",
    )?;
    Ok(())
}

fn read_occasions_jsonl(path: &Path, out: &mut Vec<MemoryOccasion>) -> Result<()> {
    let f = File::open(path)?;
    let reader = BufReader::new(f);
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let occasion: MemoryOccasion = serde_json::from_str(trimmed).map_err(|e| {
            AgentMemoryError::Corrupted(format!("{}:{}: {}", path.display(), idx + 1, e))
        })?;
        out.push(occasion);
    }
    Ok(())
}

fn write_jsonl<'a, T: Serialize + 'a>(
    path: PathBuf,
    values: impl Iterator<Item = &'a T>,
) -> Result<()> {
    let mut buf = Vec::new();
    for value in values {
        serde_json::to_writer(&mut buf, value)?;
        buf.push(b'\n');
    }
    atomic_write(&path, &buf)
}

fn walk_canonical_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for dir in [OCCASION_DIR, OBJECT_DIR, OBSERVATION_DIR, ITEM_DIR] {
        let p = root.join(dir);
        if !p.exists() {
            continue;
        }
        for entry in fs::read_dir(p)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                out.push(entry.path());
            }
        }
    }
    Ok(())
}

fn normalize_key(raw: &str) -> Result<String> {
    if raw.is_empty() {
        return Err(AgentMemoryError::Invalid("key is empty".into()));
    }
    if !raw.starts_with('/') {
        return Err(AgentMemoryError::Invalid(format!(
            "key must start with '/': {raw}"
        )));
    }
    if raw.contains('\0') || raw.contains('\n') || raw.contains('\r') {
        return Err(AgentMemoryError::Invalid(
            "key contains NUL or newline".into(),
        ));
    }
    if raw.chars().any(|c| c.is_control()) {
        return Err(AgentMemoryError::Invalid(
            "key contains control characters".into(),
        ));
    }

    let mut segs = Vec::new();
    for seg in raw.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg == "." || seg == ".." {
            return Err(AgentMemoryError::Invalid(format!(
                "key has invalid segment '{seg}'"
            )));
        }
        if seg.as_bytes().len() > MAX_SEGMENT_BYTES {
            return Err(AgentMemoryError::Invalid(format!(
                "key segment exceeds {MAX_SEGMENT_BYTES} bytes: {seg}"
            )));
        }
        for c in Path::new(seg).components() {
            match c {
                Component::Normal(_) => {}
                _ => {
                    return Err(AgentMemoryError::Invalid(format!(
                        "key segment is not a normal path component: {seg}"
                    )))
                }
            }
        }
        segs.push(seg.to_string());
    }
    if segs.is_empty() {
        return Err(AgentMemoryError::Invalid("key has no segments".into()));
    }
    if FORBIDDEN_FIRST_SEGMENTS.contains(&segs[0].as_str()) {
        return Err(AgentMemoryError::Invalid(format!(
            "key first segment must not be reserved: {}",
            segs[0]
        )));
    }
    Ok(format!("/{}", segs.join("/")))
}

fn validate_content(content: &str) -> Result<()> {
    if content.is_empty() {
        return Err(AgentMemoryError::Invalid("content is empty".into()));
    }
    if content.starts_with('\u{feff}') {
        return Err(AgentMemoryError::Invalid("content has UTF-8 BOM".into()));
    }
    if content.len() > SOFT_CONTENT_WARN_BYTES {
        log::warn!(
            "agent_memory: free item content is {} bytes, above soft warning threshold {}",
            content.len(),
            SOFT_CONTENT_WARN_BYTES
        );
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        return Err(AgentMemoryError::Invalid("reason is empty".into()));
    }
    if reason.contains('\0') {
        return Err(AgentMemoryError::Invalid("reason contains NUL".into()));
    }
    Ok(())
}

fn validate_summary(summary: &str) -> Result<()> {
    if summary.trim().is_empty() {
        return Err(AgentMemoryError::Invalid("summary is empty".into()));
    }
    if summary.chars().count() > 500 {
        return Err(AgentMemoryError::Invalid(
            "summary exceeds 500 characters".into(),
        ));
    }
    Ok(())
}

fn validate_occasion_type(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(AgentMemoryError::Invalid("invalid occasion_type".into()));
    }
    Ok(())
}

fn validate_id(value: &str, prefix: &str, name: &str) -> Result<()> {
    if !value.starts_with(prefix) {
        return Err(AgentMemoryError::Invalid(format!(
            "{name} must start with {prefix}: {value}"
        )));
    }
    if value.contains('/') || value.contains('\0') || value.chars().any(|c| c.is_control()) {
        return Err(AgentMemoryError::Invalid(format!(
            "invalid {name}: {value}"
        )));
    }
    Ok(())
}

fn validate_iso8601(value: &str) -> Result<String> {
    let dt = DateTime::parse_from_rfc3339(value)
        .map_err(|_| AgentMemoryError::Invalid(format!("invalid iso8601 timestamp: {value}")))?;
    Ok(dt
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn validate_probability(value: f64, name: &str) -> Result<()> {
    if !(0.0..=1.0).contains(&value) || value.is_nan() {
        return Err(AgentMemoryError::Invalid(format!(
            "{name} must be in 0.0..1.0"
        )));
    }
    Ok(())
}

fn validate_object_op(op: &UpsertObjectOp) -> Result<()> {
    if let Some(object_id) = &op.object_id {
        validate_id(object_id, "obj_", "object_id")?;
    }
    if op.kind.trim().is_empty() {
        return Err(AgentMemoryError::Invalid("object kind is empty".into()));
    }
    if op.canonical_name.trim().is_empty() {
        return Err(AgentMemoryError::Invalid("canonical_name is empty".into()));
    }
    validate_probability(op.confidence, "confidence")?;
    if let Some(weight) = op.weight {
        validate_probability(weight, "weight")?;
    }
    Ok(())
}

fn validate_observation_op(op: &AddObservationOp) -> Result<()> {
    if let Some(observation_id) = &op.observation_id {
        validate_id(observation_id, "obs_", "observation_id")?;
    }
    if op.kind.trim().is_empty() {
        return Err(AgentMemoryError::Invalid(
            "observation kind is empty".into(),
        ));
    }
    validate_content(&op.content)?;
    validate_probability(op.confidence, "confidence")?;
    Ok(())
}

fn validate_relation_op(op: &UpsertRelationOp) -> Result<()> {
    validate_id(&op.subject, "obj_", "subject")?;
    validate_id(&op.object, "obj_", "object")?;
    validate_predicate(&op.predicate)?;
    validate_probability(op.weight, "weight")?;
    validate_probability(op.confidence, "confidence")?;
    validate_reason(&op.write_reason)?;
    Ok(())
}

fn validate_status_op(op: &SetStatusOp) -> Result<()> {
    if op.target_kind.trim().is_empty() || op.target_id.trim().is_empty() {
        return Err(AgentMemoryError::Invalid("target is empty".into()));
    }
    validate_reason(&op.reason)?;
    Ok(())
}

fn validate_predicate(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(AgentMemoryError::Invalid("predicate is empty".into()));
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(AgentMemoryError::Invalid(format!(
            "predicate must be lowercase snake_case: {value}"
        )));
    }
    Ok(())
}

fn validate_tag(tag: &str) -> Result<()> {
    let t = tag.trim();
    let len = t.as_bytes().len();
    if len < MIN_TAG_BYTES || len > MAX_TAG_BYTES {
        return Err(AgentMemoryError::Invalid(format!(
            "tag length must be {MIN_TAG_BYTES}-{MAX_TAG_BYTES} bytes: {t:?}"
        )));
    }
    let mut has_alnum = false;
    for c in t.chars() {
        let ok = matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | ' ' | '-');
        if !ok {
            return Err(AgentMemoryError::Invalid(format!(
                "tag has forbidden character {c:?}: {t:?}"
            )));
        }
        if c.is_ascii_alphanumeric() {
            has_alnum = true;
        }
    }
    if !has_alnum {
        return Err(AgentMemoryError::Invalid(format!(
            "tag must contain at least one alphanumeric: {t:?}"
        )));
    }
    Ok(())
}

fn normalize_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tag in tags {
        let normalized = collapse_whitespace(tag).to_lowercase();
        if normalized.is_empty() {
            continue;
        }
        validate_tag(&normalized)?;
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    Ok(out)
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else if c.is_whitespace() {
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

fn normalize_alias(alias: &str) -> String {
    collapse_whitespace(alias).to_lowercase()
}

fn percent_segment(seg: &str) -> String {
    utf8_percent_encode(seg, PERCENT_SET).to_string()
}

fn touch_index(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    Ok(())
}

fn normalized_pair(a: &str, b: &str) -> String {
    if a <= b {
        format!("{a}__{b}")
    } else {
        format!("{b}__{a}")
    }
}

fn merge_unique(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn phrase_hit(haystack: &str, lowered_tag: &str) -> bool {
    !lowered_tag.is_empty() && haystack.contains(lowered_tag)
}

fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

fn claim_summary(claim: &Value) -> String {
    match claim.get("type").and_then(Value::as_str) {
        Some("free") => claim
            .get("statement")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Some("relation") => format!(
            "{} {} {}",
            claim.get("subject").and_then(Value::as_str).unwrap_or(""),
            claim.get("predicate").and_then(Value::as_str).unwrap_or(""),
            claim.get("object").and_then(Value::as_str).unwrap_or("")
        ),
        Some("attribute") => format!(
            "{} {} {}",
            claim.get("subject").and_then(Value::as_str).unwrap_or(""),
            claim.get("attribute").and_then(Value::as_str).unwrap_or(""),
            claim.get("value").and_then(Value::as_str).unwrap_or("")
        ),
        Some("object") => claim
            .get("statement")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Some("event_effect") => claim
            .get("effect")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Some("salience") => claim
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => serde_json::to_string(claim).unwrap_or_default(),
    }
}

fn memory_candidate_limit(opts: &MemoryRecallOptions) -> usize {
    opts.session_raw
        .candidate_limit
        .saturating_add(opts.event.candidate_limit)
        .saturating_add(opts.entity_observation.candidate_limit)
        .saturating_add(opts.entity_relation.candidate_limit)
        .saturating_add(opts.free.candidate_limit)
        .max(opts.max_hints)
        .max(1)
}

fn memory_budget_for(opts: &MemoryRecallOptions, hint_type: MemoryHintType) -> &MemoryHintBudget {
    match hint_type {
        MemoryHintType::SessionRaw => &opts.session_raw,
        MemoryHintType::Event => &opts.event,
        MemoryHintType::EntityObservation => &opts.entity_observation,
        MemoryHintType::EntityRelation => &opts.entity_relation,
        MemoryHintType::Free => &opts.free,
    }
}

fn memory_hint_type_for_kind(kind: &str) -> MemoryHintType {
    match kind {
        "session_raw" | "session" | "session_topic" | "session_artifact" => {
            MemoryHintType::SessionRaw
        }
        "event" | "event_effect" => MemoryHintType::Event,
        "observation" | "attribute" | "object" => MemoryHintType::EntityObservation,
        "relation" => MemoryHintType::EntityRelation,
        _ => MemoryHintType::Free,
    }
}

fn memory_load_score(item: &LoadItem) -> f32 {
    (item.weight * 10.0 + item.confidence * 6.0) as f32
}

fn append_memory_hints_for_type(
    out: &mut Vec<MemoryHint>,
    seen: &mut HashSet<String>,
    candidates: &[LoadItem],
    hint_type: MemoryHintType,
    candidate_limit: usize,
    keep_limit: usize,
    max_hints: usize,
) {
    if keep_limit == 0 || out.len() >= max_hints {
        return;
    }
    let mut kept = 0usize;
    for item in candidates
        .iter()
        .filter(|item| memory_hint_type_for_kind(&item.kind) == hint_type)
        .take(candidate_limit)
    {
        if kept >= keep_limit || out.len() >= max_hints {
            break;
        }
        let target_id = if matches!(hint_type, MemoryHintType::SessionRaw) {
            item.matched
                .iter()
                .find_map(|m| m.strip_prefix("session:").map(ToOwned::to_owned))
                .unwrap_or_else(|| item.item_id.clone())
        } else {
            item.item_id.clone()
        };
        let target_kind = if matches!(hint_type, MemoryHintType::SessionRaw) {
            "session"
        } else {
            "memory_item"
        };
        let key = format!("{target_kind}:{target_id}:{hint_type:?}");
        if !seen.insert(key) {
            continue;
        }
        let matched = item.matched.clone();
        let reason = if matched.is_empty() {
            "memory ranking matched current topic".to_string()
        } else {
            format!("matched {}", matched.join(", "))
        };
        out.push(MemoryHint {
            hint_type,
            target_kind: target_kind.to_string(),
            target_id,
            uri: None,
            hint: memory_short_hint(item, hint_type),
            reason,
            matched,
            score: memory_load_score(item),
            source_occasion: item.source_occasion.clone(),
            noticed_at: item.noticed_at.clone(),
            evidence: item.evidence.clone(),
            kind: item.kind.clone(),
        });
        kept += 1;
    }
}

fn memory_short_hint(item: &LoadItem, hint_type: MemoryHintType) -> String {
    let prefix = match hint_type {
        MemoryHintType::SessionRaw => "Related session memory",
        MemoryHintType::Event => "Relevant event memory",
        MemoryHintType::EntityObservation => "Relevant entity observation",
        MemoryHintType::EntityRelation => "Relevant entity relation",
        MemoryHintType::Free => "Relevant memory",
    };
    let mut body = collapse_whitespace(&item.content);
    if body.len() > 120 {
        let (truncated, _) = truncate_at_char_boundary(&body, 120);
        body = truncated;
    }
    if body.is_empty() {
        format!("{prefix}: {}", item.item_id)
    } else {
        format!("{prefix}: {body}")
    }
}

fn observation_text_by_id(state: &MemoryState) -> HashMap<String, String> {
    state
        .observations
        .iter()
        .map(|(id, obs)| (id.clone(), obs.content.clone()))
        .collect()
}

fn item_search_text(item: &MemoryItem, observations: &HashMap<String, String>) -> String {
    let mut parts = vec![
        item.kind.clone(),
        item.entities.join(" "),
        item.tags.join(" "),
        claim_summary(&item.claim),
    ];
    for evidence in &item.evidence {
        if let Some(text) = observations.get(evidence) {
            parts.push(text.clone());
        }
    }
    collapse_whitespace(&parts.join(" ")).to_lowercase()
}

fn evidence_is_active(state: &MemoryState, item: &MemoryItem) -> bool {
    item.evidence.iter().all(|id| {
        state
            .observations
            .get(id)
            .map(|obs| obs.status == "active")
            .unwrap_or(false)
    }) || item.evidence.is_empty()
}

fn recency_penalty(noticed_at: &str, now: DateTime<Utc>) -> f64 {
    let Ok(dt) = DateTime::parse_from_rfc3339(noticed_at) else {
        return 100.0;
    };
    let age_days = (now - dt.with_timezone(&Utc)).num_seconds().max(0) as f64 / 86_400.0;
    (age_days / 30.0).min(20.0)
}

fn resolve_alias(state: &MemoryState, alias: &str) -> Vec<String> {
    let alias_norm = normalize_alias(alias);
    let mut out = Vec::new();
    for object in state.objects.values() {
        if object.status != "active" && object.status != "merged" {
            continue;
        }
        if object
            .aliases
            .iter()
            .any(|a| a.status == "active" && normalize_alias(&a.alias) == alias_norm)
        {
            out.push(resolve_merged_object(state, &object.object_id));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn resolve_merged_object(state: &MemoryState, object_id: &str) -> String {
    let mut current = object_id.to_string();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        let Some(object) = state.objects.get(&current) else {
            break;
        };
        if object.status == "merged" {
            if let Some(next) = &object.merged_into {
                current = next.clone();
                continue;
            }
        }
        break;
    }
    current
}

pub fn parse_preamble(content: &str) -> Preamble {
    Preamble {
        body: content.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_tmp() -> (TempDir, AgentMemory) {
        let tmp = TempDir::new().unwrap();
        let m = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        (tmp, m)
    }

    #[test]
    fn init_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let _ = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        let _ = AgentMemory::open(AgentMemoryConfig::new(tmp.path())).unwrap();
        assert!(tmp.path().join(".meta/meta.json").exists());
        assert!(tmp.path().join(".meta/occasions.jsonl").exists());
        assert!(tmp.path().join(".meta/lock").exists());
        assert!(tmp.path().join("memory.sqlite").exists());
        assert!(tmp.path().join("item").exists());
    }

    #[test]
    fn set_get_remove_roundtrip_as_free_item() {
        let (_tmp, m) = open_tmp();
        m.set(
            "/user/preference/style",
            "concise english",
            "user conversation;c=1",
        )
        .unwrap();
        assert_eq!(m.get("/user/preference/style").unwrap(), "concise english");
        let items = m.load(&[], LoadOptions::default()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "free");
        m.remove("/user/preference/style", Some("user removed"))
            .unwrap();
        assert!(matches!(
            m.get("/user/preference/style"),
            Err(AgentMemoryError::NotFound(_))
        ));
    }

    #[test]
    fn list_filters_free_keys_by_prefix_and_tombstones() {
        let (_tmp, m) = open_tmp();
        m.set("/user/a", "x", "r").unwrap();
        m.set("/user/b", "y", "r").unwrap();
        m.set("/kb/c", "z", "r").unwrap();
        m.remove("/user/b", None).unwrap();
        let users = m.list(Some("/user/")).unwrap();
        assert_eq!(users, vec!["/user/a".to_string()]);
        let all = m.list(None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn key_validation_rejects_bad_input() {
        let (_tmp, m) = open_tmp();
        assert!(m.set("user/no-leading-slash", "x", "r").is_err());
        assert!(m.set("/.meta/blocked", "x", "r").is_err());
        assert!(m.set("/a/../b", "x", "r").is_err());
        assert!(m.set("/a", "", "r").is_err());
        assert!(m.set("/a", "x", "").is_err());
    }

    #[test]
    fn load_filters_by_tag_match() {
        let (_tmp, m) = open_tmp();
        m.set_free(FlatSetOp {
            key: "/user/dental".to_string(),
            content: "Dental followup at 10am".to_string(),
            reason: "r".to_string(),
            entities: Vec::new(),
            tags: vec!["dental".to_string()],
            weight: None,
            confidence: None,
        })
        .unwrap();
        m.set("/user/groceries", "Buy bread and milk", "r").unwrap();
        let items = m
            .load(&["dental".to_string()], LoadOptions::default())
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "free");
        assert!(items[0].matched.contains(&"tag:dental".to_string()));
    }

    #[test]
    fn graph_relation_loads_by_object() {
        let (_tmp, m) = open_tmp();
        let occ = m
            .add_occasion(OccasionAddInput {
                occasion_type: "session.turn".to_string(),
                summary: "User discussed a project.".to_string(),
                occurred_at: None,
                source_ref: None,
                tags: vec!["project".to_string()],
            })
            .unwrap();
        m.observe(
            &occ.occasion_id,
            AddObservationOp {
                observation_id: Some("obs_user_project".to_string()),
                kind: "explicit_statement".to_string(),
                entities: vec![],
                content: "User works on BuckyOS.".to_string(),
                source_excerpt: None,
                source_ref: None,
                confidence: 0.8,
            },
        )
        .unwrap();
        m.upsert_object(
            &occ.occasion_id,
            UpsertObjectOp {
                object_id: Some("obj_user".to_string()),
                kind: "user".to_string(),
                canonical_name: "User".to_string(),
                aliases: vec![ObjectAliasInput {
                    alias: "user".to_string(),
                    alias_type: "name".to_string(),
                    confidence: 0.9,
                }],
                evidence: vec!["obs_user_project".to_string()],
                weight: Some(0.8),
                confidence: 0.9,
                merge_into: None,
            },
        )
        .unwrap();
        m.upsert_object(
            &occ.occasion_id,
            UpsertObjectOp {
                object_id: Some("obj_buckyos".to_string()),
                kind: "project".to_string(),
                canonical_name: "BuckyOS".to_string(),
                aliases: vec![],
                evidence: vec!["obs_user_project".to_string()],
                weight: Some(0.7),
                confidence: 0.8,
                merge_into: None,
            },
        )
        .unwrap();
        m.relate(
            &occ.occasion_id,
            UpsertRelationOp {
                subject: "obj_user".to_string(),
                predicate: "works_on".to_string(),
                object: "obj_buckyos".to_string(),
                weight: 0.9,
                confidence: 0.8,
                evidence: vec!["obs_user_project".to_string()],
                write_reason: "May affect future project suggestions.".to_string(),
                replaces: Vec::new(),
            },
        )
        .unwrap();
        let opts = LoadOptions {
            objects: vec!["obj_user".to_string()],
            ..Default::default()
        };
        let items = m.load(&[], opts).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "relation");
        assert!(items[0].matched.contains(&"entity:obj_user".to_string()));
    }

    #[test]
    fn tag_validation_enforces_charset() {
        assert!(validate_tag("dental").is_ok());
        assert!(validate_tag("phone case").is_ok());
        assert!(validate_tag("a").is_err());
        assert!(validate_tag("with\"quote").is_err());
        assert!(validate_tag("中文").is_err());
    }

    #[test]
    fn format_load_items_uses_v210_fields() {
        let items = vec![LoadItem {
            item_id: "item_1".to_string(),
            kind: "free".to_string(),
            entities: vec![],
            weight: 0.5,
            confidence: 0.6,
            source_occasion: "occ_1".to_string(),
            noticed_at: "2026-05-09T10:00:00Z".to_string(),
            evidence: vec![],
            matched: vec!["tag:a".into()],
            size: 5,
            truncated: false,
            content: "hello".to_string(),
        }];
        let s = AgentMemory::format_load_items(&items);
        assert!(s.contains("ITEM item_1\n"));
        assert!(s.contains("KIND free\n"));
        assert!(s.contains("SIZE 5\n"));
        assert!(s.contains("MATCHED tag:a\n"));
        assert!(s.contains("---\nhello\nEND\n"));
    }

    #[test]
    fn recall_hints_keeps_typed_items_ahead_of_free_overflow() {
        let (_tmp, m) = open_tmp();
        for idx in 0..5 {
            m.set_free(FlatSetOp {
                key: format!("/free/{idx}"),
                content: format!("Free memory about project {idx}"),
                reason: "test".to_string(),
                entities: Vec::new(),
                tags: vec!["project".to_string()],
                weight: Some(1.0),
                confidence: Some(1.0),
            })
            .unwrap();
        }
        m.commit(
            "test.event",
            "event",
            None,
            vec!["project".to_string()],
            vec![GraphOperation::PutItem(PutItemOp {
                item_id: Some("item_event".to_string()),
                kind: "event".to_string(),
                entities: Vec::new(),
                claim: serde_json::json!({
                    "type": "event_effect",
                    "effect": "Release event affects the project timeline",
                }),
                weight: 0.2,
                confidence: 0.2,
                evidence: Vec::new(),
                write_reason: "test".to_string(),
                replaces: Vec::new(),
            })],
        )
        .unwrap();
        m.commit(
            "test.relation",
            "relation",
            None,
            vec!["project".to_string()],
            vec![GraphOperation::PutItem(PutItemOp {
                item_id: Some("item_relation".to_string()),
                kind: "relation".to_string(),
                entities: vec!["project".to_string(), "obj_b".to_string()],
                claim: serde_json::json!({
                    "type": "relation",
                    "subject": "project",
                    "predicate": "depends_on",
                    "object": "obj_b",
                }),
                weight: 0.2,
                confidence: 0.2,
                evidence: Vec::new(),
                write_reason: "test".to_string(),
                replaces: Vec::new(),
            })],
        )
        .unwrap();

        let hints = m
            .recall_hints(
                &["project".to_string()],
                MemoryRecallOptions {
                    max_hints: 3,
                    session_raw: MemoryHintBudget::new(0, 0),
                    event: MemoryHintBudget::new(4, 1),
                    entity_observation: MemoryHintBudget::new(0, 0),
                    entity_relation: MemoryHintBudget::new(4, 1),
                    free: MemoryHintBudget::new(8, 3),
                    ..MemoryRecallOptions::default()
                },
            )
            .unwrap();
        let types: Vec<MemoryHintType> = hints.iter().map(|hint| hint.hint_type).collect();
        assert!(types.contains(&MemoryHintType::Event));
        assert!(types.contains(&MemoryHintType::EntityRelation));
        assert_eq!(
            types
                .iter()
                .filter(|ty| **ty == MemoryHintType::Free)
                .count(),
            1
        );
    }

    #[test]
    fn verify_and_compact_keep_replayable_archive() {
        let (_tmp, m) = open_tmp();
        m.set("/user/a", "alpha", "r").unwrap();
        m.compact().unwrap();
        let archives: Vec<_> = fs::read_dir(m.archive_dir()).unwrap().collect();
        assert_eq!(archives.len(), 1);
        assert_eq!(m.get("/user/a").unwrap(), "alpha");
        let report = m.verify(false).unwrap();
        assert!(report.digest_mismatch.is_empty());
    }
}
