//! Unix platform layer of the worker. Paths are checked on the opened
//! descriptor (`/proc/self/fd/N` on Linux, `F_GETPATH` on macOS); writes go
//! through a verified parent directory descriptor.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use buckyos_api::{ExecutorError, ExecutorErrorCode, ExecutorPathType};

use super::{ensure_under, err, io_err, resolve_entry};

/// Permission bits carried over when a file is replaced.
pub type Mode = u32;

pub fn prepare() {
    unsafe {
        libc::umask(0o022);
    }
}

pub fn display(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub fn opened_path(file: &File) -> io::Result<PathBuf> {
    fd_path(file.as_raw_fd())
}

#[cfg(target_os = "linux")]
fn fd_path(fd: i32) -> io::Result<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
}

#[cfg(target_os = "macos")]
fn fd_path(fd: i32) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0u8; libc::MAXPATHLEN as usize];
    if unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    buf.truncate(len);
    Ok(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

/// A write target: a verified parent directory handle plus the entry name.
pub struct Target {
    dir: OwnedFd,
    dir_path: PathBuf,
    name: CString,
    name_str: String,
}

impl Target {
    /// An existing entry (possibly a symlink) is written through to its target.
    pub fn open(path: &str, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        Self::at(path, &resolve_entry(path, true)?, roots)
    }

    /// The entry itself, a symlink included (rename / remove).
    pub fn entry(path: &str, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        Self::at(path, &resolve_entry(path, false)?, roots)
    }

    fn at(path: &str, resolved: &Path, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        let parent = resolved
            .parent()
            .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no parent")))?;
        let name_str = resolved
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|n| n != "." && n != "..")
            .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no file name")))?;
        let c_parent = CString::new(parent.as_os_str().as_encoded_bytes())
            .map_err(|_| err(ExecutorErrorCode::InvalidArgument, "path contains NUL"))?;
        let fd = unsafe {
            libc::open(
                c_parent.as_ptr(),
                libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io_err(path, io::Error::last_os_error()));
        }
        let dir = unsafe { OwnedFd::from_raw_fd(fd) };
        let dir_path = fd_path(dir.as_raw_fd()).map_err(|e| io_err(path, e))?;
        ensure_under(&dir_path.join(&name_str), roots, path)?;
        let name = CString::new(name_str.clone())
            .map_err(|_| err(ExecutorErrorCode::InvalidArgument, "path contains NUL"))?;
        Ok(Target {
            dir,
            dir_path,
            name,
            name_str,
        })
    }

    pub fn real_path(&self) -> PathBuf {
        self.dir_path.join(&self.name_str)
    }

    fn shown(&self) -> String {
        self.real_path().to_string_lossy().to_string()
    }

    /// What the entry is (a symlink is `other`) and a file's size.
    pub fn kind(&self) -> Result<(ExecutorPathType, u64), ExecutorError> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::fstatat(self.dir.as_raw_fd(), self.name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        if rc != 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::NotFound {
                return Ok((ExecutorPathType::Missing, 0));
            }
            return Err(io_err(&self.shown(), e));
        }
        Ok(match st.st_mode & libc::S_IFMT {
            libc::S_IFREG => (ExecutorPathType::File, st.st_size as u64),
            libc::S_IFDIR => (ExecutorPathType::Directory, 0),
            _ => (ExecutorPathType::Other, 0),
        })
    }

    fn open_at(&self, flags: libc::c_int) -> io::Result<File> {
        let fd = unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                self.name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o644 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// The existing file opened for appending (and reading), `None` when absent.
    pub fn open_append(&self) -> Result<Option<File>, ExecutorError> {
        match self.open_at(libc::O_RDWR | libc::O_APPEND) {
            Ok(f) => Ok(Some(f)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&self.shown(), e)),
        }
    }

    pub fn create_append(&self) -> Result<File, ExecutorError> {
        self.open_at(libc::O_RDWR | libc::O_APPEND | libc::O_CREAT | libc::O_EXCL)
            .map_err(|e| io_err(&self.shown(), e))
    }

    pub fn sync_dir(&self) {
        unsafe {
            libc::fsync(self.dir.as_raw_fd());
        }
    }

    /// Move this entry to `to`. Without `replace` the move is a hard link
    /// plus unlink, which fails on an existing `to` atomically; where the
    /// file system has no hard links it falls back to a checked rename.
    pub fn rename_to(&self, to: &Target, replace: bool) -> Result<(), ExecutorError> {
        let shown = to.shown();
        let rename = || {
            let rc = unsafe { libc::renameat(self.dir.as_raw_fd(), self.name.as_ptr(), to.dir.as_raw_fd(), to.name.as_ptr()) };
            if rc != 0 {
                return Err(io_err(&shown, io::Error::last_os_error()));
            }
            Ok(())
        };
        if replace {
            rename()?;
        } else {
            let rc = unsafe { libc::linkat(self.dir.as_raw_fd(), self.name.as_ptr(), to.dir.as_raw_fd(), to.name.as_ptr(), 0) };
            if rc == 0 {
                unsafe {
                    libc::unlinkat(self.dir.as_raw_fd(), self.name.as_ptr(), 0);
                }
            } else {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::AlreadyExists {
                    return Err(err(ExecutorErrorCode::Conflict, format!("{shown} already exists")));
                }
                if to.kind()?.0 != ExecutorPathType::Missing {
                    return Err(err(ExecutorErrorCode::Conflict, format!("{shown} already exists")));
                }
                rename()?;
            }
        }
        to.sync_dir();
        self.sync_dir();
        Ok(())
    }

    pub fn remove(&self) -> Result<(), ExecutorError> {
        let rc = unsafe { libc::unlinkat(self.dir.as_raw_fd(), self.name.as_ptr(), 0) };
        if rc != 0 {
            return Err(io_err(&self.shown(), io::Error::last_os_error()));
        }
        self.sync_dir();
        Ok(())
    }

    /// Current content and permission bits, `None` when absent.
    pub fn read_existing(&self, max_bytes: u64) -> Result<Option<(Vec<u8>, Mode)>, ExecutorError> {
        let shown = self.real_path().to_string_lossy().to_string();
        let fd = unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                self.name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(io_err(&shown, e));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
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
        use std::os::unix::fs::PermissionsExt;
        Ok(Some((data, meta.permissions().mode() & 0o7777)))
    }

    /// Temp file in the same directory + fsync + renameat + dir fsync.
    pub fn replace(&self, data: &[u8], mode: Option<Mode>) -> Result<(), ExecutorError> {
        let shown = self.real_path().to_string_lossy().to_string();
        let tmp_name = format!(".{}.nx-{}", self.name_str, uuid::Uuid::new_v4().simple());
        let c_tmp = CString::new(tmp_name).unwrap();
        let fd = unsafe {
            libc::openat(
                self.dir.as_raw_fd(),
                c_tmp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode.unwrap_or(0o644) as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(io_err(&shown, io::Error::last_os_error()));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let cleanup = || unsafe {
            libc::unlinkat(self.dir.as_raw_fd(), c_tmp.as_ptr(), 0);
        };
        if let Some(mode) = mode {
            unsafe {
                libc::fchmod(file.as_raw_fd(), mode as libc::mode_t);
            }
        }
        if let Err(e) = file.write_all(data).and_then(|_| file.sync_all()) {
            cleanup();
            return Err(io_err(&shown, e));
        }
        drop(file);
        let rc = unsafe {
            libc::renameat(
                self.dir.as_raw_fd(),
                c_tmp.as_ptr(),
                self.dir.as_raw_fd(),
                self.name.as_ptr(),
            )
        };
        if rc != 0 {
            let e = io::Error::last_os_error();
            cleanup();
            return Err(io_err(&shown, e));
        }
        unsafe {
            libc::fsync(self.dir.as_raw_fd());
        }
        Ok(())
    }
}
