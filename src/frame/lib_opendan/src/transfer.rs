//! Copying a file between the runtimes of a session (primary-runtime TODO
//! §4): the session-runtime, the workspace's runtime, or another runtime of
//! the agent. The control side holds both ends' file primitives
//! (`agent_tool` [`RuntimeFiles`]) and moves the file in chunks: the first
//! creates a temporary file next to the target, the rest append under an
//! `expected_size` guard, the whole file is verified by its hash on both
//! ends, and a rename commits it. Every copy has a record in the session
//! (`<state>/transfers/<transfer_id>.json`): a retry of the same call finds
//! it, checks the target and never copies over a committed result blindly.
//!
//! One regular file per copy; directories, links and metadata (modes,
//! times) are not copied and are refused as such.

use std::path::{Path, PathBuf};

use agent_tool::runtime::files::{LocalFileBackend, PathType, RuntimeFiles, WholeWriteMode, WriteChunk};
use buckyos_api::{ExecutorError, ExecutorErrorCode};
use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::AgentStateClient;

/// Endpoint names resolved in the session: its session-runtime working
/// directory and its workspace directory.
pub const ROLE_SESSION: &str = "session";
pub const ROLE_WORKSPACE: &str = "workspace";
/// Largest file copied through a Node Executor: every chunk stays in the
/// node's operation ledger (its `max_store_bytes`, 2 GiB by default) for its
/// retention period.
pub const MAX_NODE_TRANSFER_BYTES: u64 = 512 * 1024 * 1024;
const TRANSFERS_DIR: &str = "transfers";

/// `<runtime>:<path>` as a command names it: a role (`session`,
/// `workspace`) with a path relative to its directory (or absolute inside
/// it), or a runtime id with an absolute path of that runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub runtime: String,
    pub path: String,
}

impl std::str::FromStr for Endpoint {
    type Err = OpenDanError;
    fn from_str(s: &str) -> Result<Self> {
        let (runtime, path) = s.split_once(':').ok_or_else(|| {
            OpenDanError::InvalidArgument(format!(
                "`{s}` is not <session|workspace|runtime_id>:<path>"
            ))
        })?;
        if runtime.is_empty() || path.is_empty() {
            return Err(OpenDanError::InvalidArgument(format!(
                "`{s}` is not <session|workspace|runtime_id>:<path>"
            )));
        }
        Ok(Self {
            runtime: runtime.to_string(),
            path: path.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Transferring,
    Verifying,
    Committed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferEnd {
    /// As the command named it (`session`, `workspace` or a runtime id).
    pub name: String,
    pub runtime_id: String,
    pub path: String,
    /// The real path on that runtime.
    pub resolved: String,
}

/// `<state>/transfers/<transfer_id>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRecord {
    pub transfer_id: String,
    pub from: TransferEnd,
    pub to: TransferEnd,
    /// The temporary file the copy is written to before the commit.
    pub temp_path: String,
    /// Whether an existing target is replaced (else the copy fails on it).
    pub replace: bool,
    pub size: u64,
    pub bytes_done: u64,
    /// `sha256:<hex>` of the source, verified on the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub state: TransferState,
    /// Where a failed copy stopped (`resolve`, `read`, `write`, `verify`,
    /// `commit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The tool call that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
}

impl TransferRecord {
    /// Exit code of the command: 0 committed, 4 cancelled, 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        match self.state {
            TransferState::Committed => 0,
            TransferState::Cancelled => 4,
            _ => 1,
        }
    }
}

fn records_dir(sd: &SessionDir) -> PathBuf {
    sd.state_dir().join(TRANSFERS_DIR)
}

fn record_path(sd: &SessionDir, id: &str) -> Result<PathBuf> {
    crate::ids::validate_session_id(id)
        .map_err(|_| OpenDanError::InvalidArgument(format!("bad transfer id `{id}`")))?;
    Ok(records_dir(sd).join(format!("{id}.json")))
}

fn save(sd: &SessionDir, r: &TransferRecord) -> Result<()> {
    let dir = records_dir(sd);
    std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
    crate::fsutil::atomic_replace_json(&record_path(sd, &r.transfer_id)?, r)
}

/// A copy's record.
pub fn status(sd: &SessionDir, id: &str) -> Result<TransferRecord> {
    crate::fsutil::read_json_opt(&record_path(sd, id)?)?
        .ok_or_else(|| OpenDanError::NotFound(format!("transfer {id}")))
}

/// The copies of a session, newest first.
pub fn list(sd: &SessionDir) -> Result<Vec<TransferRecord>> {
    let dir = records_dir(sd);
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(OpenDanError::io(&dir, e)),
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(id) = name.strip_suffix(".json") {
            if let Ok(r) = status(sd, id) {
                out.push(r);
            }
        }
    }
    out.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    Ok(out)
}

fn cancel_mark(sd: &SessionDir, id: &str) -> Result<PathBuf> {
    Ok(record_path(sd, id)?.with_extension("cancel"))
}

/// Ask a running copy to stop: it stops before its next chunk and removes
/// its temporary file. A finished copy is returned as it is.
pub fn request_cancel(sd: &SessionDir, id: &str) -> Result<TransferRecord> {
    let r = status(sd, id)?;
    if matches!(r.state, TransferState::Transferring | TransferState::Verifying) {
        let mark = cancel_mark(sd, id)?;
        std::fs::write(&mark, b"").map_err(|e| OpenDanError::io(&mark, e))?;
    }
    Ok(r)
}

/// One end of a copy, opened.
struct Side {
    end: TransferEnd,
    files: Box<dyn RuntimeFiles>,
    node: bool,
}

fn base_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
}

fn parent_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(0) => &trimmed[..1],
        Some(i) => &trimmed[..i],
        None => "",
    }
}

fn has_parent_part(path: &str) -> bool {
    path.split(['/', '\\']).any(|p| p == "..")
}

/// A path below `base`, given relative to it or absolute inside it.
fn below(base: &str, path: &str, what: &str) -> Result<String> {
    if has_parent_part(path) {
        return Err(OpenDanError::InvalidArgument(format!("{what} path `{path}` must not contain `..`")));
    }
    if buckyos_api::is_absolute_node_path(path) || Path::new(path).is_absolute() {
        let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_string();
        let (b, p) = (norm(base), norm(path));
        if p != b && !p.starts_with(&format!("{b}/")) {
            return Err(OpenDanError::InvalidArgument(format!(
                "{what} path `{path}` is outside its directory {base}; name another runtime with <runtime_id>:<absolute path>"
            )));
        }
        return Ok(path.to_string());
    }
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect();
    let mut joined = crate::runtime::target_join(base, &parts);
    // `dir/`: into that directory.
    if path.ends_with(['/', '\\']) {
        let sep = if joined.contains('\\') && !joined.starts_with('/') { '\\' } else { '/' };
        joined.push(sep);
    }
    Ok(joined)
}

async fn open_runtime(
    sd: &SessionDir,
    runtime_id: &str,
    kind: RuntimeKind,
    exec: Option<&serde_json::Value>,
) -> Result<(Box<dyn RuntimeFiles>, bool)> {
    if kind == RuntimeKind::Container {
        return Ok((Box::new(LocalFileBackend), false));
    }
    if !kind.is_node() {
        return Err(OpenDanError::Bind(format!("runtime `{runtime_id}` cannot take part in a copy ({kind:?})")));
    }
    let exec = exec.ok_or_else(|| OpenDanError::Bind(format!("runtime `{runtime_id}` is not enabled")))?;
    let config = crate::runtime::node_config_of(runtime_id, exec)?;
    let node = config.remote_node.clone().unwrap_or_default();
    let target = agent_tool::runtime::node::RemoteNodeTarget::open(
        crate::runtime::node_connector(),
        node.node_did.as_deref().unwrap_or_default(),
        node.profile_id.as_deref().unwrap_or_default(),
        runtime_id,
        sd.state_dir().join("runtime_ops"),
    )
    .await
    .map_err(|e| OpenDanError::Bind(format!("runtime `{runtime_id}`: {e}")))?;
    Ok((Box::new(target), true))
}

/// Resolve and open `ep` in the session `cfg`; `write`: the copy writes
/// there.
async fn open_side(
    agent: &dyn AgentStateClient,
    sd: &SessionDir,
    cfg: &SessionConfig,
    ep: &Endpoint,
    write: bool,
) -> Result<Side> {
    let (runtime_id, path, kind, exec) = match ep.runtime.as_str() {
        ROLE_SESSION => {
            let runtime_id = cfg.runtime.runtime_id().to_string();
            let frozen = cfg.runtime.frozen.as_ref();
            let kind = frozen.map(|f| f.kind).unwrap_or(RuntimeKind::Container);
            let base = if kind.is_node() {
                crate::runtime::target_workdir(sd)?.ok_or_else(|| {
                    OpenDanError::Bind("the session has no working directory on its runtime yet (it binds at its first drive)".into())
                })?
            } else {
                crate::runtime::resolve_workdir(sd)?.display().to_string()
            };
            (runtime_id, below(&base, &ep.path, "session")?, kind, frozen.and_then(|f| f.exec.clone()))
        }
        ROLE_WORKSPACE => {
            let b = cfg.workspace_binding.as_ref().ok_or_else(|| {
                OpenDanError::InvalidArgument("the session has no workspace".into())
            })?;
            if write && b.access == WorkspaceAccess::ReadOnly {
                return Err(OpenDanError::InvalidArgument(format!(
                    "workspace {} is bound read-only: nothing is copied into it",
                    b.workspace_id
                )));
            }
            let runtime_id = canonical_runtime(&b.location.runtime_id).to_string();
            let record = agent.runtimes().lookup(&runtime_id).await?;
            let kind = record.as_ref().map(|r| r.kind).unwrap_or(RuntimeKind::Container);
            let base = b.location.directory.display().to_string();
            (runtime_id, below(&base, &ep.path, "workspace")?, kind, record.and_then(|r| r.exec))
        }
        id => {
            let (runtime_id, kind, exec) = match agent.runtimes().lookup(id).await? {
                Some(record) => {
                    if record.availability != RuntimeAvailability::Available {
                        return Err(OpenDanError::Bind(format!(
                            "runtime `{id}` is not available: {}",
                            record.last_error.clone().unwrap_or_else(|| format!("{:?}", record.availability))
                        )));
                    }
                    (record.runtime_id, record.kind, record.exec)
                }
                // A host that registered no runtimes: only the agent's own
                // environment exists (as `freeze_runtime` takes it).
                None if is_native_runtime(id) => (CONTAINER_RUNTIME.to_string(), RuntimeKind::Container, None),
                None => {
                    return Err(OpenDanError::NotFound(format!(
                        "runtime {id} (see `agent-session runtime list`)"
                    )))
                }
            };
            if !(buckyos_api::is_absolute_node_path(&ep.path) || Path::new(&ep.path).is_absolute()) || has_parent_part(&ep.path) {
                return Err(OpenDanError::InvalidArgument(format!(
                    "`{id}:{}` needs an absolute path without `..` on that runtime",
                    ep.path
                )));
            }
            (runtime_id, ep.path.clone(), kind, exec)
        }
    };
    let (files, node) = open_runtime(sd, &runtime_id, kind, exec.as_ref()).await?;
    Ok(Side {
        end: TransferEnd {
            name: ep.runtime.clone(),
            runtime_id,
            resolved: path.clone(),
            path: ep.path.clone(),
        },
        files,
        node,
    })
}

fn exec_err(e: ExecutorError) -> String {
    format!("{:?}: {}", e.code, e.message)
}

/// What a copy is asked to do.
#[derive(Debug, Clone)]
pub struct CopyRequest {
    pub from: Endpoint,
    pub to: Endpoint,
    pub replace: bool,
    /// The tool call it runs for: a retry of that call is the same copy.
    pub call_id: Option<String>,
}

fn transfer_id(sid: &str, req: &CopyRequest) -> String {
    match &req.call_id {
        Some(call) => {
            let key = crate::ids::h(&[sid, call, &req.from.runtime, &req.from.path, &req.to.runtime, &req.to.path]);
            format!("t-{}", &key[..24])
        }
        None => format!("t-{}", uuid::Uuid::new_v4().simple()),
    }
}

/// Copy one file between two runtimes of session `sd` and return its
/// record. The record is `committed` only after the target's hash matched
/// the source's and the rename succeeded; a failure leaves the target as
/// it was and says at which step it stopped.
pub async fn copy(agent: &dyn AgentStateClient, sd: &SessionDir, req: &CopyRequest) -> Result<TransferRecord> {
    let cfg = sd.config()?;
    let id = transfer_id(sd.sid(), req);
    let previous = crate::fsutil::read_json_opt::<TransferRecord>(&record_path(sd, &id)?)?;
    let src = open_side(agent, sd, &cfg, &req.from, false).await?;
    let dst = open_side(agent, sd, &cfg, &req.to, true).await?;
    let source = src
        .files
        .stat(&src.end.resolved, false)
        .await
        .map_err(|e| OpenDanError::InvalidArgument(format!("source {}: {}", src.end.resolved, exec_err(e))))?;
    if source.path_type != PathType::File {
        return Err(OpenDanError::InvalidArgument(format!(
            "source {} is not a regular file: a copy moves one file (directories, links and other types are not supported)",
            src.end.resolved
        )));
    }
    let mut to = dst.end.clone();
    let target = dst.files.stat(&to.resolved, false).await;
    let into_dir = to.resolved.ends_with(['/', '\\'])
        || matches!(&target, Ok(s) if s.path_type == PathType::Directory);
    if into_dir {
        to.resolved = crate::runtime::target_join(&to.resolved, &[base_name(&src.end.resolved)]);
    }
    if src.end.runtime_id == to.runtime_id && source.path == to.resolved {
        return Err(OpenDanError::InvalidArgument("source and target are the same file".into()));
    }
    if (src.node || dst.node) && source.size > MAX_NODE_TRANSFER_BYTES {
        return Err(OpenDanError::InvalidArgument(format!(
            "{} is {} bytes; a copy through a Node Executor takes at most {MAX_NODE_TRANSFER_BYTES}",
            src.end.resolved, source.size
        )));
    }
    // A retry of the same call: a committed copy is checked, not redone.
    if let Some(prev) = previous.as_ref().filter(|p| p.from.resolved == src.end.resolved && p.to.resolved == to.resolved) {
        let landed = dst.files.stat(&to.resolved, true).await.ok().and_then(|s| s.hash);
        if prev.hash.is_some() && landed == prev.hash {
            let mut done = prev.clone();
            if done.state != TransferState::Committed {
                // The commit happened; the record did not say so yet.
                done.state = TransferState::Committed;
                done.bytes_done = done.size;
                done.finished_at_ms = Some(crate::now_ms());
                done.step = None;
                done.error = None;
                save(sd, &done)?;
            }
            return Ok(done);
        }
        if prev.state != TransferState::Committed {
            let _ = dst.files.remove(&prev.temp_path).await;
        }
    }
    let temp_path = crate::runtime::target_join(parent_of(&to.resolved), &[&format!(".{}.{id}.part", base_name(&to.resolved))]);
    let mut record = TransferRecord {
        transfer_id: id.clone(),
        from: TransferEnd {
            resolved: source.path.clone(),
            ..src.end.clone()
        },
        to,
        temp_path,
        replace: req.replace,
        size: source.size,
        bytes_done: 0,
        hash: None,
        state: TransferState::Transferring,
        step: None,
        error: None,
        call_id: req.call_id.clone(),
        started_at_ms: crate::now_ms(),
        finished_at_ms: None,
    };
    let _ = std::fs::remove_file(cancel_mark(sd, &id)?);
    save(sd, &record)?;
    let result = run(sd, &src, &dst, &mut record).await;
    let mark = cancel_mark(sd, &id)?;
    let cancelled = mark.exists();
    let _ = std::fs::remove_file(&mark);
    if let Err((step, error)) = result {
        let _ = dst.files.remove(&record.temp_path).await;
        record.state = if cancelled { TransferState::Cancelled } else { TransferState::Failed };
        record.step = Some(step.to_string());
        record.error = Some(error);
    } else {
        record.state = TransferState::Committed;
    }
    record.finished_at_ms = Some(crate::now_ms());
    save(sd, &record)?;
    Ok(record)
}

/// Move, verify, commit. `Err((step, why))`; the caller removes the
/// temporary file.
async fn run(
    sd: &SessionDir,
    src: &Side,
    dst: &Side,
    record: &mut TransferRecord,
) -> std::result::Result<(), (&'static str, String)> {
    let chunk = src
        .files
        .limits()
        .max_chunk_bytes
        .min(dst.files.limits().max_chunk_bytes)
        .max(1);
    let mark = cancel_mark(sd, &record.transfer_id).map_err(|e| ("resolve", e.to_string()))?;
    let mut offset = 0u64;
    loop {
        if mark.exists() {
            return Err(("write", "cancelled".into()));
        }
        let want = chunk.min(record.size - offset);
        let data = if want == 0 {
            Vec::new()
        } else {
            let read = src
                .files
                .read_range(&record.from.resolved, offset, want)
                .await
                .map_err(|e| ("read", exec_err(e)))?;
            if read.file_size != record.size || read.data.len() as u64 != want {
                return Err(("read", format!(
                    "the source changed during the copy ({} bytes, then {})",
                    record.size, read.file_size
                )));
            }
            read.data
        };
        let (mode, expected_size) = if offset == 0 {
            (WholeWriteMode::CreateNew, None)
        } else {
            (WholeWriteMode::Append, Some(offset))
        };
        dst.files
            .write_chunk(
                &record.temp_path,
                WriteChunk {
                    mode,
                    expected_size,
                    expected_hash: None,
                    data: &data,
                },
            )
            .await
            .map_err(|e| ("write", exec_err(e)))?;
        offset += data.len() as u64;
        record.bytes_done = offset;
        let _ = save(sd, record);
        if offset >= record.size {
            break;
        }
    }
    record.state = TransferState::Verifying;
    let _ = save(sd, record);
    let source = src
        .files
        .stat(&record.from.resolved, true)
        .await
        .map_err(|e| ("verify", exec_err(e)))?;
    let landed = dst
        .files
        .stat(&record.temp_path, true)
        .await
        .map_err(|e| ("verify", exec_err(e)))?;
    if source.size != record.size || source.hash.is_none() || source.hash != landed.hash {
        return Err(("verify", format!(
            "the copy does not match the source (source {} {:?}, copy {} {:?}): the source may have changed",
            source.size, source.hash, landed.size, landed.hash
        )));
    }
    record.hash = source.hash;
    if mark.exists() {
        return Err(("commit", "cancelled".into()));
    }
    dst.files
        .rename(&record.temp_path, &record.to.resolved, record.replace)
        .await
        .map_err(|e| match e.code {
            ExecutorErrorCode::Conflict => (
                "commit",
                format!("{} exists; pass --replace to overwrite it", record.to.resolved),
            ),
            _ => ("commit", exec_err(e)),
        })?;
    record.bytes_done = record.size;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_paths() {
        let ep: Endpoint = r"workspace:C:\work\a b\x.bin".parse().unwrap();
        assert_eq!((ep.runtime.as_str(), ep.path.as_str()), ("workspace", r"C:\work\a b\x.bin"));
        assert!("nocolon".parse::<Endpoint>().is_err());
        assert!(":x".parse::<Endpoint>().is_err());
        assert_eq!(below("/s", "in/a.bin", "session").unwrap(), "/s/in/a.bin");
        assert_eq!(below(r"C:\w", "in/a.bin", "workspace").unwrap(), r"C:\w\in\a.bin");
        assert_eq!(below("/s", "/s/x", "session").unwrap(), "/s/x");
        assert_eq!(below("/s", "out/", "session").unwrap(), "/s/out/");
        assert!(below("/s", "/other/x", "session").is_err());
        assert!(below("/s", "../x", "session").is_err());
        assert_eq!(base_name(r"C:\w\中文 文件.bin"), "中文 文件.bin");
        assert_eq!(parent_of("/s/in/a.bin"), "/s/in");
        assert_eq!(parent_of("/a.bin"), "/");
        assert_eq!(parent_of(r"C:\w\a.bin"), r"C:\w");
    }
}
