//! `remote_node`: shell and file tools executed by a Node Executor over a
//! Command Stream (doc/arch/node_exector.md §7). The host injects the
//! connector (stream + App session token); every tool call is one remote
//! operation whose locator is persisted in the run before it is sent.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use buckyos_api::{
    encode_executor_control, encode_executor_data, executor_content_hash, is_forbidden_env_name,
    read_executor_frame, ExecutorChannel, ExecutorClientInfo, ExecutorClientMessage,
    ExecutorDataFrame, ExecutorDescribeOk, ExecutorDescribeRequest, ExecutorEdit,
    ExecutorEditRequest, ExecutorError, ExecutorErrorCode, ExecutorExecRequest, ExecutorFrame,
    ExecutorLimits, ExecutorLogin, ExecutorLoginOk, ExecutorOpRef, ExecutorOpState,
    ExecutorOpStatus, ExecutorQueryRequest, ExecutorReadRequest, ExecutorServerMessage,
    ExecutorUploadEnd, ExecutorWriteMode, ExecutorWriteRequest, EXECUTOR_MAX_DATA_BYTES,
    EXECUTOR_MAX_QUERY_BYTES, NODE_EXECUTOR_PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

use super::files::{check_allowed, FileBackend, WholeEditResult, WholeWriteMode, WholeWriteResult};
use super::{
    AgentRuntime, Capabilities, RuntimeConfig, RuntimeDescriptor, RuntimeInfo, RuntimeOpenCtx,
    CURRENT_TOOL_CALL,
};
use crate::exec_tracking::{unresolved_reason, InflightAction};
use crate::llm_bash::{
    sanitize_call_id, BashRunOutput, BashRunRequest, BashRunner, CommandHandle, CommandProgress,
    RunBinding, RunBindingSlot, OUTPUT_TAIL_BYTES, TOOL_SHELL,
};
use crate::xllm::{EffectiveTools, ToolsConfig, XllmDeps, XllmError, XllmToolManager};
use crate::{AgentToolError, SessionRuntimeContext};
use llm_context::tasks::{TaskResult, TaskState};

pub const REMOTE_NODE_KIND: &str = "remote_node";
const LOCATOR_VERSION: u32 = 1;
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_SLACK: Duration = Duration::from_secs(30);
const QUERY_WAIT_MS: u64 = 25_000;
const FILE_OP_TIMEOUT: Duration = Duration::from_secs(180);
const KILL_CONFIRM: Duration = Duration::from_secs(30);

pub trait NodeIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> NodeIo for T {}

/// Supplied by the host: a fresh byte stream to the executor of a node (via
/// the local gateway) and the App session token to log in with.
#[async_trait]
pub trait NodeExecutorConnector: Send + Sync {
    async fn connect(&self, node_did: &str) -> std::io::Result<Box<dyn NodeIo>>;
    async fn session_token(&self) -> Result<String, String>;
}

fn unavailable(msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(ExecutorErrorCode::Unavailable, msg).unknown_effect()
}

fn tool_error(e: ExecutorError) -> AgentToolError {
    match e.code {
        ExecutorErrorCode::InvalidArgument
        | ExecutorErrorCode::Forbidden
        | ExecutorErrorCode::Conflict
        | ExecutorErrorCode::NotFound => AgentToolError::InvalidArgs(e.message),
        _ if e.effect_unknown => AgentToolError::Transport {
            message: format!("remote_node {:?}: {}", e.code, e.message),
            effect_unknown: true,
        },
        _ => AgentToolError::ExecFailed(format!("remote_node {:?}: {}", e.code, e.message)),
    }
}

fn now_ms() -> u64 {
    crate::now_ms()
}

struct Conn {
    writer: tokio::sync::Mutex<tokio::io::WriteHalf<Box<dyn NodeIo>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ExecutorServerMessage>>>>,
    alive: Arc<AtomicBool>,
    login: ExecutorLoginOk,
    run_id: String,
    reader: tokio::task::JoinHandle<()>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// One logged-in stream at a time; a lost stream is replaced on the next
/// request, and a request lost with it is resent under the same op_id.
pub(crate) struct NodeClient {
    connector: Arc<dyn NodeExecutorConnector>,
    node_did: String,
    profile_id: String,
    runtime_id: String,
    run: RunBindingSlot,
    default_run_id: String,
    expected_store: Option<String>,
    conn: tokio::sync::Mutex<Option<Arc<Conn>>>,
    seq: AtomicU64,
}

impl std::fmt::Debug for NodeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeClient")
            .field("node_did", &self.node_did)
            .field("profile_id", &self.profile_id)
            .finish()
    }
}

impl NodeClient {
    pub(crate) fn new(
        connector: Arc<dyn NodeExecutorConnector>,
        node_did: &str,
        profile_id: &str,
        runtime_id: &str,
        run: RunBindingSlot,
        default_run_id: &str,
        expected_store: Option<String>,
    ) -> Self {
        Self {
            connector,
            node_did: node_did.into(),
            profile_id: profile_id.into(),
            runtime_id: runtime_id.into(),
            run,
            default_run_id: default_run_id.into(),
            expected_store,
            conn: tokio::sync::Mutex::new(None),
            seq: AtomicU64::new(1),
        }
    }

    fn run_id(&self) -> String {
        self.run
            .lock()
            .expect("run binding")
            .as_ref()
            .map(|b| b.run_id.clone())
            .unwrap_or_else(|| self.default_run_id.clone())
    }

    fn rid(&self) -> String {
        format!("q{}", self.seq.fetch_add(1, Ordering::SeqCst))
    }

    async fn open_stream(&self, run_id: &str) -> Result<Conn, ExecutorError> {
        let token = self.connector.session_token().await.map_err(|e| {
            ExecutorError::new(ExecutorErrorCode::Unauthenticated, format!("no session token: {e}"))
        })?;
        let stream = self
            .connector
            .connect(&self.node_did)
            .await
            .map_err(|e| ExecutorError::new(ExecutorErrorCode::Unavailable, format!("cannot reach the node executor of {}: {e}", self.node_did)))?;
        let (mut rd, mut wr) = tokio::io::split(stream);
        let login = ExecutorClientMessage::Login(ExecutorLogin {
            v: NODE_EXECUTOR_PROTOCOL_VERSION,
            request_id: Some("login".into()),
            token,
            profile: self.profile_id.clone(),
            client: ExecutorClientInfo {
                runtime_id: self.runtime_id.clone(),
                run_id: run_id.to_string(),
            },
        });
        let bytes = encode_executor_control(&login).map_err(|e| unavailable(e.to_string()))?;
        wr.write_all(&bytes).await.map_err(|e| unavailable(e.to_string()))?;
        wr.flush().await.map_err(|e| unavailable(e.to_string()))?;
        let reply = tokio::time::timeout(LOGIN_TIMEOUT, async {
            loop {
                match read_executor_frame(&mut rd).await {
                    Ok(Some(ExecutorFrame::Control(p))) => {
                        return serde_json::from_slice::<ExecutorServerMessage>(&p)
                            .map_err(|e| unavailable(e.to_string()))
                    }
                    Ok(Some(ExecutorFrame::Data(_))) => continue,
                    Ok(None) => return Err(unavailable("stream closed during login")),
                    Err(e) => return Err(unavailable(e.to_string())),
                }
            }
        })
        .await
        .map_err(|_| unavailable("login timed out"))??;
        let login = match reply {
            ExecutorServerMessage::LoginOk(ok) => ok,
            ExecutorServerMessage::Error(e) => return Err(e.error),
            other => return Err(unavailable(format!("unexpected login reply {other:?}"))),
        };
        if let Some(expected) = &self.expected_store {
            if expected != &login.store_id {
                return Err(ExecutorError::new(
                    ExecutorErrorCode::StoreMismatch,
                    format!(
                        "the node's operation ledger changed ({} → {}); earlier operations cannot be resumed",
                        expected, login.store_id
                    ),
                ));
            }
        }
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<ExecutorServerMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let reader = {
            let pending = pending.clone();
            let alive = alive.clone();
            tokio::spawn(async move {
                loop {
                    let frame = match read_executor_frame(&mut rd).await {
                        Ok(Some(f)) => f,
                        _ => break,
                    };
                    let ExecutorFrame::Control(payload) = frame else { continue };
                    let Ok(msg) = serde_json::from_slice::<ExecutorServerMessage>(&payload) else {
                        continue;
                    };
                    let rid = match &msg {
                        ExecutorServerMessage::DescribeOk(m) => Some(m.request_id.clone()),
                        ExecutorServerMessage::Accepted(m) | ExecutorServerMessage::Status(m) => {
                            Some(m.request_id.clone())
                        }
                        ExecutorServerMessage::QueryOk(m) => Some(m.request_id.clone()),
                        ExecutorServerMessage::Error(m) => m.request_id.clone(),
                        ExecutorServerMessage::Pong(m) => Some(m.request_id.clone()),
                        ExecutorServerMessage::Closing(_) => break,
                        _ => None,
                    };
                    if let Some(rid) = rid {
                        if let Some(tx) = pending.lock().unwrap().remove(&rid) {
                            let _ = tx.send(msg);
                        }
                    }
                }
                alive.store(false, Ordering::SeqCst);
                pending.lock().unwrap().clear();
            })
        };
        Ok(Conn {
            writer: tokio::sync::Mutex::new(wr),
            pending,
            alive,
            login,
            run_id: run_id.to_string(),
            reader,
        })
    }

    async fn conn(&self) -> Result<Arc<Conn>, ExecutorError> {
        let mut guard = self.conn.lock().await;
        let run_id = self.run_id();
        if let Some(c) = guard.as_ref() {
            if c.alive.load(Ordering::SeqCst)
                && c.run_id == run_id
                && c.login.token_expires_at_ms > now_ms() + 10_000
            {
                return Ok(c.clone());
            }
        }
        let conn = Arc::new(self.open_stream(&run_id).await?);
        *guard = Some(conn.clone());
        Ok(conn)
    }

    pub(crate) async fn login_info(&self) -> Result<ExecutorLoginOk, ExecutorError> {
        Ok(self.conn().await?.login.clone())
    }

    /// Send one request (plus an upload) and wait for its reply; resend once
    /// on a new stream when the stream is lost.
    async fn request(
        &self,
        build: impl Fn(&str, &str, &str) -> ExecutorClientMessage,
        upload: Option<(&str, &[u8])>,
        wait: Duration,
    ) -> Result<ExecutorServerMessage, ExecutorError> {
        let mut last = unavailable("node executor stream lost");
        for _ in 0..2 {
            let conn = self.conn().await?;
            let rid = self.rid();
            let (tx, rx) = oneshot::channel();
            conn.pending.lock().unwrap().insert(rid.clone(), tx);
            let msg = build(&rid, &conn.login.store_id, &conn.run_id);
            let sent: std::io::Result<()> = async {
                let mut w = conn.writer.lock().await;
                w.write_all(&encode_executor_control(&msg)?).await?;
                if let Some((op_id, content)) = upload {
                    for (i, chunk) in content.chunks(EXECUTOR_MAX_DATA_BYTES).enumerate() {
                        let frame = ExecutorDataFrame {
                            op_id: op_id.to_string(),
                            channel: ExecutorChannel::Upload,
                            offset: (i * EXECUTOR_MAX_DATA_BYTES) as u64,
                            data: chunk.to_vec(),
                        };
                        w.write_all(&encode_executor_data(&frame)?).await?;
                    }
                    let end = ExecutorClientMessage::UploadEnd(ExecutorUploadEnd {
                        request_id: format!("{rid}-end"),
                        op_id: op_id.to_string(),
                    });
                    w.write_all(&encode_executor_control(&end)?).await?;
                }
                w.flush().await
            }
            .await;
            if let Err(e) = sent {
                conn.alive.store(false, Ordering::SeqCst);
                last = unavailable(e.to_string());
                continue;
            }
            match tokio::time::timeout(wait + REQUEST_SLACK, rx).await {
                Ok(Ok(ExecutorServerMessage::Error(e))) => return Err(e.error),
                Ok(Ok(reply)) => return Ok(reply),
                Ok(Err(_)) => {
                    last = unavailable("node executor stream closed");
                }
                Err(_) => {
                    conn.alive.store(false, Ordering::SeqCst);
                    last = unavailable("node executor did not answer");
                }
            }
        }
        Err(last)
    }

    pub(crate) async fn describe(&self, workdir: Option<String>) -> Result<ExecutorDescribeOk, ExecutorError> {
        match self
            .request(
                |rid, _, _| {
                    ExecutorClientMessage::Describe(ExecutorDescribeRequest {
                        request_id: rid.into(),
                        workdir: workdir.clone(),
                    })
                },
                None,
                Duration::ZERO,
            )
            .await?
        {
            ExecutorServerMessage::DescribeOk(ok) => Ok(ok),
            other => Err(unavailable(format!("unexpected reply {other:?}"))),
        }
    }

    fn accepted(reply: ExecutorServerMessage) -> Result<ExecutorOpStatus, ExecutorError> {
        match reply {
            ExecutorServerMessage::Accepted(r) | ExecutorServerMessage::Status(r) => Ok(r.op),
            other => Err(unavailable(format!("unexpected reply {other:?}"))),
        }
    }

    pub(crate) async fn submit(&self, loc: &OperationLocator, op: &RemoteOp<'_>) -> Result<ExecutorOpStatus, ExecutorError> {
        let op_id = loc.op_id.clone();
        let call_id = loc.call_id.clone();
        let reply = match op {
            RemoteOp::Exec { cmd, cwd, env, timeout_ms } => {
                self.request(
                    |rid, store, run| {
                        ExecutorClientMessage::Exec(ExecutorExecRequest {
                            request_id: rid.into(),
                            store_id: store.into(),
                            op_id: op_id.clone(),
                            run_id: run.into(),
                            call_id: call_id.clone(),
                            cmd: cmd.to_string(),
                            cwd: cwd.to_string(),
                            env_overrides: (*env).clone(),
                            execution_timeout_ms: Some(*timeout_ms),
                        })
                    },
                    None,
                    Duration::ZERO,
                )
                .await?
            }
            RemoteOp::Read { path } => {
                self.request(
                    |rid, store, run| {
                        ExecutorClientMessage::Read(ExecutorReadRequest {
                            request_id: rid.into(),
                            store_id: store.into(),
                            op_id: op_id.clone(),
                            run_id: run.into(),
                            call_id: call_id.clone(),
                            path: path.to_string(),
                            offset: 0,
                            length: None,
                        })
                    },
                    None,
                    Duration::ZERO,
                )
                .await?
            }
            RemoteOp::Write { path, mode, content } => {
                self.request(
                    |rid, store, run| {
                        ExecutorClientMessage::Write(ExecutorWriteRequest {
                            request_id: rid.into(),
                            store_id: store.into(),
                            op_id: op_id.clone(),
                            run_id: run.into(),
                            call_id: call_id.clone(),
                            path: path.to_string(),
                            mode: *mode,
                            expected_hash: None,
                            content_length: content.len() as u64,
                            content_hash: executor_content_hash(content),
                        })
                    },
                    Some((&op_id, content)),
                    Duration::ZERO,
                )
                .await?
            }
            RemoteOp::Edit { path, old, new } => {
                self.request(
                    |rid, store, run| {
                        ExecutorClientMessage::Edit(ExecutorEditRequest {
                            request_id: rid.into(),
                            store_id: store.into(),
                            op_id: op_id.clone(),
                            run_id: run.into(),
                            call_id: call_id.clone(),
                            path: path.to_string(),
                            edits: vec![ExecutorEdit {
                                old_string: old.to_string(),
                                new_string: new.to_string(),
                                replace_all: false,
                            }],
                            expected_hash: None,
                        })
                    },
                    None,
                    Duration::ZERO,
                )
                .await?
            }
        };
        Self::accepted(reply)
    }

    pub(crate) async fn query(
        &self,
        op_id: &str,
        channel: Option<ExecutorChannel>,
        offset: u64,
        wait_ms: u64,
    ) -> Result<(ExecutorOpStatus, Option<(Vec<u8>, bool)>), ExecutorError> {
        let reply = self
            .request(
                |rid, store, _| {
                    ExecutorClientMessage::Query(ExecutorQueryRequest {
                        request_id: rid.into(),
                        store_id: store.into(),
                        op_id: op_id.into(),
                        channel,
                        offset,
                        max_bytes: Some(EXECUTOR_MAX_QUERY_BYTES),
                        wait_ms: Some(wait_ms),
                    })
                },
                None,
                Duration::from_millis(wait_ms),
            )
            .await?;
        match reply {
            ExecutorServerMessage::QueryOk(q) => {
                let chunk = match q.chunk {
                    Some(c) => Some((
                        base64::engine::general_purpose::STANDARD
                            .decode(c.data_b64)
                            .map_err(|e| unavailable(e.to_string()))?,
                        c.eof,
                    )),
                    None => None,
                };
                Ok((q.op, chunk))
            }
            other => Err(unavailable(format!("unexpected reply {other:?}"))),
        }
    }

    pub(crate) async fn cancel(&self, op_id: &str) -> Result<ExecutorOpStatus, ExecutorError> {
        let reply = self
            .request(
                |rid, store, _| {
                    ExecutorClientMessage::Cancel(ExecutorOpRef {
                        request_id: rid.into(),
                        store_id: store.into(),
                        op_id: op_id.into(),
                    })
                },
                None,
                Duration::ZERO,
            )
            .await?;
        Self::accepted(reply)
    }

    /// Long-poll until terminal or `deadline`; `Ok(None)` at the deadline.
    pub(crate) async fn wait_terminal(&self, op_id: &str, deadline: Instant) -> Result<Option<ExecutorOpStatus>, ExecutorError> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let wait = (left.as_millis() as u64).min(QUERY_WAIT_MS);
            let (status, _) = self.query(op_id, None, 0, wait).await?;
            if status.state.is_terminal() {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    pub(crate) async fn read_channel(&self, op_id: &str, channel: ExecutorChannel, max: usize) -> Result<Vec<u8>, ExecutorError> {
        let mut out = Vec::new();
        loop {
            let (_, chunk) = self.query(op_id, Some(channel), out.len() as u64, 0).await?;
            let Some((data, eof)) = chunk else { break };
            if data.is_empty() {
                break;
            }
            out.extend_from_slice(&data);
            if eof || out.len() >= max {
                break;
            }
        }
        out.truncate(max);
        Ok(out)
    }
}

pub(crate) enum RemoteOp<'a> {
    Exec {
        cmd: &'a str,
        cwd: &'a str,
        env: &'a BTreeMap<String, String>,
        timeout_ms: u64,
    },
    Read {
        path: &'a str,
    },
    Write {
        path: &'a str,
        mode: ExecutorWriteMode,
        content: &'a [u8],
    },
    Edit {
        path: &'a str,
        old: &'a str,
        new: &'a str,
    },
}

impl RemoteOp<'_> {
    fn kind(&self) -> &'static str {
        match self {
            RemoteOp::Exec { .. } => "exec",
            RemoteOp::Read { .. } => "read",
            RemoteOp::Write { .. } => "write",
            RemoteOp::Edit { .. } => "edit",
        }
    }

    fn digest(&self) -> String {
        let value = match self {
            RemoteOp::Exec { cmd, cwd, env, timeout_ms } => {
                json!({"kind": "exec", "cmd": cmd, "cwd": cwd, "env": env, "timeout_ms": timeout_ms})
            }
            RemoteOp::Read { path } => json!({"kind": "read", "path": path}),
            RemoteOp::Write { path, mode, content } => json!({
                "kind": "write", "path": path, "mode": mode, "content_hash": executor_content_hash(content)
            }),
            RemoteOp::Edit { path, old, new } => {
                json!({"kind": "edit", "path": path, "old": old, "new": new})
            }
        };
        executor_content_hash(value.to_string().as_bytes())
    }
}

/// `<run_dir>/runtime_operations/<call>.json`, fsynced before the request
/// leaves, so a crashed runner can find and query (never replay) it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OperationLocator {
    pub version: u32,
    pub node_did: String,
    pub profile_id: String,
    pub runtime_id: String,
    pub store_id: String,
    pub run_id: String,
    pub call_id: String,
    pub op_id: String,
    pub kind: String,
    pub request_digest: String,
    pub created_at_ms: u64,
}

fn locator_path(run_dir: &Path, call_id: &str) -> PathBuf {
    run_dir
        .join("runtime_operations")
        .join(format!("{}.json", sanitize_call_id(call_id)))
}

fn read_locator(run_dir: &Path, call_id: &str) -> Option<OperationLocator> {
    let bytes = std::fs::read(locator_path(run_dir, call_id)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_synced(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().expect("locator dir");
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", uuid_like()));
    {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        std::io::Write::write_all(&mut f, data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    std::fs::File::open(dir)?.sync_all()
}

fn uuid_like() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = NEXT_LOCAL.fetch_add(1, Ordering::SeqCst);
    format!("{nanos:x}{:x}{n:x}", std::process::id())
}

static NEXT_LOCAL: AtomicU64 = AtomicU64::new(1);

impl NodeClient {
    /// Reuse the locator of this call (same request) or create it.
    pub(crate) async fn locator(&self, call_id: &str, op: &RemoteOp<'_>) -> Result<OperationLocator, AgentToolError> {
        let binding = self.run.lock().expect("run binding").clone();
        let Some(run_dir) = binding.as_ref().and_then(|b| b.run_dir.clone()) else {
            return Err(AgentToolError::ExecFailed(
                "Capability: remote_node needs a persistent run directory for its operation locators".into(),
            ));
        };
        let digest = op.digest();
        if let Some(existing) = read_locator(&run_dir, call_id) {
            if existing.request_digest != digest || existing.kind != op.kind() {
                return Err(AgentToolError::InvalidArgs(format!(
                    "call {call_id} already has a different remote operation"
                )));
            }
            return Ok(existing);
        }
        let login = self.login_info().await.map_err(tool_error)?;
        let loc = OperationLocator {
            version: LOCATOR_VERSION,
            node_did: self.node_did.clone(),
            profile_id: self.profile_id.clone(),
            runtime_id: self.runtime_id.clone(),
            store_id: login.store_id,
            run_id: self.run_id(),
            call_id: call_id.into(),
            op_id: format!("op-{}", uuid_like()),
            kind: op.kind().into(),
            request_digest: digest,
            created_at_ms: now_ms(),
        };
        let path = locator_path(&run_dir, call_id);
        let data = serde_json::to_vec_pretty(&loc).map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        tokio::task::spawn_blocking(move || write_synced(&path, &data))
            .await
            .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?
            .map_err(|e| AgentToolError::ExecFailed(format!("persist operation locator: {e}")))?;
        Ok(loc)
    }

    /// Persist the locator, then submit (or rejoin) the operation.
    pub(crate) async fn start_op(&self, call_id: &str, op: RemoteOp<'_>) -> Result<(OperationLocator, ExecutorOpStatus), AgentToolError> {
        let loc = self.locator(call_id, &op).await?;
        let status = self.submit(&loc, &op).await.map_err(tool_error)?;
        Ok((loc, status))
    }
}

/// Lexical resolution against the remote workdir; the node resolves
/// symlinks and enforces its roots. A Windows workdir (`C:\...`) follows
/// Windows path rules whatever OS this side runs on.
fn remote_resolve(root: &Path, raw: &str, allowed: &[PathBuf]) -> Result<PathBuf, AgentToolError> {
    let root_text = root.to_string_lossy();
    if !is_drive_path(&root_text) {
        let lexical = crate::path_utils::resolve_path_from_root(root, raw)?;
        check_allowed(&lexical, allowed)?;
        return Ok(lexical);
    }
    if raw.trim().is_empty() {
        return Err(AgentToolError::InvalidArgs("path cannot be empty".to_string()));
    }
    let lexical = if is_drive_path(raw) {
        normalize_drive_path(raw)
    } else {
        normalize_drive_path(&format!("{root_text}\\{raw}"))
    };
    let inside = |root: &PathBuf| {
        let root = normalize_drive_path(&root.to_string_lossy()).to_ascii_lowercase();
        let path = lexical.to_ascii_lowercase();
        path == root || path.starts_with(&format!("{}\\", root.trim_end_matches('\\')))
    };
    if !allowed.is_empty() && !allowed.iter().any(inside) {
        return Err(AgentToolError::InvalidArgs(format!("path not allowed by policy: {lexical}")));
    }
    Ok(PathBuf::from(lexical))
}

fn is_drive_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/')
}

/// `C:/a/./b/../c` → `C:\a\c`; `..` stops at the drive root.
fn normalize_drive_path(path: &str) -> String {
    let (drive, rest) = path.split_at(2);
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("{}\\{}", drive.to_ascii_uppercase(), parts.join("\\"))
}

fn current_call_id(what: &str) -> Result<String, AgentToolError> {
    CURRENT_TOOL_CALL.try_with(|c| c.clone()).map_err(|_| {
        AgentToolError::ExecFailed(format!(
            "Capability: remote_node {what} needs a tool call id (template __EXEC__ is not supported)"
        ))
    })
}

fn filtered_env(env: impl IntoIterator<Item = (String, String)>) -> BTreeMap<String, String> {
    env.into_iter()
        .filter(|(k, _)| !is_forbidden_env_name(k))
        .collect()
}

pub(crate) struct NodeBashRunner {
    client: Arc<NodeClient>,
    env: BTreeMap<String, String>,
    max_execution_ms: u64,
}

#[async_trait]
impl BashRunner for NodeBashRunner {
    async fn resolve_cwd(&self, root: &Path, raw: Option<&str>, restricted: bool) -> Result<PathBuf, AgentToolError> {
        let allowed = if restricted { vec![root.to_path_buf()] } else { Vec::new() };
        match raw {
            Some(raw) if !raw.trim().is_empty() => remote_resolve(root, raw.trim(), &allowed),
            _ => Ok(root.to_path_buf()),
        }
    }

    async fn start(&self, _ctx: &SessionRuntimeContext, req: BashRunRequest) -> Result<Box<dyn CommandHandle>, AgentToolError> {
        self.launch(req, self.max_execution_ms).await
    }

    async fn run(&self, _ctx: &SessionRuntimeContext, req: BashRunRequest) -> Result<BashRunOutput, AgentToolError> {
        let timeout = req.timeout_ms.max(1).min(self.max_execution_ms);
        let mut handle = self.launch(req, timeout).await?;
        match handle.wait(Duration::from_millis(timeout) + REQUEST_SLACK).await? {
            Some(out) => Ok(out),
            None => handle.kill().await,
        }
    }

    fn engine(&self) -> &str {
        REMOTE_NODE_KIND
    }
}

impl NodeBashRunner {
    async fn launch(&self, req: BashRunRequest, timeout_ms: u64) -> Result<Box<dyn CommandHandle>, AgentToolError> {
        let call_id = req.call_id.clone().ok_or_else(|| {
            AgentToolError::ExecFailed(
                "Capability: remote_node does not run commands without a tool call id (template __EXEC__ is disabled)".into(),
            )
        })?;
        let mut env = self.env.clone();
        env.extend(filtered_env(req.env.iter().cloned()));
        let cwd = req.cwd.to_string_lossy().to_string();
        let (loc, _) = self
            .client
            .start_op(
                &call_id,
                RemoteOp::Exec {
                    cmd: &req.command,
                    cwd: &cwd,
                    env: &env,
                    timeout_ms,
                },
            )
            .await?;
        Ok(Box::new(NodeCommandHandle {
            client: self.client.clone(),
            op_id: loc.op_id,
            cwd: req.cwd.clone(),
            max_output: req.max_output_bytes,
            started: Instant::now(),
            tail: Mutex::new((0, Vec::new())),
            done: None,
        }))
    }
}

pub(crate) struct NodeCommandHandle {
    client: Arc<NodeClient>,
    op_id: String,
    cwd: PathBuf,
    max_output: usize,
    started: Instant,
    tail: Mutex<(u64, Vec<u8>)>,
    done: Option<BashRunOutput>,
}

impl NodeCommandHandle {
    async fn output_of(&self, status: &ExecutorOpStatus) -> Result<BashRunOutput, AgentToolError> {
        let result = status.result.clone();
        let stdout = self
            .client
            .read_channel(&self.op_id, ExecutorChannel::Stdout, self.max_output + 1)
            .await
            .map_err(tool_error)?;
        let mut stderr = self
            .client
            .read_channel(&self.op_id, ExecutorChannel::Stderr, self.max_output + 1)
            .await
            .map_err(tool_error)?;
        let state = status.state;
        if let Some(err) = result.as_ref().and_then(|r| r.error.as_ref()) {
            if !matches!(state, ExecutorOpState::Succeeded | ExecutorOpState::Failed) || result.as_ref().and_then(|r| r.exit_code).is_none() {
                stderr.extend_from_slice(format!("\n[remote_node {:?}] {}", state, err.message).as_bytes());
            }
        }
        let exit_code = result.as_ref().and_then(|r| r.exit_code).unwrap_or(match state {
            ExecutorOpState::TimedOut => crate::llm_bash::TIMEOUT_EXIT_CODE,
            ExecutorOpState::Cancelled => 130,
            _ => -1,
        });
        let timed_out = matches!(state, ExecutorOpState::TimedOut | ExecutorOpState::Cancelled);
        let mut out = super::output(
            exit_code,
            &stdout,
            &stderr,
            self.max_output,
            timed_out,
            self.started.elapsed(),
            REMOTE_NODE_KIND,
            self.cwd.clone(),
        );
        let remote_truncated = result
            .as_ref()
            .map(|r| {
                r.stdout.as_ref().map(|s| s.truncated).unwrap_or(false)
                    || r.stderr.as_ref().map(|s| s.truncated).unwrap_or(false)
            })
            .unwrap_or(false);
        out.output_truncated |= remote_truncated;
        Ok(out)
    }
}

#[async_trait]
impl CommandHandle for NodeCommandHandle {
    async fn wait(&mut self, timeout: Duration) -> Result<Option<BashRunOutput>, AgentToolError> {
        if let Some(out) = &self.done {
            return Ok(Some(out.clone()));
        }
        let status = self
            .client
            .wait_terminal(&self.op_id, Instant::now() + timeout)
            .await
            .map_err(tool_error)?;
        match status {
            Some(status) => {
                let out = self.output_of(&status).await?;
                self.done = Some(out.clone());
                Ok(Some(out))
            }
            None => Ok(None),
        }
    }

    async fn progress(&self) -> CommandProgress {
        let offset = self.tail.lock().unwrap().0;
        if let Ok((_, Some((data, _)))) = self.client.query(&self.op_id, Some(ExecutorChannel::Stdout), offset, 0).await {
            let mut tail = self.tail.lock().unwrap();
            tail.0 += data.len() as u64;
            tail.1.extend_from_slice(&data);
            let excess = tail.1.len().saturating_sub(OUTPUT_TAIL_BYTES);
            tail.1.drain(..excess);
        }
        CommandProgress {
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            output_tail: String::from_utf8_lossy(&self.tail.lock().unwrap().1).to_string(),
        }
    }

    async fn kill(&mut self) -> Result<BashRunOutput, AgentToolError> {
        if let Some(out) = &self.done {
            return Ok(out.clone());
        }
        self.client.cancel(&self.op_id).await.map_err(tool_error)?;
        match self
            .client
            .wait_terminal(&self.op_id, Instant::now() + KILL_CONFIRM)
            .await
            .map_err(tool_error)?
        {
            Some(status) => {
                let mut out = self.output_of(&status).await?;
                out.timed_out = true;
                self.done = Some(out.clone());
                Ok(out)
            }
            None => Err(AgentToolError::Transport {
                message: format!(
                    "cancel of remote operation {} was requested but its stop is not confirmed yet",
                    self.op_id
                ),
                effect_unknown: true,
            }),
        }
    }

    fn detach(&mut self) {}

    fn cancellable(&self) -> bool {
        true
    }

    fn locator(&self) -> String {
        format!("{}:{}#{}", REMOTE_NODE_KIND, self.client.node_did, self.op_id)
    }
}

/// Files behind the executor: read is one remote read; write and edit are
/// whole operations on the node (never a remote read followed by a write).
pub(crate) struct RemoteNodeFiles {
    client: Arc<NodeClient>,
    max_file_bytes: usize,
}

impl std::fmt::Debug for RemoteNodeFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteNodeFiles").field("client", &self.client).finish()
    }
}

impl RemoteNodeFiles {
    async fn run(&self, what: &str, op: RemoteOp<'_>) -> Result<ExecutorOpStatus, AgentToolError> {
        let call_id = current_call_id(what)?;
        let (loc, _) = self.client.start_op(&call_id, op).await?;
        let status = self
            .client
            .wait_terminal(&loc.op_id, Instant::now() + FILE_OP_TIMEOUT)
            .await
            .map_err(tool_error)?
            .ok_or_else(|| AgentToolError::Transport {
                message: format!("remote {what} did not finish in time"),
                effect_unknown: true,
            })?;
        match status.state {
            ExecutorOpState::Succeeded => Ok(status),
            _ => {
                let err = status
                    .result
                    .as_ref()
                    .and_then(|r| r.error.clone())
                    .unwrap_or_else(|| ExecutorError::new(ExecutorErrorCode::Internal, format!("remote {what} {:?}", status.state)));
                Err(tool_error(err))
            }
        }
    }
}

#[async_trait]
impl FileBackend for RemoteNodeFiles {
    async fn resolve(&self, root: &Path, raw: &str, allowed: &[PathBuf]) -> Result<PathBuf, AgentToolError> {
        remote_resolve(root, raw, allowed)
    }

    /// Existence is decided by the remote operation itself.
    async fn exists(&self, _path: &Path) -> Result<bool, AgentToolError> {
        Ok(true)
    }

    async fn read(&self, path: &Path) -> Result<Vec<u8>, AgentToolError> {
        let path = path.to_string_lossy().to_string();
        let status = self.run("read", RemoteOp::Read { path: &path }).await?;
        self.client
            .read_channel(&status.op_id, ExecutorChannel::Content, self.max_file_bytes)
            .await
            .map_err(tool_error)
    }

    async fn write(&self, _path: &Path, _bytes: &[u8]) -> Result<(), AgentToolError> {
        Err(AgentToolError::ExecFailed(
            "remote_node files are written by whole remote operations".into(),
        ))
    }

    async fn write_whole(&self, path: &Path, mode: WholeWriteMode, content: &[u8]) -> Option<Result<WholeWriteResult, AgentToolError>> {
        let path = path.to_string_lossy().to_string();
        let mode = match mode {
            WholeWriteMode::Replace => ExecutorWriteMode::Replace,
            WholeWriteMode::Append => ExecutorWriteMode::Append,
            WholeWriteMode::CreateNew => ExecutorWriteMode::CreateNew,
        };
        Some(
            self.run("write", RemoteOp::Write { path: &path, mode, content })
                .await
                .map(|status| {
                    let file = status.result.and_then(|r| r.file).unwrap_or_default();
                    WholeWriteResult {
                        created: file.created,
                        changed: file.changed,
                        size: file.size as usize,
                    }
                }),
        )
    }

    async fn edit_once(&self, path: &Path, old: &str, new: &str) -> Option<Result<WholeEditResult, AgentToolError>> {
        let path_s = path.to_string_lossy().to_string();
        let outcome = async {
            let status = self.run("edit", RemoteOp::Edit { path: &path_s, old, new }).await?;
            let original = self
                .client
                .read_channel(&status.op_id, ExecutorChannel::Content, self.max_file_bytes)
                .await
                .map_err(tool_error)?;
            let file = status.result.and_then(|r| r.file).unwrap_or_default();
            Ok(WholeEditResult {
                original,
                line: file.line.unwrap_or(1) as usize,
                changed: file.changed,
            })
        }
        .await;
        Some(outcome)
    }
}

/// Reads a previous executor's operations from the run's locators and the
/// node, without starting anything.
pub(crate) struct NodeRecovery {
    connector: Arc<dyn NodeExecutorConnector>,
    node_did: String,
    profile_id: String,
}

enum Recovered {
    NoLocator,
    Status(OperationLocator, ExecutorOpStatus, String, String),
    Failed(OperationLocator, ExecutorError),
}

impl NodeRecovery {
    async fn lookup(&self, run: &RunBinding, call_id: &str) -> Recovered {
        let Some(loc) = run.run_dir.as_deref().and_then(|d| read_locator(d, call_id)) else {
            return Recovered::NoLocator;
        };
        let slot: RunBindingSlot = Arc::new(Mutex::new(Some(RunBinding {
            run_id: loc.run_id.clone(),
            run_dir: run.run_dir.clone(),
        })));
        let client = NodeClient::new(
            self.connector.clone(),
            &self.node_did,
            &self.profile_id,
            &loc.runtime_id,
            slot,
            &loc.run_id,
            Some(loc.store_id.clone()),
        );
        let status = match client.query(&loc.op_id, None, 0, 0).await {
            Ok((s, _)) => s,
            Err(e) => return Recovered::Failed(loc, e),
        };
        let tail = |summary: Option<u64>| summary.unwrap_or(0).saturating_sub(OUTPUT_TAIL_BYTES as u64);
        let result = status.result.clone();
        let mut out = String::new();
        let mut err = String::new();
        if !status.expired {
            if let Ok((_, Some((d, _)))) = client
                .query(&loc.op_id, Some(ExecutorChannel::Stdout), tail(result.as_ref().and_then(|r| r.stdout.as_ref().map(|s| s.retained_bytes))), 0)
                .await
            {
                out = String::from_utf8_lossy(&d).to_string();
            }
            if let Ok((_, Some((d, _)))) = client
                .query(&loc.op_id, Some(ExecutorChannel::Stderr), tail(result.as_ref().and_then(|r| r.stderr.as_ref().map(|s| s.retained_bytes))), 0)
                .await
            {
                err = String::from_utf8_lossy(&d).to_string();
            }
        }
        Recovered::Status(loc, status, out, err)
    }

    fn tails(out: &str, err: &str) -> String {
        let mut s = String::new();
        if !out.trim().is_empty() {
            s.push_str("\n--- stdout (tail) ---\n");
            s.push_str(out.trim_end());
        }
        if !err.trim().is_empty() {
            s.push_str("\n--- stderr (tail) ---\n");
            s.push_str(err.trim_end());
        }
        s
    }

    pub(crate) async fn describe(&self, run: &RunBinding, action: &InflightAction) -> String {
        let node = &self.node_did;
        let tool = &action.tool;
        match self.lookup(run, &action.call_id).await {
            Recovered::NoLocator => format!(
                "{tool} (remote_node {node}): the previous executor exited before this call was sent to the node (no operation locator), so it did not run there."
            ),
            Recovered::Failed(loc, e) if e.code == ExecutorErrorCode::StoreMismatch => format!(
                "{tool} (remote_node {node}): the previous executor exited while operation {} was in flight, and the node's operation ledger has since been reset. Whether it ran cannot be determined; check the node before repeating it.",
                loc.op_id
            ),
            Recovered::Failed(loc, e) if e.code == ExecutorErrorCode::NotFound => format!(
                "{tool} (remote_node {node}): the previous executor exited while sending operation {}; the node has no record of it, so it was not accepted and did not run.",
                loc.op_id
            ),
            Recovered::Failed(loc, e) => format!(
                "{tool} (remote_node {node}): the previous executor exited while operation {} was in flight and the node cannot be queried now ({:?}: {}). It may have completed or may still be running; check before repeating it.",
                loc.op_id, e.code, e.message
            ),
            Recovered::Status(loc, status, out, err) => {
                let result = status.result.as_ref();
                match status.state {
                    ExecutorOpState::Starting | ExecutorOpState::Running => format!(
                        "{tool} (remote_node {node}): the previous executor exited while operation {} was running; it is still running on the node. Its result was never returned; check it before repeating anything.{}",
                        loc.op_id,
                        Self::tails(&out, &err)
                    ),
                    state => format!(
                        "{tool} (remote_node {node}): the previous executor exited while operation {} was running. It has since ended on the node as {:?}{}{}; its result was never returned, so review it before deciding whether to repeat anything.{}",
                        loc.op_id,
                        state,
                        result.and_then(|r| r.exit_code).map(|c| format!(" with exit code {c}")).unwrap_or_default(),
                        result.and_then(|r| r.error.as_ref()).map(|e| format!(" ({})", e.message)).unwrap_or_default(),
                        Self::tails(&out, &err)
                    ),
                }
            }
        }
    }
}

#[async_trait]
impl crate::tasks::RuntimeRecoveryReader for NodeRecovery {
    async fn shell_state(&self, run: &RunBinding, task_id: &str, call_id: &str) -> Option<TaskState> {
        match self.lookup(run, call_id).await {
            Recovered::NoLocator => None,
            Recovered::Failed(loc, e) => Some(TaskState::Unknown {
                reason: format!(
                    "remote operation {} on {} cannot be queried ({:?}: {}); it may still be running",
                    loc.op_id, self.node_did, e.code, e.message
                ),
            }),
            Recovered::Status(loc, status, out, err) => match status.state {
                ExecutorOpState::Starting | ExecutorOpState::Running => Some(TaskState::Running {
                    brief: format!("remote operation {} on {}", loc.op_id, self.node_did),
                    elapsed_ms: now_ms().saturating_sub(status.accepted_at_ms),
                    output_tail: out,
                    cancellable: false,
                }),
                state => {
                    let code = status.result.as_ref().and_then(|r| r.exit_code);
                    Some(TaskState::Finished(TaskResult {
                        success: state == ExecutorOpState::Succeeded,
                        output: format!(
                            "task {task_id} (remote operation {} on {}) ended as {:?}{}; started by a previous executor.{}",
                            loc.op_id,
                            self.node_did,
                            state,
                            code.map(|c| format!(" with exit code {c}")).unwrap_or_default(),
                            Self::tails(&out, &err)
                        ),
                        tool_result: Some(llm_context::observation::ToolResultView {
                            cmd_name: Some(TOOL_SHELL.into()),
                            return_code: code,
                            summary: format!("{state:?}"),
                            ..Default::default()
                        }),
                    }))
                }
            },
        }
    }
}

struct OpenedNode {
    config: RuntimeConfig,
    descriptor: RuntimeDescriptor,
    info: RuntimeInfo,
    store_id: String,
    limits: ExecutorLimits,
}

pub struct RemoteNodeRuntime {
    config: RuntimeConfig,
    connector: Arc<dyn NodeExecutorConnector>,
    provisional: RuntimeDescriptor,
    opened: tokio::sync::OnceCell<OpenedNode>,
}

impl RemoteNodeRuntime {
    pub fn new(config: RuntimeConfig, connector: Arc<dyn NodeExecutorConnector>) -> Self {
        let provisional = RuntimeDescriptor {
            runtime_id: config.id.clone().unwrap_or_default(),
            kind: REMOTE_NODE_KIND.into(),
            host: config.remote_node.as_ref().and_then(|n| n.node_did.clone()),
            target: Value::Null,
            workdir: config.workdir.clone().unwrap_or_default(),
            capabilities: Capabilities::default(),
        };
        Self {
            config,
            connector,
            provisional,
            opened: tokio::sync::OnceCell::new(),
        }
    }

    fn target(&self) -> (String, String) {
        let node = self.config.remote_node.clone().unwrap_or_default();
        (node.node_did.unwrap_or_default(), node.profile_id.unwrap_or_default())
    }

    fn runtime_id(&self) -> String {
        let (node, profile) = self.target();
        self.config.id.clone().unwrap_or_else(|| {
            let target = json!({"node_did": node, "profile_id": profile, "workdir": self.config.workdir});
            format!(
                "{REMOTE_NODE_KIND}:{}",
                &blake3::hash(target.to_string().as_bytes()).to_hex()[..24]
            )
        })
    }

    async fn initialize(&self) -> Result<&OpenedNode, XllmError> {
        self.opened
            .get_or_try_init(|| async {
                self.config.validate(true)?;
                let (node, profile) = self.target();
                let id = self.runtime_id();
                let probe = NodeClient::new(
                    self.connector.clone(),
                    &node,
                    &profile,
                    &id,
                    crate::llm_bash::new_run_binding_slot(),
                    "probe",
                    None,
                );
                let login = probe.login_info().await.map_err(open_error)?;
                let described = probe
                    .describe(self.config.workdir.clone())
                    .await
                    .map_err(open_error)?;
                let t = described.target;
                let workdir = t.workdir.clone().unwrap_or_default();
                let target = json!({
                    "node_did": t.node_did,
                    "profile_id": t.profile_id,
                    "uid": t.uid,
                    "gid": t.gid,
                    "workdir": workdir,
                    "store_id": t.store_id,
                    "protocol_version": t.protocol_version,
                });
                let info = RuntimeInfo {
                    id: id.clone(),
                    kind: REMOTE_NODE_KIND.into(),
                    os: described.info.os.clone(),
                    arch: described.info.arch.clone(),
                    hostname: described.info.hostname.clone(),
                    shell: described.info.shell.clone(),
                    cwd: workdir.clone(),
                    tools: Vec::new(),
                    current_time: chrono::DateTime::<chrono::Utc>::from_timestamp_millis(described.info.time_ms as i64)
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_default(),
                    timezone: "UTC".into(),
                    target: target.clone(),
                };
                let mut config = self.config.clone();
                config.kind = Some(REMOTE_NODE_KIND.into());
                config.id = Some(id.clone());
                config.workdir = Some(workdir.clone());
                let descriptor = RuntimeDescriptor {
                    runtime_id: id,
                    kind: REMOTE_NODE_KIND.into(),
                    host: Some(node.clone()),
                    target,
                    workdir,
                    capabilities: Capabilities {
                        os: described.info.os,
                        network: true,
                        tools: BTreeMap::new(),
                    },
                };
                Ok(OpenedNode {
                    config,
                    descriptor,
                    info,
                    store_id: t.store_id,
                    limits: login.limits,
                })
            })
            .await
    }
}

fn open_error(e: ExecutorError) -> XllmError {
    match e.code {
        ExecutorErrorCode::StoreMismatch => XllmError::RuntimeMismatch(e.message),
        ExecutorErrorCode::Capability => XllmError::Capability(e.message),
        _ => XllmError::Capability(format!("remote_node unavailable ({:?}): {}", e.code, e.message)),
    }
}

#[async_trait]
impl AgentRuntime for RemoteNodeRuntime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        self.opened
            .get()
            .map(|s| &s.descriptor)
            .unwrap_or(&self.provisional)
    }

    fn config(&self) -> RuntimeConfig {
        self.opened
            .get()
            .map(|s| s.config.clone())
            .unwrap_or_else(|| self.config.clone())
    }

    async fn info(&self) -> Result<RuntimeInfo, XllmError> {
        let s = self.initialize().await?;
        let mut info = s.info.clone();
        info.current_time = chrono::Utc::now().to_rfc3339();
        Ok(info)
    }

    async fn open(&self, ctx: &RuntimeOpenCtx, tools: &ToolsConfig, deps: &XllmDeps) -> Result<(EffectiveTools, XllmToolManager), XllmError> {
        if !ctx.path_prefix.is_empty() {
            return Err(XllmError::Capability(
                "remote Session helper deployment is not available on remote_node".into(),
            ));
        }
        let s = self.initialize().await?;
        let (node, profile) = self.target();
        let client = Arc::new(NodeClient::new(
            self.connector.clone(),
            &node,
            &profile,
            &s.descriptor.runtime_id,
            ctx.run.clone(),
            &ctx.run_id,
            Some(s.store_id.clone()),
        ));
        let mut env = filtered_env(s.config.env.clone());
        env.extend(filtered_env(ctx.env.clone()));
        let runner: Arc<dyn BashRunner> = Arc::new(NodeBashRunner {
            client: client.clone(),
            env,
            max_execution_ms: s.limits.max_execution_ms,
        });
        let files = Arc::new(RemoteNodeFiles {
            client,
            max_file_bytes: s.limits.max_file_bytes as usize,
        });
        let (effective, mut manager) = crate::xllm::build_runtime_toolset(
            tools,
            ctx.sources.clone(),
            ctx.loop_model,
            Path::new(&s.info.cwd),
            &ctx.run_id,
            deps,
            runner.clone(),
            files,
            &s.descriptor,
            ctx.run.clone(),
        )
        .await?;
        let mut info = self.info().await?;
        info.tools = effective.all_names();
        info.tools.sort();
        info.tools.truncate(32);
        manager.set_runtime(runner, s.descriptor.clone(), info);
        manager.set_runtime_recovery(Arc::new(NodeRecovery {
            connector: self.connector.clone(),
            node_did: node,
            profile_id: profile,
        }));
        Ok((effective, manager))
    }

    async fn describe_interrupted(&self, run: &RunBinding, action: &InflightAction) -> String {
        if !matches!(
            action.tool.as_str(),
            TOOL_SHELL | crate::TOOL_READ_FILE | crate::TOOL_WRITE_FILE | crate::TOOL_EDIT_FILE | crate::TOOL_READ
        ) {
            return unresolved_reason(&action.tool);
        }
        let (node, profile) = self.target();
        NodeRecovery {
            connector: self.connector.clone(),
            node_did: node,
            profile_id: profile,
        }
        .describe(run, action)
        .await
    }
}

/// Reaches Node Executors through the local node gateway's SOCKS5 entry
/// (`CONNECT <node-host>:3184`, relayed over RTCP); the local node itself is
/// reached directly (`CONNECT 127.0.0.1:3184`).
pub struct SocksNodeConnector {
    gateway: String,
    local_node_did: Option<String>,
    token: Arc<dyn Fn() -> TokenFuture + Send + Sync>,
}

pub type TokenFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>;

impl SocksNodeConnector {
    pub fn new(
        gateway: String,
        local_node_did: Option<String>,
        token: Arc<dyn Fn() -> TokenFuture + Send + Sync>,
    ) -> Self {
        Self {
            gateway,
            local_node_did,
            token,
        }
    }

    /// From the process's BuckyOS runtime: its App session token, its node
    /// gateway host and the local device DID.
    pub fn from_buckyos_runtime() -> Option<Arc<Self>> {
        let runtime = buckyos_api::get_buckyos_api_runtime().ok()?;
        let gateway = format!(
            "{}:{}",
            runtime.get_local_gateway_host(),
            buckyos_api::NODE_EXECUTOR_ENTRY_PORT
        );
        let local = runtime.device_config.as_ref().map(|d| d.id.to_string());
        Some(Arc::new(Self::new(
            gateway,
            local,
            Arc::new(|| {
                Box::pin(async {
                    let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
                    Ok(runtime.get_session_token().await)
                })
            }),
        )))
    }

    fn target_host(&self, node_did: &str) -> std::io::Result<String> {
        if self.local_node_did.as_deref() == Some(node_did) {
            return Ok("127.0.0.1".into());
        }
        let did = name_lib::DID::from_str(node_did)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
        Ok(did.to_host_name())
    }
}

async fn socks5_connect(gateway: &str, host: &str, port: u16) -> std::io::Result<tokio::net::TcpStream> {
    use tokio::io::AsyncReadExt;
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::ConnectionRefused, m.to_string());
    let mut s = tokio::net::TcpStream::connect(gateway).await?;
    s.set_nodelay(true)?;
    s.write_all(&[5, 1, 0]).await?;
    let mut hello = [0u8; 2];
    s.read_exact(&mut hello).await?;
    if hello != [5, 0] {
        return Err(bad("node gateway refused the SOCKS5 greeting"));
    }
    let mut req = vec![5, 1, 0];
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => {
            req.push(1);
            req.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            if host.len() > 255 {
                return Err(bad("node host name too long"));
            }
            req.push(3);
            req.push(host.len() as u8);
            req.extend_from_slice(host.as_bytes());
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0 {
        return Err(bad(&format!("node gateway SOCKS5 CONNECT failed (reply {})", head[1])));
    }
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).await?;
            n[0] as usize
        }
        _ => return Err(bad("bad SOCKS5 reply")),
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest).await?;
    Ok(s)
}

#[async_trait]
impl NodeExecutorConnector for SocksNodeConnector {
    async fn connect(&self, node_did: &str) -> std::io::Result<Box<dyn NodeIo>> {
        let host = self.target_host(node_did)?;
        let stream = tokio::time::timeout(
            Duration::from_secs(20),
            socks5_connect(&self.gateway, &host, buckyos_api::NODE_EXECUTOR_SERVICE_PORT),
        )
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "node gateway connect timed out"))??;
        Ok(Box::new(stream))
    }

    async fn session_token(&self) -> Result<String, String> {
        (self.token)().await
    }
}

/// The connector of a process whose BuckyOS runtime may not exist yet (the
/// xllm CLI): initializes it on first use, then behaves like
/// `SocksNodeConnector::from_buckyos_runtime`.
pub struct BuckyosNodeConnector;

impl BuckyosNodeConnector {
    async fn inner(&self) -> std::io::Result<Arc<SocksNodeConnector>> {
        crate::xllm::ensure_buckyos_runtime()
            .await
            .map_err(|e| std::io::Error::other(format!("BuckyOS runtime unavailable: {e}")))?;
        SocksNodeConnector::from_buckyos_runtime()
            .ok_or_else(|| std::io::Error::other("BuckyOS runtime unavailable"))
    }
}

#[async_trait]
impl NodeExecutorConnector for BuckyosNodeConnector {
    async fn connect(&self, node_did: &str) -> std::io::Result<Box<dyn NodeIo>> {
        self.inner().await?.connect(node_did).await
    }

    async fn session_token(&self) -> Result<String, String> {
        self.inner().await.map_err(|e| e.to_string())?.session_token().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_workdirs_resolve_with_windows_rules_on_any_host() {
        let root = Path::new(r"C:\Users\owner\ws");
        let allowed = [root.to_path_buf()];
        let resolve = |raw: &str| remote_resolve(root, raw, &allowed).map(|p| p.to_string_lossy().to_string());
        assert_eq!(resolve("src/main.rs").unwrap(), r"C:\Users\owner\ws\src\main.rs");
        assert_eq!(resolve(r"a\..\b.txt").unwrap(), r"C:\Users\owner\ws\b.txt");
        assert_eq!(resolve(r"c:/users/OWNER/ws/x").unwrap(), r"C:\users\OWNER\ws\x");
        assert!(resolve(r"..\other").is_err());
        assert!(resolve(r"D:\ws").is_err());
        assert_eq!(remote_resolve(root, r"D:\x\..\y", &[]).unwrap(), PathBuf::from(r"D:\y"));
    }
}
