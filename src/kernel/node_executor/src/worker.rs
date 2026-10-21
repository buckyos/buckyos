//! Worker side: runs as the profile's OS identity (setgroups/setgid/setuid
//! and no_new_privs are applied by the parent at spawn). It performs one file
//! job read from stdin and writes `reply-json\n` + payload to stdout.
//!
//! Paths are checked on the opened descriptor (`/proc/self/fd/N`), so a path
//! swapped after resolution cannot redirect the I/O outside the roots.

use std::ffi::CString;
use std::io::{self, BufRead, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use buckyos_api::{
    executor_content_hash, ExecutorEdit, ExecutorError, ExecutorErrorCode, ExecutorFileResult,
    ExecutorWriteMode,
};
use serde::{Deserialize, Serialize};

pub const WORKER_ARG: &str = "--executor-worker";
const MAX_JOB_LINE: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WorkerJob {
    ResolveDir {
        path: String,
        roots: Vec<String>,
    },
    ResolveTarget {
        path: String,
        roots: Vec<String>,
    },
    Read {
        path: String,
        roots: Vec<String>,
        offset: u64,
        length: Option<u64>,
        max_bytes: u64,
    },
    Write {
        path: String,
        roots: Vec<String>,
        mode: ExecutorWriteMode,
        expected_hash: Option<String>,
        content_length: u64,
        max_bytes: u64,
    },
    Edit {
        path: String,
        roots: Vec<String>,
        edits: Vec<ExecutorEdit>,
        expected_hash: Option<String>,
        max_bytes: u64,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkerReply {
    #[serde(default)]
    pub error: Option<ExecutorError>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub file: Option<ExecutorFileResult>,
    #[serde(default)]
    pub payload_len: u64,
    #[serde(default)]
    pub file_size: u64,
}

fn err(code: ExecutorErrorCode, msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(code, msg)
}

fn io_err(path: &str, e: io::Error) -> ExecutorError {
    let code = match e.kind() {
        io::ErrorKind::NotFound => ExecutorErrorCode::NotFound,
        io::ErrorKind::PermissionDenied => ExecutorErrorCode::Forbidden,
        io::ErrorKind::AlreadyExists => ExecutorErrorCode::Conflict,
        _ => ExecutorErrorCode::InvalidArgument,
    };
    err(code, format!("{path}: {e}"))
}

/// Entry point of the worker process. Never returns.
pub fn worker_main() -> ! {
    unsafe {
        libc::umask(0o022);
    }
    let code = match run_worker() {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(io::stderr(), "executor worker: {e}");
            2
        }
    };
    std::process::exit(code)
}

fn run_worker() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut line = Vec::new();
    (&mut input)
        .take(MAX_JOB_LINE as u64)
        .read_until(b'\n', &mut line)?;
    let job: WorkerJob = serde_json::from_slice(&line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let (reply, payload) = match execute(job, &mut input) {
        Ok(v) => v,
        Err(e) => (
            WorkerReply {
                error: Some(e),
                ..Default::default()
            },
            Vec::new(),
        ),
    };
    let mut reply = reply;
    reply.payload_len = payload.len() as u64;
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &reply)?;
    out.write_all(b"\n")?;
    out.write_all(&payload)?;
    out.flush()
}

fn execute(job: WorkerJob, input: &mut impl Read) -> Result<(WorkerReply, Vec<u8>), ExecutorError> {
    match job {
        WorkerJob::ResolveDir { path, roots } => {
            let roots = canonical_roots(&roots);
            let real = std::fs::canonicalize(&path).map_err(|e| io_err(&path, e))?;
            if !real.is_dir() {
                return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not a directory")));
            }
            ensure_under(&real, &roots, &path)?;
            Ok((
                WorkerReply {
                    path: Some(real.to_string_lossy().to_string()),
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        WorkerJob::ResolveTarget { path, roots } => {
            let roots = canonical_roots(&roots);
            let real = if Path::new(&path).parent().is_some_and(|p| p.exists()) {
                Target::open(&path, &roots)?.real_path()
            } else {
                missing_parent_target(&path, &roots)?
            };
            Ok((
                WorkerReply {
                    path: Some(real.to_string_lossy().to_string()),
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        WorkerJob::Read {
            path,
            roots,
            offset,
            length,
            max_bytes,
        } => {
            if !path.starts_with('/') {
                return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not absolute")));
            }
            let roots = canonical_roots(&roots);
            let file = std::fs::File::open(&path).map_err(|e| io_err(&path, e))?;
            let real = fd_path(file.as_raw_fd()).map_err(|e| io_err(&path, e))?;
            ensure_under(&real, &roots, &path)?;
            let meta = file.metadata().map_err(|e| io_err(&path, e))?;
            if !meta.is_file() {
                return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not a regular file")));
            }
            let size = meta.len();
            let start = offset.min(size);
            let want = length.unwrap_or(size - start).min(size - start);
            if want > max_bytes {
                return Err(err(
                    ExecutorErrorCode::InvalidArgument,
                    format!("{path}: {want} bytes exceed max_file_bytes {max_bytes}; read a range"),
                ));
            }
            let mut data = Vec::with_capacity(want as usize);
            use std::io::Seek;
            let mut file = file;
            file.seek(io::SeekFrom::Start(start)).map_err(|e| io_err(&path, e))?;
            file.take(want).read_to_end(&mut data).map_err(|e| io_err(&path, e))?;
            let whole = start == 0 && data.len() as u64 == size;
            Ok((
                WorkerReply {
                    file: Some(ExecutorFileResult {
                        path: real.to_string_lossy().to_string(),
                        size,
                        hash: if whole { executor_content_hash(&data) } else { String::new() },
                        ..Default::default()
                    }),
                    file_size: size,
                    ..Default::default()
                },
                data,
            ))
        }
        WorkerJob::Write {
            path,
            roots,
            mode,
            expected_hash,
            content_length,
            max_bytes,
        } => {
            let roots = canonical_roots(&roots);
            let mut upload = Vec::with_capacity(content_length.min(max_bytes) as usize);
            input
                .take(content_length)
                .read_to_end(&mut upload)
                .map_err(|e| io_err(&path, e))?;
            if upload.len() as u64 != content_length {
                return Err(err(ExecutorErrorCode::InvalidArgument, "upload truncated"));
            }
            create_parents(&path, &roots)?;
            let target = Target::open(&path, &roots)?;
            let existing = target.read_existing(max_bytes)?;
            if let Some(expected) = &expected_hash {
                match &existing {
                    Some((old, _)) if &executor_content_hash(old) == expected => {}
                    _ => return Err(err(ExecutorErrorCode::Conflict, format!("{path} changed (expected_hash mismatch)"))),
                }
            }
            let new_content = match (mode, &existing) {
                (ExecutorWriteMode::CreateNew, Some(_)) => {
                    return Err(err(ExecutorErrorCode::Conflict, format!("{path} already exists")))
                }
                (ExecutorWriteMode::Append, Some((old, _))) => {
                    let mut v = old.clone();
                    v.extend_from_slice(&upload);
                    v
                }
                _ => upload.clone(),
            };
            if new_content.len() as u64 > max_bytes {
                return Err(err(
                    ExecutorErrorCode::InvalidArgument,
                    format!("{path}: result exceeds max_file_bytes {max_bytes}"),
                ));
            }
            let previous_hash = existing.as_ref().map(|(old, _)| executor_content_hash(old));
            let new_hash = executor_content_hash(&new_content);
            let changed = previous_hash.as_deref() != Some(new_hash.as_str());
            if changed || existing.is_none() {
                target.replace(&new_content, existing.as_ref().map(|(_, mode)| *mode))?;
            }
            Ok((
                WorkerReply {
                    file: Some(ExecutorFileResult {
                        path: target.real_path().to_string_lossy().to_string(),
                        size: new_content.len() as u64,
                        hash: new_hash,
                        previous_hash,
                        created: existing.is_none(),
                        changed,
                        bytes_written: upload.len() as u64,
                        line: None,
                        replacements: 0,
                    }),
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        WorkerJob::Edit {
            path,
            roots,
            edits,
            expected_hash,
            max_bytes,
        } => {
            let roots = canonical_roots(&roots);
            if edits.is_empty() {
                return Err(err(ExecutorErrorCode::InvalidArgument, "edits is empty"));
            }
            let target = Target::open(&path, &roots)?;
            let (original, mode) = target
                .read_existing(max_bytes)?
                .ok_or_else(|| err(ExecutorErrorCode::NotFound, format!("{path}: file not found")))?;
            let previous_hash = executor_content_hash(&original);
            if let Some(expected) = &expected_hash {
                if expected != &previous_hash {
                    return Err(err(ExecutorErrorCode::Conflict, format!("{path} changed (expected_hash mismatch)")));
                }
            }
            let (updated, line, replacements) = apply_edits(&original, &edits)?;
            if updated.len() as u64 > max_bytes {
                return Err(err(
                    ExecutorErrorCode::InvalidArgument,
                    format!("{path}: result exceeds max_file_bytes {max_bytes}"),
                ));
            }
            let new_hash = executor_content_hash(&updated);
            let changed = new_hash != previous_hash;
            if changed {
                target.replace(&updated, Some(mode))?;
            }
            Ok((
                WorkerReply {
                    file: Some(ExecutorFileResult {
                        path: target.real_path().to_string_lossy().to_string(),
                        size: updated.len() as u64,
                        hash: new_hash,
                        previous_hash: Some(previous_hash),
                        created: false,
                        changed,
                        bytes_written: if changed { updated.len() as u64 } else { 0 },
                        line: Some(line),
                        replacements,
                    }),
                    file_size: original.len() as u64,
                    ..Default::default()
                },
                original,
            ))
        }
    }
}

/// Apply edits in order with edit_file semantics: each `old_string` must occur
/// exactly once unless `replace_all`. Returns (content, first line, count).
pub fn apply_edits(original: &[u8], edits: &[ExecutorEdit]) -> Result<(Vec<u8>, u64, u64), ExecutorError> {
    let mut content = original.to_vec();
    let mut first_line = None;
    let mut replacements = 0u64;
    for edit in edits {
        let old = edit.old_string.as_bytes();
        if old.is_empty() {
            return Err(err(ExecutorErrorCode::InvalidArgument, "missing required arg `old_string`"));
        }
        if edit.old_string == edit.new_string {
            return Err(err(
                ExecutorErrorCode::InvalidArgument,
                "`new_string` must be different from `old_string`",
            ));
        }
        let positions = find_all(&content, old);
        match (positions.len(), edit.replace_all) {
            (0, _) => {
                return Err(err(
                    ExecutorErrorCode::InvalidArgument,
                    "`old_string` was not found in the file",
                ))
            }
            (1, _) | (_, true) => {}
            (n, false) => {
                return Err(err(
                    ExecutorErrorCode::InvalidArgument,
                    format!("`old_string` must match exactly once, found {n} matches"),
                ))
            }
        }
        if first_line.is_none() {
            let pos = positions[0];
            first_line = Some(content[..pos].iter().filter(|b| **b == b'\n').count() as u64 + 1);
        }
        let mut out = Vec::with_capacity(content.len());
        let mut last = 0;
        for pos in &positions {
            out.extend_from_slice(&content[last..*pos]);
            out.extend_from_slice(edit.new_string.as_bytes());
            last = pos + old.len();
        }
        out.extend_from_slice(&content[last..]);
        replacements += positions.len() as u64;
        content = out;
    }
    Ok((content, first_line.unwrap_or(1), replacements))
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            out.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// `write_file` creates missing parent directories, but only below an
/// existing ancestor that is inside the roots.
fn create_parents(path: &str, roots: &[PathBuf]) -> Result<(), ExecutorError> {
    let Some(parent) = Path::new(path).parent() else {
        return Ok(());
    };
    if parent.exists() {
        return Ok(());
    }
    let mut ancestor = parent.to_path_buf();
    while !ancestor.exists() {
        if !ancestor.pop() {
            return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} has no existing ancestor")));
        }
    }
    let real = std::fs::canonicalize(&ancestor).map_err(|e| io_err(path, e))?;
    ensure_under(&real, roots, path)?;
    std::fs::create_dir_all(parent).map_err(|e| io_err(path, e))
}

/// Where a write below missing directories will land, without creating them.
fn missing_parent_target(path: &str, roots: &[PathBuf]) -> Result<PathBuf, ExecutorError> {
    let mut ancestor = PathBuf::from(path);
    let mut rest = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no existing ancestor")))?
            .to_os_string();
        rest.push(name);
        if !ancestor.pop() {
            return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} has no existing ancestor")));
        }
    }
    let mut real = std::fs::canonicalize(&ancestor).map_err(|e| io_err(path, e))?;
    ensure_under(&real, roots, path)?;
    for part in rest.iter().rev() {
        real.push(part);
    }
    Ok(real)
}

fn canonical_roots(roots: &[String]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect()
}

fn ensure_under(real: &Path, roots: &[PathBuf], shown: &str) -> Result<(), ExecutorError> {
    if roots.iter().any(|root| real.starts_with(root)) {
        Ok(())
    } else {
        Err(err(
            ExecutorErrorCode::Forbidden,
            format!("{shown} is outside the profile workdir roots"),
        ))
    }
}

fn fd_path(fd: i32) -> io::Result<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
}

/// A write target: a verified parent directory handle plus the entry name.
struct Target {
    dir: OwnedFd,
    dir_path: PathBuf,
    name: CString,
    name_str: String,
}

impl Target {
    fn open(path: &str, roots: &[PathBuf]) -> Result<Target, ExecutorError> {
        if !path.starts_with('/') {
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

    fn real_path(&self) -> PathBuf {
        self.dir_path.join(&self.name_str)
    }

    /// Current content and permission bits, `None` when absent.
    fn read_existing(&self, max_bytes: u64) -> Result<Option<(Vec<u8>, u32)>, ExecutorError> {
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
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
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
    fn replace(&self, data: &[u8], mode: Option<u32>) -> Result<(), ExecutorError> {
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
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(old: &str, new: &str) -> ExecutorEdit {
        ExecutorEdit {
            old_string: old.into(),
            new_string: new.into(),
            replace_all: false,
        }
    }

    #[test]
    fn edits_match_edit_file_semantics() {
        let (out, line, n) = apply_edits(b"a\nb\nc\n", &[edit("b", "B")]).unwrap();
        assert_eq!(out, b"a\nB\nc\n");
        assert_eq!((line, n), (2, 1));
        let e = apply_edits(b"x x", &[edit("x", "y")]).unwrap_err();
        assert!(e.message.contains("exactly once, found 2"));
        let e = apply_edits(b"abc", &[edit("z", "y")]).unwrap_err();
        assert!(e.message.contains("not found"));
        let mut all = edit("x", "y");
        all.replace_all = true;
        assert_eq!(apply_edits(b"x x", &[all]).unwrap().0, b"y y");
    }
}
