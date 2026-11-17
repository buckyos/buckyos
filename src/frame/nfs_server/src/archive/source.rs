//! The bytes behind an index, whatever the format: a cloneable positional
//! reader over the outer file (or the stream an outer decompresses to), the
//! stamp a reopened file must still match, and the pump every member read
//! goes through.

use crate::error::*;
use serde_json::json;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the outer file must still be when it is opened again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub ino: u64,
    pub dev: u64,
    pub size: u64,
    pub mtime: i64,
}

impl Stamp {
    pub fn of(meta: &std::fs::Metadata) -> Stamp {
        let id = crate::fsutil::file_id(meta);
        let n = crate::fsutil::node_meta(meta);
        Stamp { ino: id.ino, dev: id.dev, size: meta.len(), mtime: n.mtime }
    }
}

#[derive(Debug)]
pub struct StaleOuter;

impl std::fmt::Display for StaleOuter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the archive changed since it was indexed")
    }
}

impl std::error::Error for StaleOuter {}

pub fn is_stale(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<StaleOuter>())
}

/// A file behind an index. The descriptor closes when idle and is reopened
/// on demand; a reopened file must match the stamp it was indexed at.
#[derive(Debug)]
pub struct Source {
    path: PathBuf,
    stamp: Option<Stamp>,
    pub len: u64,
    slot: Mutex<Option<(Arc<File>, Instant)>>,
}

impl Source {
    pub fn open(path: PathBuf, stamp: Option<Stamp>) -> io::Result<Arc<Source>> {
        let file = File::open(&path)?;
        let meta = file.metadata()?;
        if let Some(s) = &stamp {
            if Stamp::of(&meta) != *s {
                return Err(io::Error::other(StaleOuter));
            }
        }
        Ok(Arc::new(Source { path, stamp, len: meta.len(), slot: Mutex::new(Some((Arc::new(file), Instant::now()))) }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn file(&self) -> io::Result<Arc<File>> {
        let mut slot = self.slot.lock().unwrap();
        if let Some((f, used)) = slot.as_mut() {
            *used = Instant::now();
            return Ok(f.clone());
        }
        let file = File::open(&self.path)?;
        let meta = file.metadata()?;
        if meta.len() != self.len || self.stamp.as_ref().is_some_and(|s| Stamp::of(&meta) != *s) {
            return Err(io::Error::other(StaleOuter));
        }
        let file = Arc::new(file);
        *slot = Some((file.clone(), Instant::now()));
        Ok(file)
    }

    /// A descriptor of the indexed bytes (a replaced file reads as stale).
    pub fn std_file(&self) -> io::Result<File> {
        self.file()?.try_clone()
    }

    /// Drops an idle descriptor (readers in flight keep their own).
    pub fn close_idle(&self, idle: Duration) {
        let mut slot = self.slot.lock().unwrap();
        if slot.as_ref().is_some_and(|(_, used)| used.elapsed() >= idle) {
            *slot = None;
        }
    }

    pub fn reader(self: &Arc<Self>) -> PosReader {
        PosReader { src: self.clone(), file: None, pos: 0 }
    }
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(f, buf, off)
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(f, buf, off)
}

/// `Read + Seek` by position over a shared descriptor; clones keep their own cursor.
#[derive(Debug, Clone)]
pub struct PosReader {
    src: Arc<Source>,
    file: Option<Arc<File>>,
    pos: u64,
}

impl PosReader {
    pub fn size(&self) -> u64 {
        self.src.len
    }
}

impl Read for PosReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.file.is_none() {
            self.file = Some(self.src.file()?);
        }
        let n = read_at(self.file.as_ref().unwrap(), buf, self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for PosReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let next = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.src.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = next.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek out of range"))?;
        Ok(self.pos)
    }
}

pub fn read_exact_at(r: &mut PosReader, at: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    r.seek(SeekFrom::Start(at))?;
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Index limits shared by every format.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_central_directory_bytes: u64,
    pub max_entries: u64,
    pub max_materialize_bytes: u64,
    pub max_decoder_memory: u64,
}

pub fn quota(what: &str, limit: u64) -> NfsError {
    NfsError::new(ErrorCode::QuotaExceeded, format!("the archive's {what} exceeds the limit")).with("limit", json!(limit))
}

pub fn corrupt(msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::ArchiveCorrupt, msg)
}

pub fn integrity(msg: impl Into<String>) -> NfsError {
    corrupt(msg).with("reason", json!("decrypt_or_integrity"))
}

pub fn unsupported(reason: &str, msg: impl Into<String>) -> NfsError {
    NfsError::new(ErrorCode::Unsupported, msg).with("reason", json!(reason))
}

pub fn io_error(e: io::Error) -> NfsError {
    if is_stale(&e) {
        return stale("the archive changed; open it again");
    }
    match e.kind() {
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => corrupt(format!("archive data: {e}")),
        _ => NfsError::from(e),
    }
}

/// Feeds exactly the declared size of a decoding member to `sink`, then
/// checks the member ends there (decoders check CRC / authentication when
/// they reach the end). `limit` stops early (a partial range): no end check.
pub fn pump(mut r: impl Read, declared: u64, limit: Option<u64>, cancel: &AtomicBool, sink: &mut dyn FnMut(&[u8]) -> io::Result<bool>) -> NfsResult<bool> {
    let want = limit.map(|l| l.min(declared)).unwrap_or(declared);
    let mut left = want;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let n = r.read(&mut buf[..left.min(64 * 1024) as usize]).map_err(decode_error)?;
        if n == 0 {
            return Err(integrity("the member is shorter than declared"));
        }
        left -= n as u64;
        if !sink(&buf[..n]).map_err(NfsError::from)? {
            return Ok(false);
        }
    }
    if want < declared {
        return Ok(true);
    }
    let mut extra = [0u8; 1];
    match r.read(&mut extra).map_err(decode_error)? {
        0 => Ok(true),
        _ => Err(integrity("the member is longer than declared")),
    }
}

pub fn decode_error(e: io::Error) -> NfsError {
    if is_stale(&e) {
        return stale("the archive changed; open it again");
    }
    integrity(format!("the member does not decode: {e}"))
}
