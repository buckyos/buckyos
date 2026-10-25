//! Windows platform layer of the worker, which runs as the host's own user.
//! Paths are checked on the opened handle (`GetFinalPathNameByHandleW`). The
//! parent directory of a write target stays open without FILE_SHARE_DELETE,
//! so it cannot be renamed or replaced while the file is swapped in.

use std::ffi::{c_void, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use buckyos_api::{ExecutorError, ExecutorErrorCode, ExecutorPathType};

use super::{ensure_under, err, io_err, resolve_entry};

const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

#[link(name = "kernel32")]
extern "system" {
    fn GetFinalPathNameByHandleW(file: *mut c_void, path: *mut u16, len: u32, flags: u32) -> u32;
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
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
    /// An existing entry (possibly a symlink) is written through to its target.
    pub fn open(path: &str, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        Self::at(path, &resolve_entry(path, true)?, roots)
    }

    /// The entry itself, a reparse point included (rename / remove).
    pub fn entry(path: &str, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        Self::at(path, &resolve_entry(path, false)?, roots)
    }

    fn at(path: &str, resolved: &Path, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
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

    /// What the entry is (a reparse point is `other`) and a file's size.
    pub fn kind(&self) -> Result<(ExecutorPathType, u64), ExecutorError> {
        match std::fs::symlink_metadata(self.real_path()) {
            Ok(m) if m.file_type().is_symlink() => Ok((ExecutorPathType::Other, 0)),
            Ok(m) if m.is_file() => Ok((ExecutorPathType::File, m.len())),
            Ok(m) if m.is_dir() => Ok((ExecutorPathType::Directory, 0)),
            Ok(_) => Ok((ExecutorPathType::Other, 0)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((ExecutorPathType::Missing, 0)),
            Err(e) => Err(io_err(&display(&self.real_path()), e)),
        }
    }

    /// The existing file opened for appending (and reading), `None` when absent.
    pub fn open_append(&self) -> Result<Option<File>, ExecutorError> {
        match OpenOptions::new()
            .read(true)
            .append(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.real_path())
        {
            Ok(f) => Ok(Some(f)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&display(&self.real_path()), e)),
        }
    }

    pub fn create_append(&self) -> Result<File, ExecutorError> {
        OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .open(self.real_path())
            .map_err(|e| io_err(&display(&self.real_path()), e))
    }

    pub fn sync_dir(&self) {}

    /// Move this entry to `to`; without `replace` an existing `to` fails the
    /// move itself.
    pub fn rename_to(&self, to: &Target, replace: bool) -> Result<(), ExecutorError> {
        let wide = |p: PathBuf| p.as_os_str().encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (from, dest) = (wide(self.real_path()), wide(to.real_path()));
        let flags = MOVEFILE_WRITE_THROUGH | if replace { MOVEFILE_REPLACE_EXISTING } else { 0 };
        if unsafe { MoveFileExW(from.as_ptr(), dest.as_ptr(), flags) } == 0 {
            let e = io::Error::last_os_error();
            let shown = display(&to.real_path());
            if e.kind() == io::ErrorKind::AlreadyExists {
                return Err(err(ExecutorErrorCode::Conflict, format!("{shown} already exists")));
            }
            return Err(io_err(&shown, e));
        }
        Ok(())
    }

    pub fn remove(&self) -> Result<(), ExecutorError> {
        std::fs::remove_file(self.real_path()).map_err(|e| io_err(&display(&self.real_path()), e))
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
