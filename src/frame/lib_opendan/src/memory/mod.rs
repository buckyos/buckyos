//! Agent Memory component (Memory requirements v1.6, appendix A).
//!
//! One facade over the two layers: the perception stream
//! (`state/perception/<sid>.jsonl`, written by any authorized Session) and
//! the Memory Graph (`<agent_root>/memory`, changed only by consolidation
//! commits under the consolidation lease, M-32). Topic state and
//! observation progress belong to the Session ([`ObservationState`]); Memory
//! keeps no subscription registry and no change log (A.9).
//!
//! Every call is blocking file IO; async callers wrap it in
//! `spawn_blocking`.

mod changes;
mod consolidation;
mod hint;
mod observation;
mod perception;
pub mod preview;
mod read;
mod recall;
mod snapshot;
mod source;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_tool::agent_memory::{
    AgentMemory, AgentMemoryConfig, AgentMemoryError, FaultHook, GraphView,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::error::OpenDanError;
use crate::state::AgentLayout;

pub use agent_tool::agent_memory::{
    ActorKind, AffectedFilter, AmbiguousAlias, Basis, CommitResult, CommitStatus,
    ConsolidationCommit, DerivedStatus, Disposition, DispositionOutcome, Grants, GraphOperation,
    IndexUse, ItemKind, ItemMeta, ItemStatus, MemoryItem, ProducedBy, QueryScope, Scope, SourceRef,
    SourceType,
};
pub use changes::{Change, ChangeKind, ChangeSet};
pub use consolidation::{
    Batch, BatchOptions, CleanupReport, Consolidator, Material, MaterialState, PendingWork,
};
pub use hint::{Budget, Hint, HintLayer, HintState};
pub use observation::{
    observe, set_topic, Delivery, LastQuery, ObservationState, TopicConfig, TopicOutcome,
    TopicState, TopicTag, TopicUpdate, Valve,
};
pub use perception::{PerceptionInput, PerceptionReceipt, ReceiptStatus};
pub use read::{
    BasisEntry, CognitionView, PerceptionView, ProvenanceView, ReadView, RevisionEntry,
    SourceAvailability,
};
pub use recall::{MemoryQuery, QueryResult, RecallStatus, TopicQuery, TopicResult, TopicScope};
pub use snapshot::MemorySnapshot;
pub use source::{SourceEventInfo, SourceEvents};

/// The Agent-level lease a consolidation commit must hold (the existing
/// self-improve lease, reused for the Memory consolidation Goal).
pub const CONSOLIDATION_LEASE: &str = "self_improve";
/// Upper bound of a deferral window (§5.9; an evaluation start value).
pub const DEFAULT_DEFERRAL_WINDOW: Duration = Duration::from_secs(72 * 3600);

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("invalid argument: {0}")]
    Invalid(String),
    /// Missing, or not visible to the caller (the two are not told apart).
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("ambiguous alias `{alias}`: {candidates:?}")]
    Ambiguous {
        alias: String,
        candidates: Vec<String>,
    },
    #[error("lock timeout: {0}")]
    LockTimeout(String),
    #[error("stale lease: {0}")]
    StaleLease(String),
    #[error("commit outcome unknown: {0}")]
    CommitUnknown(String),
    #[error("memory is read-only: {0}")]
    ReadOnly(String),
    #[error("memory store is corrupted: {0}")]
    Corrupted(String),
    /// Storage cannot be read right now (IO, needs migration); never an
    /// empty result.
    #[error("memory store unavailable: {0}")]
    Unavailable(String),
    /// A page cursor from another snapshot; restart the query.
    #[error("stale cursor: {0}")]
    StaleCursor(String),
}

pub type MemoryResult<T> = std::result::Result<T, MemoryError>;

impl From<AgentMemoryError> for MemoryError {
    fn from(e: AgentMemoryError) -> Self {
        match e {
            AgentMemoryError::Invalid(m) => MemoryError::Invalid(m),
            AgentMemoryError::NotFound(m) => MemoryError::NotFound(m),
            AgentMemoryError::Conflict(m) => MemoryError::Conflict(m),
            AgentMemoryError::Ambiguous { alias, candidates } => {
                MemoryError::Ambiguous { alias, candidates }
            }
            AgentMemoryError::LockTimeout(m) => MemoryError::LockTimeout(m),
            AgentMemoryError::StaleLease(m) => MemoryError::StaleLease(m),
            AgentMemoryError::CommitUnknown(m) => MemoryError::CommitUnknown(m),
            AgentMemoryError::ReadOnly(m) => MemoryError::ReadOnly(m),
            AgentMemoryError::Corrupted(m) => MemoryError::Corrupted(m),
            AgentMemoryError::NeedsMigration(m) => {
                MemoryError::Unavailable(format!("needs migration: {m}"))
            }
            AgentMemoryError::Io(e) => MemoryError::Unavailable(e.to_string()),
            AgentMemoryError::Sqlite(e) => MemoryError::Unavailable(e.to_string()),
            AgentMemoryError::Json(e) => MemoryError::Corrupted(e.to_string()),
        }
    }
}

impl From<OpenDanError> for MemoryError {
    fn from(e: OpenDanError) -> Self {
        match e {
            OpenDanError::InvalidArgument(m) => MemoryError::Invalid(m),
            OpenDanError::NotFound(m) => MemoryError::NotFound(m),
            OpenDanError::Busy { resource, .. } => MemoryError::LockTimeout(resource),
            OpenDanError::LeaseLost(r) => MemoryError::PermissionDenied(format!("lease lost: {r}")),
            OpenDanError::Json { path, reason } => {
                MemoryError::Corrupted(format!("{path}: {reason}"))
            }
            other => MemoryError::Unavailable(other.to_string()),
        }
    }
}

impl From<MemoryError> for OpenDanError {
    fn from(e: MemoryError) -> Self {
        match e {
            MemoryError::Invalid(m) => OpenDanError::InvalidArgument(m),
            MemoryError::NotFound(m) => OpenDanError::NotFound(m),
            other => OpenDanError::Other(format!("memory: {other}")),
        }
    }
}

/// Time source; tests and the simulation drive it by hand.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;

    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(self.now_ms() as i64)
            .single()
            .unwrap_or_else(Utc::now)
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        crate::now_ms()
    }
}

/// A clock that only moves when told to.
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn at(t: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(t.timestamp_millis() as u64)))
    }

    pub fn set(&self, t: DateTime<Utc>) {
        self.0.store(t.timestamp_millis() as u64, Ordering::SeqCst);
    }

    pub fn advance(&self, d: Duration) {
        self.0.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

pub fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn iso_ms(ms: u64) -> String {
    iso(Utc
        .timestamp_millis_opt(ms as i64)
        .single()
        .unwrap_or_else(Utc::now))
}

/// Recording policy of the writing Session (decided before persistence).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordPolicy {
    #[default]
    Normal,
    /// "Do not remember this": nothing is written, not even temporarily.
    DoNotRecord,
}

/// Identity of a Memory user, bound by the host (never by the model).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caller {
    pub session_id: String,
    /// Principals this Session may read for (A.3.7).
    pub grants: Grants,
    /// Subjects stamped on its perceptions by default.
    pub default_subjects: Vec<String>,
    #[serde(default)]
    pub record_policy: RecordPolicy,
}

impl Caller {
    pub fn new(session_id: &str, grants: &[&str], default_subjects: &[&str]) -> Self {
        Self {
            session_id: session_id.to_string(),
            grants: grants.iter().map(|s| s.to_string()).collect(),
            default_subjects: default_subjects.iter().map(|s| s.to_string()).collect(),
            record_policy: RecordPolicy::Normal,
        }
    }
}

#[derive(Clone)]
pub struct MemoryConfig {
    pub clock: Arc<dyn Clock>,
    pub sources: Option<Arc<dyn SourceEvents>>,
    pub lock_timeout: Duration,
    pub deferral_window: Duration,
    /// Changes beyond this make `changes_since` ask for a resync.
    pub max_changes: usize,
    pub graph_faults: Option<FaultHook>,
    /// Inject a cleanup failure for a perception file (tests).
    pub cleanup_fault: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            sources: None,
            lock_timeout: Duration::from_secs(5),
            deferral_window: DEFAULT_DEFERRAL_WINDOW,
            max_changes: 200,
            graph_faults: None,
            cleanup_fault: None,
        }
    }
}

/// The Memory component of one Agent.
pub struct Memory {
    layout: AgentLayout,
    cfg: MemoryConfig,
    reader: Mutex<Option<AgentMemory>>,
    writer: Mutex<Option<AgentMemory>>,
}

impl Memory {
    pub fn open(agent_root: impl AsRef<Path>, cfg: MemoryConfig) -> Self {
        Self {
            layout: AgentLayout::new(agent_root.as_ref()),
            cfg,
            reader: Mutex::new(None),
            writer: Mutex::new(None),
        }
    }

    pub fn layout(&self) -> &AgentLayout {
        &self.layout
    }

    pub fn config(&self) -> &MemoryConfig {
        &self.cfg
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.cfg.clock.now()
    }

    pub fn now_ms(&self) -> u64 {
        self.cfg.clock.now_ms()
    }

    pub fn memory_root(&self) -> PathBuf {
        self.layout.memory_dir()
    }

    fn graph_config(&self) -> AgentMemoryConfig {
        let clock = self.cfg.clock.clone();
        let mut c =
            AgentMemoryConfig::new(self.memory_root()).with_clock(Arc::new(move || clock.now()));
        c.lock_timeout = self.cfg.lock_timeout;
        c.faults = self.cfg.graph_faults.clone();
        c
    }

    /// Lock-free Graph reader (reopened once the root gets initialized).
    pub fn graph(&self) -> MemoryResult<AgentMemory> {
        let mut r = self.reader.lock().expect("memory reader");
        if let Some(g) = r.as_ref().filter(|g| g.is_initialized()) {
            return Ok(g.clone());
        }
        let g = AgentMemory::open_read(self.graph_config())?;
        *r = Some(g.clone());
        Ok(g)
    }

    fn graph_writer(&self) -> MemoryResult<AgentMemory> {
        let mut w = self.writer.lock().expect("memory writer");
        if let Some(g) = w.as_ref() {
            return Ok(g.clone());
        }
        let g = AgentMemory::open(self.graph_config())?;
        *w = Some(g.clone());
        Ok(g)
    }

    /// Replayed Graph (unrestricted; callers apply visibility).
    pub fn graph_view(&self) -> MemoryResult<Arc<GraphView>> {
        Ok(self.graph()?.view()?)
    }

    /// Administrative handle (init, verify, compact, repair, migrate).
    pub fn admin(&self) -> MemoryResult<AgentMemory> {
        self.graph_writer()
    }

    /// Current version vector without opening the Graph (A.9 fast path).
    pub fn snapshot(&self) -> MemoryResult<MemorySnapshot> {
        snapshot::current(self)
    }
}
