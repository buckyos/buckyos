//! Command Stream wire format (doc/arch/node_exector.md §7.3).
//!
//! A frame is `kind:u8 | len:u32be | payload`. Control frames (kind 1) carry
//! one JSON message; data frames (kind 2) carry
//! `channel:u8 | offset:u64be | op_id_len:u16be | op_id | bytes`.

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::node_executor::{hex_lower, ExecutorLimits, ExecutorOp};

pub const EXECUTOR_FRAME_CONTROL: u8 = 1;
pub const EXECUTOR_FRAME_DATA: u8 = 2;
pub const EXECUTOR_MAX_CONTROL_FRAME: usize = 1024 * 1024;
pub const EXECUTOR_MAX_DATA_BYTES: usize = 16 * 1024;
pub const EXECUTOR_MAX_OP_ID: usize = 128;
pub const EXECUTOR_MAX_QUERY_BYTES: u64 = 256 * 1024;
pub const EXECUTOR_MAX_QUERY_WAIT_MS: u64 = 30_000;
pub const EXECUTOR_UPLOAD_DEADLINE_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorChannel {
    Stdout,
    Stderr,
    /// Bytes returned by `read`, or the pre-edit snapshot of `edit`.
    Content,
    Upload,
}

impl ExecutorChannel {
    pub fn code(&self) -> u8 {
        match self {
            ExecutorChannel::Stdout => 1,
            ExecutorChannel::Stderr => 2,
            ExecutorChannel::Content => 3,
            ExecutorChannel::Upload => 4,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(ExecutorChannel::Stdout),
            2 => Some(ExecutorChannel::Stderr),
            3 => Some(ExecutorChannel::Content),
            4 => Some(ExecutorChannel::Upload),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutorChannel::Stdout => "stdout",
            ExecutorChannel::Stderr => "stderr",
            ExecutorChannel::Content => "content",
            ExecutorChannel::Upload => "upload",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorPosture {
    Normal,
    Frozen,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorErrorCode {
    Unauthenticated,
    Forbidden,
    InvalidArgument,
    Capability,
    Conflict,
    Busy,
    CapacityExceeded,
    Disabled,
    Unavailable,
    AuthExpired,
    ResultExpired,
    StoreMismatch,
    NotFound,
    ProtocolError,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorError {
    pub code: ExecutorErrorCode,
    pub message: String,
    #[serde(default)]
    pub effect_unknown: bool,
}

impl ExecutorError {
    pub fn new(code: ExecutorErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            effect_unknown: false,
        }
    }

    pub fn unknown_effect(mut self) -> Self {
        self.effect_unknown = true;
        self
    }
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ExecutorError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorClientInfo {
    pub runtime_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorLogin {
    pub v: u32,
    #[serde(default)]
    pub request_id: Option<String>,
    pub token: String,
    pub profile: String,
    pub client: ExecutorClientInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorDescribeRequest {
    pub request_id: String,
    #[serde(default)]
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorExecRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    pub run_id: String,
    pub call_id: String,
    pub cmd: String,
    pub cwd: String,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub execution_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorReadRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    pub run_id: String,
    pub call_id: String,
    pub path: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub length: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorWriteMode {
    Replace,
    Append,
    CreateNew,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorWriteRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    pub run_id: String,
    pub call_id: String,
    pub path: String,
    pub mode: ExecutorWriteMode,
    #[serde(default)]
    pub expected_hash: Option<String>,
    pub content_length: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorUploadEnd {
    pub request_id: String,
    pub op_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorEdit {
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorEditRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    pub run_id: String,
    pub call_id: String,
    pub path: String,
    pub edits: Vec<ExecutorEdit>,
    #[serde(default)]
    pub expected_hash: Option<String>,
}

/// `mkdir` and `stat`: the minimal directory operations a control side
/// needs to place a workspace on the node (integration TODO §3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorPathRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    pub run_id: String,
    pub call_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorPathType {
    File,
    Directory,
    Missing,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorComputerRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    pub run_id: String,
    pub call_id: String,
    pub request: crate::ComputerRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorOpRef {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorQueryRequest {
    pub request_id: String,
    pub store_id: String,
    pub op_id: String,
    #[serde(default)]
    pub channel: Option<ExecutorChannel>,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorPing {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorClientMessage {
    Login(ExecutorLogin),
    Describe(ExecutorDescribeRequest),
    Exec(ExecutorExecRequest),
    Read(ExecutorReadRequest),
    Write(ExecutorWriteRequest),
    UploadEnd(ExecutorUploadEnd),
    Edit(ExecutorEditRequest),
    Mkdir(ExecutorPathRequest),
    Stat(ExecutorPathRequest),
    Computer(ExecutorComputerRequest),
    Cancel(ExecutorOpRef),
    Query(ExecutorQueryRequest),
    Ping(ExecutorPing),
}

impl ExecutorClientMessage {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            ExecutorClientMessage::Login(m) => m.request_id.as_deref(),
            ExecutorClientMessage::Describe(m) => Some(&m.request_id),
            ExecutorClientMessage::Exec(m) => Some(&m.request_id),
            ExecutorClientMessage::Read(m) => Some(&m.request_id),
            ExecutorClientMessage::Write(m) => Some(&m.request_id),
            ExecutorClientMessage::UploadEnd(m) => Some(&m.request_id),
            ExecutorClientMessage::Edit(m) => Some(&m.request_id),
            ExecutorClientMessage::Mkdir(m) | ExecutorClientMessage::Stat(m) => Some(&m.request_id),
            ExecutorClientMessage::Computer(m) => Some(&m.request_id),
            ExecutorClientMessage::Cancel(m) => Some(&m.request_id),
            ExecutorClientMessage::Query(m) => Some(&m.request_id),
            ExecutorClientMessage::Ping(m) => Some(&m.request_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorLoginOk {
    #[serde(default)]
    pub request_id: Option<String>,
    pub posture: ExecutorPosture,
    pub principal: String,
    pub app_instance_id: String,
    pub caps: Vec<ExecutorOp>,
    pub limits: ExecutorLimits,
    pub revision: String,
    pub generation: u64,
    pub auth_valid_until_ms: u64,
    pub token_expires_at_ms: u64,
    pub store_id: String,
    pub server_epoch: String,
}

/// Stable target of a remote runtime; changes other than store_id are a
/// RuntimeMismatch for the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRuntimeTarget {
    pub node_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_id: Option<String>,
    pub profile_id: String,
    /// The actual execution identity, when reported by the host.
    #[serde(default)]
    pub uid: Option<u32>,
    #[serde(default)]
    pub gid: Option<u32>,
    #[serde(default)]
    pub workdir: Option<String>,
    /// The profile's workdir roots: where a control side may place
    /// session and workspace directories.
    #[serde(default)]
    pub workdir_roots: Vec<String>,
    pub store_id: String,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRuntimeInfo {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub shell: String,
    pub time_ms: u64,
    #[serde(default)]
    pub path: Option<String>,
    /// `current_user` (form A-desktop, form B/C) or `switch_user` (form
    /// A-system): how the host executes.
    #[serde(default)]
    pub identity_mode: Option<String>,
    /// The desktop Driver's capabilities (`ui`, `capture`, `input`,
    /// `accessibility`, each `{supported, ready, reason?}`) when one is
    /// injected; absent on hosts without ComputerUse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorDescribeOk {
    pub request_id: String,
    pub target: ExecutorRuntimeTarget,
    pub info: ExecutorRuntimeInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorOpKind {
    Exec,
    Read,
    Write,
    Edit,
    Mkdir,
    Stat,
    ComputerInspect,
    ComputerObserve,
    ComputerAct,
}

impl ExecutorOpKind {
    pub fn op(&self) -> ExecutorOp {
        match self {
            ExecutorOpKind::Exec => ExecutorOp::Exec,
            ExecutorOpKind::Read => ExecutorOp::Read,
            ExecutorOpKind::Write => ExecutorOp::Write,
            ExecutorOpKind::Edit => ExecutorOp::Edit,
            ExecutorOpKind::Mkdir => ExecutorOp::Mkdir,
            ExecutorOpKind::Stat => ExecutorOp::Stat,
            ExecutorOpKind::ComputerInspect => ExecutorOp::ComputerInspect,
            ExecutorOpKind::ComputerObserve => ExecutorOp::ComputerObserve,
            ExecutorOpKind::ComputerAct => ExecutorOp::ComputerAct,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorOpState {
    Starting,
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Unknown,
}

impl ExecutorOpState {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, ExecutorOpState::Starting | ExecutorOpState::Running)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorOutputSummary {
    pub total_bytes: u64,
    pub retained_bytes: u64,
    pub truncated: bool,
    #[serde(default)]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorFileResult {
    pub path: String,
    pub size: u64,
    pub hash: String,
    #[serde(default)]
    pub previous_hash: Option<String>,
    #[serde(default)]
    pub created: bool,
    #[serde(default)]
    pub changed: bool,
    #[serde(default)]
    pub bytes_written: u64,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub replacements: u64,
    /// `stat` / `mkdir`: what the path is after the operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_type: Option<ExecutorPathType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorOpResult {
    pub state: ExecutorOpState,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub signal: Option<i32>,
    #[serde(default)]
    pub error: Option<ExecutorError>,
    #[serde(default)]
    pub stdout: Option<ExecutorOutputSummary>,
    #[serde(default)]
    pub stderr: Option<ExecutorOutputSummary>,
    #[serde(default)]
    pub content: Option<ExecutorOutputSummary>,
    #[serde(default)]
    pub file: Option<ExecutorFileResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computer: Option<crate::ComputerResult>,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorOpStatus {
    pub op_id: String,
    pub kind: ExecutorOpKind,
    pub state: ExecutorOpState,
    pub accepted_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub result: Option<ExecutorOpResult>,
    /// Output/content was garbage collected; only the tombstone remains.
    #[serde(default)]
    pub expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorOpReply {
    pub request_id: String,
    pub op: ExecutorOpStatus,
    /// The request matched an operation that was already accepted.
    #[serde(default)]
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorChunk {
    pub channel: ExecutorChannel,
    pub offset: u64,
    pub next_offset: u64,
    pub retained_from: u64,
    pub truncated: bool,
    pub eof: bool,
    pub data_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorQueryReply {
    pub request_id: String,
    pub op: ExecutorOpStatus,
    #[serde(default)]
    pub chunk: Option<ExecutorChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorStarted {
    pub op_id: String,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorFinished {
    pub op_id: String,
    pub result: ExecutorOpResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorErrorReply {
    #[serde(default)]
    pub request_id: Option<String>,
    pub error: ExecutorError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorCloseReason {
    Revoked,
    AuthExpired,
    TokenExpired,
    Disabled,
    Posture,
    Shutdown,
    ProtocolError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorClosing {
    pub reason: ExecutorCloseReason,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorServerMessage {
    LoginOk(ExecutorLoginOk),
    DescribeOk(ExecutorDescribeOk),
    Accepted(ExecutorOpReply),
    Status(ExecutorOpReply),
    QueryOk(ExecutorQueryReply),
    Started(ExecutorStarted),
    Finished(ExecutorFinished),
    Error(ExecutorErrorReply),
    Closing(ExecutorClosing),
    Pong(ExecutorPing),
}

/// The normalized operation that the request digest covers. Optional wire
/// fields are filled with protocol defaults, never with config-dependent
/// values, so a retransmission keeps the same digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorOpSpec {
    Computer { request: crate::ComputerRequest },
    Exec {
        cmd: String,
        cwd: String,
        env_overrides: BTreeMap<String, String>,
        execution_timeout_ms: Option<u64>,
    },
    Read {
        path: String,
        offset: u64,
        length: Option<u64>,
    },
    Write {
        path: String,
        mode: ExecutorWriteMode,
        expected_hash: Option<String>,
        content_length: u64,
        content_hash: String,
    },
    Edit {
        path: String,
        edits: Vec<ExecutorEdit>,
        expected_hash: Option<String>,
    },
    Mkdir {
        path: String,
    },
    Stat {
        path: String,
    },
}

impl ExecutorOpSpec {
    pub fn kind(&self) -> ExecutorOpKind {
        match self {
            ExecutorOpSpec::Computer { request } => match request.required_op() {
                ExecutorOp::ComputerInspect => ExecutorOpKind::ComputerInspect,
                ExecutorOp::ComputerObserve => ExecutorOpKind::ComputerObserve,
                _ => ExecutorOpKind::ComputerAct,
            },
            ExecutorOpSpec::Exec { .. } => ExecutorOpKind::Exec,
            ExecutorOpSpec::Read { .. } => ExecutorOpKind::Read,
            ExecutorOpSpec::Write { .. } => ExecutorOpKind::Write,
            ExecutorOpSpec::Edit { .. } => ExecutorOpKind::Edit,
            ExecutorOpSpec::Mkdir { .. } => ExecutorOpKind::Mkdir,
            ExecutorOpSpec::Stat { .. } => ExecutorOpKind::Stat,
        }
    }

    pub fn from_mkdir(req: &ExecutorPathRequest) -> Self {
        ExecutorOpSpec::Mkdir { path: req.path.clone() }
    }

    pub fn from_stat(req: &ExecutorPathRequest) -> Self {
        ExecutorOpSpec::Stat { path: req.path.clone() }
    }

    pub fn from_exec(req: &ExecutorExecRequest) -> Self {
        ExecutorOpSpec::Exec {
            cmd: req.cmd.clone(),
            cwd: req.cwd.clone(),
            env_overrides: req.env_overrides.clone(),
            execution_timeout_ms: req.execution_timeout_ms,
        }
    }

    pub fn from_read(req: &ExecutorReadRequest) -> Self {
        ExecutorOpSpec::Read {
            path: req.path.clone(),
            offset: req.offset,
            length: req.length,
        }
    }

    pub fn from_write(req: &ExecutorWriteRequest) -> Self {
        ExecutorOpSpec::Write {
            path: req.path.clone(),
            mode: req.mode,
            expected_hash: req.expected_hash.clone(),
            content_length: req.content_length,
            content_hash: req.content_hash.clone(),
        }
    }

    pub fn from_edit(req: &ExecutorEditRequest) -> Self {
        ExecutorOpSpec::Edit {
            path: req.path.clone(),
            edits: req.edits.clone(),
            expected_hash: req.expected_hash.clone(),
        }
    }
}

/// Digest of the full operation identity and content, excluding request_id,
/// token and connection state.
pub fn executor_request_digest(
    runtime_id: &str,
    run_id: &str,
    call_id: &str,
    op_id: &str,
    spec: &ExecutorOpSpec,
) -> String {
    let value = serde_json::json!({
        "runtime_id": runtime_id,
        "run_id": run_id,
        "call_id": call_id,
        "op_id": op_id,
        "op": spec,
    });
    let bytes = serde_json::to_vec(&value).unwrap_or_default();
    format!("sha256:{}", hex_lower(&Sha256::digest(&bytes)))
}

pub fn executor_content_hash(data: &[u8]) -> String {
    format!("sha256:{}", hex_lower(&Sha256::digest(data)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorDataFrame {
    pub op_id: String,
    pub channel: ExecutorChannel,
    pub offset: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorFrame {
    Control(Vec<u8>),
    Data(ExecutorDataFrame),
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub fn encode_executor_control<T: Serialize>(msg: &T) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(msg).map_err(|e| invalid(e.to_string()))?;
    if payload.len() > EXECUTOR_MAX_CONTROL_FRAME {
        return Err(invalid("control frame too large"));
    }
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(EXECUTOR_FRAME_CONTROL);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn encode_executor_data(frame: &ExecutorDataFrame) -> io::Result<Vec<u8>> {
    if frame.data.len() > EXECUTOR_MAX_DATA_BYTES {
        return Err(invalid("data frame too large"));
    }
    if frame.op_id.is_empty() || frame.op_id.len() > EXECUTOR_MAX_OP_ID {
        return Err(invalid("invalid op_id in data frame"));
    }
    let len = 1 + 8 + 2 + frame.op_id.len() + frame.data.len();
    let mut out = Vec::with_capacity(len + 5);
    out.push(EXECUTOR_FRAME_DATA);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.push(frame.channel.code());
    out.extend_from_slice(&frame.offset.to_be_bytes());
    out.extend_from_slice(&(frame.op_id.len() as u16).to_be_bytes());
    out.extend_from_slice(frame.op_id.as_bytes());
    out.extend_from_slice(&frame.data);
    Ok(out)
}

/// Read one frame; `Ok(None)` on a clean EOF at a frame boundary.
pub async fn read_executor_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<ExecutorFrame>> {
    let mut kind = [0u8; 1];
    match reader.read(&mut kind).await? {
        0 => return Ok(None),
        _ => {}
    }
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    match kind[0] {
        EXECUTOR_FRAME_CONTROL => {
            if len > EXECUTOR_MAX_CONTROL_FRAME {
                return Err(invalid("control frame too large"));
            }
            let mut payload = vec![0u8; len];
            reader.read_exact(&mut payload).await?;
            Ok(Some(ExecutorFrame::Control(payload)))
        }
        EXECUTOR_FRAME_DATA => {
            if len < 11 || len > 11 + EXECUTOR_MAX_OP_ID + EXECUTOR_MAX_DATA_BYTES {
                return Err(invalid("invalid data frame length"));
            }
            let mut payload = vec![0u8; len];
            reader.read_exact(&mut payload).await?;
            let channel = ExecutorChannel::from_code(payload[0])
                .ok_or_else(|| invalid("unknown data channel"))?;
            let offset = u64::from_be_bytes(payload[1..9].try_into().unwrap());
            let id_len = u16::from_be_bytes(payload[9..11].try_into().unwrap()) as usize;
            if id_len == 0 || id_len > EXECUTOR_MAX_OP_ID || 11 + id_len > len {
                return Err(invalid("invalid op_id in data frame"));
            }
            let op_id = String::from_utf8(payload[11..11 + id_len].to_vec())
                .map_err(|_| invalid("op_id is not utf-8"))?;
            let data = payload[11 + id_len..].to_vec();
            if data.len() > EXECUTOR_MAX_DATA_BYTES {
                return Err(invalid("data frame too large"));
            }
            Ok(Some(ExecutorFrame::Data(ExecutorDataFrame {
                op_id,
                channel,
                offset,
                data,
            })))
        }
        other => Err(invalid(format!("unknown frame kind {other}"))),
    }
}

pub async fn write_executor_control<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    msg: &T,
) -> io::Result<()> {
    let bytes = encode_executor_control(msg)?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub async fn write_executor_data<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &ExecutorDataFrame,
) -> io::Result<()> {
    let bytes = encode_executor_data(frame)?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub fn is_valid_executor_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= EXECUTOR_MAX_OP_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_survive_split_and_coalesced_reads() {
        let login = ExecutorClientMessage::Login(ExecutorLogin {
            v: 1,
            request_id: None,
            token: "t".into(),
            profile: "dev".into(),
            client: ExecutorClientInfo {
                runtime_id: "node-dev".into(),
                run_id: "r1".into(),
            },
        });
        let data = ExecutorDataFrame {
            op_id: "op-1".into(),
            channel: ExecutorChannel::Upload,
            offset: 42,
            data: vec![7u8; 100],
        };
        let mut bytes = encode_executor_control(&login).unwrap();
        bytes.extend(encode_executor_data(&data).unwrap());

        let (mut client, mut server) = tokio::io::duplex(3);
        let writer = tokio::spawn(async move {
            for chunk in bytes.chunks(3) {
                client.write_all(chunk).await.unwrap();
            }
        });
        let first = read_executor_frame(&mut server).await.unwrap().unwrap();
        let ExecutorFrame::Control(payload) = first else {
            panic!("expected control");
        };
        let parsed: ExecutorClientMessage = serde_json::from_slice(&payload).unwrap();
        assert_eq!(parsed, login);
        let second = read_executor_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(second, ExecutorFrame::Data(data));
        writer.await.unwrap();
        assert!(read_executor_frame(&mut server).await.unwrap().is_none());
    }

    #[test]
    fn tagged_messages_reject_unknown_fields() {
        let ok = serde_json::json!({"type": "ping", "request_id": "1"});
        assert!(serde_json::from_value::<ExecutorClientMessage>(ok).is_ok());
        let bad = serde_json::json!({"type": "ping", "request_id": "1", "x": 2});
        assert!(serde_json::from_value::<ExecutorClientMessage>(bad).is_err());
    }

    #[test]
    fn digest_ignores_request_id_but_covers_content() {
        let mut req = ExecutorExecRequest {
            request_id: "a".into(),
            store_id: "s".into(),
            op_id: "op".into(),
            run_id: "r".into(),
            call_id: "c".into(),
            cmd: "ls".into(),
            cwd: "/w".into(),
            env_overrides: BTreeMap::new(),
            execution_timeout_ms: None,
        };
        let d1 = executor_request_digest("rt", "r", "c", "op", &ExecutorOpSpec::from_exec(&req));
        req.request_id = "b".into();
        let d2 = executor_request_digest("rt", "r", "c", "op", &ExecutorOpSpec::from_exec(&req));
        assert_eq!(d1, d2);
        req.cmd = "pwd".into();
        let d3 = executor_request_digest("rt", "r", "c", "op", &ExecutorOpSpec::from_exec(&req));
        assert_ne!(d1, d3);
    }
}
