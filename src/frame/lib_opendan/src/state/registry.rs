//! Session registry files `state/sessions/<sid>.json` (§6.2).
//!
//! - `register`: creator publishes the entry once (`publish_noreplace`).
//! - `report_state` / `update_location`: the driver (session lease holder) is
//!   the single writer of an entry after creation.
//! - `verify` never rewrites entries: it drops a sidecar mark in
//!   `state/sessions/.unreachable/<sid>`, so the single-writer rule holds.
//! - `query` scans entries with an mtime cache (derived view).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use buckyos_api::msg_queue::MsgQueueClient;

use crate::channel::Waker;
use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::Lease;
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::SessionRegistry;

pub(super) struct FsRegistry {
    layout: AgentLayout,
    queue: Option<Arc<MsgQueueClient>>,
    waker: Option<Arc<dyn Waker>>,
    /// Parsed entries keyed by (mtime, len) — a derived cache; our own
    /// writes invalidate it (coarse mtime on some mounts).
    cache: Mutex<HashMap<String, ((SystemTime, u64), RegistryEntry)>>,
}

impl FsRegistry {
    pub(super) fn new(
        layout: AgentLayout,
        queue: Option<Arc<MsgQueueClient>>,
        waker: Option<Arc<dyn Waker>>,
    ) -> Self {
        Self {
            layout,
            queue,
            waker,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn read_entry(&self, sid: &str) -> Result<Option<RegistryEntry>> {
        let path = self.layout.registry_entry(sid);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(OpenDanError::io(&path, e)),
        };
        let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
        if let Some((t, e)) = self.cache.lock().expect("registry cache").get(sid) {
            if *t == stamp {
                let mut e = e.clone();
                e.unreachable = self.layout.unreachable_mark(sid).exists();
                return Ok(Some(e));
            }
        }
        let Some(mut entry) = fsutil::read_json_opt::<RegistryEntry>(&path)? else {
            return Ok(None);
        };
        self.cache
            .lock()
            .expect("registry cache")
            .insert(sid.to_string(), (stamp, entry.clone()));
        entry.unreachable = self.layout.unreachable_mark(sid).exists();
        Ok(Some(entry))
    }

    fn write_entry(&self, entry: &RegistryEntry) -> Result<()> {
        let mut e = entry.clone();
        e.unreachable = false;
        self.cache
            .lock()
            .expect("registry cache")
            .remove(&entry.session_id);
        fsutil::atomic_replace_json(&self.layout.registry_entry(&entry.session_id), &e)
    }

    fn matches(e: &RegistryEntry, q: &RegistryQuery) -> bool {
        if let Some(k) = q.kind {
            if e.kind != k {
                return false;
            }
        }
        if let Some(d) = &q.driver {
            if &e.driver.principal != d {
                return false;
            }
        }
        if !q.run_state_in.is_empty() && !q.run_state_in.contains(&e.status.run_state) {
            return false;
        }
        if q.not_finished_or_pending == Some(true)
            && e.status.run_state == RunState::Finished
            && e.status.acceptance != Acceptance::Pending
        {
            return false;
        }
        if let Some(a) = &q.artifact_id {
            if e.artifact_id.as_deref() != Some(a.as_str()) {
                return false;
            }
        }
        if let Some(p) = q.pending_decision {
            if e.status.pending_decision.is_some() != p {
                return false;
            }
        }
        true
    }
}

fn same_identity(a: &RegistryEntry, b: &RegistryEntry) -> bool {
    a.idempotency_key.is_some()
        && a.idempotency_key == b.idempotency_key
        && a.created_by.principal == b.created_by.principal
        && a.driver == b.driver
        && a.kind == b.kind
}

#[async_trait]
impl SessionRegistry for FsRegistry {
    async fn register(&self, entry: RegistryEntry, _who: &str) -> Result<RegistryEntry> {
        crate::ids::validate_session_id(&entry.session_id)?;
        let path = self.layout.registry_entry(&entry.session_id);
        // File version: authorization = ability to write state/sessions/.
        if fsutil::publish_noreplace_json(&path, &entry)? {
            return Ok(entry);
        }
        let cur = self
            .read_entry(&entry.session_id)?
            .ok_or_else(|| OpenDanError::SessionIdConflict(entry.session_id.clone()))?;
        if cur.location == entry.location || same_identity(&cur, &entry) {
            return Ok(cur);
        }
        Err(OpenDanError::SessionIdConflict(entry.session_id.clone()))
    }

    async fn report_state(&self, lease: &Lease, sid: &str, status: SessionStatus) -> Result<bool> {
        lease.check()?;
        let Some(mut cur) = self.read_entry(sid)? else {
            return Err(OpenDanError::Unregistered(sid.to_string()));
        };
        if status.rev <= cur.status.rev {
            return Ok(false);
        }
        cur.status = status;
        lease.fenced(|| self.write_entry(&cur))?;
        let _ = std::fs::remove_file(self.layout.unreachable_mark(sid));
        Ok(true)
    }

    async fn lookup(&self, sid: &str) -> Result<Option<RegistryEntry>> {
        if crate::ids::validate_session_id(sid).is_err() {
            return Ok(None);
        }
        self.read_entry(sid)
    }

    async fn query(&self, q: &RegistryQuery) -> Result<Vec<RegistryEntry>> {
        let dir = self.layout.sessions_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(sid) = name.strip_suffix(".json") else {
                continue;
            };
            if sid.starts_with('.') {
                continue;
            }
            match self.read_entry(sid) {
                Ok(Some(entry)) if Self::matches(&entry, q) => out.push(entry),
                Ok(_) => {}
                Err(err) => log::warn!("registry entry {sid} unreadable: {err}"),
            }
        }
        out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(out)
    }

    async fn update_location(&self, lease: &Lease, sid: &str, location: &Path) -> Result<()> {
        lease.check()?;
        let Some(mut cur) = self.read_entry(sid)? else {
            return Err(OpenDanError::Unregistered(sid.to_string()));
        };
        cur.location = location.display().to_string();
        cur.location_rev += 1;
        lease.fenced(|| self.write_entry(&cur))?;
        let _ = std::fs::remove_file(self.layout.unreachable_mark(sid));
        Ok(())
    }

    async fn verify(&self) -> Result<Vec<String>> {
        let mut marked = Vec::new();
        for e in self.query(&RegistryQuery::default()).await? {
            let cfg = Path::new(&e.location)
                .join(STATE_DIR)
                .join(SESSION_CONFIG_FILE);
            let mark = self.layout.unreachable_mark(&e.session_id);
            if cfg.is_file() {
                let _ = std::fs::remove_file(&mark);
            } else {
                fsutil::atomic_replace(&mark, crate::now_ms().to_string().as_bytes())?;
                marked.push(e.session_id.clone());
            }
        }
        Ok(marked)
    }

    async fn post_input(&self, sid: &str, input: &Input, who: &str) -> Result<u64> {
        let entry = self
            .read_entry(sid)?
            .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
        // Best-effort pre-check; the consumer is authoritative.
        if entry.status.run_state == RunState::Finished && !input.allowed_after_finish() {
            return Err(OpenDanError::SessionFinished(sid.to_string()));
        }
        let queue = entry
            .input_queue
            .clone()
            .ok_or_else(|| OpenDanError::Channel(format!("session {sid} has no input queue")))?;
        let client = self
            .queue
            .as_ref()
            .ok_or_else(|| OpenDanError::Channel("no queue client configured".into()))?;
        let index = crate::channel::kmsg::post_to_queue(client, &queue, input, who).await?;
        if let (Some(w), Some(ev)) = (&self.waker, &entry.wake_event) {
            w.notify(ev, serde_json::json!({ "sid": sid })).await;
        }
        Ok(index)
    }
}
