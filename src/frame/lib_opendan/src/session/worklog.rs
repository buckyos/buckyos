//! `worklog.jsonl` access: batch append, tail truncation, reverse reads.
//!
//! The runtime never scans the whole file: reads go backwards from the
//! committed end and stop at the summary start offset (or when the caller has
//! enough). The only forward read is compaction's bounded range.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::fsutil::{self, ReverseLines};
use crate::lock::Lease;
use crate::protocol::{WorklogBody, WorklogEntry};

#[derive(Debug, Clone)]
pub struct Worklog {
    path: PathBuf,
}

impl Worklog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> Result<u64> {
        fsutil::file_len(&self.path)
    }

    /// Append entries numbered from `first_seq`; one write + one fsync.
    /// Returns `(last_seq, end_offset)`.
    pub fn append(
        &self,
        lease: &Lease,
        first_seq: u64,
        bodies: Vec<WorklogBody>,
    ) -> Result<(u64, u64)> {
        lease.check()?;
        let mut seq = first_seq;
        let mut entries = Vec::with_capacity(bodies.len());
        for body in bodies {
            entries.push(WorklogEntry { seq, body });
            seq += 1;
        }
        let data = fsutil::to_json_lines(&entries)?;
        let end = fsutil::append_batch(&self.path, &data)?;
        Ok((seq.saturating_sub(1).max(first_seq.saturating_sub(1)), end))
    }

    /// Drop the uncommitted tail after `committed_bytes`.
    pub fn truncate_to(&self, lease: &Lease, committed_bytes: u64) -> Result<bool> {
        lease.check()?;
        fsutil::truncate_to(&self.path, committed_bytes)
    }

    /// Reverse reader from `end` down to `stop` (newest first).
    pub fn reverse(&self, end: u64, stop: u64) -> Result<ReverseLines> {
        ReverseLines::open(&self.path, end, stop)
    }

    /// Up to `n` most recent committed entries (newest first).
    pub fn recent(&self, end: u64, n: usize) -> Result<Vec<(u64, WorklogEntry)>> {
        let mut r = self.reverse(end, 0)?;
        let mut out = Vec::new();
        while out.len() < n {
            match r.next_json::<WorklogEntry>()? {
                Some(e) => out.push(e),
                None => break,
            }
        }
        Ok(out)
    }

    /// Bounded forward read of `[start, end)` (compaction / audit).
    pub fn read_range(&self, start: u64, end: u64) -> Result<Vec<(u64, WorklogEntry)>> {
        let mut out = Vec::new();
        for line in fsutil::read_range_lines(&self.path, start, end)? {
            let e: WorklogEntry = serde_json::from_slice(&line.bytes)
                .map_err(|e| crate::error::OpenDanError::json(&self.path, e))?;
            out.push((line.offset, e));
        }
        Ok(out)
    }

    /// Full forward scan — audit / export tools only.
    pub fn read_all_for_audit(&self) -> Result<Vec<WorklogEntry>> {
        let len = self.len()?;
        Ok(self.read_range(0, len)?.into_iter().map(|(_, e)| e).collect())
    }
}
