//! Perception stream `state/perception/<sid>.jsonl` (§6.3, Memory A.5).
//!
//! One writer per file (the session driver, under the session lease); `seq`
//! strictly increasing; appends are idempotent by `seq` and by
//! `idempotency_key`. Appends and the consolidation cleanup of a file both
//! hold the short file lock `<sid>.jsonl.lock`; cleanup replaces the file
//! (new file, fsync, rename) so it never races an append. The byte cursor is
//! only a scan accelerator of the runner's self-improve path.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::{FileLock, Lease};
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::Perception;

/// How long an append or a cleanup waits for the short file lock.
pub const PERCEPTION_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

pub fn perception_lock_path(layout: &AgentLayout, sid: &str) -> PathBuf {
    layout.perception_dir().join(format!("{sid}.jsonl.lock"))
}

/// Take the short lock of one perception file.
pub fn lock_perception_file(layout: &AgentLayout, sid: &str, timeout: Duration) -> Result<FileLock> {
    crate::ids::validate_session_id(sid)?;
    let path = perception_lock_path(layout, sid);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(l) = FileLock::try_acquire(&path)? {
            return Ok(l);
        }
        if Instant::now() >= deadline {
            return Err(OpenDanError::Busy {
                resource: format!("perception:{sid}"),
                holder: None,
            });
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Digest of what a record says (kept after cleanup).
pub fn content_digest(r: &PerceptionRecord) -> String {
    let j = |v: &serde_json::Value| serde_json::to_string(v).unwrap_or_default();
    crate::ids::h(&[
        &r.kind,
        &r.summary,
        &j(&r.payload),
        &serde_json::to_string(&r.scope).unwrap_or_default(),
        r.suggested_kind.as_deref().unwrap_or(""),
        r.memory_intent.as_deref().unwrap_or(""),
        &r.cites.join(","),
        &serde_json::to_string(&r.source_ref).unwrap_or_default(),
        r.occurred_at.as_deref().unwrap_or(""),
        &r.tags.join(","),
        &r.objects.join(","),
    ])
}

/// Every complete record of a file (an unterminated tail is an append in
/// flight and is skipped; a bad complete line is logged and skipped).
pub fn read_records(path: &Path) -> Result<Vec<PerceptionRecord>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(OpenDanError::io(path, e)),
    };
    let complete = match bytes.iter().rposition(|b| *b == b'\n') {
        Some(i) => &bytes[..=i],
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for (n, line) in complete.split(|b| *b == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<PerceptionRecord>(line) {
            Ok(r) => out.push(r),
            Err(e) => log::warn!("perception {}: bad line {}: {e}", path.display(), n + 1),
        }
    }
    Ok(out)
}

/// Last committed seq of a perception file (one backwards read). A damaged
/// last line falls back to the highest readable seq, so one damaged file
/// does not stop every Session's snapshot.
pub fn tail_seq(path: &Path) -> Result<u64> {
    match agent_tool::agent_memory::tail_seq(path) {
        Ok(seq) => Ok(seq),
        Err(agent_tool::agent_memory::AgentMemoryError::Corrupted(e)) => {
            log::warn!("perception {}: damaged last line ({e}); scanning", path.display());
            Ok(read_records(path)?.iter().map(|r| r.seq).max().unwrap_or(0))
        }
        Err(e) => Err(OpenDanError::Other(format!("{}: {e}", path.display()))),
    }
}

/// Under the short lock: an unterminated tail left by an interrupted append
/// is completed when it is a whole record and dropped when it is not, so the
/// next append never fuses with it.
fn repair_tail_locked(path: &Path) -> Result<()> {
    let len = fsutil::file_len(path)?;
    if len == 0 || fsutil::read_at(path, len - 1, 1)?.first() == Some(&b'\n') {
        return Ok(());
    }
    let bytes = std::fs::read(path).map_err(|e| OpenDanError::io(path, e))?;
    let start = bytes.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
    if serde_json::from_slice::<PerceptionRecord>(&bytes[start..]).is_ok() {
        fsutil::append_batch(path, b"\n")?;
    } else {
        log::warn!(
            "perception {}: dropping {} bytes of an interrupted append",
            path.display(),
            bytes.len() - start
        );
        fsutil::truncate_to(path, start as u64)?;
    }
    Ok(())
}

fn same_record(stored: &PerceptionRecord, incoming: &PerceptionRecord) -> bool {
    stored.kind == incoming.kind
        && match &stored.content_digest {
            Some(d) => Some(d) == incoming.content_digest.as_ref(),
            None => stored.summary == incoming.summary,
        }
}

/// Result of an append under the short lock.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppendOutcome {
    pub last_seq: u64,
    /// Records skipped as replays: `(idempotency_key, existing seq)`.
    pub replayed: Vec<(String, u64)>,
}

/// Append under the short lock already held by the caller. A record whose
/// `seq` is already in the file is a replay only when it is the same record
/// (one file has one seq allocator; anything else is an error, never a
/// silent drop); a known `idempotency_key` with the same content is a
/// replay, with other content a conflict.
pub fn append_locked(lease: &Lease, path: &Path, mut records: Vec<PerceptionRecord>) -> Result<AppendOutcome> {
    lease.check()?;
    repair_tail_locked(path)?;
    let last = tail_seq(path)?;
    for r in &mut records {
        if r.content_digest.is_none() {
            r.content_digest = Some(content_digest(r));
        }
    }
    if records.iter().any(|r| r.seq <= last) {
        let stored: HashMap<u64, PerceptionRecord> =
            read_records(path)?.into_iter().map(|r| (r.seq, r)).collect();
        for r in records.iter().filter(|r| r.seq <= last) {
            if !stored.get(&r.seq).is_some_and(|e| same_record(e, r)) {
                return Err(OpenDanError::InvalidArgument(format!(
                    "perception seq {} of {} is already used by a different record",
                    r.seq, r.session_id
                )));
            }
        }
    }
    let keyed = records.iter().any(|r| r.idempotency_key.is_some());
    let known: HashMap<String, (u64, Option<String>)> = if keyed {
        read_records(path)?
            .into_iter()
            .filter_map(|r| r.idempotency_key.clone().map(|k| (k, (r.seq, r.content_digest.clone()))))
            .collect()
    } else {
        HashMap::new()
    };
    let mut out = AppendOutcome {
        last_seq: last,
        replayed: Vec::new(),
    };
    let mut fresh: Vec<PerceptionRecord> = Vec::new();
    for r in records.into_iter().filter(|r| r.seq > last) {
        if let Some(k) = &r.idempotency_key {
            if let Some((seq, digest)) = known.get(k) {
                if digest.is_some() && digest != &r.content_digest {
                    return Err(OpenDanError::InvalidArgument(format!(
                        "idempotency key {k} already recorded as {seq} with different content"
                    )));
                }
                out.replayed.push((k.clone(), *seq));
                continue;
            }
        }
        fresh.push(r);
    }
    fresh.sort_by_key(|r| r.seq);
    let mut prev = last;
    for r in &fresh {
        if r.seq <= prev {
            return Err(OpenDanError::InvalidArgument(format!(
                "perception seq {} is not strictly increasing",
                r.seq
            )));
        }
        prev = r.seq;
    }
    if !fresh.is_empty() {
        lease.fenced(|| fsutil::append_batch(path, &fsutil::to_json_lines(&fresh)?))?;
        out.last_seq = prev;
    }
    Ok(out)
}

/// Replace the bodies of disposed records by their cleared markers (A.5):
/// write a new file, fsync, rename. Identity, key, digest, scope and source
/// stay. Returns how many records were cleared. `fail` injects a failure
/// before the rename (tests).
pub fn clear_records(
    layout: &AgentLayout,
    sid: &str,
    markers: &BTreeMap<u64, ClearedMarker>,
    timeout: Duration,
    fail: bool,
) -> Result<usize> {
    let _lock = lock_perception_file(layout, sid, timeout)?;
    let path = layout.perception_file(sid);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(OpenDanError::io(&path, e)),
    };
    let mut out = Vec::with_capacity(bytes.len());
    let mut cleared = 0usize;
    let end = bytes.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
    for line in bytes[..end].split_inclusive(|b| *b == b'\n') {
        let body = line.strip_suffix(b"\n").unwrap_or(line);
        match serde_json::from_slice::<PerceptionRecord>(body) {
            Ok(r) if r.cleared.is_none() && markers.contains_key(&r.seq) => {
                let marker = markers[&r.seq].clone();
                let kept = PerceptionRecord {
                    seq: r.seq,
                    at_ms: r.at_ms,
                    session_id: r.session_id,
                    kind: r.kind,
                    source: r.source,
                    idempotency_key: r.idempotency_key,
                    content_digest: r.content_digest,
                    scope: r.scope,
                    suggested_kind: r.suggested_kind,
                    memory_intent: r.memory_intent,
                    cites: r.cites,
                    occurred_at: r.occurred_at,
                    source_ref: r.source_ref,
                    backfilled: r.backfilled,
                    tags: r.tags,
                    cleared: Some(marker),
                    ..Default::default()
                };
                out.extend_from_slice(&serde_json::to_vec(&kept).map_err(|e| OpenDanError::json(&path, e))?);
                out.push(b'\n');
                cleared += 1;
            }
            _ => out.extend_from_slice(line),
        }
    }
    // An interrupted append's tail is left to the next append's repair.
    out.extend_from_slice(&bytes[end..]);
    if cleared == 0 {
        return Ok(0);
    }
    if fail {
        return Err(OpenDanError::Other(format!("injected cleanup failure for {sid}")));
    }
    fsutil::atomic_replace(&path, &out)?;
    Ok(cleared)
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklogItem {
    pub session_id: String,
    pub from_offset: u64,
    pub to_offset: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backlog {
    pub items: Vec<BacklogItem>,
}

impl Backlog {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|i| i.to_offset - i.from_offset).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Cursor after consuming the whole backlog.
    pub fn advanced(&self, base: &PerceptionCursor) -> PerceptionCursor {
        let mut c = base.clone();
        for i in &self.items {
            c.offsets.insert(i.session_id.clone(), i.to_offset);
        }
        c.updated_at_ms = crate::now_ms();
        c
    }

    /// Stable digest of the window (idempotency key of a self-improve run).
    pub fn window_digest(&self) -> String {
        let mut parts: Vec<String> = self
            .items
            .iter()
            .map(|i| format!("{}:{}-{}", i.session_id, i.from_offset, i.to_offset))
            .collect();
        parts.sort();
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        crate::ids::h(&refs)
    }
}

pub(super) struct FsPerception {
    layout: AgentLayout,
}

impl FsPerception {
    pub(super) fn new(layout: AgentLayout) -> Self {
        Self { layout }
    }

    fn kind_of(&self, sid: &str) -> Option<SessionKind> {
        fsutil::read_json_opt::<RegistryEntry>(&self.layout.registry_entry(sid))
            .ok()
            .flatten()
            .map(|e| e.kind)
    }
}

#[async_trait]
impl Perception for FsPerception {
    async fn append(&self, lease: &Lease, sid: &str, records: Vec<PerceptionRecord>) -> Result<u64> {
        lease.check()?;
        let _lock = lock_perception_file(&self.layout, sid, PERCEPTION_LOCK_TIMEOUT)?;
        Ok(append_locked(lease, &self.layout.perception_file(sid), records)?.last_seq)
    }

    async fn last_seq(&self, sid: &str) -> Result<u64> {
        let path = self.layout.perception_file(sid);
        let len = fsutil::file_len(&path)?;
        Ok(fsutil::tail_json::<PerceptionRecord>(&path, len)?
            .map(|r| r.seq)
            .unwrap_or(0))
    }

    async fn cursor(&self) -> Result<PerceptionCursor> {
        Ok(fsutil::read_json_opt(&self.layout.perception_cursor())?.unwrap_or_default())
    }

    async fn backlog(&self, cursor: &PerceptionCursor) -> Result<Backlog> {
        let dir = self.layout.perception_dir();
        let mut items = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Backlog::default()),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(sid) = name.strip_suffix(".jsonl") else {
                continue; // also skips `<sid>.jsonl.lock`
            };
            if sid.starts_with('.') {
                continue; // cursor / audit files
            }
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            let off = cursor.offsets.get(sid).copied().unwrap_or(0);
            // Anti self-echo: self-improve sessions do not feed the watermark.
            if size > off && self.kind_of(sid) != Some(SessionKind::SelfImprove) {
                items.push(BacklogItem {
                    session_id: sid.to_string(),
                    from_offset: off,
                    to_offset: size,
                });
            }
        }
        items.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(Backlog { items })
    }

    async fn read(&self, item: &BacklogItem) -> Result<Vec<PerceptionRecord>> {
        let path = self.layout.perception_file(&item.session_id);
        let mut out = Vec::new();
        for line in fsutil::read_range_lines(&path, item.from_offset, item.to_offset)? {
            match serde_json::from_slice::<PerceptionRecord>(&line.bytes) {
                Ok(r) => out.push(r),
                Err(e) => log::warn!(
                    "perception {}: bad line at {}: {e}",
                    item.session_id,
                    line.offset
                ),
            }
        }
        Ok(out)
    }

    async fn commit_cursor(&self, lease: &Lease, cursor: &PerceptionCursor) -> Result<()> {
        if lease.resource() != "self_improve" {
            return Err(OpenDanError::InvalidArgument(
                "the perception cursor is only advanced under the self_improve lease".into(),
            ));
        }
        let cur: PerceptionCursor =
            fsutil::read_json_opt(&self.layout.perception_cursor())?.unwrap_or_default();
        let mut next = cur.clone();
        for (sid, off) in &cursor.offsets {
            let e = next.offsets.entry(sid.clone()).or_insert(0);
            if *off > *e {
                *e = *off; // never moves backwards
            }
        }
        next.updated_at_ms = crate::now_ms();
        lease.fenced(|| fsutil::atomic_replace_json(&self.layout.perception_cursor(), &next))
    }
}

/// Build the automatic `run_digest` record, written when a run ends. `turn`
/// is the Turn the run ended in; `turn_status` is set when that end also
/// closed the Turn (a fork child or a hand-over ends a run, not a Turn).
#[allow(clippy::too_many_arguments)]
pub fn run_digest(
    sid: &str,
    seq: u64,
    run_id: &str,
    turn: u64,
    turn_status: Option<TurnStatus>,
    topic: &Topic,
    objects: Vec<String>,
    summary: &str,
    worklog_seq: u64,
) -> PerceptionRecord {
    PerceptionRecord {
        seq,
        at_ms: crate::now_ms(),
        session_id: sid.to_string(),
        kind: "run_digest".to_string(),
        source: "session".to_string(),
        tags: topic.tags.clone(),
        objects,
        summary: summary.chars().take(500).collect(),
        payload: serde_json::json!({
            "run_id": run_id,
            "turn": turn,
            "turn_status": turn_status,
        }),
        refs: serde_json::json!({ "worklog_seq": worklog_seq }),
        ..Default::default()
    }
}
