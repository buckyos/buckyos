//! `runs/` — xllm's run directory layout (`run.json` + `snapshots/NNNN.json`
//! + `.lock`), reused verbatim so xllm can continue an unfinished native run.
//!
//! [`RunHandle`] is an executing run: it holds the run lock and implements the
//! commit primitives of §8.3 / §8.5:
//! - `publish_input_checkpoint`: snapshot fsync → run.json with
//!   `host_commit_pending` (inputs not committed to state.json yet);
//! - `complete_host_commit`: clear the gate after state.json was committed;
//! - `register_inflight`: persisted before a tool runs;
//! - `checkpoint_with_results`: snapshot fsync → run.json publishing the new
//!   snapshot pointer and clearing only the in-flight actions it covers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_tool::exec_tracking::{persisted_outcome_ids, InflightAction};
use agent_tool::xllm::{FileLock as RunLockFile, RunRecord, RunStatus, RunStore};
use llm_context::state::LLMContextSnapshot;

use crate::error::{OpenDanError, RecoveryBlocked, Result};

#[derive(Debug, Clone)]
pub struct SessionRuns {
    dir: PathBuf,
    store: RunStore,
}

impl SessionRuns {
    pub fn new(dir: PathBuf) -> Self {
        let store = RunStore::disk(dir.clone());
        Self { dir, store }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn store(&self) -> &RunStore {
        &self.store
    }

    /// Run ids present on disk (directories only).
    pub fn list(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(OpenDanError::io(&self.dir, e)),
        };
        for e in entries.flatten() {
            if e.path().is_dir() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with('.') {
                    out.push(name);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn exists(&self, run_id: &str) -> bool {
        self.dir.join(run_id).is_dir()
    }

    pub fn record(&self, run_id: &str) -> Result<RunRecord> {
        Ok(self.store.read_record(run_id)?)
    }

    /// Take the run execution lock (shared with xllm).
    pub fn try_lock(&self, run_id: &str) -> Result<Option<RunLockFile>> {
        Ok(self.store.lock_run(run_id)?)
    }

    /// Create a new run directory and lock it.
    pub fn create_locked(&self) -> Result<(String, RunLockFile)> {
        self.store.ensure_writable()?;
        for _ in 0..4 {
            let run_id = self.store.create_run()?;
            if let Some(lock) = self.store.lock_run(&run_id)? {
                return Ok((run_id, lock));
            }
        }
        Err(OpenDanError::Other("cannot lock a freshly created run".into()))
    }

    /// Load a run for recovery: record + the published snapshot. Unsupported,
    /// corrupted or missing data blocks recovery (the run stays untouched).
    pub fn load_checked(&self, run_id: &str) -> Result<(RunRecord, Option<LLMContextSnapshot>)> {
        let blocked = |reason: String| {
            OpenDanError::RecoveryBlocked(RecoveryBlocked {
                reason,
                run_id: Some(run_id.to_string()),
                refs: vec![self.dir.join(run_id).display().to_string()],
            })
        };
        let record = self
            .store
            .read_record(run_id)
            .map_err(|e| blocked(format!("run record unusable: {e}")))?;
        if record.version != agent_tool::xllm::RUN_RECORD_VERSION {
            return Err(blocked(format!(
                "run record version {} is not supported",
                record.version
            )));
        }
        let snapshot = match record.latest_snapshot_idx {
            Some(idx) => Some(
                self.store
                    .get_snapshot(run_id, idx)
                    .map_err(|e| blocked(format!("published snapshot {idx} unusable: {e}")))?,
            ),
            None => None,
        };
        if let Some(s) = &snapshot {
            if s.state.snapshot_version > llm_context::SNAPSHOT_FORMAT_VERSION {
                return Err(blocked(format!(
                    "snapshot format version {} is not supported",
                    s.state.snapshot_version
                )));
            }
        }
        Ok((record, snapshot))
    }

    /// Remove a run directory. The caller holds the run lock and verified
    /// that no execution of it is still unconfirmed.
    pub fn remove_locked(&self, run_id: &str, _lock: &RunLockFile) -> Result<()> {
        Ok(self.store.remove_run(run_id)?)
    }
}

/// The host's recorded end-of-run decision, if any.
pub fn finish_info(record: &RunRecord) -> Option<serde_json::Value> {
    record
        .host
        .as_ref()
        .and_then(|h| h.extra.get("finish"))
        .filter(|v| !v.is_null())
        .cloned()
}

/// An executing run: record in memory, run lock held.
#[derive(Clone)]
pub struct RunHandle {
    run_id: String,
    store: RunStore,
    record: Arc<Mutex<RunRecord>>,
    lock: Arc<RunLockFile>,
}

impl std::fmt::Debug for RunHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunHandle").field("run_id", &self.run_id).finish()
    }
}

impl RunHandle {
    pub fn new(store: RunStore, record: RunRecord, lock: RunLockFile) -> Self {
        Self {
            run_id: record.run_id.clone(),
            store,
            record: Arc::new(Mutex::new(record)),
            lock: Arc::new(lock),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn lock_file(&self) -> &RunLockFile {
        &self.lock
    }

    pub fn record(&self) -> RunRecord {
        self.record.lock().expect("run record lock").clone()
    }

    fn update(&self, f: impl FnOnce(&mut RunRecord)) -> Result<()> {
        let mut rec = self.record.lock().expect("run record lock");
        let mut next = rec.clone();
        f(&mut next);
        next.updated_at_ms = crate::now_ms();
        next.pid = std::process::id();
        self.store.write_record(&next)?;
        *rec = next;
        Ok(())
    }

    /// Persist the record as is (e.g. first write of a new run).
    pub fn write(&self) -> Result<()> {
        self.update(|_| {})
    }

    /// § 8.3 ①②: snapshot (fsync) then run.json publishing the pointer and
    /// the host commit gate. No inference / tool may start until
    /// [`Self::complete_host_commit`].
    pub fn publish_input_checkpoint(
        &self,
        snapshot: &LLMContextSnapshot,
        input_seq: u64,
    ) -> Result<u32> {
        let idx = self.store.put_snapshot(&self.run_id, snapshot)?;
        let covered = persisted_outcome_ids(snapshot);
        self.update(|r| {
            r.latest_snapshot_idx = Some(idx);
            r.host_commit_pending = Some(input_seq);
            r.status = RunStatus::Running;
            r.handover = None;
            r.inflight.retain(|a| !covered.contains(&a.call_id));
        })?;
        Ok(idx)
    }

    /// § 8.3 ④: clear the gate after state.json committed the batch.
    pub fn complete_host_commit(&self) -> Result<()> {
        if self.record.lock().expect("run record lock").host_commit_pending.is_none() {
            return Ok(());
        }
        self.update(|r| r.host_commit_pending = None)
    }

    pub fn host_commit_pending(&self) -> Option<u64> {
        self.record.lock().expect("run record lock").host_commit_pending
    }

    /// Execution admission (§8.5): the host input gate must be clear.
    pub fn require_execution_admitted(&self) -> Result<()> {
        match self.host_commit_pending() {
            None => Ok(()),
            Some(seq) => Err(OpenDanError::blocked(
                format!("input batch {seq} is not committed yet"),
                Some(&self.run_id),
            )),
        }
    }

    /// § 8.5: persisted (fsync) before the tool starts.
    pub fn register_inflight(&self, action: InflightAction) -> Result<()> {
        self.update(|r| {
            r.inflight.retain(|a| a.call_id != action.call_id);
            r.inflight.push(action);
        })
    }

    pub fn inflight(&self) -> Vec<InflightAction> {
        self.record.lock().expect("run record lock").inflight.clone()
    }

    /// § 8.5: results and snapshot first, then one atomic run.json write
    /// publishing the pointer, the status and clearing covered in-flight
    /// actions. Failure stops progress (the caller propagates it).
    pub fn checkpoint_with_results(
        &self,
        snapshot: &LLMContextSnapshot,
        status: Option<RunStatus>,
    ) -> Result<u32> {
        let idx = self.store.put_snapshot(&self.run_id, snapshot)?;
        let covered = persisted_outcome_ids(snapshot);
        self.update(|r| {
            r.latest_snapshot_idx = Some(idx);
            if let Some(s) = status {
                r.status = s;
            }
            r.inflight.retain(|a| !covered.contains(&a.call_id));
        })?;
        Ok(idx)
    }

    /// Like [`Self::checkpoint_with_results`], and in the same run.json write
    /// record the host's end-of-run decision (`host.extra.finish`) so a redo
    /// of the finish after a crash reaches the same result.
    pub fn checkpoint_finish(
        &self,
        snapshot: &LLMContextSnapshot,
        status: RunStatus,
        finish: serde_json::Value,
    ) -> Result<u32> {
        let idx = self.store.put_snapshot(&self.run_id, snapshot)?;
        let covered = persisted_outcome_ids(snapshot);
        self.update(|r| {
            r.latest_snapshot_idx = Some(idx);
            r.status = status;
            r.inflight.retain(|a| !covered.contains(&a.call_id));
            if let Some(h) = r.host.as_mut() {
                if !h.extra.is_object() {
                    h.extra = serde_json::json!({});
                }
                h.extra["finish"] = finish;
            }
        })?;
        Ok(idx)
    }

    pub fn set_status(
        &self,
        status: RunStatus,
        last_error: Option<agent_tool::xllm::RunErrorRecord>,
    ) -> Result<()> {
        self.update(|r| {
            r.status = status;
            if last_error.is_some() {
                r.last_error = last_error;
            }
            if status == RunStatus::Running {
                // Executing again: the hand-over it stopped at was committed.
                r.handover = None;
            }
        })
    }

    /// The run yields at a behavior hand-over (`next_behavior = target`):
    /// snapshot and, in the same run.json write, the non-terminal hand-over
    /// record. Whoever recovers the session commits the transfer exactly
    /// once from it; no executor infers on the run meanwhile.
    pub fn checkpoint_handover(&self, snapshot: &LLMContextSnapshot, target: &str) -> Result<u32> {
        let idx = self.store.put_snapshot(&self.run_id, snapshot)?;
        let covered = persisted_outcome_ids(snapshot);
        self.update(|r| {
            r.latest_snapshot_idx = Some(idx);
            r.status = RunStatus::Paused;
            r.inflight.retain(|a| !covered.contains(&a.call_id));
            r.handover = Some(agent_tool::xllm::RunHandover {
                next_behavior: target.to_string(),
                at_ms: crate::now_ms(),
            });
        })?;
        Ok(idx)
    }

    /// Record the run's cumulative context usage and add the Rounds
    /// (`llm_requests`) made since the previous call: run.json
    /// `usage.llm_requests` sums every executor's calls and is never reset.
    pub fn record_usage(
        &self,
        usage: Option<&buckyos_api::AiUsage>,
        llm_requests: u64,
    ) -> Result<()> {
        let usage = usage.cloned();
        self.update(|r| {
            if let Some(u) = usage {
                r.usage.main = Some(u);
            }
            r.usage.llm_requests += llm_requests;
        })
    }

    /// Drop old snapshots, keeping the latest few (§8.7 X1).
    pub fn prune(&self, keep_latest: usize) -> Result<usize> {
        let keep: Vec<u32> = self
            .record
            .lock()
            .expect("run record lock")
            .latest_snapshot_idx
            .into_iter()
            .collect();
        Ok(self.store.prune_snapshots(&self.run_id, &keep, keep_latest)?)
    }
}
