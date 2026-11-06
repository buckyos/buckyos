//! Media representation pipeline on the workflow service
//! (doc/llm_context/LLM Understand Media.md §4.5–§4.8,
//! notepads/llm-media-pipeline-workflow-todo.md T4).
//!
//! The agent never converts media itself: a conversion is a run of a system
//! `media.<family>` definition. This module freezes inputs into the Named
//! Store, drives the run (ensure / resume / bounded recovery), reads the
//! manifest and artifacts back, and reports the decision, trace and coverage.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buckyos_api::{
    get_buckyos_api_runtime, MediaEntry, MediaEntryKind, MediaEntryStatus, MediaManifest,
    RunStatus, ThunkExecutionError, WorkflowCreateRunReq, WorkflowRpcError, WorkflowRunGraph,
    WorkflowRunOutput, WorkflowServiceClient,
};
use ndn_lib::{ChunkHasher, ChunkId, ChunkList, FileObject, NamedObject, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const FAMILY_IMAGE: &str = "media.image";
pub const FAMILY_PDF: &str = "media.pdf";
pub const FAMILY_VIDEO: &str = "media.video";
pub const FAMILY_DOCUMENT: &str = "media.document";

const DEFAULT_WAIT_MS: u64 = 2_000;
const POLL_INTERVAL_MS: u64 = 400;
const TOTAL_DEADLINE_SECS: u64 = 1_800;
const MAX_RECOVERIES: u32 = 3;
const PENDING_CHECK_AFTER_SECS: u64 = 5;

/// Limits the image pipeline is asked to fit to and the direct-pass check
/// uses; env overrides are clamped to the definition's schema maxima.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ImageLimits {
    pub max_long_edge: u32,
    pub max_pixels: u64,
    pub max_bytes: u64,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_long_edge: 2048,
            max_pixels: 2048 * 2048,
            max_bytes: 3 * 1024 * 1024 + 512 * 1024,
        }
    }
}

impl ImageLimits {
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            max_long_edge: env_positive("LLM_UNDERSTAND_MEDIA_IMAGE_MAX_EDGE")
                .unwrap_or(default.max_long_edge)
                .clamp(64, 4096),
            max_pixels: env_positive("LLM_UNDERSTAND_MEDIA_IMAGE_MAX_PIXELS")
                .unwrap_or(default.max_pixels)
                .clamp(4096, 16_777_216),
            max_bytes: env_positive("LLM_UNDERSTAND_MEDIA_IMAGE_MAX_BYTES")
                .unwrap_or(default.max_bytes)
                .clamp(16_384, 20_971_520),
        }
    }

    pub fn allows(&self, width: u32, height: u32, bytes: u64) -> bool {
        width.max(height) <= self.max_long_edge
            && u64::from(width) * u64::from(height) <= self.max_pixels
            && bytes <= self.max_bytes
    }

    pub fn pipeline_input(&self, obj_id: &str) -> Value {
        json!({
            "obj_id": obj_id,
            "max_long_edge": self.max_long_edge,
            "max_pixels": self.max_pixels,
            "max_bytes": self.max_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentLimits {
    pub dpi: u32,
    pub max_long_edge: u32,
    pub max_pages: u32,
    /// Direct pass-through ceiling for protocols that accept documents.
    pub direct_max_bytes: u64,
}

impl DocumentLimits {
    pub fn from_env() -> Self {
        Self {
            dpi: env_positive("LLM_UNDERSTAND_MEDIA_PDF_DPI")
                .unwrap_or(144)
                .clamp(36, 300),
            max_long_edge: env_positive("LLM_UNDERSTAND_MEDIA_PAGE_MAX_EDGE")
                .unwrap_or(2048)
                .clamp(256, 4096),
            max_pages: env_positive("LLM_UNDERSTAND_MEDIA_MAX_PAGES")
                .unwrap_or(50)
                .clamp(1, 200),
            direct_max_bytes: env_positive("LLM_UNDERSTAND_MEDIA_DOCUMENT_MAX_BYTES")
                .unwrap_or(20 * 1024 * 1024),
        }
    }

    pub fn pipeline_input(&self, obj_id: &str, format: Option<&str>) -> Value {
        let mut input = json!({
            "obj_id": obj_id,
            "dpi": self.dpi,
            "max_long_edge": self.max_long_edge,
            "page_range": [],
            "limits": {"max_pages": self.max_pages},
        });
        if let Some(format) = format {
            input["format"] = Value::String(format.to_string());
        }
        input
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct VideoLimits {
    pub max_frames: u32,
    pub frame_max_edge: u32,
}

impl VideoLimits {
    pub fn from_env() -> Self {
        Self {
            max_frames: env_positive("LLM_UNDERSTAND_MEDIA_VIDEO_FRAMES")
                .unwrap_or(16)
                .clamp(1, 64),
            frame_max_edge: env_positive("LLM_UNDERSTAND_MEDIA_VIDEO_FRAME_MAX_EDGE")
                .unwrap_or(1280)
                .clamp(128, 2048),
        }
    }

    pub fn pipeline_input(&self, obj_id: &str) -> Value {
        json!({
            "obj_id": obj_id,
            "strategy": "uniform",
            "max_frames": self.max_frames,
            "frame_max_edge": self.frame_max_edge,
        })
    }
}

fn env_positive<T: std::str::FromStr + PartialOrd + Default>(key: &str) -> Option<T> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<T>().ok())
        .filter(|value| *value > T::default())
}

/// Width / height from the header of a PNG, JPEG, GIF or WebP file; `None`
/// for anything else, which then goes through the pipeline.
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) && bytes.len() >= 24 {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return (w > 0 && h > 0).then_some((w, h));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        if bytes.len() < 10 {
            return None;
        }
        let w = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
        let h = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        return (w > 0 && h > 0).then_some((w, h));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return webp_dimensions(bytes);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        return jpeg_dimensions(bytes);
    }
    None
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let chunk = bytes.get(12..16)?;
    let payload = bytes.get(20..)?;
    match chunk {
        b"VP8X" => {
            let w = 1 + u32::from_le_bytes([payload[4], payload[5], payload[6], 0]);
            let h = 1 + u32::from_le_bytes([payload[7], payload[8], payload[9], 0]);
            Some((w, h))
        }
        b"VP8L" => {
            let b = payload.get(1..5)?;
            let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8 " => {
            let b = payload.get(6..10)?;
            Some((
                u32::from(u16::from_le_bytes([b[0], b[1]]) & 0x3fff),
                u32::from(u16::from_le_bytes([b[2], b[3]]) & 0x3fff),
            ))
        }
        _ => None,
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2usize;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xff {
            i += 1;
            continue;
        }
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        let is_sof = matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf);
        if is_sof {
            let h = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
            let w = u32::from(u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]));
            return (w > 0 && h > 0).then_some((w, h));
        }
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        i += 2 + len;
    }
    None
}

/// Protocol adapters whose message schema carries `Document` parts
/// (src/frame/aicc/src/protocol: claude_messages, gemini, openai_responses).
pub fn protocol_accepts_documents(protocol_adapter_id: &str) -> bool {
    matches!(
        protocol_adapter_id,
        "claude-messages" | "gemini-interactions" | "openai-responses"
    )
}

/// Office formats `media.document` converts; the key is the lower-case file
/// extension, the value the pipeline `format` param.
pub fn document_format_from_name(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "docx" => "docx",
        "pptx" => "pptx",
        "odt" => "odt",
        "odp" => "odp",
        "doc" => "doc",
        "ppt" => "ppt",
        "rtf" => "rtf",
        _ => return None,
    })
}

pub fn document_format_from_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.oasis.opendocument.presentation" => "odp",
        "application/msword" => "doc",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/rtf" | "text/rtf" => "rtf",
        _ => return None,
    })
}

// ----- input freezing ------------------------------------------------------

/// Stores bytes in the Named Store as a FileObject (same chunking as
/// `BucketClient::put_reader`, so identical bytes get identical content ids)
/// and returns its id.
pub async fn freeze_bytes(name: &str, mime: &str, bytes: &[u8]) -> Result<ObjId, String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let store = runtime
        .get_named_store()
        .await
        .map_err(|e| format!("named store: {e}"))?;
    let chunk_size = ndn_lib::CHUNK_DEFAULT_SIZE as usize;
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    loop {
        let end = (offset + chunk_size).min(bytes.len());
        let piece = &bytes[offset..end];
        let hasher = ChunkHasher::new(None).map_err(|e| format!("chunk hasher: {e}"))?;
        let method = hasher.hash_method;
        let hash = hasher.calc_from_bytes(piece);
        let chunk_id = ChunkId::from_mix_hash_result_by_hash_method(piece.len() as u64, &hash, method)
            .map_err(|e| format!("chunk id: {e}"))?;
        if !store.have_chunk(&chunk_id).await {
            store
                .put_chunk(&chunk_id, piece)
                .await
                .map_err(|e| format!("store chunk: {e}"))?;
        }
        chunks.push(chunk_id);
        offset = end;
        if offset >= bytes.len() {
            break;
        }
    }
    let content = if chunks.len() == 1 {
        chunks[0].to_string()
    } else {
        let list = ChunkList::from_chunk_list(chunks).map_err(|e| format!("chunk list: {e}"))?;
        let (list_id, list_str) = list.gen_obj_id();
        store
            .put_object(&list_id, &list_str)
            .await
            .map_err(|e| format!("store chunk list: {e}"))?;
        list_id.to_string()
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut file = FileObject::default();
    file.content_obj.name = name.to_string();
    file.content_obj.create_time = now;
    file.content_obj.last_update_time = now;
    file.size = bytes.len() as u64;
    file.content = content;
    file.meta
        .insert("mime_type".to_string(), Value::String(mime.to_string()));
    let (obj_id, obj_str) = file.gen_obj_id();
    store
        .put_object(&obj_id, &obj_str)
        .await
        .map_err(|e| format!("store file object: {e}"))?;
    Ok(obj_id)
}

/// FileObject facts without reading its content: size, name, declared mime.
pub struct ObjectHead {
    pub size: u64,
    pub name: Option<String>,
    pub mime: Option<String>,
    pub head: Vec<u8>,
}

/// Reads the FileObject and at most `head_bytes` of its content.
pub async fn object_head(obj_id: &ObjId, head_bytes: usize) -> Result<ObjectHead, String> {
    use tokio::io::AsyncReadExt;
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let store = runtime
        .get_named_store()
        .await
        .map_err(|e| format!("named store: {e}"))?;
    let body = store
        .get_object(obj_id)
        .await
        .map_err(|e| format!("read {obj_id}: {e}"))?;
    let file: FileObject = serde_json::from_str(&body)
        .map_err(|e| format!("{obj_id} is not a file object: {e}"))?;
    let (mut reader, total) = store
        .open_reader(obj_id, None)
        .await
        .map_err(|e| format!("open {obj_id}: {e}"))?;
    let mut head = Vec::with_capacity(head_bytes.min(total as usize));
    (&mut reader)
        .take(head_bytes as u64)
        .read_to_end(&mut head)
        .await
        .map_err(|e| format!("read {obj_id}: {e}"))?;
    let name = (!file.content_obj.name.is_empty()).then(|| file.content_obj.name.clone());
    let mime = file
        .meta
        .get("mime_type")
        .or_else(|| file.meta.get("mime"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ObjectHead {
        size: if file.size > 0 { file.size } else { total },
        name,
        mime,
        head,
    })
}

pub async fn read_object_bytes(obj_id: &str) -> Result<Vec<u8>, String> {
    let id = ObjId::new(obj_id).map_err(|e| format!("invalid artifact id `{obj_id}`: {e}"))?;
    Ok(crate::read_tool::read_named_object(&id).await?.bytes)
}

pub async fn read_manifest(obj_id: &str) -> Result<MediaManifest, String> {
    let id = ObjId::new(obj_id).map_err(|e| format!("invalid manifest id `{obj_id}`: {e}"))?;
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let store = runtime
        .get_named_store()
        .await
        .map_err(|e| format!("named store: {e}"))?;
    let body = store
        .get_object(&id)
        .await
        .map_err(|e| format!("read manifest {obj_id}: {e}"))?;
    ndn_lib::load_named_obj::<MediaManifest>(&body).map_err(|e| format!("parse manifest {obj_id}: {e}"))
}

// ----- run ledger ----------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LedgerEntry {
    run_id: String,
    workflow_id: String,
    revision: String,
    obj_id: String,
    input: Value,
    first_requested_at: u64,
    recoveries: u32,
}

/// Where a session remembers the run it is waiting for, keyed by the request
/// identity (`family + obj_id + effective params`), so a later call with the
/// same media polls the same run instead of ensuring again.
pub struct RunLedger {
    dir: PathBuf,
}

impl RunLedger {
    pub fn for_session(session_id: Option<&str>) -> Self {
        let session_dir = session_id
            .filter(|id| !id.trim().is_empty())
            .and_then(|id| {
                std::env::var(crate::runtime_context::OPENDAN_AGENT_ROOT_ENV)
                    .ok()
                    .filter(|root| !root.trim().is_empty())
                    .map(|root| PathBuf::from(root).join("sessions").join(id))
            })
            .filter(|dir| dir.is_dir());
        let dir = session_dir
            .map(|dir| dir.join(".llm_understand_media").join("runs"))
            .unwrap_or_else(|| std::env::temp_dir().join("llm_understand_media-runs"));
        Self { dir }
    }

    pub fn request_key(workflow_id: &str, obj_id: &str, input: &Value) -> String {
        let (id, _) = ndn_lib::build_named_object_by_json(
            "mediareq",
            &json!({"workflow_id": workflow_id, "obj_id": obj_id, "input": input}),
        );
        hex::encode(&id.obj_hash)
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    fn load(&self, key: &str) -> Option<LedgerEntry> {
        let text = std::fs::read_to_string(self.path(key)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self, key: &str, entry: &LedgerEntry) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        if let Ok(text) = serde_json::to_string(entry) {
            let _ = std::fs::write(self.path(key), text);
        }
    }

    fn remove(&self, key: &str) {
        let _ = std::fs::remove_file(self.path(key));
    }
}

// ----- run driving ---------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StepTrace {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ThunkExecutionError>,
}

/// `details.pipeline_trace`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PipelineTrace {
    pub workflow_id: String,
    pub run_id: String,
    pub revision: String,
    pub status: String,
    #[serde(default)]
    pub reused: bool,
    #[serde(default)]
    pub recoveries: u32,
    #[serde(default)]
    pub steps: BTreeMap<String, StepTrace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_error: Option<ThunkExecutionError>,
}

pub enum PipelineOutcome {
    Completed {
        output: BTreeMap<String, Value>,
        trace: PipelineTrace,
    },
    Pending {
        trace: PipelineTrace,
        resume: Value,
        check_after_secs: u64,
    },
    Failed {
        error: ThunkExecutionError,
        trace: PipelineTrace,
    },
}

pub struct PipelineRequest {
    pub workflow_id: &'static str,
    pub obj_id: String,
    pub input: Value,
    pub resume_run_id: Option<String>,
    pub session_id: Option<String>,
    pub wait_ms: u64,
}

impl PipelineRequest {
    pub fn new(workflow_id: &'static str, obj_id: &str, input: Value) -> Self {
        Self {
            workflow_id,
            obj_id: obj_id.to_string(),
            input,
            resume_run_id: None,
            session_id: None,
            wait_ms: DEFAULT_WAIT_MS,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn status_label(status: RunStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}").to_ascii_lowercase())
}

fn is_terminal(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Completed | RunStatus::Failed | RunStatus::Aborted | RunStatus::BudgetExhausted
    )
}

fn kernel_error(context: &str, err: &kRPC::RPCErrors) -> String {
    format!("workflow service (kernel) error during {context}: {err}")
}

async fn trace_of(
    client: &WorkflowServiceClient,
    run_id: &str,
    workflow_id: &str,
    revision: &str,
    status: RunStatus,
    reused: bool,
    recoveries: u32,
) -> PipelineTrace {
    let mut trace = PipelineTrace {
        workflow_id: workflow_id.to_string(),
        run_id: run_id.to_string(),
        revision: revision.to_string(),
        status: status_label(status),
        reused,
        recoveries,
        steps: BTreeMap::new(),
        run_error: None,
    };
    if let Ok(graph) = client.get_run_graph(run_id).await {
        fill_trace(&mut trace, &graph);
    }
    trace
}

fn fill_trace(trace: &mut PipelineTrace, graph: &WorkflowRunGraph) {
    trace.status = status_label(graph.status);
    if trace.revision.is_empty() {
        trace.revision = graph.revision.clone();
    }
    for (node, state) in &graph.node_states {
        trace.steps.insert(
            node.clone(),
            StepTrace {
                state: serde_json::to_value(state)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                source: graph.node_sources.get(node).cloned(),
                attempts: graph.node_attempts.get(node).copied().unwrap_or(0),
                error: graph.node_errors.get(node).cloned(),
            },
        );
    }
    trace.run_error = graph
        .metrics
        .get("run_error")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
}

fn failure_of(output: &WorkflowRunOutput) -> ThunkExecutionError {
    output.first_error().cloned().unwrap_or_else(|| {
        ThunkExecutionError::new(
            "run_failed",
            format!("run {} ended as {}", output.run_id, status_label(output.status)),
        )
    })
}

/// Drives one conversion request to a terminal state, a pending answer or a
/// diagnosed failure. The run is ensured once; later calls with the same
/// request poll the recorded run. A lost run (`run_not_found`) or an
/// explicitly retryable failure is re-ensured at most `MAX_RECOVERIES` times
/// inside `TOTAL_DEADLINE_SECS`.
pub async fn run_pipeline(request: PipelineRequest) -> Result<PipelineOutcome, String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let client = runtime
        .get_workflow_service_client()
        .await
        .map_err(|e| format!("workflow service (kernel) client unavailable: {e}"))?;
    let ledger = RunLedger::for_session(request.session_id.as_deref());
    let key = RunLedger::request_key(request.workflow_id, &request.obj_id, &request.input);
    let mut entry = ledger.load(&key);
    if let Some(run_id) = request.resume_run_id.as_ref() {
        if entry.as_ref().map(|e| &e.run_id) != Some(run_id) {
            entry = Some(LedgerEntry {
                run_id: run_id.clone(),
                workflow_id: request.workflow_id.to_string(),
                revision: String::new(),
                obj_id: request.obj_id.clone(),
                input: request.input.clone(),
                first_requested_at: now_secs(),
                recoveries: 0,
            });
        }
    }
    let started = Instant::now();
    let wait = Duration::from_millis(request.wait_ms);
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        if attempts > MAX_RECOVERIES + 1 {
            return Err("media pipeline recovery limit exceeded".to_string());
        }
        let (run_id, revision, reused, recoveries, first_requested_at) = match entry.clone() {
            Some(existing) => {
                if now_secs().saturating_sub(existing.first_requested_at) > TOTAL_DEADLINE_SECS {
                    ledger.remove(&key);
                    return Ok(PipelineOutcome::Failed {
                        error: ThunkExecutionError::new(
                            "deadline_exceeded",
                            format!(
                                "media pipeline run {} did not finish within {} seconds",
                                existing.run_id, TOTAL_DEADLINE_SECS
                            ),
                        ),
                        trace: PipelineTrace {
                            workflow_id: existing.workflow_id,
                            run_id: existing.run_id,
                            revision: existing.revision,
                            status: "unknown".to_string(),
                            recoveries: existing.recoveries,
                            ..Default::default()
                        },
                    });
                }
                (
                    existing.run_id,
                    existing.revision,
                    true,
                    existing.recoveries,
                    existing.first_requested_at,
                )
            }
            None => {
                let created = client
                    .create_run(
                        WorkflowCreateRunReq::new(request.workflow_id, request.input.clone())
                            .ensure(request.wait_ms),
                    )
                    .await
                    .map_err(|e| kernel_error("create_run", &e))?;
                let new_entry = LedgerEntry {
                    run_id: created.run_id.clone(),
                    workflow_id: request.workflow_id.to_string(),
                    revision: created.revision.clone(),
                    obj_id: request.obj_id.clone(),
                    input: request.input.clone(),
                    first_requested_at: now_secs(),
                    recoveries: 0,
                };
                ledger.save(&key, &new_entry);
                (created.run_id, created.revision, created.reused, 0, now_secs())
            }
        };

        // Poll until terminal or the short wait is spent.
        let output = loop {
            match client.get_run_output(&run_id).await {
                Ok(output) => {
                    if output.terminal || is_terminal(output.status) {
                        break Some(output);
                    }
                    if started.elapsed() >= wait {
                        let trace = trace_of(&client, &run_id, request.workflow_id, &revision, output.status, reused, recoveries).await;
                        return Ok(PipelineOutcome::Pending {
                            trace,
                            resume: json!({
                                "run_id": run_id,
                                "workflow_id": request.workflow_id,
                                "obj_id": request.obj_id,
                                "input": request.input,
                                "revision": revision,
                                "first_requested_at": first_requested_at,
                            }),
                            check_after_secs: PENDING_CHECK_AFTER_SECS,
                        });
                    }
                    tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
                }
                Err(err) => {
                    let lost = WorkflowRpcError::from_rpc_error(&err)
                        .map(|e| e.is_not_found())
                        .unwrap_or(false);
                    if !lost {
                        return Err(kernel_error("get_run_output", &err));
                    }
                    break None;
                }
            }
        };

        let Some(output) = output else {
            // The service forgot the run (restart): re-ensure within budget.
            if recoveries >= MAX_RECOVERIES {
                ledger.remove(&key);
                return Ok(PipelineOutcome::Failed {
                    error: ThunkExecutionError::new(
                        "run_lost",
                        format!("media pipeline run {run_id} was lost {recoveries} times; giving up"),
                    ),
                    trace: PipelineTrace {
                        workflow_id: request.workflow_id.to_string(),
                        run_id,
                        revision,
                        status: "not_found".to_string(),
                        recoveries,
                        ..Default::default()
                    },
                });
            }
            let created = client
                .create_run(
                    WorkflowCreateRunReq::new(request.workflow_id, request.input.clone())
                        .ensure(request.wait_ms),
                )
                .await
                .map_err(|e| kernel_error("create_run", &e))?;
            entry = Some(LedgerEntry {
                run_id: created.run_id,
                workflow_id: request.workflow_id.to_string(),
                revision: created.revision,
                obj_id: request.obj_id.clone(),
                input: request.input.clone(),
                first_requested_at,
                recoveries: recoveries + 1,
            });
            ledger.save(&key, entry.as_ref().unwrap());
            continue;
        };

        let revision = if revision.is_empty() { output.revision.clone() } else { revision };
        let trace = trace_of(&client, &run_id, request.workflow_id, &revision, output.status, reused, recoveries).await;
        if output.status == RunStatus::Completed {
            ledger.remove(&key);
            return Ok(PipelineOutcome::Completed {
                output: output.output,
                trace,
            });
        }
        let error = failure_of(&output);
        if error.retryable && recoveries < MAX_RECOVERIES {
            let created = client
                .create_run(
                    WorkflowCreateRunReq::new(request.workflow_id, request.input.clone())
                        .ensure(request.wait_ms),
                )
                .await
                .map_err(|e| kernel_error("create_run", &e))?;
            entry = Some(LedgerEntry {
                run_id: created.run_id,
                workflow_id: request.workflow_id.to_string(),
                revision: created.revision,
                obj_id: request.obj_id.clone(),
                input: request.input.clone(),
                first_requested_at,
                recoveries: recoveries + 1,
            });
            ledger.save(&key, entry.as_ref().unwrap());
            continue;
        }
        ledger.remove(&key);
        return Ok(PipelineOutcome::Failed { error, trace });
    }
}

// ----- coverage / content planning ----------------------------------------

/// `details.coverage`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Coverage {
    pub family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default)]
    pub requested: Vec<u64>,
    #[serde(default)]
    pub processed: Vec<u64>,
    #[serde(default)]
    pub failed: Vec<u64>,
    /// Coordinates whose text was given to the model.
    #[serde(default)]
    pub text_delivered: Vec<u64>,
    /// Coordinates whose image was given to the model.
    #[serde(default)]
    pub image_delivered: Vec<u64>,
    /// Coordinates the model did not see at all, with the reason.
    #[serde(default)]
    pub uncovered: Vec<Uncovered>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default)]
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Uncovered {
    #[serde(rename = "where")]
    pub location: u64,
    pub reason: String,
}

pub struct PagePlan {
    pub page: u64,
    pub text: Option<String>,
    pub image_obj_id: Option<String>,
}

pub struct ContentPlan {
    pub texts: Vec<(u64, String)>,
    pub images: Vec<(u64, String)>,
    pub coverage: Coverage,
}

fn entry_coordinate(entry: &MediaEntry) -> Option<u64> {
    entry
        .location
        .page
        .map(u64::from)
        .or(entry.location.time_ms)
        .or(entry.location.index.map(u64::from))
}

/// Terms of the goal that drive lexical page selection.
fn goal_terms(goal: &str) -> Vec<String> {
    let mut terms: Vec<String> = goal
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.to_lowercase())
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

fn lexical_score(text: &str, terms: &[String]) -> usize {
    let lower = text.to_lowercase();
    terms.iter().filter(|t| lower.contains(t.as_str())).count()
}

/// Chooses what the model sees from a page manifest: every page text while
/// it fits `text_char_budget`, otherwise the pages that lexically match the
/// goal; page images for pages without text first (visual coverage), then
/// for the selected pages, up to `max_images`.
pub fn plan_pages(
    goal: &str,
    manifest: &MediaManifest,
    texts: &BTreeMap<u64, String>,
    text_char_budget: usize,
    max_images: usize,
) -> ContentPlan {
    let terms = goal_terms(goal);
    let mut coverage = Coverage {
        family: manifest.family.clone(),
        total: manifest.range.total,
        requested: manifest.range.requested.clone(),
        processed: manifest.range.processed.clone(),
        failed: manifest.range.failed.clone(),
        complete: manifest.complete,
        ..Default::default()
    };
    let mut pages: BTreeMap<u64, PagePlan> = BTreeMap::new();
    let mut failed_reasons: BTreeMap<u64, String> = BTreeMap::new();
    for entry in &manifest.entries {
        let Some(coord) = entry_coordinate(entry) else {
            continue;
        };
        let plan = pages.entry(coord).or_insert(PagePlan {
            page: coord,
            text: None,
            image_obj_id: None,
        });
        match entry.status {
            MediaEntryStatus::Failed => {
                failed_reasons.insert(
                    coord,
                    entry
                        .error
                        .as_ref()
                        .map(|e| format!("{}: {}", e.code, e.message))
                        .unwrap_or_else(|| "failed".to_string()),
                );
            }
            MediaEntryStatus::Unprocessed => {
                failed_reasons
                    .entry(coord)
                    .or_insert_with(|| "not processed (outside the requested range)".to_string());
            }
            _ => {}
        }
        if entry.text_obj_id.is_some() {
            if let Some(text) = texts.get(&coord) {
                if !text.trim().is_empty() {
                    plan.text = Some(text.clone());
                }
            }
        }
        if let Some(image) = entry.image_obj_id.as_ref() {
            plan.image_obj_id = Some(image.clone());
        }
    }

    // Text selection.
    let total_chars: usize = pages.values().filter_map(|p| p.text.as_ref()).map(|t| t.chars().count()).sum();
    let mut selected_text: BTreeSet<u64> = BTreeSet::new();
    if total_chars <= text_char_budget {
        selected_text.extend(pages.values().filter(|p| p.text.is_some()).map(|p| p.page));
    } else {
        let mut ranked: Vec<(usize, u64)> = pages
            .values()
            .filter_map(|p| p.text.as_ref().map(|t| (lexical_score(t, &terms), p.page)))
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut used = 0usize;
        for (_, page) in ranked {
            let len = pages[&page].text.as_ref().map(|t| t.chars().count()).unwrap_or(0);
            if used + len > text_char_budget {
                continue;
            }
            used += len;
            selected_text.insert(page);
        }
    }

    // Image selection: pages without text first, then selected text pages.
    let mut selected_images: Vec<u64> = Vec::new();
    let mut no_text: Vec<u64> = pages
        .values()
        .filter(|p| p.text.is_none() && p.image_obj_id.is_some())
        .map(|p| p.page)
        .collect();
    if no_text.len() > max_images {
        let mut ranked: Vec<(usize, u64)> = no_text
            .iter()
            .map(|p| (lexical_score(&format!("page {p}"), &terms), *p))
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        no_text = ranked.into_iter().map(|(_, p)| p).collect();
    }
    for page in no_text {
        if selected_images.len() >= max_images {
            break;
        }
        selected_images.push(page);
    }
    for page in &selected_text {
        if selected_images.len() >= max_images {
            break;
        }
        if pages[page].image_obj_id.is_some() && !selected_images.contains(page) {
            selected_images.push(*page);
        }
    }
    selected_images.sort_unstable();

    let texts_out: Vec<(u64, String)> = selected_text
        .iter()
        .map(|p| (*p, pages[p].text.clone().unwrap_or_default()))
        .collect();
    let images_out: Vec<(u64, String)> = selected_images
        .iter()
        .map(|p| (*p, pages[p].image_obj_id.clone().unwrap_or_default()))
        .collect();
    coverage.text_delivered = selected_text.iter().copied().collect();
    coverage.image_delivered = selected_images.clone();
    for (coord, plan) in &pages {
        let seen = selected_text.contains(coord) || selected_images.contains(coord);
        if seen {
            continue;
        }
        let reason = failed_reasons.get(coord).cloned().unwrap_or_else(|| {
            if plan.text.is_some() || plan.image_obj_id.is_some() {
                "omitted by the token budget".to_string()
            } else {
                "no representation produced".to_string()
            }
        });
        coverage.uncovered.push(Uncovered {
            location: *coord,
            reason,
        });
    }
    for coord in &manifest.range.requested {
        if !pages.contains_key(coord) {
            coverage.uncovered.push(Uncovered {
                location: *coord,
                reason: "no manifest entry".to_string(),
            });
        }
    }
    coverage.uncovered.sort_by_key(|u| u.location);
    coverage.uncovered.dedup_by_key(|u| u.location);
    ContentPlan {
        texts: texts_out,
        images: images_out,
        coverage,
    }
}

pub fn frame_entries(manifest: &MediaManifest) -> Vec<&MediaEntry> {
    manifest
        .entries
        .iter()
        .filter(|e| e.kind == MediaEntryKind::Frame)
        .collect()
}

pub fn audio_entry(manifest: &MediaManifest) -> Option<&MediaEntry> {
    manifest.entries.iter().find(|e| e.kind == MediaEntryKind::Audio)
}

pub fn audio_mime(codec: Option<&str>) -> &'static str {
    match codec.map(|c| c.to_ascii_lowercase()).as_deref() {
        Some("mp3") | Some("mp3float") => "audio/mpeg",
        Some("aac") => "audio/aac",
        Some("opus") | Some("vorbis") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("pcm_s16le") | Some("wav") => "audio/wav",
        _ => "audio/mpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{MediaEntryError, MediaSourceRef, MediaWhere};

    fn entry(page: u32, status: MediaEntryStatus, text: bool, image: bool) -> MediaEntry {
        MediaEntry {
            kind: MediaEntryKind::Page,
            location: MediaWhere { source: MediaSourceRef::Original, page: Some(page), time_ms: None, index: None },
            text_obj_id: text.then(|| format!("cyfile:t{page}")),
            image_obj_id: image.then(|| format!("cyfile:i{page}")),
            audio_obj_id: None,
            status,
            error: (status == MediaEntryStatus::Failed).then(|| MediaEntryError { code: "tool_failed".into(), message: "render".into(), retryable: true }),
        }
    }

    #[test]
    fn header_dimensions() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((640, 480)));
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x10, 0x00, 0x08, 0x00]);
        assert_eq!(image_dimensions(&gif), Some((16, 8)));
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
        jpeg.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0x2c, 0x01, 0x90, 0x03]);
        assert_eq!(image_dimensions(&jpeg), Some((400, 300)));
        assert_eq!(image_dimensions(b"%PDF-1.4"), None);
        assert!(ImageLimits::default().allows(2048, 2048, 100));
        assert!(!ImageLimits::default().allows(2049, 10, 100));
    }

    #[test]
    fn page_planning_prefers_goal_matches_and_covers_scanned_pages() {
        let mut manifest = MediaManifest::new("pdf", "sha256:x");
        manifest.range.total = Some(5);
        manifest.range.requested = vec![1, 2, 3, 4, 5];
        manifest.entries = vec![
            entry(1, MediaEntryStatus::Ok, true, true),
            entry(2, MediaEntryStatus::Ok, true, true),
            entry(3, MediaEntryStatus::NoText, false, true),
            entry(4, MediaEntryStatus::Failed, false, false),
            entry(5, MediaEntryStatus::Ok, true, true),
        ];
        manifest.recompute_coverage();
        let texts = BTreeMap::from([
            (1u64, "introduction about cats".to_string()),
            (2u64, "the answer is on the last page".to_string()),
            (5u64, "final answer: forty-two".to_string()),
        ]);
        let plan = plan_pages("what is the final answer", &manifest, &texts, 25, 2);
        let text_pages: Vec<u64> = plan.texts.iter().map(|(p, _)| *p).collect();
        assert!(text_pages.contains(&5), "{text_pages:?}");
        assert!(!text_pages.contains(&1), "{text_pages:?}");
        let image_pages: Vec<u64> = plan.images.iter().map(|(p, _)| *p).collect();
        assert!(image_pages.contains(&3), "scanned page gets visual coverage: {image_pages:?}");
        assert_eq!(image_pages.len(), 2);
        assert!(plan.coverage.uncovered.iter().any(|u| u.location == 4 && u.reason.starts_with("tool_failed")));
        assert!(plan.coverage.uncovered.iter().any(|u| u.location == 1));
        assert!(!plan.coverage.complete);
        assert_eq!(plan.coverage.failed, vec![4]);

        let all = plan_pages("anything", &manifest, &texts, 10_000, 16);
        assert_eq!(all.texts.len(), 3);
        assert_eq!(all.images.len(), 4);
        assert_eq!(all.coverage.uncovered.len(), 1);
    }

    #[test]
    fn request_key_is_stable_and_formats_resolve() {
        let a = RunLedger::request_key("media.pdf", "cyfile:a", &json!({"dpi": 144, "limits": {"max_pages": 50}}));
        let b = RunLedger::request_key("media.pdf", "cyfile:a", &json!({"limits": {"max_pages": 50}, "dpi": 144}));
        assert_eq!(a, b);
        assert_ne!(a, RunLedger::request_key("media.pdf", "cyfile:b", &json!({"dpi": 144})));
        assert_eq!(document_format_from_name("Report.DOCX"), Some("docx"));
        assert_eq!(document_format_from_name("x.pdf"), None);
        assert_eq!(document_format_from_mime("application/vnd.openxmlformats-officedocument.presentationml.presentation"), Some("pptx"));
        assert!(protocol_accepts_documents("claude-messages"));
        assert!(!protocol_accepts_documents("openai-chat-completions"));
        assert_eq!(audio_mime(Some("aac")), "audio/aac");
    }
}
