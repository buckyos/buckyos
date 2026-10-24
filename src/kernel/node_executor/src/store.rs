//! Private operation ledger (doc/arch/node_exector.md §11).
//!
//! ```text
//! <root>/meta.json                 {schema_version, node_did, store_id}
//! <root>/owner.lock                single-writer flock, never replaced
//! <root>/operations/<key-hash>/    request.json (immutable), state.json,
//!                                  result.json (write-once), stdout, stderr,
//!                                  content, upload
//! <root>/tombstones/<key-hash>.json
//! <root>/tmp/                      unaccepted uploads and records being published
//! ```

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use buckyos_api::{
    ExecutorChannel, ExecutorError, ExecutorErrorCode, ExecutorLimits, ExecutorOpKind,
    ExecutorOpResult, ExecutorOpSpec, ExecutorOpState, ExecutorOpStatus, ExecutorOutputSummary,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fsutil::{
    atomic_replace_json, dir_size, ensure_private_dir, file_len, fsync_dir,
    publish_noreplace_json, read_json_opt, read_range, write_new_synced,
};
use crate::now_ms;

pub const STORE_SCHEMA_VERSION: u32 = 1;
const RECORD_OVERHEAD_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Actor {
    pub agent_did: String,
    pub app_instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpIdentity {
    pub actor: Actor,
    pub profile_id: String,
    pub runtime_id: String,
    pub run_id: String,
    pub call_id: String,
    pub op_id: String,
}

fn sha256_json(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn op_key_hash(
    store_id: &str,
    actor: &Actor,
    profile_id: &str,
    runtime_id: &str,
    op_id: &str,
) -> String {
    sha256_json(&serde_json::json!([
        "op",
        store_id,
        actor.agent_did,
        actor.app_instance_id,
        profile_id,
        runtime_id,
        op_id
    ]))
}

impl OpIdentity {
    pub fn key_hash(&self, store_id: &str) -> String {
        op_key_hash(
            store_id,
            &self.actor,
            &self.profile_id,
            &self.runtime_id,
            &self.op_id,
        )
    }

    pub fn call_hash(&self, store_id: &str) -> String {
        sha256_json(&serde_json::json!([
            "call",
            store_id,
            self.actor.agent_did,
            self.actor.app_instance_id,
            self.profile_id,
            self.runtime_id,
            self.run_id,
            self.call_id
        ]))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpBudget {
    pub execution_timeout_ms: u64,
    pub max_output_bytes: u64,
    pub max_file_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpRequestRecord {
    pub schema_version: u32,
    pub store_id: String,
    pub key_hash: String,
    pub call_hash: String,
    pub identity: OpIdentity,
    pub digest: String,
    pub spec: ExecutorOpSpec,
    pub accepted_at_ms: u64,
    pub server_epoch: String,
    pub auth_generation: u64,
    pub budget: OpBudget,
    pub reservation_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpStateRecord {
    pub schema_version: u32,
    pub version: u64,
    pub state: ExecutorOpState,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub cancel_requested: bool,
    pub server_epoch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpResultRecord {
    pub schema_version: u32,
    pub key_hash: String,
    pub digest: String,
    pub result: ExecutorOpResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TombstoneRecord {
    pub schema_version: u32,
    pub key_hash: String,
    pub call_hash: String,
    pub identity: OpIdentity,
    pub digest: String,
    pub kind: ExecutorOpKind,
    pub accepted_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    pub result: ExecutorOpResult,
    pub expired_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreMeta {
    schema_version: u32,
    node_did: String,
    store_id: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub digest: String,
    pub state: ExecutorOpState,
    pub finished_at_ms: Option<u64>,
    pub tombstoned: bool,
    bytes: u64,
    reserved: u64,
}

#[derive(Default)]
struct Index {
    entries: HashMap<String, IndexEntry>,
    calls: HashMap<String, String>,
    used_bytes: u64,
    reserved_bytes: u64,
}

#[derive(Debug)]
pub enum StoreOpenError {
    Locked(String),
    Degraded(String),
    Io(io::Error),
}

impl std::fmt::Display for StoreOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreOpenError::Locked(m) => write!(f, "ledger locked: {m}"),
            StoreOpenError::Degraded(m) => write!(f, "ledger degraded: {m}"),
            StoreOpenError::Io(e) => write!(f, "ledger io error: {e}"),
        }
    }
}

impl std::error::Error for StoreOpenError {}

impl From<io::Error> for StoreOpenError {
    fn from(e: io::Error) -> Self {
        StoreOpenError::Io(e)
    }
}

pub enum Dedupe {
    New,
    Existing(String),
}

pub struct Store {
    root: PathBuf,
    store_id: String,
    _lock: File,
    index: Mutex<Index>,
}

fn has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

fn internal(e: impl std::fmt::Display) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::Internal, e.to_string())
}

pub fn channel_file(channel: ExecutorChannel) -> &'static str {
    channel.as_str()
}

impl Store {
    pub fn open(root: &Path, node_did: &str) -> Result<Store, StoreOpenError> {
        ensure_private_dir(root)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(root.join("owner.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|e| StoreOpenError::Locked(e.to_string()))?;

        let ops_dir = root.join("operations");
        let tomb_dir = root.join("tombstones");
        let tmp_dir = root.join("tmp");
        let meta_path = root.join("meta.json");
        let meta: StoreMeta = match read_json_opt::<StoreMeta>(&meta_path)
            .map_err(|e| StoreOpenError::Degraded(e.to_string()))?
        {
            None => {
                if has_entries(&ops_dir) || has_entries(&tomb_dir) {
                    return Err(StoreOpenError::Degraded(
                        "meta.json is missing while operation records exist".into(),
                    ));
                }
                let meta = StoreMeta {
                    schema_version: STORE_SCHEMA_VERSION,
                    node_did: node_did.to_string(),
                    store_id: format!("st-{}", uuid::Uuid::new_v4().simple()),
                    created_at_ms: now_ms(),
                };
                atomic_replace_json(&meta_path, &meta)?;
                meta
            }
            Some(meta) => {
                if meta.schema_version != STORE_SCHEMA_VERSION {
                    return Err(StoreOpenError::Degraded(format!(
                        "unknown ledger schema {}",
                        meta.schema_version
                    )));
                }
                if meta.node_did != node_did {
                    return Err(StoreOpenError::Degraded(format!(
                        "ledger belongs to {}, not {node_did}",
                        meta.node_did
                    )));
                }
                meta
            }
        };
        for dir in [&ops_dir, &tomb_dir, &tmp_dir] {
            ensure_private_dir(dir)?;
        }
        for entry in std::fs::read_dir(&tmp_dir)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }

        let store = Store {
            root: root.to_path_buf(),
            store_id: meta.store_id,
            _lock: lock,
            index: Mutex::new(Index::default()),
        };
        store.rebuild_index()?;
        Ok(store)
    }

    fn rebuild_index(&self) -> Result<(), StoreOpenError> {
        let mut index = Index::default();
        for entry in std::fs::read_dir(self.root.join("tombstones"))?.flatten() {
            let path = entry.path();
            let tomb: TombstoneRecord = read_json_opt(&path)
                .map_err(|e| StoreOpenError::Degraded(e.to_string()))?
                .ok_or_else(|| StoreOpenError::Degraded(format!("{} vanished", path.display())))?;
            if tomb.schema_version != STORE_SCHEMA_VERSION
                || path.file_stem().and_then(|s| s.to_str()) != Some(tomb.key_hash.as_str())
            {
                return Err(StoreOpenError::Degraded(format!(
                    "invalid tombstone {}",
                    path.display()
                )));
            }
            index.calls.insert(tomb.call_hash.clone(), tomb.key_hash.clone());
            index.used_bytes += file_len(&path);
            index.entries.insert(
                tomb.key_hash.clone(),
                IndexEntry {
                    digest: tomb.digest,
                    state: tomb.result.state,
                    finished_at_ms: Some(tomb.result.finished_at_ms),
                    tombstoned: true,
                    bytes: 0,
                    reserved: 0,
                },
            );
        }
        for entry in std::fs::read_dir(self.root.join("operations"))?.flatten() {
            let dir = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let request: OpRequestRecord = read_json_opt(&dir.join("request.json"))
                .map_err(|e| StoreOpenError::Degraded(e.to_string()))?
                .ok_or_else(|| {
                    StoreOpenError::Degraded(format!("{} lacks request.json", dir.display()))
                })?;
            let mut state: OpStateRecord = read_json_opt(&dir.join("state.json"))
                .map_err(|e| StoreOpenError::Degraded(e.to_string()))?
                .ok_or_else(|| {
                    StoreOpenError::Degraded(format!("{} lacks state.json", dir.display()))
                })?;
            if request.schema_version != STORE_SCHEMA_VERSION
                || request.key_hash != name
                || request.store_id != self.store_id
                || index.entries.contains_key(&name)
            {
                return Err(StoreOpenError::Degraded(format!(
                    "invalid operation record {}",
                    dir.display()
                )));
            }
            let result: Option<OpResultRecord> = read_json_opt(&dir.join("result.json"))
                .map_err(|e| StoreOpenError::Degraded(e.to_string()))?;
            let result = match result {
                Some(r) => r.result,
                None => {
                    let unknown = unknown_result(&dir, state.state, &request);
                    publish_noreplace_json(
                        &dir.join("result.json"),
                        &OpResultRecord {
                            schema_version: STORE_SCHEMA_VERSION,
                            key_hash: request.key_hash.clone(),
                            digest: request.digest.clone(),
                            result: unknown.clone(),
                        },
                    )?;
                    unknown
                }
            };
            if state.state != result.state {
                state.state = result.state;
                state.version += 1;
                atomic_replace_json(&dir.join("state.json"), &state)?;
            }
            let bytes = dir_size(&dir);
            index.used_bytes += bytes;
            index.calls.insert(request.call_hash.clone(), name.clone());
            index.entries.insert(
                name,
                IndexEntry {
                    digest: request.digest,
                    state: result.state,
                    finished_at_ms: Some(result.finished_at_ms),
                    tombstoned: false,
                    bytes,
                    reserved: 0,
                },
            );
        }
        *self.index.lock().unwrap() = index;
        Ok(())
    }

    pub fn store_id(&self) -> &str {
        &self.store_id
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    pub fn op_dir(&self, key_hash: &str) -> PathBuf {
        self.root.join("operations").join(key_hash)
    }

    pub fn entry(&self, key_hash: &str) -> Option<IndexEntry> {
        self.index.lock().unwrap().entries.get(key_hash).cloned()
    }

    /// Same key with the same digest is the same call; a different digest,
    /// or the same run/call under another op_id, is a Conflict.
    pub fn dedupe(&self, key_hash: &str, call_hash: &str, digest: &str) -> Result<Dedupe, ExecutorError> {
        let index = self.index.lock().unwrap();
        Self::dedupe_locked(&index, key_hash, call_hash, digest)
    }

    fn dedupe_locked(
        index: &Index,
        key_hash: &str,
        call_hash: &str,
        digest: &str,
    ) -> Result<Dedupe, ExecutorError> {
        if let Some(entry) = index.entries.get(key_hash) {
            if entry.digest == digest {
                return Ok(Dedupe::Existing(key_hash.to_string()));
            }
            return Err(ExecutorError::new(
                ExecutorErrorCode::Conflict,
                "op_id was already used with a different request",
            ));
        }
        if index.calls.contains_key(call_hash) {
            return Err(ExecutorError::new(
                ExecutorErrorCode::Conflict,
                "this run/call already has another operation",
            ));
        }
        Ok(Dedupe::New)
    }

    fn capacity_check(
        index: &Index,
        limits: &ExecutorLimits,
        extra_bytes: u64,
    ) -> Result<(), ExecutorError> {
        if index.entries.len() as u64 >= limits.max_records {
            return Err(ExecutorError::new(
                ExecutorErrorCode::CapacityExceeded,
                "operation ledger reached max_records",
            ));
        }
        if index.used_bytes + index.reserved_bytes + extra_bytes > limits.max_store_bytes {
            return Err(ExecutorError::new(
                ExecutorErrorCode::CapacityExceeded,
                "operation ledger reached max_store_bytes",
            ));
        }
        Ok(())
    }

    pub fn has_unknown_computer_actions(&self) -> bool {
        let keys: Vec<_> = self.index.lock().unwrap().entries.iter()
            .filter(|(_, entry)| entry.state == ExecutorOpState::Unknown)
            .map(|(key, _)| key.clone()).collect();
        keys.into_iter().any(|key| self.status(&key).is_ok_and(|status|
            status.kind == ExecutorOpKind::ComputerAct))
    }

    pub fn reservation_for(spec: &ExecutorOpSpec, budget: &OpBudget) -> u64 {
        let body = match spec {
            ExecutorOpSpec::Computer { request } => budget.max_file_bytes.min(buckyos_api::COMPUTER_MAX_IMAGE_BYTES as u64) + serde_json::to_vec(request).map_or(0, |v| v.len() as u64),
            ExecutorOpSpec::Exec { cmd, .. } => cmd.len() as u64 + 2 * budget.max_output_bytes,
            ExecutorOpSpec::Read { .. } => budget.max_file_bytes,
            ExecutorOpSpec::Write { content_length, .. } => *content_length,
            ExecutorOpSpec::Edit { edits, .. } => {
                budget.max_file_bytes
                    + edits
                        .iter()
                        .map(|e| (e.old_string.len() + e.new_string.len()) as u64)
                        .sum::<u64>()
            }
        };
        body + RECORD_OVERHEAD_BYTES
    }

    /// Reserve space for an upload that is not yet a record.
    pub fn reserve(&self, limits: &ExecutorLimits, bytes: u64) -> Result<(), ExecutorError> {
        let mut index = self.index.lock().unwrap();
        Self::capacity_check(&index, limits, bytes)?;
        index.reserved_bytes += bytes;
        Ok(())
    }

    pub fn release(&self, bytes: u64) {
        let mut index = self.index.lock().unwrap();
        index.reserved_bytes = index.reserved_bytes.saturating_sub(bytes);
    }

    /// Atomically publish an accepted operation in `starting`. `upload` is a
    /// file in tmp/ whose `upload_reserved` bytes move into the record.
    pub fn admit(
        &self,
        record: &OpRequestRecord,
        limits: &ExecutorLimits,
        upload: Option<(&Path, u64)>,
    ) -> Result<Dedupe, ExecutorError> {
        let mut index = self.index.lock().unwrap();
        if let Dedupe::Existing(key) =
            Self::dedupe_locked(&index, &record.key_hash, &record.call_hash, &record.digest)?
        {
            return Ok(Dedupe::Existing(key));
        }
        let upload_reserved = upload.map(|(_, n)| n).unwrap_or(0);
        let extra = record.reservation_bytes.saturating_sub(upload_reserved);
        Self::capacity_check(&index, limits, extra)?;

        let staging = self.tmp_dir().join(format!("rec-{}", uuid::Uuid::new_v4().simple()));
        let publish = || -> io::Result<()> {
            std::fs::create_dir(&staging)?;
            write_new_synced(
                &staging.join("request.json"),
                &serde_json::to_vec_pretty(record).map_err(io::Error::other)?,
            )?;
            let state = OpStateRecord {
                schema_version: STORE_SCHEMA_VERSION,
                version: 1,
                state: ExecutorOpState::Starting,
                started_at_ms: None,
                pid: None,
                cancel_requested: false,
                server_epoch: record.server_epoch.clone(),
            };
            write_new_synced(
                &staging.join("state.json"),
                &serde_json::to_vec_pretty(&state).map_err(io::Error::other)?,
            )?;
            if let Some((path, _)) = upload {
                std::fs::rename(path, staging.join("upload"))?;
            }
            fsync_dir(&staging)?;
            let target = self.op_dir(&record.key_hash);
            std::fs::rename(&staging, &target)?;
            fsync_dir(&self.root.join("operations"))
        };
        if let Err(e) = publish() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(internal(format!("persist operation: {e}")));
        }
        index.reserved_bytes = index.reserved_bytes.saturating_sub(upload_reserved);
        index.reserved_bytes += record.reservation_bytes;
        index
            .calls
            .insert(record.call_hash.clone(), record.key_hash.clone());
        index.entries.insert(
            record.key_hash.clone(),
            IndexEntry {
                digest: record.digest.clone(),
                state: ExecutorOpState::Starting,
                finished_at_ms: None,
                tombstoned: false,
                bytes: 0,
                reserved: record.reservation_bytes,
            },
        );
        Ok(Dedupe::New)
    }

    pub fn read_request(&self, key_hash: &str) -> io::Result<OpRequestRecord> {
        read_json_opt(&self.op_dir(key_hash).join("request.json"))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "request.json"))
    }

    pub fn read_state(&self, key_hash: &str) -> io::Result<OpStateRecord> {
        read_json_opt(&self.op_dir(key_hash).join("state.json"))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "state.json"))
    }

    pub fn update_state(
        &self,
        key_hash: &str,
        f: impl FnOnce(&mut OpStateRecord),
    ) -> io::Result<OpStateRecord> {
        let _guard = self.index.lock().unwrap();
        let path = self.op_dir(key_hash).join("state.json");
        let mut state: OpStateRecord = read_json_opt(&path)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "state.json"))?;
        if state.state.is_terminal() {
            return Ok(state);
        }
        f(&mut state);
        state.version += 1;
        atomic_replace_json(&path, &state)?;
        Ok(state)
    }

    /// Publish the immutable terminal result, then the state. A result that
    /// already exists wins and is returned.
    pub fn publish_result(&self, key_hash: &str, result: ExecutorOpResult) -> io::Result<ExecutorOpResult> {
        let dir = self.op_dir(key_hash);
        let request = self.read_request(key_hash)?;
        let record = OpResultRecord {
            schema_version: STORE_SCHEMA_VERSION,
            key_hash: key_hash.to_string(),
            digest: request.digest,
            result: result.clone(),
        };
        let final_result = match publish_noreplace_json(&dir.join("result.json"), &record) {
            Ok(()) => result,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let existing: OpResultRecord = read_json_opt(&dir.join("result.json"))?
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "result.json"))?;
                existing.result
            }
            Err(e) => return Err(e),
        };
        let mut index = self.index.lock().unwrap();
        let path = dir.join("state.json");
        if let Some(mut state) = read_json_opt::<OpStateRecord>(&path)? {
            if !state.state.is_terminal() {
                state.state = final_result.state;
                state.version += 1;
                atomic_replace_json(&path, &state)?;
            }
        }
        let bytes = dir_size(&dir);
        if let Some(entry) = index.entries.get_mut(key_hash) {
            let reserved = entry.reserved;
            entry.state = final_result.state;
            entry.finished_at_ms = Some(final_result.finished_at_ms);
            entry.reserved = 0;
            let old = entry.bytes;
            entry.bytes = bytes;
            index.reserved_bytes = index.reserved_bytes.saturating_sub(reserved);
            index.used_bytes = index.used_bytes.saturating_sub(old) + bytes;
        }
        Ok(final_result)
    }

    pub fn read_result(&self, key_hash: &str) -> io::Result<Option<ExecutorOpResult>> {
        Ok(read_json_opt::<OpResultRecord>(&self.op_dir(key_hash).join("result.json"))?
            .map(|r| r.result))
    }

    pub fn status(&self, key_hash: &str) -> Result<ExecutorOpStatus, ExecutorError> {
        let entry = self
            .entry(key_hash)
            .ok_or_else(|| ExecutorError::new(ExecutorErrorCode::NotFound, "operation not found"))?;
        if entry.tombstoned {
            let tomb: TombstoneRecord =
                read_json_opt(&self.root.join("tombstones").join(format!("{key_hash}.json")))
                    .map_err(internal)?
                    .ok_or_else(|| internal("tombstone vanished"))?;
            return Ok(ExecutorOpStatus {
                op_id: tomb.identity.op_id,
                kind: tomb.kind,
                state: tomb.result.state,
                accepted_at_ms: tomb.accepted_at_ms,
                started_at_ms: tomb.started_at_ms,
                cancel_requested: false,
                pid: None,
                result: Some(tomb.result),
                expired: true,
            });
        }
        let request = self.read_request(key_hash).map_err(internal)?;
        let state = self.read_state(key_hash).map_err(internal)?;
        let result = self.read_result(key_hash).map_err(internal)?;
        Ok(ExecutorOpStatus {
            op_id: request.identity.op_id,
            kind: request.spec.kind(),
            state: result.as_ref().map(|r| r.state).unwrap_or(state.state),
            accepted_at_ms: request.accepted_at_ms,
            started_at_ms: state.started_at_ms,
            cancel_requested: state.cancel_requested,
            pid: state.pid,
            result,
            expired: false,
        })
    }

    pub fn read_chunk(
        &self,
        key_hash: &str,
        channel: ExecutorChannel,
        offset: u64,
        max: u64,
    ) -> io::Result<(Vec<u8>, u64)> {
        let path = self.op_dir(key_hash).join(channel_file(channel));
        let len = file_len(&path);
        if offset >= len {
            return Ok((Vec::new(), len));
        }
        Ok((read_range(&path, offset, max)?, len))
    }

    /// Tombstone, then delete, terminal records older than the retention.
    pub fn gc(&self, now: u64, retention_secs: u64) -> io::Result<usize> {
        let candidates: Vec<String> = {
            let index = self.index.lock().unwrap();
            index
                .entries
                .iter()
                .filter(|(_, e)| {
                    !e.tombstoned
                        && e.state.is_terminal()
                        && e.finished_at_ms
                            .map(|f| f + retention_secs * 1000 <= now)
                            .unwrap_or(false)
                })
                .map(|(k, _)| k.clone())
                .collect()
        };
        let mut removed = 0;
        for key in candidates {
            let dir = self.op_dir(&key);
            let request = self.read_request(&key)?;
            let state = self.read_state(&key)?;
            let Some(mut result) = self.read_result(&key)? else {
                continue;
            };
            result.content = None;
            if let Some(computer) = result.computer.as_mut() {
                if computer.feedback.state == buckyos_api::ComputerFeedbackState::Included {
                    computer.feedback.state = buckyos_api::ComputerFeedbackState::Failed;
                    computer.observation = None;
                    computer.error.get_or_insert(buckyos_api::ComputerError {
                        code: "result_expired".into(), message: "observation artifact was garbage collected; execution facts remain available".into(),
                    });
                }
            }
            let tomb = TombstoneRecord {
                schema_version: STORE_SCHEMA_VERSION,
                key_hash: key.clone(),
                call_hash: request.call_hash.clone(),
                identity: request.identity.clone(),
                digest: request.digest.clone(),
                kind: request.spec.kind(),
                accepted_at_ms: request.accepted_at_ms,
                started_at_ms: state.started_at_ms,
                result,
                expired_at_ms: now,
            };
            let tomb_path = self.root.join("tombstones").join(format!("{key}.json"));
            match publish_noreplace_json(&tomb_path, &tomb) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            let mut index = self.index.lock().unwrap();
            if let Some(entry) = index.entries.get_mut(&key) {
                entry.tombstoned = true;
                let bytes = entry.bytes;
                entry.bytes = 0;
                index.used_bytes = index.used_bytes.saturating_sub(bytes) + file_len(&tomb_path);
            }
            drop(index);
            std::fs::remove_dir_all(&dir)?;
            removed += 1;
        }
        if removed > 0 {
            fsync_dir(&self.root.join("operations"))?;
        }
        Ok(removed)
    }
}

fn unknown_result(dir: &Path, previous: ExecutorOpState, request: &OpRequestRecord) -> ExecutorOpResult {
    let summary = |name: &str| {
        let len = file_len(&dir.join(name));
        Some(ExecutorOutputSummary {
            total_bytes: len,
            retained_bytes: len,
            truncated: false,
            hash: None,
        })
    };
    let ran = previous == ExecutorOpState::Running;
    ExecutorOpResult {
        state: ExecutorOpState::Unknown,
        exit_code: None,
        signal: None,
        error: Some(
            ExecutorError::new(
                ExecutorErrorCode::Unavailable,
                if ran {
                    "the executor restarted while the operation was running; it may still be running"
                } else {
                    "the executor restarted before confirming whether the operation started"
                },
            )
            .unknown_effect(),
        ),
        stdout: summary("stdout"),
        stderr: summary("stderr"),
        content: None,
        file: None,
        computer: match &request.spec {
            ExecutorOpSpec::Computer { request: computer } => Some(buckyos_api::ComputerResult::failed(
                &request.identity.op_id, computer.binding_id(), buckyos_api::ComputerExecutionState::Unknown,
                "execution_unknown", "daemon restarted without a persisted completion; query this operation and never replay it",
            )),
            _ => None,
        },
        finished_at_ms: now_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn record(store: &Store, op_id: &str, call_id: &str, cmd: &str) -> OpRequestRecord {
        let identity = OpIdentity {
            actor: Actor {
                agent_did: "did:web:jarvis.test.buckyos.io".into(),
                app_instance_id: "jarvis.test.buckyos.io@devtest".into(),
            },
            profile_id: "dev".into(),
            runtime_id: "node-dev".into(),
            run_id: "run1".into(),
            call_id: call_id.into(),
            op_id: op_id.into(),
        };
        let spec = ExecutorOpSpec::Exec {
            cmd: cmd.into(),
            cwd: "/tmp".into(),
            env_overrides: BTreeMap::new(),
            execution_timeout_ms: None,
        };
        OpRequestRecord {
            schema_version: STORE_SCHEMA_VERSION,
            store_id: store.store_id().into(),
            key_hash: identity.key_hash(store.store_id()),
            call_hash: identity.call_hash(store.store_id()),
            digest: buckyos_api::executor_request_digest("node-dev", "run1", call_id, op_id, &spec),
            identity,
            spec,
            accepted_at_ms: now_ms(),
            server_epoch: "ep".into(),
            auth_generation: 1,
            budget: OpBudget {
                execution_timeout_ms: 1000,
                max_output_bytes: 1024,
                max_file_bytes: 1024,
            },
            reservation_bytes: 4096,
        }
    }

    #[test]
    fn restart_marks_unfinished_operations_unknown_and_keeps_store_id() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ledger");
        let limits = ExecutorLimits::default();
        let (store_id, key) = {
            let store = Store::open(&root, "did:dev:a").unwrap();
            assert!(matches!(
                Store::open(&root, "did:dev:a"),
                Err(StoreOpenError::Locked(_))
            ));
            let rec = record(&store, "op1", "c1", "echo hi");
            store.admit(&rec, &limits, None).unwrap();
            (store.store_id().to_string(), rec.key_hash)
        };
        let store = Store::open(&root, "did:dev:a").unwrap();
        assert_eq!(store.store_id(), store_id);
        let status = store.status(&key).unwrap();
        assert_eq!(status.state, ExecutorOpState::Unknown);
        assert!(status.result.unwrap().error.unwrap().effect_unknown);
        // The dedupe facts survive the restart.
        let again = record(&store, "op1", "c1", "echo other");
        assert!(store.admit(&again, &limits, None).is_err());
        drop(store);

        assert!(matches!(
            Store::open(&root, "did:dev:b"),
            Err(StoreOpenError::Degraded(_))
        ));
        std::fs::remove_file(root.join("meta.json")).unwrap();
        assert!(matches!(
            Store::open(&root, "did:dev:a"),
            Err(StoreOpenError::Degraded(_))
        ));
    }

    #[test]
    fn restarted_computer_action_keeps_unknown_fact_and_fences_resource() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ledger");
        let key = {
            let store = Store::open(&root, "did:dev:a").unwrap();
            let mut rec = record(&store, "click", "click", "unused");
            rec.spec = ExecutorOpSpec::Computer {
                request: serde_json::from_value(serde_json::json!({
                    "command": "click", "binding_id": "dev:display:1", "feedback": "none",
                    "target": {"kind": "point", "observation_id": "obs", "x": 1, "y": 1, "space": "screenshot"}
                })).unwrap(),
            };
            rec.digest = buckyos_api::executor_request_digest("node-dev", "run1", "click", "click", &rec.spec);
            store.admit(&rec, &ExecutorLimits::default(), None).unwrap();
            store.update_state(&rec.key_hash, |s| s.state = ExecutorOpState::Running).unwrap();
            rec.key_hash
        };
        let store = Store::open(&root, "did:dev:a").unwrap();
        let result = store.status(&key).unwrap().result.unwrap();
        let computer = result.computer.unwrap();
        assert_eq!(computer.operation_id, "click");
        assert_eq!(computer.binding_id, "dev:display:1");
        assert_eq!(computer.execution.state, buckyos_api::ComputerExecutionState::Unknown);
        assert_eq!(computer.operation_state, buckyos_api::ComputerOperationState::Unknown);
        assert!(store.has_unknown_computer_actions());
        store.gc(now_ms() + 10_000, 1).unwrap();
        assert!(store.has_unknown_computer_actions());
    }

    #[test]
    fn screenshot_gc_preserves_execution_but_marks_feedback_expired() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("ledger"), "did:dev:a").unwrap();
        let mut rec = record(&store, "observe", "observe", "unused");
        rec.spec = ExecutorOpSpec::Computer { request: buckyos_api::ComputerRequest::Observe {
            binding_id: "dev:display:1".into(), mode: buckyos_api::ComputerObservationMode::Visual,
        }};
        rec.digest = buckyos_api::executor_request_digest("node-dev", "run1", "observe", "observe", &rec.spec);
        store.admit(&rec, &ExecutorLimits::default(), None).unwrap();
        let mut computer = buckyos_api::ComputerResult::new("observe", "dev:display:1", buckyos_api::ComputerExecutionState::Completed);
        computer.feedback.state = buckyos_api::ComputerFeedbackState::Included;
        store.publish_result(&rec.key_hash, ExecutorOpResult {
            state: ExecutorOpState::Succeeded, exit_code: None, signal: None, error: None,
            stdout: None, stderr: None, content: None, file: None, computer: Some(computer), finished_at_ms: 1,
        }).unwrap();
        store.gc(10_000, 1).unwrap();
        let status = store.status(&rec.key_hash).unwrap();
        assert!(status.expired);
        assert_eq!(status.state, ExecutorOpState::Succeeded);
        let computer = status.result.unwrap().computer.unwrap();
        assert_eq!(computer.execution.state, buckyos_api::ComputerExecutionState::Completed);
        assert_eq!(computer.operation_state, buckyos_api::ComputerOperationState::Completed);
        assert_eq!(computer.feedback.state, buckyos_api::ComputerFeedbackState::Failed);
        assert_eq!(computer.error.unwrap().code, "result_expired");
    }

    #[test]
    fn gc_keeps_tombstones_and_capacity_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("ledger"), "did:dev:a").unwrap();
        let mut limits = ExecutorLimits::default();
        let rec = record(&store, "op1", "c1", "echo hi");
        store.admit(&rec, &limits, None).unwrap();
        store
            .publish_result(
                &rec.key_hash,
                ExecutorOpResult {
                    state: ExecutorOpState::Succeeded,
                    exit_code: Some(0),
                    signal: None,
                    error: None,
                    stdout: None,
                    stderr: None,
                    content: None,
                    file: None,
                    computer: None,
                    finished_at_ms: 1,
                },
            )
            .unwrap();
        assert_eq!(store.gc(10_000, 1).unwrap(), 1);
        let status = store.status(&rec.key_hash).unwrap();
        assert!(status.expired);
        assert_eq!(status.state, ExecutorOpState::Succeeded);
        assert!(matches!(
            store.dedupe(&rec.key_hash, &rec.call_hash, &rec.digest).unwrap(),
            Dedupe::Existing(_)
        ));
        limits.max_records = 1;
        let second = record(&store, "op2", "c2", "echo hi");
        let err = store.admit(&second, &limits, None).err().unwrap();
        assert_eq!(err.code, ExecutorErrorCode::CapacityExceeded);
    }
}
