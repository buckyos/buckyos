use crate::{path_utils::resolve_path_from_root, AgentToolError};
use async_trait::async_trait;
use buckyos_api::{ExecutorError, ExecutorErrorCode};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[async_trait]
pub trait FileBackend: Send + Sync + std::fmt::Debug {
    async fn resolve(
        &self,
        root: &Path,
        raw: &str,
        allowed: &[PathBuf],
    ) -> Result<PathBuf, AgentToolError>;
    async fn exists(&self, path: &Path) -> Result<bool, AgentToolError>;
    async fn read(&self, path: &Path) -> Result<Vec<u8>, AgentToolError>;
    async fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), AgentToolError>;
    /// Backends behind an executor perform a whole `write_file` as one remote
    /// operation; `None` means read/write run here.
    async fn write_whole(
        &self,
        _path: &Path,
        _mode: WholeWriteMode,
        _content: &[u8],
    ) -> Option<Result<WholeWriteResult, AgentToolError>> {
        None
    }
    /// Same for `edit_file`: match exactly once and replace on the target.
    async fn edit_once(
        &self,
        _path: &Path,
        _old: &str,
        _new: &str,
    ) -> Option<Result<WholeEditResult, AgentToolError>> {
        None
    }
    /// What `path` is on the target.
    async fn path_type(&self, path: &Path) -> Result<PathType, AgentToolError>;
    /// Create `path` and its missing parents; an existing directory is fine.
    async fn mkdir(&self, path: &Path) -> Result<(), AgentToolError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathType {
    File,
    Directory,
    Missing,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WholeWriteMode {
    Replace,
    Append,
    CreateNew,
}

#[derive(Debug, Clone)]
pub struct WholeWriteResult {
    pub created: bool,
    pub changed: bool,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct WholeEditResult {
    /// The content before the edit, for the diff.
    pub original: Vec<u8>,
    pub line: usize,
    pub changed: bool,
}

#[derive(Debug)]
pub struct LocalFileBackend;

fn canonical_missing(path: &Path) -> Result<PathBuf, AgentToolError> {
    let mut ancestor = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&ancestor) {
            Ok(mut real) => {
                for part in missing.iter().rev() {
                    real.push(part);
                }
                return Ok(crate::path_utils::normalize_abs_path(&real));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&ancestor).is_ok() {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "dangling symlink: {}",
                        ancestor.display()
                    )));
                }
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| {
                        AgentToolError::InvalidArgs(format!("cannot resolve {}", path.display()))
                    })?
                    .to_os_string();
                missing.push(name);
                if !ancestor.pop() {
                    return Err(AgentToolError::InvalidArgs(
                        "path has no existing ancestor".into(),
                    ));
                }
            }
            Err(e) => return Err(AgentToolError::ExecFailed(e.to_string())),
        }
    }
}

pub fn check_allowed(path: &Path, roots: &[PathBuf]) -> Result<(), AgentToolError> {
    if roots.is_empty() || roots.iter().any(|root| path.starts_with(root)) {
        return Ok(());
    }
    Err(AgentToolError::InvalidArgs(format!(
        "path not allowed by policy: {}",
        path.display()
    )))
}

#[async_trait]
impl FileBackend for LocalFileBackend {
    async fn resolve(
        &self,
        root: &Path,
        raw: &str,
        allowed: &[PathBuf],
    ) -> Result<PathBuf, AgentToolError> {
        let lexical = resolve_path_from_root(root, raw)?;
        check_allowed(&lexical, allowed)?;
        let candidate = if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            root.join(raw)
        };
        let real = canonical_missing(&candidate)?;
        let roots = allowed
            .iter()
            .map(|r| canonical_missing(r))
            .collect::<Result<Vec<_>, _>>()?;
        check_allowed(&real, &roots)?;
        Ok(real)
    }
    async fn exists(&self, path: &Path) -> Result<bool, AgentToolError> {
        match tokio::fs::metadata(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(AgentToolError::ExecFailed(e.to_string())),
        }
    }
    async fn read(&self, path: &Path) -> Result<Vec<u8>, AgentToolError> {
        tokio::fs::read(path)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("read file failed: {e}")))
    }
    async fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), AgentToolError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        }
        tokio::fs::write(path, bytes)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("write file failed: {e}")))
    }
    async fn path_type(&self, path: &Path) -> Result<PathType, AgentToolError> {
        match tokio::fs::metadata(path).await {
            Ok(m) if m.is_dir() => Ok(PathType::Directory),
            Ok(m) if m.is_file() => Ok(PathType::File),
            Ok(_) => Ok(PathType::Other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PathType::Missing),
            Err(e) => Err(AgentToolError::ExecFailed(e.to_string())),
        }
    }
    async fn mkdir(&self, path: &Path) -> Result<(), AgentToolError> {
        tokio::fs::create_dir_all(path)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("mkdir failed: {e}")))
    }
}

/// Largest chunk the native runtime moves per primitive call.
pub const LOCAL_MAX_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

/// Limits of a runtime's streaming file primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileLimits {
    /// Largest `read_range` length and `write_chunk` payload.
    pub max_chunk_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    /// The real path on the runtime (for a missing path: below its real parent).
    pub path: String,
    pub path_type: PathType,
    pub size: u64,
    /// `sha256:<hex>` of a file's whole content, when requested.
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRead {
    pub path: String,
    pub data: Vec<u8>,
    /// The file's size when the range was read.
    pub file_size: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct WriteChunk<'a> {
    pub mode: WholeWriteMode,
    /// The file's current size must be this (a missing file has none).
    pub expected_size: Option<u64>,
    /// The file's current content must hash to this (costs a pass over it).
    pub expected_hash: Option<&'a str>,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkWritten {
    pub path: String,
    /// The file's size after the write.
    pub size: u64,
    pub created: bool,
    /// The whole file's hash after the write; an `append` reports it only
    /// when it was given `expected_hash`.
    pub hash: Option<String>,
}

/// Streaming file primitives of one runtime for the control side (copies
/// between runtimes). Paths are absolute paths of that runtime, in its own
/// syntax; a chunk never exceeds `limits().max_chunk_bytes`. Errors carry
/// the Node Executor's codes: `Conflict` for an existing target or a failed
/// `expected_*` guard, `NotFound`, `Forbidden` outside the runtime's roots.
#[async_trait]
pub trait RuntimeFiles: Send + Sync {
    fn limits(&self) -> FileLimits;
    async fn stat(&self, path: &str, hash: bool) -> Result<FileStat, ExecutorError>;
    async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<RangeRead, ExecutorError>;
    /// `append` extends the file in place; `replace` and `create_new` write
    /// the chunk as the whole content. Missing parents are created.
    async fn write_chunk(&self, path: &str, chunk: WriteChunk<'_>) -> Result<ChunkWritten, ExecutorError>;
    /// Move a regular file; an existing `to` is replaced only with `replace`.
    async fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), ExecutorError>;
    /// Delete a regular file; `Ok(false)` when it was already missing.
    async fn remove(&self, path: &str) -> Result<bool, ExecutorError>;
}

fn file_error(path: &Path, e: std::io::Error) -> ExecutorError {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => ExecutorErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ExecutorErrorCode::Forbidden,
        std::io::ErrorKind::AlreadyExists => ExecutorErrorCode::Conflict,
        _ => ExecutorErrorCode::Internal,
    };
    ExecutorError::new(code, format!("{}: {e}", path.display()))
}

fn invalid(message: String) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::InvalidArgument, message)
}

fn conflict(message: String) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::Conflict, message)
}

fn absolute(path: &str) -> Result<PathBuf, ExecutorError> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(invalid(format!("{path} is not absolute")));
    }
    Ok(p)
}

fn hash_prefix(file: &mut std::fs::File, len: u64, hasher: &mut Sha256) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut rest = len;
    let mut buf = vec![0u8; 64 * 1024];
    while rest > 0 {
        let want = (rest as usize).min(buf.len());
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "file shrank while hashing"));
        }
        hasher.update(&buf[..n]);
        rest -= n as u64;
    }
    Ok(())
}

fn digest_text(hasher: Sha256) -> String {
    format!("sha256:{:x}", hasher.finalize())
}

fn local_stat(path: &str, hash: bool) -> Result<FileStat, ExecutorError> {
    let p = absolute(path)?;
    let meta = match std::fs::metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let real = canonical_missing(&p).map_err(|e| invalid(e.to_string()))?;
            return Ok(FileStat {
                path: real.display().to_string(),
                path_type: PathType::Missing,
                size: 0,
                hash: None,
            });
        }
        Err(e) => return Err(file_error(&p, e)),
    };
    let real = std::fs::canonicalize(&p).map_err(|e| file_error(&p, e))?;
    let path_type = if meta.is_file() {
        PathType::File
    } else if meta.is_dir() {
        PathType::Directory
    } else {
        PathType::Other
    };
    let (size, hash) = if path_type == PathType::File && hash {
        let mut file = std::fs::File::open(&real).map_err(|e| file_error(&p, e))?;
        let size = file.metadata().map_err(|e| file_error(&p, e))?.len();
        let mut hasher = Sha256::new();
        hash_prefix(&mut file, size, &mut hasher).map_err(|e| file_error(&p, e))?;
        (size, Some(digest_text(hasher)))
    } else {
        (if path_type == PathType::File { meta.len() } else { 0 }, None)
    };
    Ok(FileStat {
        path: real.display().to_string(),
        path_type,
        size,
        hash,
    })
}

fn local_read_range(path: &str, offset: u64, length: u64) -> Result<RangeRead, ExecutorError> {
    let p = absolute(path)?;
    if length > LOCAL_MAX_CHUNK_BYTES {
        return Err(invalid(format!("{path}: {length} bytes exceed the chunk limit {LOCAL_MAX_CHUNK_BYTES}")));
    }
    let mut file = std::fs::File::open(&p).map_err(|e| file_error(&p, e))?;
    let meta = file.metadata().map_err(|e| file_error(&p, e))?;
    if !meta.is_file() {
        return Err(invalid(format!("{path} is not a regular file")));
    }
    let size = meta.len();
    let start = offset.min(size);
    let mut data = Vec::with_capacity(length.min(size - start) as usize);
    file.seek(SeekFrom::Start(start)).map_err(|e| file_error(&p, e))?;
    (&mut file)
        .take(length.min(size - start))
        .read_to_end(&mut data)
        .map_err(|e| file_error(&p, e))?;
    let real = std::fs::canonicalize(&p).map_err(|e| file_error(&p, e))?;
    Ok(RangeRead {
        path: real.display().to_string(),
        data,
        file_size: size,
    })
}

fn local_write_chunk(path: &str, chunk: WriteChunk<'_>) -> Result<ChunkWritten, ExecutorError> {
    let p = absolute(path)?;
    if chunk.data.len() as u64 > LOCAL_MAX_CHUNK_BYTES {
        return Err(invalid(format!("{path}: chunk exceeds the chunk limit {LOCAL_MAX_CHUNK_BYTES}")));
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| file_error(&p, e))?;
    }
    let size = match std::fs::metadata(&p) {
        Ok(m) if !m.is_file() => return Err(invalid(format!("{path} is not a regular file"))),
        Ok(m) => Some(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(file_error(&p, e)),
    };
    if chunk.mode == WholeWriteMode::CreateNew && size.is_some() {
        return Err(conflict(format!("{path} already exists")));
    }
    if (chunk.expected_size.is_some() || chunk.expected_hash.is_some()) && size.is_none() {
        return Err(conflict(format!("{path} is missing")));
    }
    if let Some(expected) = chunk.expected_size {
        if Some(expected) != size {
            return Err(conflict(format!("{path} is not {expected} bytes (expected_size mismatch)")));
        }
    }
    let mut hasher = None;
    if let Some(expected) = chunk.expected_hash {
        let mut file = std::fs::File::open(&p).map_err(|e| file_error(&p, e))?;
        let mut h = Sha256::new();
        hash_prefix(&mut file, size.unwrap_or(0), &mut h).map_err(|e| file_error(&p, e))?;
        if digest_text(h.clone()) != expected {
            return Err(conflict(format!("{path} changed (expected_hash mismatch)")));
        }
        hasher = Some(h);
    }
    let (new_size, hash) = match chunk.mode {
        WholeWriteMode::Append => {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&p)
                .map_err(|e| file_error(&p, e))?;
            file.write_all(chunk.data)
                .and_then(|_| file.sync_all())
                .map_err(|e| file_error(&p, e))?;
            let hash = hasher.map(|mut h| {
                h.update(chunk.data);
                digest_text(h)
            });
            (size.unwrap_or(0) + chunk.data.len() as u64, hash)
        }
        WholeWriteMode::CreateNew | WholeWriteMode::Replace => {
            let dir = p.parent().ok_or_else(|| invalid(format!("{path} has no parent")))?;
            let name = p.file_name().ok_or_else(|| invalid(format!("{path} has no file name")))?;
            let tmp = dir.join(format!(".{}.rt-{}", name.to_string_lossy(), super::node::uuid_like()));
            let written = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .and_then(|mut f| f.write_all(chunk.data).and_then(|_| f.sync_all()));
            let committed = match written {
                Err(e) => Err(file_error(&p, e)),
                Ok(()) if chunk.mode == WholeWriteMode::CreateNew => move_noreplace(&tmp, &p),
                Ok(()) => std::fs::rename(&tmp, &p).map_err(|e| file_error(&p, e)),
            };
            if committed.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            committed?;
            let mut h = Sha256::new();
            h.update(chunk.data);
            (chunk.data.len() as u64, Some(digest_text(h)))
        }
    };
    let real = std::fs::canonicalize(&p).map_err(|e| file_error(&p, e))?;
    Ok(ChunkWritten {
        path: real.display().to_string(),
        size: new_size,
        created: size.is_none(),
        hash,
    })
}

fn local_kind(p: &Path) -> Result<PathType, ExecutorError> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => Ok(PathType::Other),
        Ok(m) if m.is_file() => Ok(PathType::File),
        Ok(m) if m.is_dir() => Ok(PathType::Directory),
        Ok(_) => Ok(PathType::Other),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PathType::Missing),
        Err(e) => Err(file_error(p, e)),
    }
}

fn local_rename(from: &str, to: &str, replace: bool) -> Result<(), ExecutorError> {
    let (src, dst) = (absolute(from)?, absolute(to)?);
    match local_kind(&src)? {
        PathType::File => {}
        PathType::Missing => {
            return Err(ExecutorError::new(ExecutorErrorCode::NotFound, format!("{from}: file not found")))
        }
        _ => return Err(invalid(format!("{from} is not a regular file"))),
    }
    match local_kind(&dst)? {
        PathType::Missing => {}
        PathType::File if replace => {}
        PathType::File => return Err(conflict(format!("{to} already exists"))),
        _ => return Err(conflict(format!("{to} exists and is not a regular file"))),
    }
    if replace {
        return std::fs::rename(&src, &dst).map_err(|e| file_error(&dst, e));
    }
    move_noreplace(&src, &dst)
}

/// Move `src` to a missing `dst`: a hard link fails atomically on an
/// existing `dst`; without hard links, a checked rename.
fn move_noreplace(src: &Path, dst: &Path) -> Result<(), ExecutorError> {
    match std::fs::hard_link(src, dst) {
        Ok(()) => std::fs::remove_file(src).map_err(|e| file_error(src, e)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(conflict(format!("{} already exists", dst.display())))
        }
        Err(_) => {
            if local_kind(dst)? != PathType::Missing {
                return Err(conflict(format!("{} already exists", dst.display())));
            }
            std::fs::rename(src, dst).map_err(|e| file_error(dst, e))
        }
    }
}

fn local_remove(path: &str) -> Result<bool, ExecutorError> {
    let p = absolute(path)?;
    match local_kind(&p)? {
        PathType::Missing => Ok(false),
        PathType::File => std::fs::remove_file(&p).map(|_| true).map_err(|e| file_error(&p, e)),
        _ => Err(invalid(format!("{path} is not a regular file"))),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ExecutorError> + Send + 'static,
) -> Result<T, ExecutorError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ExecutorError::new(ExecutorErrorCode::Internal, e.to_string()))?
}

#[async_trait]
impl RuntimeFiles for LocalFileBackend {
    fn limits(&self) -> FileLimits {
        FileLimits {
            max_chunk_bytes: LOCAL_MAX_CHUNK_BYTES,
        }
    }
    async fn stat(&self, path: &str, hash: bool) -> Result<FileStat, ExecutorError> {
        let path = path.to_string();
        blocking(move || local_stat(&path, hash)).await
    }
    async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<RangeRead, ExecutorError> {
        let path = path.to_string();
        blocking(move || local_read_range(&path, offset, length)).await
    }
    async fn write_chunk(&self, path: &str, chunk: WriteChunk<'_>) -> Result<ChunkWritten, ExecutorError> {
        let path = path.to_string();
        let (mode, expected_size) = (chunk.mode, chunk.expected_size);
        let expected_hash = chunk.expected_hash.map(str::to_string);
        let data = chunk.data.to_vec();
        blocking(move || {
            local_write_chunk(
                &path,
                WriteChunk {
                    mode,
                    expected_size,
                    expected_hash: expected_hash.as_deref(),
                    data: &data,
                },
            )
        })
        .await
    }
    async fn rename(&self, from: &str, to: &str, replace: bool) -> Result<(), ExecutorError> {
        let (from, to) = (from.to_string(), to.to_string());
        blocking(move || local_rename(&from, &to, replace)).await
    }
    async fn remove(&self, path: &str) -> Result<bool, ExecutorError> {
        let path = path.to_string();
        blocking(move || local_remove(&path)).await
    }
}
