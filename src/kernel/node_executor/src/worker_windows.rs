//! Windows platform layer of the worker, which runs as the host's own user.
//! Paths are checked on the opened handle (`GetFinalPathNameByHandleW`). The
//! parent directory of a write target stays open without FILE_SHARE_DELETE,
//! so it cannot be renamed or replaced while the file is swapped in.

use std::ffi::{c_void, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use buckyos_api::{ExecutorError, ExecutorErrorCode};

use super::{ensure_under, err, io_err};

const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[link(name = "kernel32")]
extern "system" {
    fn GetFinalPathNameByHandleW(file: *mut c_void, path: *mut u16, len: u32, flags: u32) -> u32;
}

/// Windows keeps no permission bits to carry over; a replaced file gets the
/// directory's inherited ACL.
pub type Mode = ();

pub fn prepare() {}

/// `\\?\C:\x` → `C:\x` and `\\?\UNC\host\share` → `\\host\share`; other
/// verbatim forms stay as they are.
pub fn display(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => text.to_string(),
    }
}

/// Final path of an open file or directory, in the `\\?\` form that
/// `std::fs::canonicalize` returns.
pub fn opened_path(file: &File) -> io::Result<PathBuf> {
    let mut buf = vec![0u16; 512];
    loop {
        let len = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle() as *mut c_void, buf.as_mut_ptr(), buf.len() as u32, 0)
        } as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len < buf.len() {
            buf.truncate(len);
            return Ok(PathBuf::from(OsString::from_wide(&buf)));
        }
        buf.resize(len, 0);
    }
}

/// A write target: a pinned, verified parent directory plus the entry name.
pub struct Target {
    _dir: File,
    dir_path: PathBuf,
    name: String,
}

impl Target {
    pub fn open(path: &str, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        if !crate::is_host_absolute(path) {
            return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not absolute")));
        }
        // An existing entry (possibly a symlink) is written through to its target.
        let resolved = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let p = Path::new(path);
                let parent = p
                    .parent()
                    .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no parent")))?;
                let name = p
                    .file_name()
                    .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no file name")))?;
                std::fs::canonicalize(parent)
                    .map_err(|e| io_err(path, e))?
                    .join(name)
            }
            Err(e) => return Err(io_err(path, e)),
        };
        let parent = resolved
            .parent()
            .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no parent")))?;
        let name = resolved
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| n != "." && n != "..")
            .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no file name")))?;
        let dir = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent)
            .map_err(|e| io_err(path, e))?;
        if !dir.metadata().map_err(|e| io_err(path, e))?.is_dir() {
            return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path}: parent is not a directory")));
        }
        let dir_path = opened_path(&dir).map_err(|e| io_err(path, e))?;
        ensure_under(&dir_path.join(&name), roots, path)?;
        Ok(Target {
            _dir: dir,
            dir_path,
            name,
        })
    }

    pub fn real_path(&self) -> PathBuf {
        self.dir_path.join(&self.name)
    }

    /// Current content, `None` when absent. A reparse point in place of the
    /// file is not followed.
    pub fn read_existing(&self, max_bytes: u64) -> Result<Option<(Vec<u8>, Mode)>, ExecutorError> {
        let shown = display(&self.real_path());
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.real_path())
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err(&shown, e)),
        };
        let meta = file.metadata().map_err(|e| io_err(&shown, e))?;
        if !meta.is_file() {
            return Err(err(ExecutorErrorCode::InvalidArgument, format!("{shown} is not a regular file")));
        }
        if meta.len() > max_bytes {
            return Err(err(
                ExecutorErrorCode::InvalidArgument,
                format!("{shown}: {} bytes exceed max_file_bytes {max_bytes}", meta.len()),
            ));
        }
        let mut data = Vec::with_capacity(meta.len() as usize);
        (&mut file)
            .take(max_bytes + 1)
            .read_to_end(&mut data)
            .map_err(|e| io_err(&shown, e))?;
        Ok(Some((data, ())))
    }

    /// Temp file in the same directory + flush + replacing rename.
    pub fn replace(&self, data: &[u8], _mode: Option<Mode>) -> Result<(), ExecutorError> {
        let shown = display(&self.real_path());
        let tmp = self
            .dir_path
            .join(format!(".{}.nx-{}", self.name, uuid::Uuid::new_v4().simple()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| io_err(&shown, e))?;
        if let Err(e) = file.write_all(data).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err(&shown, e));
        }
        drop(file);
        if let Err(e) = std::fs::rename(&tmp, self.real_path()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err(&shown, e));
        }
        Ok(())
    }
}
