//! Long-held exclusive file locks (§5).
//!
//! The right to advance a session (or execute a run, run self-improve, move an
//! artifact head) is an exclusive `flock` held for the whole critical period.
//! The OS releases it when the holder exits or crashes; there is no TTL, no
//! renewal and no clock dependency. The lock file content is holder
//! information, rewritten **in place** through the same descriptor — lock
//! files are never replaced (flock lives on the inode) and never deleted.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use fs2::FileExt;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::protocol::{HolderInfo, LockInfo};

fn held_table() -> &'static Mutex<HashSet<(u64, u64)>> {
    static T: OnceLock<Mutex<HashSet<(u64, u64)>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(unix)]
fn file_key(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn file_key(meta: &std::fs::Metadata) -> (u64, u64) {
    (0, meta.len())
}

/// An exclusive flock on a lock file. Dropping it releases the lock.
///
/// The descriptor is opened with `O_CLOEXEC` (Rust's default) so processes
/// started by `shell` never inherit it.
pub struct FileLock {
    file: File,
    path: PathBuf,
    key: (u64, u64),
}

impl std::fmt::Debug for FileLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLock").field("path", &self.path).finish()
    }
}

impl FileLock {
    /// Non-blocking exclusive lock; `Ok(None)` when another holder (another
    /// process, or another descriptor in this process) has it.
    pub fn try_acquire(path: &Path) -> Result<Option<FileLock>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| OpenDanError::io(dir, e))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| OpenDanError::io(path, e))?;
        let meta = file.metadata().map_err(|e| OpenDanError::io(path, e))?;
        let key = file_key(&meta);
        {
            let table = held_table().lock().expect("lock table");
            if table.contains(&key) {
                return Ok(None);
            }
        }
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                return Ok(None)
            }
            Err(e) => return Err(OpenDanError::io(path, e)),
        }
        let mut table = held_table().lock().expect("lock table");
        if !table.insert(key) {
            // Raced with another acquirer in this process.
            let _ = FileExt::unlock(&file);
            return Ok(None);
        }
        Ok(Some(FileLock {
            file,
            path: path.to_path_buf(),
            key,
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the lock file content through the locked descriptor.
    pub fn read_content(&self) -> Result<Vec<u8>> {
        read_all_at(&self.file, &self.path)
    }

    /// Rewrite the lock file content in place (pwrite + ftruncate + fsync).
    pub fn rewrite(&self, data: &[u8]) -> Result<()> {
        write_all_at(&self.file, data, &self.path)?;
        self.file
            .set_len(data.len() as u64)
            .map_err(|e| OpenDanError::io(&self.path, e))?;
        self.file
            .sync_all()
            .map_err(|e| OpenDanError::io(&self.path, e))?;
        Ok(())
    }

    /// Still holding the lock on the file currently at `path`? Detects a lock
    /// file that was replaced or deleted (a protocol violation that would let
    /// a second holder in).
    pub fn still_valid(&self) -> bool {
        match std::fs::metadata(&self.path) {
            Ok(meta) => file_key(&meta) == self.key,
            Err(_) => false,
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        if let Ok(mut t) = held_table().lock() {
            t.remove(&self.key);
        }
    }
}

#[cfg(unix)]
fn read_all_at(file: &File, path: &Path) -> Result<Vec<u8>> {
    use std::os::unix::fs::FileExt as _;
    let len = file.metadata().map_err(|e| OpenDanError::io(path, e))?.len();
    let mut buf = vec![0u8; len as usize];
    let mut off = 0usize;
    while off < buf.len() {
        let n = file
            .read_at(&mut buf[off..], off as u64)
            .map_err(|e| OpenDanError::io(path, e))?;
        if n == 0 {
            break;
        }
        off += n;
    }
    buf.truncate(off);
    Ok(buf)
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], path: &Path) -> Result<()> {
    use std::os::unix::fs::FileExt as _;
    file.write_all_at(data, 0)
        .map_err(|e| OpenDanError::io(path, e))
}

#[cfg(not(unix))]
fn read_all_at(file: &File, path: &Path) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = file.try_clone().map_err(|e| OpenDanError::io(path, e))?;
    f.seek(SeekFrom::Start(0)).map_err(|e| OpenDanError::io(path, e))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| OpenDanError::io(path, e))?;
    Ok(buf)
}

#[cfg(not(unix))]
fn write_all_at(file: &File, data: &[u8], path: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = file.try_clone().map_err(|e| OpenDanError::io(path, e))?;
    f.seek(SeekFrom::Start(0)).map_err(|e| OpenDanError::io(path, e))?;
    f.write_all(data).map_err(|e| OpenDanError::io(path, e))
}

/// Read the holder info of a lock file without locking (display only; a
/// half-written file is re-read once, then reported as unknown).
pub fn read_holder_info(path: &Path) -> Option<LockInfo> {
    for _ in 0..2 {
        match std::fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => {
                if let Ok(info) = serde_json::from_slice::<LockInfo>(&bytes) {
                    return Some(info);
                }
            }
            _ => return None,
        }
    }
    None
}

/// Result of a lease acquisition attempt.
pub enum Acquire {
    Acquired(Lease),
    Busy(Option<LockInfo>),
}

impl Acquire {
    pub fn into_result(self, resource: &str) -> Result<Lease> {
        match self {
            Acquire::Acquired(l) => Ok(l),
            Acquire::Busy(info) => Err(OpenDanError::Busy {
                resource: resource.to_string(),
                holder: info.and_then(|i| serde_json::to_value(i).ok()),
            }),
        }
    }
}

/// A held lock plus the epoch of this acquisition.
pub struct Lease {
    lock: FileLock,
    info: LockInfo,
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("resource", &self.info.resource)
            .field("epoch", &self.info.epoch)
            .finish()
    }
}

impl Lease {
    /// Try to take `resource` using `path` as its lock file.
    pub fn acquire(resource: &str, path: &Path, holder: HolderInfo) -> Result<Acquire> {
        let Some(lock) = FileLock::try_acquire(path)? else {
            return Ok(Acquire::Busy(read_holder_info(path)));
        };
        let prev = lock
            .read_content()
            .ok()
            .and_then(|b| serde_json::from_slice::<LockInfo>(&b).ok());
        let info = LockInfo {
            resource: resource.to_string(),
            epoch: prev.map(|p| p.epoch).unwrap_or(0) + 1,
            holder,
            acquired_at_ms: crate::now_ms(),
            released_at_ms: None,
        };
        let data = serde_json::to_vec_pretty(&info).map_err(|e| OpenDanError::json(path, e))?;
        lock.rewrite(&data)?;
        Ok(Acquire::Acquired(Lease { lock, info }))
    }

    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    pub fn epoch(&self) -> u64 {
        self.info.epoch
    }

    pub fn resource(&self) -> &str {
        &self.info.resource
    }

    /// Admission check for new writes / side effects. On a single node a
    /// flock never disappears while the process lives; the check still
    /// catches a replaced or deleted lock file.
    pub fn held(&self) -> bool {
        self.lock.still_valid()
    }

    pub fn check(&self) -> Result<()> {
        if self.held() {
            Ok(())
        } else {
            Err(OpenDanError::LeaseLost(self.info.resource.clone()))
        }
    }

    /// Run a write under the lease (`fenced` in the plan).
    pub fn fenced<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.check()?;
        f()
    }

    /// Release explicitly: record `released_at_ms`, then unlock.
    pub fn release(mut self) {
        self.info.released_at_ms = Some(crate::now_ms());
        if let Ok(data) = serde_json::to_vec_pretty(&self.info) {
            let _ = self.lock.rewrite(&data);
        }
        drop(self);
    }

    pub fn holder_json(&self) -> Value {
        serde_json::to_value(&self.info).unwrap_or(Value::Null)
    }
}
