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
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use buckyos_api::msg_queue::MsgQueueClient;

use crate::channel::Waker;
use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::{FileLock, Lease};
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::SessionRegistry;

fn daily_number(sid: &str, date: &str) -> Option<u64> {
    let (kind, suffix) = sid.split_once('-')?;
    if !matches!(kind, "ui" | "work" | "sc" | "goal") {
        return None;
    }
    let number = suffix.strip_prefix(date)?.strip_prefix('-')?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

fn validate_reserved_id(path: &Path, sid: &str) -> Result<()> {
    let valid = sid
        .split_once('-')
        .and_then(|(_, suffix)| suffix.split_once('-'))
        .is_some_and(|(date, number)| {
            date.len() == 8
                && date.bytes().all(|b| b.is_ascii_digit())
                && chrono::NaiveDate::parse_from_str(date, "%Y%m%d").is_ok()
                && daily_number(sid, date)
                    .is_some_and(|value| value > 0 && value.to_string() == number)
        });
    if !valid {
        return Err(OpenDanError::json(
            path,
            format!("invalid reserved session id `{sid}`"),
        ));
    }
    Ok(())
}

fn max_directory_number(parent: &Path, date: &str) -> Result<u64> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(OpenDanError::io(parent, e)),
    };
    let mut maximum = 0;
    for entry in entries {
        let entry = entry.map_err(|e| OpenDanError::io(parent, e))?;
        if entry
            .file_type()
            .map_err(|e| OpenDanError::io(entry.path(), e))?
            .is_dir()
        {
            if let Some(number) = daily_number(&entry.file_name().to_string_lossy(), date) {
                maximum = maximum.max(number);
            }
        }
    }
    Ok(maximum)
}

pub(super) fn reserve_session_id(
    layout: &AgentLayout,
    kind: SessionKind,
    key: &str,
    parent_dir: Option<&Path>,
) -> Result<String> {
    reserve_session_id_on_date(
        layout,
        kind,
        key,
        parent_dir,
        &chrono::Local::now().format("%Y%m%d").to_string(),
    )
}

fn reserve_session_id_on_date(
    layout: &AgentLayout,
    kind: SessionKind,
    key: &str,
    parent_dir: Option<&Path>,
    date: &str,
) -> Result<String> {
    if key.trim().is_empty() {
        return Err(OpenDanError::InvalidArgument(
            "session reservation key is required".into(),
        ));
    }
    let lock_path = layout.locks_dir().join("session-ids.lock");
    let deadline = Instant::now() + Duration::from_secs(5);
    let _lock = loop {
        if let Some(lock) = FileLock::try_acquire(&lock_path)? {
            break lock;
        }
        if Instant::now() >= deadline {
            return Err(OpenDanError::Busy {
                resource: lock_path.display().to_string(),
                holder: None,
            });
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let reservations = layout.sessions_dir().join(".ids");
    std::fs::create_dir_all(&reservations).map_err(|e| OpenDanError::io(&reservations, e))?;
    let reservation = reservations.join(format!("{}.json", crate::ids::h(&[key])));
    if let Some(sid) = fsutil::read_json_opt::<String>(&reservation)? {
        validate_reserved_id(&reservation, &sid)?;
        if !sid.starts_with(&format!("{}-", kind.id_prefix())) {
            return Err(OpenDanError::InvalidArgument(
                "session reservation kind differs from its key".into(),
            ));
        }
        return Ok(sid);
    }
    let default_parent = layout.default_sessions_parent();
    let mut maximum = max_directory_number(&default_parent, date)?;
    if let Some(parent) = parent_dir {
        maximum = maximum.max(max_directory_number(parent, date)?);
    }
    let registry = layout.sessions_dir();
    for entry in std::fs::read_dir(&registry).map_err(|e| OpenDanError::io(&registry, e))? {
        let entry = entry.map_err(|e| OpenDanError::io(&registry, e))?;
        let name = entry.file_name();
        if let Some(sid) = name.to_string_lossy().strip_suffix(".json") {
            if let Some(number) = daily_number(sid, date) {
                maximum = maximum.max(number);
            }
        }
    }
    for entry in std::fs::read_dir(&reservations).map_err(|e| OpenDanError::io(&reservations, e))? {
        let entry = entry.map_err(|e| OpenDanError::io(&reservations, e))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let sid: String = fsutil::read_json(&path)?;
        validate_reserved_id(&path, &sid)?;
        if let Some(number) = daily_number(&sid, date) {
            maximum = maximum.max(number);
        }
    }
    let sid = loop {
        maximum = maximum.checked_add(1).ok_or_else(|| {
            OpenDanError::InvalidArgument(format!("session directory numbers exhausted for {date}"))
        })?;
        let sid = format!("{}-{date}-{maximum}", kind.id_prefix());
        let mut occupied = false;
        for parent in std::iter::once(default_parent.as_path()).chain(parent_dir) {
            match std::fs::symlink_metadata(parent.join(&sid)) {
                Ok(_) => occupied = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(OpenDanError::io(parent.join(&sid), e)),
            }
        }
        if !occupied {
            break sid;
        }
    };
    fsutil::atomic_replace_json(&reservation, &sid)?;
    Ok(sid)
}

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
        if let Some(g) = &q.goal_id {
            if e.goal_id.as_deref() != Some(g.as_str()) {
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

#[cfg(test)]
mod id_tests {
    use super::*;

    #[test]
    fn reservation_uses_the_maximum_across_kinds_directories_registry_and_reservations() {
        let root = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let layout = AgentLayout::new(root.path());
        for name in [
            "work-20260101-3",
            "ui-20260101-14",
            "sc-20251231-999",
            "goal-20260102-88",
            "work-20260101-999x",
            "work-20260101--100",
            "unrelated-20260101-999",
        ] {
            std::fs::create_dir_all(layout.default_sessions_parent().join(name)).unwrap();
        }
        std::fs::write(
            layout.default_sessions_parent().join("work-20260101-500"),
            "keep",
        )
        .unwrap();
        std::fs::create_dir(custom.path().join("goal-20260101-31")).unwrap();
        let reservations = layout.sessions_dir().join(".ids");
        std::fs::create_dir_all(&reservations).unwrap();
        std::fs::write(layout.registry_entry("ui-20260101-47"), "{}").unwrap();
        fsutil::atomic_replace_json(&reservations.join("reserved.json"), &"sc-20260101-58")
            .unwrap();
        let sid = reserve_session_id_on_date(
            &layout,
            SessionKind::Work,
            "next",
            Some(custom.path()),
            "20260101",
        )
        .unwrap();
        assert_eq!(sid, "work-20260101-59");
        let next =
            reserve_session_id_on_date(&layout, SessionKind::Ui, "next-ui", None, "20260101")
                .unwrap();
        assert_eq!(next, "ui-20260101-60");
    }

    #[test]
    fn reservation_survives_restart_and_retries_on_a_later_day() {
        let root = tempfile::tempdir().unwrap();
        let first = reserve_session_id_on_date(
            &AgentLayout::new(root.path()),
            SessionKind::Work,
            "request",
            None,
            "20260101",
        )
        .unwrap();
        let reopened = AgentLayout::new(root.path());
        assert_eq!(first, "work-20260101-1");
        assert_eq!(
            reserve_session_id_on_date(&reopened, SessionKind::Work, "request", None, "20260102")
                .unwrap(),
            first
        );
        assert_eq!(
            reserve_session_id_on_date(&reopened, SessionKind::Ui, "tomorrow", None, "20260102")
                .unwrap(),
            "ui-20260102-1"
        );
        assert_eq!(
            reserve_session_id_on_date(&reopened, SessionKind::Goal, "later", None, "20260102")
                .unwrap(),
            "goal-20260102-2"
        );
        assert!(!reopened.default_sessions_parent().exists());
        assert!(reserve_session_id_on_date(
            &reopened,
            SessionKind::Goal,
            "request",
            None,
            "20260102"
        )
        .is_err());
    }

    #[test]
    fn custom_parent_numbers_and_file_collisions_are_reserved_agent_wide() {
        let root = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let layout = AgentLayout::new(root.path());
        std::fs::create_dir(custom.path().join("ui-20260101-23")).unwrap();
        std::fs::write(custom.path().join("work-20260101-24"), "keep").unwrap();
        let first = reserve_session_id_on_date(
            &layout,
            SessionKind::Work,
            "custom",
            Some(custom.path()),
            "20260101",
        )
        .unwrap();
        assert_eq!(first, "work-20260101-25");
        let other = reserve_session_id_on_date(
            &layout,
            SessionKind::SelfCheck,
            "default",
            None,
            "20260101",
        )
        .unwrap();
        assert_eq!(other, "sc-20260101-26");
        assert_eq!(
            std::fs::read_to_string(custom.path().join("work-20260101-24")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn corrupt_reservations_and_exhausted_numbers_do_not_allocate() {
        let root = tempfile::tempdir().unwrap();
        let layout = AgentLayout::new(root.path());
        let reservations = layout.sessions_dir().join(".ids");
        std::fs::create_dir_all(&reservations).unwrap();
        let corrupt = reservations.join("corrupt.json");
        std::fs::write(&corrupt, "{").unwrap();
        assert!(matches!(
            reserve_session_id_on_date(&layout, SessionKind::Work, "next", None, "20260101"),
            Err(OpenDanError::Json { .. })
        ));
        std::fs::remove_file(corrupt).unwrap();
        std::fs::create_dir_all(
            layout
                .default_sessions_parent()
                .join(format!("work-20260101-{}", u64::MAX)),
        )
        .unwrap();
        assert!(matches!(
            reserve_session_id_on_date(&layout, SessionKind::Work, "next", None, "20260101"),
            Err(OpenDanError::InvalidArgument(_))
        ));
        assert_eq!(std::fs::read_dir(&reservations).unwrap().count(), 0);
    }

    #[test]
    fn reservation_records_require_generated_ids_on_lookup_and_scan() {
        let root = tempfile::tempdir().unwrap();
        let layout = AgentLayout::new(root.path());
        let reservations = layout.sessions_dir().join(".ids");
        std::fs::create_dir_all(&reservations).unwrap();
        let own_key = reservations.join(format!("{}.json", crate::ids::h(&["next"])));
        let other_key = reservations.join("other.json");
        for sid in [
            "work-foo",
            "work-20260230-1",
            "work-20260101-0",
            "work-20260101-01",
            "work-20260101-18446744073709551616",
            "other-20260101-1",
        ] {
            fsutil::atomic_replace_json(&other_key, &sid).unwrap();
            assert!(matches!(
                reserve_session_id_on_date(&layout, SessionKind::Work, "next", None, "20260101"),
                Err(OpenDanError::Json { .. })
            ));
            fsutil::atomic_replace_json(&own_key, &sid).unwrap();
            assert!(matches!(
                reserve_session_id_on_date(&layout, SessionKind::Work, "next", None, "20260101"),
                Err(OpenDanError::Json { .. })
            ));
            std::fs::remove_file(&own_key).unwrap();
        }
    }

    #[test]
    fn concurrent_reservations_share_a_counter_and_reuse_the_same_key() {
        let root = tempfile::tempdir().unwrap();
        let layout = AgentLayout::new(root.path());
        let barrier = std::sync::Barrier::new(12);
        let mut numbers = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..12)
                .map(|i| {
                    let layout = &layout;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        let kind = [
                            SessionKind::Work,
                            SessionKind::Ui,
                            SessionKind::Goal,
                            SessionKind::SelfCheck,
                        ][i % 4];
                        let sid = reserve_session_id_on_date(
                            layout,
                            kind,
                            &format!("request-{i}"),
                            None,
                            "20260101",
                        )
                        .unwrap();
                        daily_number(&sid, "20260101").unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        numbers.sort();
        assert_eq!(numbers, (1..=12).collect::<Vec<_>>());
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..12)
                .map(|_| {
                    let layout = &layout;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        reserve_session_id_on_date(
                            layout,
                            SessionKind::Work,
                            "shared",
                            None,
                            "20260101",
                        )
                        .unwrap()
                    })
                })
                .collect();
            for handle in handles {
                assert_eq!(handle.join().unwrap(), "work-20260101-13");
            }
        });
        assert_eq!(
            std::fs::read_dir(layout.sessions_dir().join(".ids"))
                .unwrap()
                .count(),
            13
        );
    }
}

#[async_trait]
impl SessionRegistry for FsRegistry {
    async fn reserve_id(
        &self,
        kind: SessionKind,
        key: &str,
        parent_dir: Option<&Path>,
    ) -> Result<String> {
        reserve_session_id(&self.layout, kind, key, parent_dir)
    }

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

    async fn post_input(&self, sid: &str, input: &PostedInput) -> Result<u64> {
        let entry = self
            .read_entry(sid)?
            .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
        // Best-effort pre-check; the consumer is authoritative. A finished
        // session takes only a `decide`, and only while it has a decision
        // to take (otherwise its queue is released).
        if entry.status.run_state == RunState::Finished
            && !(input.input.allowed_after_finish() && entry.status.takes_input())
        {
            return Err(OpenDanError::SessionFinished(sid.to_string()));
        }
        input.validate()?;
        let queue = entry
            .input_queue
            .clone()
            .ok_or_else(|| OpenDanError::Channel(format!("session {sid} has no input queue")))?;
        let client = self
            .queue
            .as_ref()
            .ok_or_else(|| OpenDanError::Channel("no queue client configured".into()))?;
        // A queue whose data was lost is recreated by the driver only: it
        // resets the consumption progress first, the new queue numbering
        // deliveries from 1 again.
        let missing = |e: ::kRPC::RPCErrors| {
            if crate::channel::kmsg::is_queue_not_found(&e) {
                OpenDanError::QueueMissing {
                    session_id: sid.to_string(),
                }
            } else {
                e.into()
            }
        };
        // Capacity check and append are one critical section for every
        // producer that reaches the session through the registry: at most
        // `MAX_PENDING_INPUTS` records wait on the bus, a full bus refuses
        // the append. Pending = delivered and not consumed by the committed
        // state (consumed positions waiting for the cumulative ack are not
        // counted twice).
        let _post_lock = match crate::session::SessionDir::open(&entry.location) {
            Ok(sd) => {
                let state = sd.state()?;
                let config = sd.config()?;
                if state.schema != SESSION_STATE_SCHEMA || config.schema != SESSION_CONFIG_SCHEMA {
                    return Err(OpenDanError::SessionReadonly {
                        session_id: sid.to_string(),
                        reason: format!(
                            "it uses `{}` / `{}`; migrate it before posting or driving",
                            state.schema, config.schema
                        ),
                    });
                }
                let lock_path = sd.state_dir().join(POST_LOCK_FILE);
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(&lock_path)
                    .map_err(|e| OpenDanError::io(&lock_path, e))?;
                fs2::FileExt::lock_exclusive(&file)
                    .map_err(|e| OpenDanError::io(&lock_path, e))?;
                // Re-read under the lock.
                let state = sd.state()?;
                let stats = client.get_queue_stats(&queue).await.map_err(missing)?;
                let src = config.channels.kmsg().map(|(id, _, _)| id.to_string());
                let pending = src
                    .map(|id| state.pending_inputs(&id, stats.last_index))
                    .unwrap_or(0);
                if pending >= MAX_PENDING_INPUTS {
                    return Err(OpenDanError::InputFull {
                        session_id: sid.to_string(),
                        pending,
                    });
                }
                Some(file)
            }
            // The directory is not readable from here (another host): the
            // consumer still validates every record.
            Err(_) => None,
        };
        let msg = crate::channel::kmsg::encode_input(input)?;
        let index = client.post_message(&queue, msg).await.map_err(missing)?;
        if let (Some(w), Some(ev)) = (&self.waker, &entry.wake_event) {
            w.notify(ev, serde_json::json!({ "sid": sid })).await;
        }
        Ok(index)
    }
}
