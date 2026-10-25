//! Worker side: runs as the profile's OS identity on a root Unix daemon (the
//! identity switch is applied by the parent at spawn, see
//! `spawn::drop_privileges`) and as the host's own user on Windows. It
//! performs one file job read from stdin and writes `reply-json\n` + payload
//! to stdout.
//!
//! Paths are checked on the opened file or directory (see the platform
//! modules), so a path swapped after resolution cannot redirect the I/O
//! outside the roots.

use std::io::{self, BufRead, Read, Seek, Write};
use std::path::{Path, PathBuf};

use buckyos_api::{
    executor_content_hash, ExecutorEdit, ExecutorError, ExecutorErrorCode, ExecutorFileResult,
    ExecutorPathType, ExecutorWriteMode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const WORKER_ARG: &str = "--executor-worker";

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
        #[serde(default)]
        expected_size: Option<u64>,
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
    Mkdir {
        path: String,
        roots: Vec<String>,
    },
    Stat {
        path: String,
        roots: Vec<String>,
        #[serde(default)]
        hash: bool,
    },
    Rename {
        from: String,
        to: String,
        replace: bool,
        roots: Vec<String>,
    },
    Remove {
        path: String,
        roots: Vec<String>,
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

#[cfg(unix)]
#[path = "worker_unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "worker_windows.rs"]
mod sys;

use sys::Target;

const MAX_JOB_LINE: usize = 2 * 1024 * 1024;

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
    sys::prepare();
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
                    path: Some(sys::display(&real)),
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
                    path: Some(sys::display(&real)),
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
            if !crate::is_host_absolute(&path) {
                return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not absolute")));
            }
            let roots = canonical_roots(&roots);
            let file = std::fs::File::open(&path).map_err(|e| io_err(&path, e))?;
            let real = sys::opened_path(&file).map_err(|e| io_err(&path, e))?;
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
                        path: sys::display(&real),
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
            expected_size,
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
            if mode == ExecutorWriteMode::Append {
                let file = append(&target, &upload, expected_size, expected_hash.as_deref())?;
                return Ok((
                    WorkerReply {
                        file: Some(file),
                        ..Default::default()
                    },
                    Vec::new(),
                ));
            }
            let existing = target.read_existing(max_bytes)?;
            if let Some(expected) = expected_size {
                if existing.as_ref().map(|(old, _)| old.len() as u64) != Some(expected) {
                    return Err(err(ExecutorErrorCode::Conflict, format!("{path} changed (expected_size mismatch)")));
                }
            }
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
                        path: sys::display(&target.real_path()),
                        size: new_content.len() as u64,
                        hash: new_hash,
                        previous_hash,
                        created: existing.is_none(),
                        changed,
                        bytes_written: upload.len() as u64,
                        line: None,
                        replacements: 0,
                        path_type: Some(ExecutorPathType::File),
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
                        path: sys::display(&target.real_path()),
                        size: updated.len() as u64,
                        hash: new_hash,
                        previous_hash: Some(previous_hash),
                        created: false,
                        changed,
                        bytes_written: if changed { updated.len() as u64 } else { 0 },
                        line: Some(line),
                        replacements,
                        path_type: Some(ExecutorPathType::File),
                    }),
                    file_size: original.len() as u64,
                    ..Default::default()
                },
                original,
            ))
        }
        WorkerJob::Stat { path, roots, hash } => {
            if !crate::is_host_absolute(&path) {
                return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not absolute")));
            }
            let roots = canonical_roots(&roots);
            let (real, path_type, size) = match std::fs::metadata(&path) {
                Ok(meta) => {
                    let real = std::fs::canonicalize(&path).map_err(|e| io_err(&path, e))?;
                    let kind = if meta.is_dir() {
                        ExecutorPathType::Directory
                    } else if meta.is_file() {
                        ExecutorPathType::File
                    } else {
                        ExecutorPathType::Other
                    };
                    (real, kind, if meta.is_file() { meta.len() } else { 0 })
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    (missing_parent_target(&path, &roots)?, ExecutorPathType::Missing, 0)
                }
                Err(e) => return Err(io_err(&path, e)),
            };
            ensure_under(&real, &roots, &path)?;
            let (size, hash) = if hash && path_type == ExecutorPathType::File {
                let mut file = std::fs::File::open(&real).map_err(|e| io_err(&path, e))?;
                let opened = sys::opened_path(&file).map_err(|e| io_err(&path, e))?;
                ensure_under(&opened, &roots, &path)?;
                let size = file.metadata().map_err(|e| io_err(&path, e))?.len();
                let mut hasher = Sha256::new();
                hash_prefix(&mut file, size, &mut hasher).map_err(|e| io_err(&path, e))?;
                (size, format!("sha256:{:x}", hasher.finalize()))
            } else {
                (size, String::new())
            };
            Ok((
                WorkerReply {
                    path: Some(sys::display(&real)),
                    file: Some(ExecutorFileResult {
                        path: sys::display(&real),
                        size,
                        hash,
                        path_type: Some(path_type),
                        ..Default::default()
                    }),
                    file_size: size,
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        WorkerJob::Rename { from, to, replace, roots } => {
            let roots = canonical_roots(&roots);
            let source = Target::entry(&from, &roots)?;
            let size = match source.kind()? {
                (ExecutorPathType::File, size) => size,
                (ExecutorPathType::Missing, _) => {
                    return Err(err(ExecutorErrorCode::NotFound, format!("{from}: file not found")))
                }
                _ => return Err(err(ExecutorErrorCode::InvalidArgument, format!("{from} is not a regular file"))),
            };
            let dest = Target::entry(&to, &roots)?;
            if dest.real_path() == source.real_path() {
                return Err(err(ExecutorErrorCode::InvalidArgument, "rename source and destination are the same"));
            }
            let replaced = match dest.kind()?.0 {
                ExecutorPathType::Missing => false,
                ExecutorPathType::File if replace => true,
                ExecutorPathType::File => {
                    return Err(err(ExecutorErrorCode::Conflict, format!("{to} already exists")))
                }
                _ => return Err(err(ExecutorErrorCode::Conflict, format!("{to} exists and is not a regular file"))),
            };
            source.rename_to(&dest, replace)?;
            Ok((
                WorkerReply {
                    path: Some(sys::display(&dest.real_path())),
                    file: Some(ExecutorFileResult {
                        path: sys::display(&dest.real_path()),
                        size,
                        created: !replaced,
                        changed: true,
                        path_type: Some(ExecutorPathType::File),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        WorkerJob::Remove { path, roots } => {
            let roots = canonical_roots(&roots);
            let parent_exists = Path::new(&path).parent().is_some_and(|p| p.exists());
            let (real, changed) = if !parent_exists {
                (missing_parent_target(&path, &roots)?, false)
            } else {
                let target = Target::entry(&path, &roots)?;
                match target.kind()?.0 {
                    ExecutorPathType::Missing => (target.real_path(), false),
                    ExecutorPathType::File => {
                        target.remove()?;
                        (target.real_path(), true)
                    }
                    _ => return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not a regular file"))),
                }
            };
            Ok((
                WorkerReply {
                    path: Some(sys::display(&real)),
                    file: Some(ExecutorFileResult {
                        path: sys::display(&real),
                        changed,
                        path_type: Some(ExecutorPathType::Missing),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
        WorkerJob::Mkdir { path, roots } => {
            let roots = canonical_roots(&roots);
            let real = match std::fs::metadata(&path) {
                Ok(meta) => {
                    let real = std::fs::canonicalize(&path).map_err(|e| io_err(&path, e))?;
                    ensure_under(&real, &roots, &path)?;
                    if !meta.is_dir() {
                        return Err(err(ExecutorErrorCode::Conflict, format!("{path} exists and is not a directory")));
                    }
                    real
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    let real = missing_parent_target(&path, &roots)?;
                    std::fs::create_dir_all(&real).map_err(|e| io_err(&path, e))?;
                    std::fs::canonicalize(&real).map_err(|e| io_err(&path, e))?
                }
                Err(e) => return Err(io_err(&path, e)),
            };
            Ok((
                WorkerReply {
                    path: Some(sys::display(&real)),
                    file: Some(ExecutorFileResult {
                        path: sys::display(&real),
                        path_type: Some(ExecutorPathType::Directory),
                        created: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Vec::new(),
            ))
        }
    }
}

/// `write` in `append` mode: the existing file is extended in place, never
/// loaded, so a transfer can grow it beyond max_file_bytes chunk by chunk.
/// `expected_hash` costs a pass over the existing content; `expected_size`
/// does not. A missing file matches neither.
fn append(
    target: &Target,
    data: &[u8],
    expected_size: Option<u64>,
    expected_hash: Option<&str>,
) -> Result<ExecutorFileResult, ExecutorError> {
    let shown = sys::display(&target.real_path());
    let (mut file, created) = match target.open_append()? {
        Some(file) => (file, false),
        None if expected_size.is_some() || expected_hash.is_some() => {
            return Err(err(ExecutorErrorCode::Conflict, format!("{shown} is missing")))
        }
        None => (target.create_append()?, true),
    };
    let meta = file.metadata().map_err(|e| io_err(&shown, e))?;
    if !meta.is_file() {
        return Err(err(ExecutorErrorCode::InvalidArgument, format!("{shown} is not a regular file")));
    }
    let size = meta.len();
    if expected_size.is_some_and(|expected| expected != size) {
        return Err(err(
            ExecutorErrorCode::Conflict,
            format!("{shown} is {size} bytes, not {} (expected_size mismatch)", expected_size.unwrap_or(0)),
        ));
    }
    let mut hasher = None;
    let mut previous_hash = None;
    if let Some(expected) = expected_hash {
        let mut h = Sha256::new();
        hash_prefix(&mut file, size, &mut h).map_err(|e| io_err(&shown, e))?;
        let previous = format!("sha256:{:x}", h.clone().finalize());
        if previous != expected {
            return Err(err(ExecutorErrorCode::Conflict, format!("{shown} changed (expected_hash mismatch)")));
        }
        previous_hash = Some(previous);
        hasher = Some(h);
    }
    file.write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|e| io_err(&shown, e))?;
    if created {
        target.sync_dir();
    }
    let hash = hasher
        .map(|mut h| {
            h.update(data);
            format!("sha256:{:x}", h.finalize())
        })
        .unwrap_or_default();
    Ok(ExecutorFileResult {
        path: shown,
        size: size + data.len() as u64,
        hash,
        previous_hash,
        created,
        changed: created || !data.is_empty(),
        bytes_written: data.len() as u64,
        line: None,
        replacements: 0,
        path_type: Some(ExecutorPathType::File),
    })
}

fn hash_prefix(file: &mut std::fs::File, len: u64, hasher: &mut Sha256) -> io::Result<()> {
    file.seek(io::SeekFrom::Start(0))?;
    let mut rest = len;
    let mut buf = vec![0u8; 64 * 1024];
    while rest > 0 {
        let n = file.read(&mut buf[..(rest as usize).min(64 * 1024)])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "file shrank while hashing"));
        }
        hasher.update(&buf[..n]);
        rest -= n as u64;
    }
    Ok(())
}

/// The real location of `path`: with `follow` an existing entry resolves to
/// what it points at (a write goes through a symlink), without it only the
/// parent is resolved and the entry itself is named.
fn resolve_entry(path: &str, follow: bool) -> Result<PathBuf, ExecutorError> {
    if !crate::is_host_absolute(path) {
        return Err(err(ExecutorErrorCode::InvalidArgument, format!("{path} is not absolute")));
    }
    if follow {
        match std::fs::canonicalize(path) {
            Ok(p) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(path, e)),
        }
    }
    let p = Path::new(path);
    let parent = p
        .parent()
        .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no parent")))?;
    let name = p
        .file_name()
        .ok_or_else(|| err(ExecutorErrorCode::InvalidArgument, format!("{path} has no file name")))?;
    Ok(std::fs::canonicalize(parent).map_err(|e| io_err(path, e))?.join(name))
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
