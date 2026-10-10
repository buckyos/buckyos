//! Agent Memory Graph, schema 3.0 (Memory requirements, appendix A).
//!
//! `.meta/occasions.jsonl` is the append-only commit log and replay truth.
//! Canonical JSON files, graph snapshots, path indexes and SQLite are derived
//! and rebuilt aside. Writers serialize on `.meta/lock`; readers never take
//! it and replay the log (cached per log stamp), ignoring an in-flight tail.

mod derived;
mod log;
mod migrate;
mod model;
mod recall;
mod state;
mod text;

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

pub use derived::{seg as percent_segment, SQLITE_FILE};
pub use log::{occasion_digest, tail_seq, ArchiveEntry, ArchiveManifest, Tail};
pub use migrate::MigrationReport;
pub use model::*;
pub use recall::{
    group_by_semantic_kind, item_search_text, AliasCandidate, AmbiguousAlias, FadeConfig, IndexUse,
    RecallItem, RecallOutcome, RecallQuery, RecallResult, ScoreParts, DEFAULT_BODY_TRUNCATE_BYTES,
    DEFAULT_MAX_BYTES, DEFAULT_MAX_RECORDS,
};
pub use state::{
    normalize_key, relation_key, validate_perception_ref, AffectedFilter, ApplyReport,
    DispositionState, GraphView, ItemChange, ObjectOutcome, ObjectResult, SharedAlias,
    DEFAULT_FREE_CONFIDENCE, DEFAULT_FREE_WEIGHT,
};
pub use text::{
    collapse_whitespace, fts_enters, fts_hits, fts_tokens, normalize_alias, normalize_tags,
    phrase_hit, validate_tag,
};

use state::ApplyCtx;

pub const SCHEMA_VERSION: &str = "3.0";
/// Multilingual: CJK bigram + word tokens (TD-13, TD-24).
pub const PRIMARY_LANGUAGE: &str = "mul";
pub const FTS_TOKENIZER: &str = "unicode61 remove_diacritics 2 + cjk-bigram";
pub const TIME_MODEL: &str = "dual:occurred_at+noticed_at";
pub const COMPACTION_STRATEGY: &str = "archive+manifest";
pub const META_DIR: &str = ".meta";
pub const META_JSON: &str = "meta.json";
pub const OCCASIONS_LOG_FILE: &str = "occasions.jsonl";
pub const LOCK_FILE: &str = "lock";
pub const ARCHIVE_DIR: &str = "archive";
pub const LEGACY_DIR: &str = "legacy-2.10";
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_SUMMARY_CHARS: usize = 500;
/// Bounds of one recall query (TD-13).
pub const MAX_QUERY_ENTRIES: usize = 32;
pub const MAX_QUERY_TEXT_BYTES: usize = 4096;

#[derive(Debug, Error)]
pub enum AgentMemoryError {
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// A revision, disposition or idempotency conflict; re-read and merge.
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("ambiguous alias `{alias}`: {candidates:?}")]
    Ambiguous {
        alias: String,
        candidates: Vec<String>,
    },
    #[error("lock contention: {0}")]
    LockTimeout(String),
    /// The commit was fenced: a newer consolidation lease already committed.
    #[error("stale lease: {0}")]
    StaleLease(String),
    /// The append may or may not have landed; reopen and look the
    /// idempotency key up instead of appending again.
    #[error("commit outcome unknown: {0}")]
    CommitUnknown(String),
    #[error("read-only memory: {0}")]
    ReadOnly(String),
    #[error("memory needs migration: {0}")]
    NeedsMigration(String),
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
    /// CLI exit codes (A.7).
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::LockTimeout(_) => 2,
            Self::Corrupted(_) => 3,
            _ => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgentMemoryError>;

/// Injected failures for recovery tests (T04).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPoint {
    /// Fail before anything is written.
    BeforeAppend,
    /// Write half of the envelope line, then fail.
    AppendPartial,
    /// Write the whole line, then report an fsync failure.
    AppendBeforeFsync,
    /// Commit lands; rebuilding the derived state fails.
    BeforeMaterialize,
}

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;
pub type FaultHook = Arc<dyn Fn(FaultPoint) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct AgentMemoryConfig {
    pub root: PathBuf,
    pub lock_timeout: Duration,
    pub clock: Clock,
    pub faults: Option<FaultHook>,
    /// `actor_session_id` of administrative single operations.
    pub admin_actor: String,
}

impl std::fmt::Debug for AgentMemoryConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMemoryConfig")
            .field("root", &self.root)
            .field("lock_timeout", &self.lock_timeout)
            .field("admin_actor", &self.admin_actor)
            .finish()
    }
}

impl AgentMemoryConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            clock: Arc::new(Utc::now),
            faults: None,
            admin_actor: "admin".to_string(),
        }
    }

    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_faults(mut self, faults: FaultHook) -> Self {
        self.faults = Some(faults);
        self
    }

    fn fault(&self, p: FaultPoint) -> bool {
        self.faults.as_ref().is_some_and(|f| f(p))
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
    pub created_at: String,
}

impl MetaJson {
    fn current(now: &str) -> Self {
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
                time_model: TIME_MODEL.to_string(),
            },
            index: IndexMeta {
                engine: "sqlite-fts5".to_string(),
                tokenizer: FTS_TOKENIZER.to_string(),
                mechanical_indexes: [
                    "by_entity",
                    "by_alias",
                    "by_pair",
                    "by_kind",
                    "by_predicate",
                    "by_relation",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            },
            compaction_strategy: COMPACTION_STRATEGY.to_string(),
            created_at: now.to_string(),
        }
    }
}

fn schema_parts(v: &str) -> (String, String) {
    let mut it = v.split('.');
    (
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
    )
}

/// TD-24: another major refuses (needs migration); another minor, language
/// or tokenizer mounts read-only.
fn check_meta(meta: &Value) -> Result<Option<String>> {
    let found = meta
        .get("schema_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    let (major, minor) = schema_parts(found);
    let (want_major, want_minor) = schema_parts(SCHEMA_VERSION);
    if major != want_major {
        return Err(AgentMemoryError::NeedsMigration(format!(
            "schema_version {found}; this build reads {SCHEMA_VERSION} (run migrate)"
        )));
    }
    let lang = meta
        .get("primary_language")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tok = meta
        .pointer("/index/tokenizer")
        .and_then(Value::as_str)
        .unwrap_or("");
    if minor != want_minor {
        return Ok(Some(format!(
            "schema_version {found} differs from {SCHEMA_VERSION}"
        )));
    }
    if lang != PRIMARY_LANGUAGE || tok != FTS_TOKENIZER {
        return Ok(Some(format!(
            "primary_language {lang} / tokenizer {tok} differ"
        )));
    }
    Ok(None)
}

/// Who commits: the real Session and the consolidation lease epoch (fencing
/// token, A.4.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitActor {
    pub session_id: String,
    pub lease_epoch: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommitRequest {
    pub occasion_type: String,
    pub summary: String,
    pub source_ref: Option<SourceRef>,
    pub tags: Vec<String>,
    pub operations: Vec<GraphOperation>,
    pub dispositions: Vec<Disposition>,
    pub idempotency_key: Option<String>,
    pub produced_by: Option<ProducedBy>,
    pub parent_occasion: Option<String>,
    /// Apply the consolidation rules (evidence, scope, basis, traceable
    /// sources).
    pub consolidation: bool,
}

/// A.4.1 envelope as the Memory consolidation Goal submits it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationCommit {
    pub idempotency_key: String,
    pub summary: String,
    pub produced_by: ProducedBy,
    #[serde(default)]
    pub operations: Vec<GraphOperation>,
    #[serde(default)]
    pub dispositions: Vec<Disposition>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitStatus {
    Committed,
    /// Same idempotency key and plan: the earlier result is returned.
    AlreadyCommitted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DerivedStatus {
    Current,
    /// The log is committed; rebuilding the derived state failed and is
    /// retried by the next write or `verify --repair`.
    PendingRepair(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitResult {
    pub occasion_id: String,
    pub seq: u64,
    pub status: CommitStatus,
    pub report: ApplyReport,
    pub derived: DerivedStatus,
    /// A torn tail from an interrupted earlier append was dropped first.
    pub recovered_torn_tail: bool,
}

#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    pub occasions: usize,
    pub objects: usize,
    pub observations: usize,
    pub items: usize,
    /// Log integrity: bad lines, digests, seq order, archives, torn tail.
    pub log_errors: Vec<String>,
    /// Missing occasions, evidence or cognition references.
    pub reference_errors: Vec<String>,
    /// Active aliases shared by several objects (reported, never merged).
    pub alias_conflicts: Vec<String>,
    /// Stale or missing derived artifacts.
    pub derived_issues: Vec<String>,
    pub repaired: bool,
}

impl VerifyReport {
    pub fn has_unrecoverable(&self) -> bool {
        !self.log_errors.is_empty() || !self.reference_errors.is_empty()
    }

    pub fn is_clean(&self) -> bool {
        !self.has_unrecoverable() && self.derived_issues.is_empty()
    }
}

/// Legacy CLI `load` options.
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct LogStamp {
    live: (u64, u64),
    manifest: Option<(u64, u128)>,
}

#[derive(Clone)]
pub struct AgentMemory {
    cfg: AgentMemoryConfig,
    initialized: bool,
    read_only: Option<String>,
    cache: Arc<Mutex<Option<(LogStamp, Arc<GraphView>)>>>,
}

impl std::fmt::Debug for AgentMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMemory")
            .field("root", &self.cfg.root)
            .finish()
    }
}

fn now_iso(cfg: &AgentMemoryConfig) -> String {
    (cfg.clock)().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn read_meta_value(path: &Path) -> Result<Option<Value>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b).map_err(|e| {
            AgentMemoryError::Corrupted(format!("{}: {e}", path.display()))
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl AgentMemory {
    /// Open for reading and writing; initializes an empty root. Holds the
    /// writer lock only while initializing.
    pub fn open(cfg: AgentMemoryConfig) -> Result<Self> {
        let meta_dir = cfg.root.join(META_DIR);
        if read_meta_value(&meta_dir.join(META_JSON))?.is_none() {
            fs::create_dir_all(&meta_dir)?;
            let m = Self {
                cfg: cfg.clone(),
                initialized: false,
                read_only: None,
                cache: Arc::new(Mutex::new(None)),
            };
            m.with_writer_lock(|| m.init_locked())?;
        }
        Self::open_existing(cfg, true)
    }

    pub fn init(cfg: AgentMemoryConfig) -> Result<Self> {
        Self::open(cfg)
    }

    /// Open without creating anything and without the writer lock. A root
    /// that was never initialized reads as an empty Graph.
    pub fn open_read(cfg: AgentMemoryConfig) -> Result<Self> {
        Self::open_existing(cfg, false)
    }

    fn open_existing(cfg: AgentMemoryConfig, need_init: bool) -> Result<Self> {
        let meta = read_meta_value(&cfg.root.join(META_DIR).join(META_JSON))?;
        let (initialized, read_only) = match &meta {
            None if need_init => {
                return Err(AgentMemoryError::Corrupted(
                    "meta.json missing after init".into(),
                ))
            }
            None => (false, Some("memory root is not initialized".to_string())),
            Some(m) => (true, check_meta(m)?),
        };
        Ok(Self {
            cfg,
            initialized,
            read_only,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    fn init_locked(&self) -> Result<()> {
        if self.meta_json_path().exists() {
            return Ok(());
        }
        for d in [
            derived::OCCASION_DIR,
            derived::OBJECT_DIR,
            derived::OBSERVATION_DIR,
            derived::ITEM_DIR,
            derived::INDEX_DIR,
            derived::GRAPH_DIR,
        ] {
            fs::create_dir_all(self.cfg.root.join(d))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        derived::materialize(&self.cfg.root, &GraphView::default())?;
        let meta = MetaJson::current(&now_iso(&self.cfg));
        derived::atomic_write(&self.meta_json_path(), &serde_json::to_vec_pretty(&meta)?)
    }

    pub fn root(&self) -> &Path {
        &self.cfg.root
    }

    pub fn config(&self) -> &AgentMemoryConfig {
        &self.cfg
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn read_only_reason(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    fn meta_dir(&self) -> PathBuf {
        self.cfg.root.join(META_DIR)
    }
    fn meta_json_path(&self) -> PathBuf {
        self.meta_dir().join(META_JSON)
    }
    pub fn log_path(&self) -> PathBuf {
        self.meta_dir().join(OCCASIONS_LOG_FILE)
    }
    fn lock_path(&self) -> PathBuf {
        self.meta_dir().join(LOCK_FILE)
    }
    fn archive_dir(&self) -> PathBuf {
        self.meta_dir().join(ARCHIVE_DIR)
    }

    fn now(&self) -> DateTime<Utc> {
        (self.cfg.clock)()
    }

    fn stamp(&self) -> Result<LogStamp> {
        let live = match fs::metadata(self.log_path()) {
            Ok(m) => (m.len(), file_identity(&m)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (0, 0),
            Err(e) => return Err(e.into()),
        };
        let manifest = fs::metadata(self.archive_dir().join("manifest.json"))
            .ok()
            .map(|m| {
                (
                    m.len(),
                    m.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0),
                )
            });
        Ok(LogStamp { live, manifest })
    }

    /// Seq of the last committed occasion, without replaying (snapshot fast
    /// path, A.9).
    pub fn graph_seq(&self) -> Result<u64> {
        if !self.initialized {
            return Ok(0);
        }
        let live = tail_seq(&self.log_path())?;
        if live > 0 {
            return Ok(live);
        }
        Ok(log::read_manifest(&self.archive_dir())?.last_seq())
    }

    /// The replayed Graph, cached until the log changes.
    pub fn view(&self) -> Result<Arc<GraphView>> {
        if !self.initialized {
            return Ok(Arc::new(GraphView::default()));
        }
        let stamp = self.stamp()?;
        if let Some((s, v)) = self.cache.lock().expect("memory cache").as_ref() {
            if *s == stamp {
                return Ok(v.clone());
            }
        }
        let view = Arc::new(self.replay()?);
        *self.cache.lock().expect("memory cache") = Some((stamp, view.clone()));
        Ok(view)
    }

    fn replay(&self) -> Result<GraphView> {
        let mut occasions = Vec::new();
        let live = self.log_path();
        for p in log::log_paths(&self.archive_dir(), &live, false)? {
            let read = log::read_log(&p)?;
            if p != live && read.tail != Tail::Clean {
                return Err(AgentMemoryError::Corrupted(format!(
                    "archived log {} has an incomplete tail",
                    p.display()
                )));
            }
            occasions.extend(read.occasions);
        }
        occasions.sort_by_key(|o| o.seq);
        let mut view = GraphView::default();
        let mut prev: Option<&MemoryOccasion> = None;
        for o in &occasions {
            if let Some(p) = prev.filter(|p| p.seq == o.seq) {
                // The same envelope in an archive and the live log is the
                // middle of a compaction; anything else is corruption.
                if p.digest == o.digest {
                    continue;
                }
                return Err(AgentMemoryError::Corrupted(format!(
                    "duplicate occasion seq {}",
                    o.seq
                )));
            }
            prev = Some(o);
            state::apply_occasion(o, &mut view, ApplyCtx::REPLAY).map_err(|e| {
                AgentMemoryError::Corrupted(format!("replay of {} failed: {e}", o.occasion_id))
            })?;
        }
        Ok(view)
    }

    fn ensure_writable(&self) -> Result<()> {
        match &self.read_only {
            Some(r) => Err(AgentMemoryError::ReadOnly(r.clone())),
            None => Ok(()),
        }
    }

    fn with_writer_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        fs::create_dir_all(self.meta_dir())?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.lock_path())?;
        let deadline = Instant::now() + self.cfg.lock_timeout;
        loop {
            match lock.try_lock_exclusive() {
                Ok(()) => break,
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(AgentMemoryError::LockTimeout(format!(
                            "could not acquire {}: {e}",
                            self.lock_path().display()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        let r = f();
        let _ = FileExt::unlock(&lock);
        r
    }

    /// Under the writer lock: drop a torn tail, complete a missing newline.
    fn recover_tail_locked(&self) -> Result<bool> {
        let read = log::read_log(&self.log_path())?;
        match read.tail {
            Tail::Clean => Ok(false),
            Tail::MissingNewline => {
                log::complete_missing_newline(&self.log_path())?;
                Ok(false)
            }
            Tail::Torn { offset, len } => {
                ::log::warn!(
                    "agent_memory: dropping a torn tail of {len} bytes (an interrupted append) at {offset}"
                );
                log::truncate_tail(&self.log_path(), offset)?;
                Ok(true)
            }
        }
    }

    fn report_for(view: &GraphView, occ: &MemoryOccasion) -> ApplyReport {
        let mut report = ApplyReport::default();
        for item in view.items() {
            for rev in view.item_history(&item.item_id) {
                if rev.source_occasion == occ.occasion_id {
                    report.items.push(ItemChange {
                        item_id: rev.item_id.clone(),
                        revision: rev.revision,
                        status: rev.status,
                        created: rev.revision == 1,
                    });
                }
            }
        }
        report.observations = view
            .observations()
            .filter(|o| o.source_occasion == occ.occasion_id)
            .map(|o| o.observation_id.clone())
            .collect();
        report.objects = view
            .objects()
            .filter(|o| o.last_occasion == occ.occasion_id || o.source_occasion == occ.occasion_id)
            .map(|o| ObjectResult {
                object_id: o.object_id.clone(),
                outcome: if o.status == ObjectStatus::Merged {
                    ObjectOutcome::Merged
                } else if o.source_occasion == occ.occasion_id {
                    ObjectOutcome::Created
                } else {
                    ObjectOutcome::Updated
                },
                shared_aliases: Vec::new(),
            })
            .collect();
        report
    }

    fn plan_digest(req: &CommitRequest) -> Result<String> {
        let v = json!({ "operations": req.operations, "dispositions": req.dispositions });
        Ok(format!(
            "blake3:{}",
            blake3::hash(&serde_json::to_vec(&v)?).to_hex()
        ))
    }

    /// The result of an earlier commit with this idempotency key.
    pub fn lookup_commit(&self, key: &str) -> Result<Option<CommitResult>> {
        let view = self.view()?;
        Ok(view.occasion_for_key(key).map(|occ| CommitResult {
            occasion_id: occ.occasion_id.clone(),
            seq: occ.seq,
            status: CommitStatus::AlreadyCommitted,
            report: Self::report_for(&view, occ),
            derived: DerivedStatus::Current,
            recovered_torn_tail: false,
        }))
    }

    /// One occasion, atomically: validate on a copy of the state, append
    /// and fsync (the commit point), then rebuild derived state.
    pub fn commit(&self, actor: &CommitActor, mut req: CommitRequest) -> Result<CommitResult> {
        self.ensure_writable()?;
        normalize_times(&mut req)?;
        if req.occasion_type.trim().is_empty() || req.occasion_type.chars().any(char::is_control) {
            return Err(AgentMemoryError::Invalid("invalid occasion_type".into()));
        }
        if req.summary.trim().is_empty() || req.summary.chars().count() > MAX_SUMMARY_CHARS {
            return Err(AgentMemoryError::Invalid(format!(
                "summary must be 1..={MAX_SUMMARY_CHARS} characters"
            )));
        }
        if actor.session_id.trim().is_empty() {
            return Err(AgentMemoryError::Invalid(
                "actor session id is empty".into(),
            ));
        }
        if let Some(k) = &req.idempotency_key {
            if k.trim().is_empty() || k.chars().any(char::is_control) {
                return Err(AgentMemoryError::Invalid("invalid idempotency_key".into()));
            }
        }
        if req.consolidation
            && req
                .produced_by
                .as_ref()
                .and_then(|p| p.goal_run.as_ref())
                .is_none_or(|g| g.trim().is_empty())
        {
            return Err(AgentMemoryError::Invalid(
                "a consolidation commit needs produced_by.goal_run".into(),
            ));
        }
        if let Some(s) = &req.source_ref {
            s.validate()?;
        }
        let tags = normalize_tags(&req.tags)?;
        let plan_digest = Self::plan_digest(&req)?;
        self.with_writer_lock(|| {
            let recovered = self.recover_tail_locked()?;
            let view = self.view()?;
            if let Some(key) = &req.idempotency_key {
                if let Some(occ) = view.occasion_for_key(key) {
                    if occ.plan_digest.as_deref() == Some(plan_digest.as_str()) {
                        return Ok(CommitResult {
                            occasion_id: occ.occasion_id.clone(),
                            seq: occ.seq,
                            status: CommitStatus::AlreadyCommitted,
                            report: Self::report_for(&view, occ),
                            derived: DerivedStatus::Current,
                            recovered_torn_tail: recovered,
                        });
                    }
                    return Err(AgentMemoryError::Conflict(format!(
                        "idempotency key {key} was committed by {} with a different plan",
                        occ.occasion_id
                    )));
                }
            }
            if let Some(epoch) = actor.lease_epoch {
                if epoch < view.max_lease_epoch() {
                    return Err(AgentMemoryError::StaleLease(format!(
                        "lease epoch {epoch} is older than {} already committed",
                        view.max_lease_epoch()
                    )));
                }
            }
            let seq = view.seq() + 1;
            let now = now_iso(&self.cfg);
            let mut occ = MemoryOccasion {
                schema_version: SCHEMA_VERSION.to_string(),
                occasion_id: format!("occ_{seq:016}"),
                seq,
                occurred_at: now.clone(),
                noticed_at: now,
                occasion_type: req.occasion_type.clone(),
                actor_session_id: Some(actor.session_id.clone()),
                lease_epoch: actor.lease_epoch,
                idempotency_key: req.idempotency_key.clone(),
                plan_digest: req.idempotency_key.as_ref().map(|_| plan_digest.clone()),
                produced_by: req.produced_by.clone(),
                parent_occasion: req.parent_occasion.clone(),
                source_ref: req.source_ref.clone(),
                summary: req.summary.clone(),
                tags: tags.clone(),
                operations: req.operations.clone(),
                dispositions: req.dispositions.clone(),
                digest: String::new(),
            };
            occ.digest = occasion_digest(&occ)?;
            if let Some(parent) = &req.parent_occasion {
                if view.occasion(parent).is_none() {
                    return Err(AgentMemoryError::NotFound(parent.clone()));
                }
            }
            let mut next = (*view).clone();
            let report = state::apply_occasion(
                &occ,
                &mut next,
                ApplyCtx {
                    strict: true,
                    consolidation: req.consolidation,
                },
            )?;
            self.append_locked(&occ)?;
            let next = Arc::new(next);
            if let Ok(stamp) = self.stamp() {
                *self.cache.lock().expect("memory cache") = Some((stamp, next.clone()));
            }
            let derived = if self.cfg.fault(FaultPoint::BeforeMaterialize) {
                DerivedStatus::PendingRepair("injected materialize failure".into())
            } else {
                match derived::materialize(&self.cfg.root, &next) {
                    Ok(()) => DerivedStatus::Current,
                    Err(e) => DerivedStatus::PendingRepair(e.to_string()),
                }
            };
            Ok(CommitResult {
                occasion_id: occ.occasion_id.clone(),
                seq,
                status: CommitStatus::Committed,
                report,
                derived,
                recovered_torn_tail: recovered,
            })
        })
    }

    fn append_locked(&self, occ: &MemoryOccasion) -> Result<()> {
        if self.cfg.fault(FaultPoint::BeforeAppend) {
            return Err(AgentMemoryError::Io(std::io::Error::other(
                "injected failure before append",
            )));
        }
        let mut line = serde_json::to_vec(occ)?;
        line.push(b'\n');
        if self.cfg.fault(FaultPoint::AppendPartial) {
            let half = &line[..line.len() / 2];
            let _ = log::append_line(&self.log_path(), half);
            return Err(AgentMemoryError::CommitUnknown(
                "injected interrupted append".into(),
            ));
        }
        log::append_line(&self.log_path(), &line)
            .map_err(|e| AgentMemoryError::CommitUnknown(format!("append/fsync failed: {e}")))?;
        if self.cfg.fault(FaultPoint::AppendBeforeFsync) {
            return Err(AgentMemoryError::CommitUnknown(
                "injected fsync failure after write".into(),
            ));
        }
        Ok(())
    }

    /// A.4.1: graph operations and per-material dispositions in one occasion.
    pub fn commit_consolidation(
        &self,
        actor: &CommitActor,
        c: ConsolidationCommit,
    ) -> Result<CommitResult> {
        self.commit(
            actor,
            CommitRequest {
                occasion_type: "consolidation".to_string(),
                summary: c.summary,
                source_ref: None,
                tags: c.tags,
                operations: c.operations,
                dispositions: c.dispositions,
                idempotency_key: Some(c.idempotency_key),
                produced_by: Some(c.produced_by),
                parent_occasion: None,
                consolidation: true,
            },
        )
    }

    /// Rebuild derived state when it lags the log.
    pub fn repair_derived(&self) -> Result<bool> {
        self.ensure_writable()?;
        self.with_writer_lock(|| {
            self.recover_tail_locked()?;
            let view = self.view()?;
            if derived::sqlite_seq(&self.cfg.root) == Some(view.seq()) {
                return Ok(false);
            }
            derived::materialize(&self.cfg.root, &view)?;
            Ok(true)
        })
    }

    // ---------------------------------------------------------------
    // Administrative single operations (A.4.3): each call is one occasion
    // under the admin actor; wrappers record their parent occasion.
    // ---------------------------------------------------------------

    fn admin_actor(&self) -> CommitActor {
        CommitActor {
            session_id: self.cfg.admin_actor.clone(),
            lease_epoch: None,
        }
    }

    fn admin_commit(
        &self,
        occasion_type: &str,
        summary: String,
        parent: Option<&str>,
        operations: Vec<GraphOperation>,
    ) -> Result<CommitResult> {
        self.commit(
            &self.admin_actor(),
            CommitRequest {
                occasion_type: occasion_type.to_string(),
                summary,
                operations,
                parent_occasion: parent.map(str::to_string),
                ..CommitRequest::default()
            },
        )
    }

    pub fn add_occasion(&self, input: OccasionAddInput) -> Result<OccasionAddResult> {
        if let Some(t) = &input.occurred_at {
            state::validate_iso8601(t)?;
        }
        let r = self.commit(
            &self.admin_actor(),
            CommitRequest {
                occasion_type: input.occasion_type,
                summary: input.summary,
                source_ref: input.source_ref,
                tags: input.tags,
                ..CommitRequest::default()
            },
        )?;
        Ok(OccasionAddResult {
            occasion_id: r.occasion_id,
            seq: r.seq,
        })
    }

    pub fn set(&self, key: &str, content: &str, reason: &str) -> Result<CommitResult> {
        self.set_free(FlatSetOp {
            key: key.to_string(),
            content: content.to_string(),
            reason: reason.to_string(),
            entities: Vec::new(),
            weight: None,
            confidence: None,
            evidence: Vec::new(),
            expected_revision: None,
            meta: ItemMeta::default(),
        })
    }

    pub fn set_free(&self, mut op: FlatSetOp) -> Result<CommitResult> {
        op.key = normalize_key(&op.key)?;
        self.admin_commit(
            "memory.write",
            "Set free memory item.".into(),
            None,
            vec![GraphOperation::SetFree(op)],
        )
    }

    pub fn remove(&self, key: &str, reason: Option<&str>) -> Result<CommitResult> {
        let key = normalize_key(key)?;
        self.admin_commit(
            "memory.write",
            "Remove free memory item.".into(),
            None,
            vec![GraphOperation::RemoveFree(FlatRemoveOp {
                key,
                reason: reason.map(str::to_string),
            })],
        )
    }

    fn wrapped(&self, parent: &str, op: GraphOperation) -> Result<CommitResult> {
        state::validate_id(parent, "occ_", "occasion_id")?;
        self.admin_commit(
            "curator.action",
            format!("Apply graph operation for {parent}."),
            Some(parent),
            vec![op],
        )
    }

    pub fn observe(&self, occasion_id: &str, op: AddObservationOp) -> Result<CommitResult> {
        self.wrapped(occasion_id, GraphOperation::AddObservation(op))
    }

    pub fn upsert_object(&self, occasion_id: &str, op: UpsertObjectOp) -> Result<CommitResult> {
        self.wrapped(occasion_id, GraphOperation::UpsertObject(op))
    }

    pub fn reinforce_object(
        &self,
        occasion_id: &str,
        op: ReinforceObjectWeightOp,
    ) -> Result<CommitResult> {
        self.wrapped(occasion_id, GraphOperation::ReinforceObjectWeight(op))
    }

    pub fn relate(&self, occasion_id: &str, op: UpsertRelationOp) -> Result<CommitResult> {
        self.wrapped(occasion_id, GraphOperation::UpsertRelation(op))
    }

    pub fn set_status(&self, occasion_id: &str, op: SetStatusOp) -> Result<CommitResult> {
        self.wrapped(occasion_id, GraphOperation::SetStatus(op))
    }

    // ---------------------------------------------------------------
    // Reads
    // ---------------------------------------------------------------

    pub fn get(&self, key: &str) -> Result<String> {
        let key = normalize_key(key)?;
        let view = self.view()?;
        match view.free_item(&key) {
            Some(item) if item.status == ItemStatus::Active => Ok(item.statement()),
            _ => Err(AgentMemoryError::NotFound(key)),
        }
    }

    pub fn get_item_json(&self, item_id: &str) -> Result<String> {
        let view = self.view()?;
        let (id, rev) = match item_id.split_once('@') {
            Some((id, r)) => (
                id,
                Some(r.parse::<u64>().map_err(|_| {
                    AgentMemoryError::Invalid(format!("bad revision in {item_id}"))
                })?),
            ),
            None => (item_id, None),
        };
        let item = match rev {
            Some(r) => view.item_at(id, r),
            None => view.item(id),
        }
        .ok_or_else(|| AgentMemoryError::NotFound(item_id.to_string()))?;
        Ok(serde_json::to_string_pretty(item)?)
    }

    pub fn get_object_json(&self, object_id: &str) -> Result<String> {
        let view = self.view()?;
        let o = view
            .object(object_id)
            .ok_or_else(|| AgentMemoryError::NotFound(object_id.to_string()))?;
        Ok(serde_json::to_string_pretty(o)?)
    }

    pub fn get_observation_json(&self, observation_id: &str) -> Result<String> {
        let view = self.view()?;
        let o = view
            .observation(observation_id)
            .ok_or_else(|| AgentMemoryError::NotFound(observation_id.to_string()))?;
        Ok(serde_json::to_string_pretty(o)?)
    }

    pub fn list(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let view = self.view()?;
        let mut keys: Vec<String> = view
            .free_keys()
            .filter(|(k, id)| {
                view.item(id)
                    .is_some_and(|i| i.status == ItemStatus::Active)
                    && prefix.is_none_or(|p| k.as_str() == p || k.starts_with(p))
            })
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        Ok(keys)
    }

    pub fn list_objects(&self, kind: Option<&str>) -> Result<Vec<String>> {
        let kind: Option<ObjectKind> = kind.map(str::parse).transpose()?;
        let view = self.view()?;
        Ok(view
            .objects()
            .filter(|o| o.status == ObjectStatus::Active && kind.is_none_or(|k| o.kind == k))
            .map(|o| o.object_id.clone())
            .collect())
    }

    /// Recall (A.6.1). Full text goes through the FTS index when it matches
    /// the log, otherwise through an in-memory scan (reported in `index`).
    pub fn recall(&self, q: &RecallQuery) -> Result<RecallOutcome> {
        if q.tags.len() > MAX_QUERY_ENTRIES
            || q.objects.len() > MAX_QUERY_ENTRIES
            || q.aliases.len() > MAX_QUERY_ENTRIES
        {
            return Err(AgentMemoryError::Invalid(format!(
                "a recall takes at most {MAX_QUERY_ENTRIES} tags, objects and aliases each"
            )));
        }
        if q.text
            .as_ref()
            .is_some_and(|t| t.len() > MAX_QUERY_TEXT_BYTES)
        {
            return Err(AgentMemoryError::Invalid(format!(
                "recall text exceeds {MAX_QUERY_TEXT_BYTES} bytes"
            )));
        }
        let (tags, ignored) = text::normalize_query_tags(&q.tags)?;
        let view = self.view()?;
        let mut text = q.text.clone().unwrap_or_default();
        for t in &tags {
            text.push(' ');
            text.push_str(t);
        }
        let (fts, index) = if fts_tokens(&text).is_empty() {
            (None, IndexUse::Scan("no full-text query".into()))
        } else {
            match derived::fts_lookup(&self.cfg.root, view.seq(), &text) {
                Ok(set) => (Some(set), IndexUse::Fts),
                Err(reason) => (None, IndexUse::Scan(reason)),
            }
        };
        Ok(view.recall(q, &tags, ignored, fts.as_ref(), index))
    }

    /// Legacy `load` for the CLI: unrestricted visibility, no range filter;
    /// "not triggered" is an empty list here.
    pub fn load(&self, tags: &[String], opts: LoadOptions) -> Result<Vec<LoadItem>> {
        let q = RecallQuery {
            tags: tags.to_vec(),
            objects: opts.objects,
            aliases: opts.aliases,
            max_records: opts.max_records,
            max_bytes: opts.max_bytes,
            body_truncate_bytes: opts.body_truncate_bytes,
            now: opts.current_time.or_else(|| Some(self.now())),
            ..RecallQuery::default()
        };
        Ok(self
            .recall(&q)?
            .items()
            .iter()
            .map(|i| LoadItem {
                item_id: i.item_id.clone(),
                revision: i.revision,
                kind: i.kind.to_string(),
                entities: i.entities.clone(),
                weight: i.weight,
                confidence: i.confidence,
                basis: i.basis.map(|b| b.to_string()),
                scope: i.scope.clone(),
                state: i.status.to_string(),
                source_occasion: i.source_occasion.clone(),
                noticed_at: i.noticed_at.clone(),
                evidence: i.evidence.clone(),
                matched: i.matched.clone(),
                size: i.size,
                truncated: i.truncated,
                content: i.content.clone(),
            })
            .collect())
    }

    pub fn format_load_items(items: &[LoadItem]) -> String {
        let mut out = String::new();
        for item in items {
            out.push_str(&format!("ITEM {}\n", item.item_id));
            out.push_str(&format!("REVISION {}\n", item.revision));
            out.push_str(&format!("KIND {}\n", item.kind));
            out.push_str(&format!("ENTITIES {}\n", item.entities.join(",")));
            out.push_str(&format!("WEIGHT {:.3}\n", item.weight));
            out.push_str(&format!("CONFIDENCE {:.3}\n", item.confidence));
            out.push_str(&format!("BASIS {}\n", item.basis.as_deref().unwrap_or("-")));
            out.push_str(&format!(
                "SCOPE {}\n",
                item.scope
                    .as_ref()
                    .map(Scope::key)
                    .unwrap_or_else(|| "-".into())
            ));
            out.push_str(&format!("STATE {}\n", item.state));
            out.push_str(&format!("SOURCE_OCCASION {}\n", item.source_occasion));
            out.push_str(&format!("NOTICED_AT {}\n", item.noticed_at));
            out.push_str(&format!("EVIDENCE {}\n", item.evidence.join(",")));
            out.push_str(&format!("MATCHED {}\n", item.matched.join(",")));
            out.push_str(&format!("SIZE {}\n", item.size));
            out.push_str(&format!("TRUNCATED {}\n", u8::from(item.truncated)));
            out.push_str("---\n");
            out.push_str(&item.content);
            if !item.content.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("END\n");
        }
        out
    }

    // ---------------------------------------------------------------
    // Maintenance
    // ---------------------------------------------------------------

    /// Check the log and derived state (A.8). Without `repair` this only
    /// reports; `repair` rebuilds derived state and never rewrites the log or
    /// merges conflicting aliases.
    pub fn verify(&self, repair: bool) -> Result<VerifyReport> {
        let mut report = VerifyReport::default();
        if !self.initialized {
            report
                .derived_issues
                .push("memory root is not initialized".into());
            return Ok(report);
        }
        if let Err(e) = log::log_paths(&self.archive_dir(), &self.log_path(), true) {
            report.log_errors.push(e.to_string());
        }
        match log::read_log(&self.log_path()) {
            Ok(r) => {
                if let Tail::Torn { offset, len } = r.tail {
                    report.log_errors.push(format!(
                        "torn tail of {len} bytes at {offset} (an interrupted append)"
                    ));
                }
            }
            Err(e) => report.log_errors.push(e.to_string()),
        }
        let view = match self.replay() {
            Ok(v) => v,
            Err(e) => {
                report.log_errors.push(e.to_string());
                return Ok(report);
            }
        };
        report.occasions = view.occasions().len();
        report.objects = view.objects().count();
        report.observations = view.observations().count();
        report.items = view.items().count();
        let mut prev = 0;
        for o in view.occasions() {
            if o.seq <= prev {
                report
                    .log_errors
                    .push(format!("seq {} after {prev}", o.seq));
            }
            prev = o.seq;
            if let Some(p) = &o.parent_occasion {
                if view.occasion(p).is_none() {
                    report
                        .reference_errors
                        .push(format!("{} parent {p} missing", o.occasion_id));
                }
            }
        }
        for o in view.objects() {
            if view.occasion(&o.source_occasion).is_none() {
                report.reference_errors.push(format!(
                    "object {} source_occasion {}",
                    o.object_id, o.source_occasion
                ));
            }
            for e in &o.evidence {
                if view.observation(e).is_none() {
                    report
                        .reference_errors
                        .push(format!("object {} evidence {e}", o.object_id));
                }
            }
        }
        for o in view.observations() {
            if view.occasion(&o.source_occasion).is_none() {
                report.reference_errors.push(format!(
                    "observation {} source_occasion {}",
                    o.observation_id, o.source_occasion
                ));
            }
        }
        for i in view.items() {
            for rev in view.item_history(&i.item_id) {
                if view.occasion(&rev.source_occasion).is_none() {
                    report.reference_errors.push(format!(
                        "item {} source_occasion {}",
                        rev.reference(),
                        rev.source_occasion
                    ));
                }
            }
            for e in &i.evidence {
                if view.observation(e).is_none() {
                    report
                        .reference_errors
                        .push(format!("item {} evidence {e}", i.item_id));
                }
            }
        }
        let mut alias_owners: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for o in view.objects().filter(|o| o.status == ObjectStatus::Active) {
            for a in o.aliases.iter().filter(|a| a.status == AliasStatus::Active) {
                alias_owners
                    .entry(normalize_alias(&a.alias))
                    .or_default()
                    .push(o.object_id.clone());
            }
        }
        for (alias, owners) in alias_owners.into_iter().filter(|(_, v)| v.len() > 1) {
            report
                .alias_conflicts
                .push(format!("{alias}: {}", owners.join(",")));
        }
        match derived::sqlite_seq(&self.cfg.root) {
            Some(s) if s == view.seq() => {}
            Some(s) => report
                .derived_issues
                .push(format!("sqlite at seq {s}, log at {}", view.seq())),
            None => report
                .derived_issues
                .push("sqlite cache missing or unreadable".into()),
        }
        if !self.cfg.root.join(derived::INDEX_DIR).is_dir() {
            report.derived_issues.push("path index missing".into());
        }
        for i in view.items() {
            if !derived::canonical_path(&self.cfg.root, derived::ITEM_DIR, &i.item_id).exists() {
                report
                    .derived_issues
                    .push(format!("canonical item {} missing", i.item_id));
            }
        }
        if repair {
            self.ensure_writable()?;
            self.with_writer_lock(|| {
                self.recover_tail_locked()?;
                let v = self.view()?;
                derived::materialize(&self.cfg.root, &v)
            })?;
            report.repaired = true;
        }
        Ok(report)
    }

    /// Archive the live log (with a manifest entry) and start a new one. No
    /// state snapshot is produced: replay always reads archives + live log
    /// (TD-23).
    pub fn compact(&self) -> Result<()> {
        self.ensure_writable()?;
        self.with_writer_lock(|| {
            self.recover_tail_locked()?;
            let view = self.view()?;
            let live = log::read_log(&self.log_path())?;
            if live.occasions.is_empty() {
                return Ok(());
            }
            let dir = self.archive_dir();
            fs::create_dir_all(&dir)?;
            let mut manifest = log::read_manifest(&dir)?;
            let file = format!("occasions_{:016}.jsonl", live.occasions[0].seq);
            let dst = dir.join(&file);
            // Readers never see a gap: the archive appears (same inode) while
            // the live log still holds the same lines (replay drops identical
            // duplicates), the manifest lists it, then an empty live log
            // replaces the full one.
            if fs::hard_link(self.log_path(), &dst).is_err() {
                fs::copy(self.log_path(), &dst)?;
                fs::File::open(&dst)?.sync_all()?;
            }
            derived::sync_dir(&dir)?;
            manifest.archives.push(ArchiveEntry {
                file,
                first_seq: live.occasions[0].seq,
                last_seq: live.occasions.last().map(|o| o.seq).unwrap_or(0),
                count: live.occasions.len(),
                digest: log::file_digest(&dst)?,
            });
            derived::atomic_write(
                &dir.join("manifest.json"),
                &serde_json::to_vec_pretty(&manifest)?,
            )?;
            derived::atomic_write(&self.log_path(), b"")?;
            derived::materialize(&self.cfg.root, &view)
        })
    }

    /// Migrate a 2.10 root to 3.0 explicitly. Old logs, meta and snapshot are
    /// moved to `.meta/legacy-2.10/`; nothing is deleted.
    pub fn migrate(cfg: AgentMemoryConfig) -> Result<MigrationReport> {
        let meta_path = cfg.root.join(META_DIR).join(META_JSON);
        let meta = read_meta_value(&meta_path)?
            .ok_or_else(|| AgentMemoryError::NotFound("meta.json".into()))?;
        let from = meta
            .get("schema_version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if from == SCHEMA_VERSION {
            return Ok(MigrationReport {
                from: from.clone(),
                to: from,
                ..MigrationReport::default()
            });
        }
        if schema_parts(&from).0 != "2" {
            return Err(AgentMemoryError::NeedsMigration(format!(
                "no migration from schema {from}"
            )));
        }
        let handle = Self {
            cfg: cfg.clone(),
            initialized: false,
            read_only: None,
            cache: Arc::new(Mutex::new(None)),
        };
        handle.with_writer_lock(|| {
            let meta_dir = cfg.root.join(META_DIR);
            let legacy = meta_dir.join(LEGACY_DIR);
            // 1. Preserve the originals (once; a rerun after a crash converts
            //    from this copy again).
            if !legacy.join(".complete").exists() {
                fs::create_dir_all(legacy.join(ARCHIVE_DIR))?;
                let mut copies = vec![
                    (
                        meta_dir.join(OCCASIONS_LOG_FILE),
                        legacy.join(OCCASIONS_LOG_FILE),
                    ),
                    (meta_dir.join("state.jsonl"), legacy.join("state.jsonl")),
                    (meta_path.clone(), legacy.join(META_JSON)),
                ];
                if let Ok(rd) = fs::read_dir(meta_dir.join(ARCHIVE_DIR)) {
                    for e in rd.flatten() {
                        copies.push((e.path(), legacy.join(ARCHIVE_DIR).join(e.file_name())));
                    }
                }
                for (from, to) in copies {
                    if from.is_file() {
                        fs::copy(&from, &to)?;
                        fs::File::open(&to)?.sync_all()?;
                    }
                }
                derived::atomic_write(&legacy.join(".complete"), b"")?;
            }
            // 2. Convert from the preserved copy.
            let mut notes = Vec::new();
            let mut occasions = migrate::convert_legacy_logs(&legacy, &mut notes)?;
            let mut view = GraphView::default();
            for o in &mut occasions {
                o.digest = occasion_digest(o)?;
                state::apply_occasion(o, &mut view, ApplyCtx::REPLAY).map_err(|e| {
                    AgentMemoryError::Corrupted(format!(
                        "converted {} does not replay: {e}",
                        o.occasion_id
                    ))
                })?;
            }
            let mut buf = Vec::new();
            for o in &occasions {
                serde_json::to_writer(&mut buf, o)?;
                buf.push(b'\n');
            }
            // 3. Swap in the converted log, drop the old archives and
            //    snapshot (their copies stay in the legacy directory), and
            //    only then declare the new schema.
            derived::atomic_write(&meta_dir.join(OCCASIONS_LOG_FILE), &buf)?;
            let _ = fs::remove_dir_all(meta_dir.join(ARCHIVE_DIR));
            let _ = fs::remove_file(meta_dir.join("state.jsonl"));
            let new_meta = MetaJson::current(&now_iso(&cfg));
            derived::atomic_write(&meta_path, &serde_json::to_vec_pretty(&new_meta)?)?;
            derived::materialize(&cfg.root, &view)?;
            Ok(MigrationReport {
                from: from.clone(),
                to: SCHEMA_VERSION.to_string(),
                occasions: occasions.len(),
                notes,
                legacy_dir: legacy.display().to_string(),
            })
        })
    }
}

/// Store every deadline and validity end as UTC seconds (`…Z`), so that
/// stored times compare correctly as strings.
fn normalize_times(req: &mut CommitRequest) -> Result<()> {
    for d in &mut req.dispositions {
        if let Some(t) = &d.deadline {
            d.deadline = Some(state::validate_iso8601(t)?);
        }
    }
    for op in &mut req.operations {
        let meta = match op {
            GraphOperation::PutItem(o) => &mut o.meta,
            GraphOperation::UpsertRelation(o) => &mut o.meta,
            GraphOperation::SetFree(o) => &mut o.meta,
            _ => continue,
        };
        if let Some(t) = &meta.valid_until {
            meta.valid_until = Some(state::validate_iso8601(t)?);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn file_identity(m: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    m.ino()
}

#[cfg(not(unix))]
fn file_identity(m: &fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct LoadItem {
    pub item_id: String,
    pub revision: u64,
    pub kind: String,
    pub entities: Vec<String>,
    pub weight: f64,
    pub confidence: f64,
    pub basis: Option<String>,
    pub scope: Option<Scope>,
    pub state: String,
    pub source_occasion: String,
    pub noticed_at: String,
    pub evidence: Vec<String>,
    pub matched: Vec<String>,
    pub size: usize,
    pub truncated: bool,
    pub content: String,
}

#[cfg(test)]
mod tests;
