//! File primitives shared by every implementation language (§4.7).
//!
//! - `atomic_replace`: whole-file replacement (tmp + fsync + rename + dir fsync).
//! - `append_batch`: single-writer append of complete JSON lines + one fsync.
//! - `truncate_to`: drop an uncommitted tail (the only worklog modification).
//! - `reverse_lines`: read complete lines backwards from `end` to `stop`.
//! - `publish_noreplace`: write-once files (binding.json, registry entries).
//! - `publish_dir`: atomic directory publication (session creation).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{OpenDanError, Result};

pub const REVERSE_BLOCK: u64 = 64 * 1024;

fn tmp_name(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    path.with_file_name(format!(".{name}.tmp-{}", uuid::Uuid::new_v4().simple()))
}

/// fsync a directory so a rename / link inside it is durable.
pub fn fsync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let f = File::open(dir).map_err(|e| OpenDanError::io(dir, e))?;
        f.sync_all().map_err(|e| OpenDanError::io(dir, e))?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}

fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn write_tmp(tmp: &Path, data: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)
        .map_err(|e| OpenDanError::io(tmp, e))?;
    f.write_all(data).map_err(|e| OpenDanError::io(tmp, e))?;
    f.sync_all().map_err(|e| OpenDanError::io(tmp, e))?;
    Ok(())
}

/// Replace `path` atomically with `data`. Never use it for lock files: flock
/// is bound to the inode and a rename hands new openers a different inode.
pub fn atomic_replace(path: &Path, data: &[u8]) -> Result<()> {
    let dir = parent_of(path);
    std::fs::create_dir_all(dir).map_err(|e| OpenDanError::io(dir, e))?;
    let tmp = tmp_name(path);
    if let Err(e) = write_tmp(&tmp, data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(OpenDanError::io(path, e));
    }
    fsync_dir(dir)
}

pub fn atomic_replace_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| OpenDanError::json(path, e))?;
    atomic_replace(path, &data)
}

/// Read a JSON file; `Ok(None)` when it does not exist.
pub fn read_json_opt<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| OpenDanError::json(path, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(OpenDanError::io(path, e)),
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    read_json_opt(path)?.ok_or_else(|| OpenDanError::NotFound(path.display().to_string()))
}

/// Append complete lines in one write + one fsync. Returns the file length
/// after the append (the new end position).
pub fn append_batch(path: &Path, data: &[u8]) -> Result<u64> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| OpenDanError::io(dir, e))?;
    }
    let existed = path.exists();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| OpenDanError::io(path, e))?;
    if !data.is_empty() {
        f.write_all(data).map_err(|e| OpenDanError::io(path, e))?;
    }
    f.sync_all().map_err(|e| OpenDanError::io(path, e))?;
    let end = f.metadata().map_err(|e| OpenDanError::io(path, e))?.len();
    if !existed {
        fsync_dir(parent_of(path))?;
    }
    Ok(end)
}

/// Serialize objects as JSON lines.
pub fn to_json_lines<T: Serialize>(objs: &[T]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for o in objs {
        let line = serde_json::to_vec(o).map_err(|e| OpenDanError::Other(e.to_string()))?;
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    Ok(out)
}

/// Truncate `path` to `len` when it is longer (drops an uncommitted tail).
pub fn truncate_to(path: &Path, len: u64) -> Result<bool> {
    let f = match OpenOptions::new().write(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(OpenDanError::io(path, e)),
    };
    let size = f.metadata().map_err(|e| OpenDanError::io(path, e))?.len();
    if size <= len {
        return Ok(false);
    }
    f.set_len(len).map_err(|e| OpenDanError::io(path, e))?;
    f.sync_all().map_err(|e| OpenDanError::io(path, e))?;
    Ok(true)
}

pub fn file_len(path: &Path) -> Result<u64> {
    match std::fs::metadata(path) {
        Ok(m) => Ok(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(OpenDanError::io(path, e)),
    }
}

/// Read `[offset, offset+len)`.
pub fn read_at(path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
    let mut f = File::open(path).map_err(|e| OpenDanError::io(path, e))?;
    f.seek(SeekFrom::Start(offset))
        .map_err(|e| OpenDanError::io(path, e))?;
    let mut buf = vec![0u8; len as usize];
    let mut filled = 0usize;
    while filled < buf.len() {
        let n = f
            .read(&mut buf[filled..])
            .map_err(|e| OpenDanError::io(path, e))?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

/// A line yielded by [`ReverseLines`]: raw bytes and its byte offset.
#[derive(Debug, Clone)]
pub struct LineAt {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// Reads complete lines backwards (newest first) from `end` down to `stop`.
/// Only reads blocks it needs: the amount read is bounded by what the caller
/// consumes, independent of the file size.
pub struct ReverseLines {
    file: Option<File>,
    path: PathBuf,
    pos: u64,
    stop: u64,
    block: u64,
    /// Bytes of the partially read line at the front of the buffer.
    carry: Vec<u8>,
    pending: Vec<LineAt>,
    /// Total bytes read so far (for tests / benchmarks).
    pub bytes_read: u64,
}

impl ReverseLines {
    pub fn open(path: &Path, end: u64, stop: u64) -> Result<Self> {
        let file = match File::open(path) {
            Ok(f) => Some(f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(OpenDanError::io(path, e)),
        };
        let size = match &file {
            Some(f) => f.metadata().map_err(|e| OpenDanError::io(path, e))?.len(),
            None => 0,
        };
        let end = end.min(size);
        Ok(Self {
            file,
            path: path.to_path_buf(),
            pos: end,
            stop: stop.min(end),
            block: REVERSE_BLOCK,
            carry: Vec::new(),
            pending: Vec::new(),
            bytes_read: 0,
        })
    }

    pub fn with_block(mut self, block: u64) -> Self {
        self.block = block.max(1);
        self
    }

    fn fill(&mut self) -> Result<bool> {
        let Some(file) = self.file.as_mut() else {
            return Ok(false);
        };
        if self.pos <= self.stop {
            if !self.carry.is_empty() {
                let bytes = std::mem::take(&mut self.carry);
                let offset = self.pos;
                if !bytes.is_empty() {
                    self.pending.push(LineAt { offset, bytes });
                }
                return Ok(true);
            }
            return Ok(false);
        }
        let n = self.block.min(self.pos - self.stop);
        self.pos -= n;
        file.seek(SeekFrom::Start(self.pos))
            .map_err(|e| OpenDanError::io(&self.path, e))?;
        let mut chunk = vec![0u8; n as usize];
        file.read_exact(&mut chunk)
            .map_err(|e| OpenDanError::io(&self.path, e))?;
        self.bytes_read += n;
        chunk.extend_from_slice(&self.carry);
        self.carry.clear();
        // Split into lines; the first piece may be a partial line unless we
        // reached `stop`.
        let mut starts = vec![0usize];
        for (i, b) in chunk.iter().enumerate() {
            if *b == b'\n' && i + 1 <= chunk.len() {
                starts.push(i + 1);
            }
        }
        let mut lines: Vec<(usize, usize)> = Vec::new();
        for w in 0..starts.len() {
            let s = starts[w];
            let e = if w + 1 < starts.len() {
                starts[w + 1] - 1
            } else {
                chunk.len()
            };
            lines.push((s, e));
        }
        let first_complete = if self.pos > self.stop { 1 } else { 0 };
        if self.pos > self.stop {
            let (s, e) = lines[0];
            self.carry = chunk[s..e].to_vec();
        }
        // pending is consumed from the back (newest first): push oldest first.
        let mut batch = Vec::new();
        for &(s, e) in lines.iter().skip(first_complete) {
            if e > s {
                batch.push(LineAt {
                    offset: self.pos + s as u64,
                    bytes: chunk[s..e].to_vec(),
                });
            }
        }
        // pending holds items to pop from the end → keep chronological order
        // and pop from the back.
        let mut rest = std::mem::take(&mut self.pending);
        batch.append(&mut rest);
        self.pending = batch;
        Ok(true)
    }

    pub fn next_line(&mut self) -> Result<Option<LineAt>> {
        loop {
            if let Some(l) = self.pending.pop() {
                return Ok(Some(l));
            }
            if !self.fill()? {
                return Ok(None);
            }
        }
    }

    /// Next line parsed as JSON; lines that fail to parse are errors.
    pub fn next_json<T: DeserializeOwned>(&mut self) -> Result<Option<(u64, T)>> {
        match self.next_line()? {
            None => Ok(None),
            Some(l) => serde_json::from_slice(&l.bytes)
                .map(|v| Some((l.offset, v)))
                .map_err(|e| OpenDanError::json(&self.path, format!("line at {}: {e}", l.offset))),
        }
    }
}

/// Read complete lines in `[start, end)` in file order (bounded forward read,
/// used only by compaction and audit/export tools).
pub fn read_range_lines(path: &Path, start: u64, end: u64) -> Result<Vec<LineAt>> {
    if end <= start {
        return Ok(Vec::new());
    }
    let data = read_at(path, start, end - start)?;
    let mut out = Vec::new();
    let mut s = 0usize;
    for (i, b) in data.iter().enumerate() {
        if *b == b'\n' {
            if i > s {
                out.push(LineAt {
                    offset: start + s as u64,
                    bytes: data[s..i].to_vec(),
                });
            }
            s = i + 1;
        }
    }
    if s < data.len() {
        out.push(LineAt {
            offset: start + s as u64,
            bytes: data[s..].to_vec(),
        });
    }
    Ok(out)
}

/// Last complete line of `[0, end)` parsed as JSON.
pub fn tail_json<T: DeserializeOwned>(path: &Path, end: u64) -> Result<Option<T>> {
    let mut r = ReverseLines::open(path, end, 0)?;
    Ok(r.next_json::<T>()?.map(|(_, v)| v))
}

/// Publish a write-once file. Returns `false` when `path` already exists
/// (the existing content is left untouched).
pub fn publish_noreplace(path: &Path, data: &[u8]) -> Result<bool> {
    let dir = parent_of(path);
    std::fs::create_dir_all(dir).map_err(|e| OpenDanError::io(dir, e))?;
    let tmp = tmp_name(path);
    write_tmp(&tmp, data)?;
    let res = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match res {
        Ok(()) => {
            fsync_dir(dir)?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(OpenDanError::io(path, e)),
    }
}

pub fn publish_noreplace_json<T: Serialize>(path: &Path, value: &T) -> Result<bool> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| OpenDanError::json(path, e))?;
    publish_noreplace(path, &data)
}

/// Atomically publish a directory: `false` when `dst` already exists.
pub fn publish_dir(tmp: &Path, dst: &Path) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let from = CString::new(tmp.as_os_str().as_bytes())
            .map_err(|e| OpenDanError::InvalidArgument(e.to_string()))?;
        let to = CString::new(dst.as_os_str().as_bytes())
            .map_err(|e| OpenDanError::InvalidArgument(e.to_string()))?;
        // renameat2(RENAME_NOREPLACE): never replaces an existing (even empty)
        // directory, unlike rename(2).
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            fsync_dir(parent_of(dst))?;
            return Ok(true);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(code) if code == libc::EEXIST || code == libc::ENOTEMPTY => return Ok(false),
            Some(code) if code == libc::ENOSYS || code == libc::EINVAL => {
                // Filesystem without renameat2 support: fall through.
            }
            _ => return Err(OpenDanError::io(dst, err)),
        }
    }
    // Portable fallback: check-then-rename (a tiny race window remains; the
    // registry publication catches duplicates).
    if dst.exists() {
        return Ok(false);
    }
    std::fs::rename(tmp, dst).map_err(|e| OpenDanError::io(dst, e))?;
    fsync_dir(parent_of(dst))?;
    Ok(true)
}

/// Relative path check for attachment references (§4.1 rule 6).
pub fn is_safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
