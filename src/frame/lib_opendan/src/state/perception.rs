//! Perception stream `state/perception/<sid>.jsonl` (§6.3).
//!
//! Single writer per file (the session driver, under the session lease);
//! `seq` strictly increasing; appends are idempotent on retry. The
//! consolidation cursor is only advanced by the `self_improve` lease holder.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::Lease;
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::Perception;

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
        let path = self.layout.perception_file(sid);
        let len = fsutil::file_len(&path)?;
        let last = fsutil::tail_json::<PerceptionRecord>(&path, len)?
            .map(|r| r.seq)
            .unwrap_or(0);
        let mut fresh: Vec<PerceptionRecord> = records.into_iter().filter(|r| r.seq > last).collect();
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
        if fresh.is_empty() {
            return Ok(last);
        }
        lease.fenced(|| fsutil::append_batch(&path, &fsutil::to_json_lines(&fresh)?))?;
        Ok(prev)
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
                continue;
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

/// Build the automatic `round_digest` record.
pub fn round_digest(
    sid: &str,
    seq: u64,
    round: u64,
    topic: &Topic,
    objects: Vec<String>,
    summary: &str,
    worklog_seq: u64,
) -> PerceptionRecord {
    PerceptionRecord {
        seq,
        at_ms: crate::now_ms(),
        session_id: sid.to_string(),
        kind: "round_digest".to_string(),
        source: "session".to_string(),
        tags: topic.tags.clone(),
        objects,
        summary: summary.chars().take(500).collect(),
        payload: serde_json::json!({ "round": round }),
        refs: serde_json::json!({ "worklog_seq": worklog_seq }),
    }
}
