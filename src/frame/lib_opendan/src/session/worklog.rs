//! `worklog.jsonl` access: batch append, tail truncation, reverse reads.
//!
//! The runtime never scans the whole file: reads go backwards from the
//! committed end and stop at the summary start offset (or when the caller has
//! enough). Forward reads serve compaction, audit, and incremental pages.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::fsutil::{self, ReverseLines};
use crate::lock::Lease;
use crate::protocol::{WorklogBody, WorklogEntry};

#[derive(Debug, Serialize)]
pub struct WorklogPage {
    pub entries: Vec<WorklogEntry>,
    pub next_before: Option<u64>,
    pub next_after: u64,
}

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

    pub fn page(
        &self,
        committed: u64,
        turn: u64,
        before: Option<u64>,
        after: Option<u64>,
        limit: usize,
    ) -> Result<WorklogPage> {
        use crate::error::OpenDanError;
        if (before.is_some() && after.is_some()) || limit == 0 || limit > 200 {
            return Err(OpenDanError::InvalidArgument("invalid worklog page options".into()));
        }
        let cursor = before.or(after).unwrap_or(committed);
        if cursor > committed {
            return Err(OpenDanError::InvalidArgument("worklog cursor exceeds committed history".into()));
        }
        let io = |e| OpenDanError::io(&self.path, e);
        let mut file = File::open(&self.path).map_err(io)?;
        if cursor > 0 {
            file.seek(SeekFrom::Start(cursor - 1)).map_err(io)?;
            let mut byte = [0];
            file.read_exact(&mut byte).map_err(io)?;
            if byte[0] != b'\n' {
                return Err(OpenDanError::InvalidArgument("worklog cursor is not a record boundary".into()));
            }
        }
        let mut entries = Vec::new();
        if after.is_some() {
            file.seek(SeekFrom::Start(cursor)).map_err(io)?;
            let mut reader = BufReader::new(file.take(committed - cursor));
            let mut offset = cursor;
            let mut line = Vec::new();
            for _ in 0..2000 {
                line.clear();
                let len = reader.read_until(b'\n', &mut line).map_err(io)?;
                if len == 0 { break; }
                offset += len as u64;
                let entry: WorklogEntry = serde_json::from_slice(&line)
                    .map_err(|e| OpenDanError::json(&self.path, e))?;
                if entry.body.turn() == Some(turn) { entries.push(entry); }
                if entries.len() == limit { break; }
            }
            return Ok(WorklogPage { entries, next_before: None, next_after: offset });
        }
        let mut reader = self.reverse(cursor, 0)?;
        let mut offset = cursor;
        for _ in 0..2000 {
            let Some((start, entry)) = reader.next_json::<WorklogEntry>()? else {
                offset = 0;
                break;
            };
            offset = start;
            if entry.body.turn().is_some_and(|t| t < turn) {
                offset = 0;
                break;
            }
            if entry.body.turn() == Some(turn) { entries.push(entry); }
            if entries.len() == limit { break; }
        }
        entries.reverse();
        Ok(WorklogPage { entries, next_before: (offset > 0).then_some(offset), next_after: committed })
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
