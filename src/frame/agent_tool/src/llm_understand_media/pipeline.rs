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

use tokio::io::{AsyncRead, AsyncReadExt};

use buckyos_api::{
    get_buckyos_api_runtime, MediaEntry, MediaEntryKind, MediaEntryStatus, MediaManifest,
    RunStatus, ThunkExecutionError, WorkflowCreateRunReq, WorkflowRpcError, WorkflowRunGraph,
    WorkflowRunOutput, WorkflowServiceClient,
};
use ndn_lib::{ChunkHasher, ChunkId, ChunkList, FileObject, NamedObject, NdnError, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const FAMILY_IMAGE: &str = "media.image";
pub const FAMILY_PDF: &str = "media.pdf";
pub const FAMILY_VIDEO: &str = "media.video";
pub const FAMILY_DOCUMENT: &str = "media.document";

const CREATE_WAIT_MS: u64 = 2_000;
const POLL_MIN_MS: u64 = 400;
const POLL_MAX_MS: u64 = 3_000;
/// Longer than the longest `guards.max_duration` of the system definitions.
const TOTAL_DEADLINE_SECS: u64 = 3_600;
const MAX_RECOVERIES: u32 = 3;
/// Most pages `media.pdf` / `media.document` take in one run.
pub const CATALOG_WINDOW_PAGES: u32 = 200;
/// Most bytes workflow takes as one input (`ExecutionLimits::max_input_bytes`).
pub const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const INDEX_CHARS: usize = 6_000;

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
    /// Pages of one document that become catalog entries.
    pub max_pages: u32,
    /// A document with more pages is long: never direct, always a catalog.
    pub long_doc_pages: u32,
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
            max_pages: env_positive("LLM_UNDERSTAND_MEDIA_MAX_PAGES").unwrap_or(2000),
            long_doc_pages: env_positive("LLM_UNDERSTAND_MEDIA_LONG_DOC_PAGES").unwrap_or(30),
            direct_max_bytes: env_positive("LLM_UNDERSTAND_MEDIA_DOCUMENT_MAX_BYTES")
                .unwrap_or(20 * 1024 * 1024),
        }
    }

    pub fn pipeline_input(
        &self,
        obj_id: &str,
        format: Option<&str>,
        page_range: &[u32],
        max_pages: u32,
    ) -> Value {
        let mut input = json!({
            "obj_id": obj_id,
            "dpi": self.dpi,
            "max_long_edge": self.max_long_edge,
            "page_range": page_range,
            "limits": {"max_pages": max_pages},
        });
        if let Some(format) = format {
            input["format"] = Value::String(format.to_string());
        }
        input
    }

    /// The first window takes every page from the start, so it is the same run
    /// for documents of any length and also reports the total.
    pub fn first_window_pages(&self) -> u32 {
        self.max_pages.min(CATALOG_WINDOW_PAGES)
    }

    pub fn later_windows(&self, total_pages: u64) -> Vec<Vec<u32>> {
        let last = total_pages.min(u64::from(self.max_pages)) as u32;
        let mut windows = Vec::new();
        let mut first = self.first_window_pages() + 1;
        while first <= last {
            let end = first.saturating_add(CATALOG_WINDOW_PAGES - 1).min(last);
            windows.push((first..=end).collect());
            first = end + 1;
        }
        windows
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

/// Stores content in the Named Store as a FileObject (same chunking as
/// `BucketClient::put_reader`, so identical bytes get identical content ids)
/// and returns its id. Time fields stay 0: the same file freezes to the same
/// id, so a repeated request finds its run again.
pub async fn freeze_reader<R: AsyncRead + Unpin>(
    name: &str,
    mime: &str,
    size: u64,
    mut reader: R,
) -> Result<ObjId, String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let store = runtime
        .get_named_store()
        .await
        .map_err(|e| format!("named store: {e}"))?;
    let chunk_size = ndn_lib::CHUNK_DEFAULT_SIZE as usize;
    let mut buffer = vec![0u8; chunk_size.min(size.max(1) as usize)];
    let mut chunks = Vec::new();
    let mut total = 0u64;
    loop {
        let mut filled = 0usize;
        while filled < buffer.len() {
            let read = reader
                .read(&mut buffer[filled..])
                .await
                .map_err(|e| format!("read media: {e}"))?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 && !chunks.is_empty() {
            break;
        }
        let piece = &buffer[..filled];
        let hasher = ChunkHasher::new(None).map_err(|e| format!("chunk hasher: {e}"))?;
        let method = hasher.hash_method;
        let hash = hasher.calc_from_bytes(piece);
        let chunk_id = ChunkId::from_mix_hash_result_by_hash_method(piece.len() as u64, &hash, method)
            .map_err(|e| format!("chunk id: {e}"))?;
        // Also when the store has it: only what this session uploaded may be referenced.
        store
            .put_chunk(&chunk_id, piece)
            .await
            .map_err(|e| format!("store chunk: {e}"))?;
        chunks.push(chunk_id);
        total += filled as u64;
        if filled < buffer.len() {
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
    let mut file = FileObject::default();
    file.content_obj.name = name.to_string();
    file.content_obj.create_time = 0;
    file.content_obj.last_update_time = 0;
    file.size = total;
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

/// Reads the FileObject and at most `head_bytes` of its content; `None` when
/// the store refuses this identity.
pub async fn object_head(obj_id: &ObjId, head_bytes: usize) -> Result<Option<ObjectHead>, String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let store = runtime
        .get_named_store()
        .await
        .map_err(|e| format!("named store: {e}"))?;
    let body = match store.get_object(obj_id).await {
        Ok(body) => body,
        Err(NdnError::PermissionDenied(_)) => return Ok(None),
        Err(e) => return Err(format!("read {obj_id}: {e}")),
    };
    let file: FileObject = serde_json::from_str(&body)
        .map_err(|e| format!("{obj_id} is not a file object: {e}"))?;
    let (mut reader, total) = match store.open_reader(obj_id, None).await {
        Ok(opened) => opened,
        Err(NdnError::PermissionDenied(_)) => return Ok(None),
        Err(e) => return Err(format!("open {obj_id}: {e}")),
    };
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
    Ok(Some(ObjectHead {
        size: if file.size > 0 { file.size } else { total },
        name,
        mime,
        head,
    }))
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

/// Where the run being waited for is remembered, keyed by the request identity
/// (`family + obj_id + effective params`), so a later call with the same media
/// (the shell stopped the last one, the agent asked again) follows the same run.
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
    /// Runs of the later page windows of a long document.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_runs: Vec<PipelineTrace>,
}

pub enum PipelineOutcome {
    Completed {
        output: BTreeMap<String, Value>,
        trace: PipelineTrace,
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
    pub session_id: Option<String>,
}

impl PipelineRequest {
    pub fn new(workflow_id: &'static str, obj_id: &str, input: Value) -> Self {
        Self {
            workflow_id,
            obj_id: obj_id.to_string(),
            input,
            session_id: None,
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
        ..Default::default()
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

async fn ensure_run(
    client: &WorkflowServiceClient,
    request: &PipelineRequest,
    first_requested_at: u64,
    recoveries: u32,
) -> Result<(LedgerEntry, bool), String> {
    let created = client
        .create_run(
            WorkflowCreateRunReq::new(request.workflow_id, request.input.clone())
                .ensure(CREATE_WAIT_MS),
        )
        .await
        .map_err(|e| kernel_error("create_run", &e))?;
    let reused = created.reused;
    Ok((
        LedgerEntry {
            run_id: created.run_id,
            workflow_id: request.workflow_id.to_string(),
            revision: created.revision,
            obj_id: request.obj_id.clone(),
            input: request.input.clone(),
            first_requested_at,
            recoveries,
        },
        reused,
    ))
}

/// `None`: the service no longer knows the run.
async fn wait_terminal(
    client: &WorkflowServiceClient,
    run_id: &str,
    deadline: Instant,
) -> Result<Option<Result<WorkflowRunOutput, RunStatus>>, String> {
    let mut poll = Duration::from_millis(POLL_MIN_MS);
    loop {
        match client.get_run_output(run_id).await {
            Ok(output) if output.terminal || is_terminal(output.status) => {
                return Ok(Some(Ok(output)));
            }
            Ok(output) => {
                if Instant::now() >= deadline {
                    return Ok(Some(Err(output.status)));
                }
            }
            Err(err) => {
                let lost = WorkflowRpcError::from_rpc_error(&err)
                    .map(|e| e.is_not_found())
                    .unwrap_or(false);
                if lost {
                    return Ok(None);
                }
                return Err(kernel_error("get_run_output", &err));
            }
        }
        tokio::time::sleep(poll).await;
        poll = (poll * 2).min(Duration::from_millis(POLL_MAX_MS));
    }
}

/// Waits for one conversion request to end. The run is ensured once; a later
/// call with the same request follows the recorded run. A lost run
/// (`run_not_found`) or an explicitly retryable failure is re-ensured at most
/// `MAX_RECOVERIES` times inside `TOTAL_DEADLINE_SECS`.
pub async fn run_pipeline(request: PipelineRequest) -> Result<PipelineOutcome, String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("no buckyos runtime: {e}"))?;
    let client = runtime
        .get_workflow_service_client()
        .await
        .map_err(|e| format!("workflow service (kernel) client unavailable: {e}"))?;
    let ledger = RunLedger::for_session(request.session_id.as_deref());
    let key = RunLedger::request_key(request.workflow_id, &request.obj_id, &request.input);
    let deadline = Instant::now() + Duration::from_secs(TOTAL_DEADLINE_SECS);
    let mut entry = ledger
        .load(&key)
        .filter(|e| now_secs().saturating_sub(e.first_requested_at) <= TOTAL_DEADLINE_SECS);
    let mut reused = entry.is_some();
    loop {
        let current = match entry.take() {
            Some(existing) => existing,
            None => {
                let (created, was_reused) = ensure_run(&client, &request, now_secs(), 0).await?;
                reused = was_reused;
                ledger.save(&key, &created);
                created
            }
        };
        let trace_for = |status: &str| PipelineTrace {
            workflow_id: request.workflow_id.to_string(),
            run_id: current.run_id.clone(),
            revision: current.revision.clone(),
            status: status.to_string(),
            reused,
            recoveries: current.recoveries,
            ..Default::default()
        };
        let waited = wait_terminal(&client, &current.run_id, deadline).await?;
        let output = match waited {
            Some(Ok(output)) => output,
            Some(Err(status)) => {
                return Ok(PipelineOutcome::Failed {
                    error: ThunkExecutionError::new(
                        "deadline_exceeded",
                        format!(
                            "media pipeline run {} did not finish within {TOTAL_DEADLINE_SECS} seconds",
                            current.run_id
                        ),
                    ),
                    trace: trace_for(&status_label(status)),
                });
            }
            None if current.recoveries >= MAX_RECOVERIES => {
                ledger.remove(&key);
                return Ok(PipelineOutcome::Failed {
                    error: ThunkExecutionError::new(
                        "run_lost",
                        format!(
                            "media pipeline run {} was lost {} times; giving up",
                            current.run_id, current.recoveries
                        ),
                    ),
                    trace: trace_for("not_found"),
                });
            }
            None => {
                let (created, was_reused) = ensure_run(
                    &client,
                    &request,
                    current.first_requested_at,
                    current.recoveries + 1,
                )
                .await?;
                reused = was_reused;
                ledger.save(&key, &created);
                entry = Some(created);
                continue;
            }
        };
        let revision = if current.revision.is_empty() {
            output.revision.clone()
        } else {
            current.revision.clone()
        };
        let trace = trace_of(
            &client,
            &current.run_id,
            request.workflow_id,
            &revision,
            output.status,
            reused,
            current.recoveries,
        )
        .await;
        if output.status == RunStatus::Completed {
            ledger.remove(&key);
            return Ok(PipelineOutcome::Completed {
                output: output.output,
                trace,
            });
        }
        let error = failure_of(&output);
        if error.retryable && current.recoveries < MAX_RECOVERIES {
            let (created, was_reused) = ensure_run(
                &client,
                &request,
                current.first_requested_at,
                current.recoveries + 1,
            )
            .await?;
            reused = was_reused;
            ledger.save(&key, &created);
            entry = Some(created);
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

impl Coverage {
    /// The line the calling agent reads: what the model did not see, by reason.
    pub fn uncovered_note(&self) -> Option<String> {
        if self.uncovered.is_empty() {
            return None;
        }
        let mut by_reason: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
        for item in &self.uncovered {
            by_reason.entry(item.reason.as_str()).or_default().push(item.location);
        }
        let parts = by_reason
            .iter()
            .map(|(reason, places)| format!("{reason}: {}", format_ranges(places)))
            .collect::<Vec<_>>()
            .join("; ");
        let (unit, hint) = if self.family == "video" {
            ("frames (by ms)", "")
        } else {
            ("pages", " Name a page in the goal (\"page 12\") to read it.")
        };
        let of = self.total.filter(|_| self.family != "video").unwrap_or(self.requested.len() as u64);
        Some(format!(
            "Note: {} of {of} {unit} were not shown to the model ({parts}).{hint}",
            self.uncovered.len()
        ))
    }
}

/// `[1, 2, 3, 7]` -> `1-3, 7`.
fn format_ranges(places: &[u64]) -> String {
    let mut sorted = places.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut iter = sorted.into_iter().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().unwrap_or(end);
        }
        parts.push(if start == end { start.to_string() } else { format!("{start}-{end}") });
    }
    parts.join(", ")
}

pub struct PagePlan {
    pub page: u64,
    pub text: Option<String>,
    pub image_obj_id: Option<String>,
}

pub struct ContentPlan {
    /// Every page of the document, when the model will not see them all.
    pub index: Option<String>,
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

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'
        | '\u{f900}'..='\u{faff}' | '\u{ac00}'..='\u{d7af}')
}

/// Terms of the goal that drive lexical page selection: words of other scripts,
/// and overlapping character pairs of CJK text, which has no spaces to split on.
fn goal_terms(goal: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in goal.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = word.to_lowercase().chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let cjk = is_cjk(chars[start]);
            let end = (start..chars.len())
                .find(|i| is_cjk(chars[*i]) != cjk)
                .unwrap_or(chars.len());
            let piece = &chars[start..end];
            if cjk {
                terms.extend(piece.windows(2).map(|pair| pair.iter().collect::<String>()));
            } else if piece.len() >= 3 || (piece.len() == 2 && piece.iter().any(char::is_ascii_digit)) {
                terms.push(piece.iter().collect());
            }
            start = end;
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn lexical_score(text: &str, terms: &[String]) -> usize {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    terms.iter().filter(|t| compact.contains(t.as_str())).count()
}

fn leading_number(text: &str) -> Option<(u64, &str)> {
    let text = text.trim_start();
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    if digits == 0 {
        return None;
    }
    Some((text[..digits].parse().ok()?, &text[digits..]))
}

/// Pages the goal names: `page 12`, `p. 12`, `pages 3-5`, `第12页`, `第3到5页`.
fn goal_pages(goal: &str) -> BTreeSet<u64> {
    let lower = goal.to_lowercase();
    let mut pages = BTreeSet::new();
    for marker in ["pages", "page", "p.", "第"] {
        let mut rest = lower.as_str();
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            let Some((first, after)) = leading_number(rest) else {
                continue;
            };
            let range_end = after
                .trim_start()
                .strip_prefix(['-', '–', '~', '到', '至'])
                .and_then(leading_number);
            let (last, after) = range_end.unwrap_or((first, after));
            if marker == "第" && !after.trim_start().starts_with('页') {
                continue;
            }
            pages.extend(first..=last.min(first.saturating_add(u64::from(CATALOG_WINDOW_PAGES))));
        }
    }
    pages
}

/// `pages` reordered so that any prefix of about `fits` entries spreads over the
/// whole list instead of covering only its start.
fn spread(pages: &[u64], fits: usize) -> Vec<u64> {
    let fits = fits.clamp(1, pages.len().max(1));
    let mut order: Vec<u64> = (0..fits)
        .filter_map(|i| pages.get(i * pages.len() / fits).copied())
        .collect();
    let picked: BTreeSet<u64> = order.iter().copied().collect();
    order.extend(pages.iter().copied().filter(|p| !picked.contains(p)));
    order
}

/// The list of every page: one line each with the start of its text when that
/// fits `budget`, ranges of the same kind otherwise.
fn render_index(pages: &BTreeMap<u64, PagePlan>, budget: usize) -> String {
    let kind = |plan: &PagePlan| match (plan.text.is_some(), plan.image_obj_id.is_some()) {
        (true, true) => "text+image",
        (true, false) => "text",
        (false, true) => "image",
        (false, false) => "nothing",
    };
    let snippet_chars = (budget / pages.len().max(1)).saturating_sub(16).min(60);
    let mut out = String::from("Index of the whole document (what the pipeline has for each page):\n");
    if snippet_chars >= 16 {
        for plan in pages.values() {
            let snippet: String = plan
                .text
                .as_deref()
                .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
                .unwrap_or("")
                .chars()
                .take(snippet_chars)
                .collect();
            out.push_str(&format!("{} [{}] {}\n", plan.page, kind(plan), snippet));
        }
        return out;
    }
    let mut by_kind: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for plan in pages.values() {
        by_kind.entry(kind(plan)).or_default().push(plan.page);
    }
    for (name, places) in by_kind {
        out.push_str(&format!("{name}: {}\n", format_ranges(&places)));
    }
    out
}

/// Chooses what the model sees from a page manifest. Text: every page while it
/// fits `text_char_budget`, otherwise the pages the goal names, then those that
/// lexically match it; with no match at all, pages spread over the document.
/// Images: named pages, up to half the budget for matching pages (their figures
/// are what text search cannot show), pages that only have an image, then the
/// other selected pages.
pub fn plan_pages(
    goal: &str,
    manifest: &MediaManifest,
    texts: &BTreeMap<u64, String>,
    unreadable: &BTreeMap<u64, String>,
    text_char_budget: usize,
    max_images: usize,
) -> ContentPlan {
    let terms = goal_terms(goal);
    let named = goal_pages(goal);
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
                    .or_insert_with(|| "not processed (outside the page limit)".to_string());
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
    let text_pages: Vec<u64> = pages.values().filter(|p| p.text.is_some()).map(|p| p.page).collect();
    let text_len = |page: u64| pages[&page].text.as_ref().map(|t| t.chars().count()).unwrap_or(0);
    let total_chars: usize = text_pages.iter().map(|p| text_len(*p)).sum();
    let mut scored: Vec<(usize, u64)> = text_pages
        .iter()
        .map(|p| {
            let score = if named.contains(p) {
                usize::MAX
            } else {
                pages[p].text.as_deref().map(|t| lexical_score(t, &terms)).unwrap_or(0)
            };
            (score, *p)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let matching: Vec<u64> = scored.iter().filter(|(s, _)| *s > 0).map(|(_, p)| *p).collect();
    let ranked: Vec<u64> = if matching.is_empty() {
        let average = total_chars / text_pages.len().max(1);
        spread(&text_pages, text_char_budget / average.max(1))
    } else {
        scored.iter().map(|(_, p)| *p).collect()
    };
    let mut selected_text: BTreeSet<u64> = BTreeSet::new();
    let mut used = 0usize;
    for page in &ranked {
        let len = text_len(*page);
        if used + len > text_char_budget {
            continue;
        }
        used += len;
        selected_text.insert(*page);
    }

    // Image selection.
    let has_image = |page: &u64| pages.get(page).is_some_and(|p| p.image_obj_id.is_some());
    let mut selected_images: Vec<u64> = Vec::new();
    let take = |page: u64, limit: usize, chosen: &mut Vec<u64>| {
        if chosen.len() < limit && !chosen.contains(&page) {
            chosen.push(page);
        }
    };
    for page in named.iter().filter(|p| has_image(p)) {
        take(*page, max_images, &mut selected_images);
    }
    for page in matching.iter().filter(|p| selected_text.contains(*p) && has_image(p)) {
        take(*page, (max_images / 2).max(selected_images.len()), &mut selected_images);
    }
    let image_only: Vec<u64> = pages
        .values()
        .filter(|p| p.text.is_none() && p.image_obj_id.is_some())
        .map(|p| p.page)
        .collect();
    for page in spread(&image_only, max_images.saturating_sub(selected_images.len())) {
        take(page, max_images, &mut selected_images);
    }
    for page in ranked.iter().filter(|p| selected_text.contains(*p) && has_image(p)) {
        take(*page, max_images, &mut selected_images);
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
    for (coord, reason) in unreadable {
        coverage.uncovered.push(Uncovered {
            location: *coord,
            reason: format!("text unreadable: {reason}"),
        });
    }
    for (coord, plan) in &pages {
        let seen = selected_text.contains(coord) || selected_images.contains(coord);
        if seen {
            continue;
        }
        let reason = failed_reasons.get(coord).cloned().unwrap_or_else(|| {
            if plan.text.is_some() || plan.image_obj_id.is_some() {
                "omitted by the budget".to_string()
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
    let index = (!coverage.uncovered.is_empty()).then(|| render_index(&pages, INDEX_CHARS));
    ContentPlan {
        index,
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
        let none = BTreeMap::new();
        let plan = plan_pages("what is the final answer", &manifest, &texts, &none, 25, 2);
        let text_pages: Vec<u64> = plan.texts.iter().map(|(p, _)| *p).collect();
        assert!(text_pages.contains(&5), "{text_pages:?}");
        assert!(!text_pages.contains(&1), "{text_pages:?}");
        let image_pages: Vec<u64> = plan.images.iter().map(|(p, _)| *p).collect();
        assert!(image_pages.contains(&3), "scanned page gets visual coverage: {image_pages:?}");
        assert!(image_pages.contains(&5), "the matching page gets its image: {image_pages:?}");
        assert_eq!(image_pages.len(), 2);
        assert!(plan.coverage.uncovered.iter().any(|u| u.location == 4 && u.reason.starts_with("tool_failed")));
        assert!(plan.coverage.uncovered.iter().any(|u| u.location == 1));
        assert!(!plan.coverage.complete);
        assert_eq!(plan.coverage.failed, vec![4]);
        assert!(plan.index.as_deref().unwrap().contains("Index of the whole document"));

        let all = plan_pages("anything", &manifest, &texts, &none, 10_000, 16);
        assert_eq!(all.texts.len(), 3);
        assert_eq!(all.images.len(), 4);
        assert_eq!(all.coverage.uncovered.len(), 1);
    }

    fn long_document(pages: u32) -> (MediaManifest, BTreeMap<u64, String>) {
        let mut manifest = MediaManifest::new("pdf", "sha256:x");
        manifest.range.total = Some(u64::from(pages));
        manifest.range.requested = (1..=u64::from(pages)).collect();
        manifest.entries = (1..=pages).map(|p| entry(p, MediaEntryStatus::Ok, true, true)).collect();
        manifest.recompute_coverage();
        let texts = (1..=u64::from(pages)).map(|p| (p, format!("ordinary filler {p:04}"))).collect();
        (manifest, texts)
    }

    #[test]
    fn a_chinese_goal_finds_the_page_that_answers_it() {
        let (manifest, mut texts) = long_document(120);
        texts.insert(97, "第三季度 营收 为 四十二 亿元".to_string());
        let plan = plan_pages("第三季度的营收是多少", &manifest, &texts, &BTreeMap::new(), 20, 4);
        assert_eq!(plan.coverage.text_delivered, vec![97]);
        assert!(plan.images.iter().any(|(p, _)| *p == 97));
    }

    #[test]
    fn a_named_page_is_read_and_a_goal_without_matches_spreads_over_the_document() {
        let (manifest, texts) = long_document(100);
        let plan = plan_pages("translate 第88页", &manifest, &texts, &BTreeMap::new(), 100, 2);
        assert!(plan.coverage.text_delivered.contains(&88));
        assert!(plan.coverage.image_delivered.contains(&88));

        let plan = plan_pages("zzz", &manifest, &texts, &BTreeMap::new(), 200, 0);
        let delivered = &plan.coverage.text_delivered;
        assert!(delivered.len() >= 5);
        assert!(delivered.iter().any(|p| *p > 80), "{delivered:?}");
        assert!(delivered.iter().any(|p| *p < 20), "{delivered:?}");
    }

    #[test]
    fn unread_text_is_reported_and_the_note_names_ranges() {
        let (manifest, texts) = long_document(30);
        let unreadable = BTreeMap::from([(7u64, "gone".to_string())]);
        let plan = plan_pages("zzz", &manifest, &texts, &unreadable, 60, 0);
        assert!(plan.coverage.uncovered.iter().any(|u| u.location == 7 && u.reason.contains("gone")));
        let note = plan.coverage.uncovered_note().unwrap();
        assert!(note.contains("were not shown to the model"), "{note}");
        assert!(note.contains("omitted by the budget"), "{note}");
        assert_eq!(format_ranges(&[7, 1, 2, 3, 9, 10]), "1-3, 7, 9-10");
    }

    #[test]
    fn goal_terms_and_pages() {
        assert!(goal_terms("Q3 revenue").contains(&"q3".to_string()));
        assert!(goal_terms("Q3 revenue").contains(&"revenue".to_string()));
        assert!(!goal_terms("is a 3").contains(&"3".to_string()));
        assert_eq!(goal_terms("营收多少"), vec!["多少", "收多", "营收"]);
        assert_eq!(goal_pages("see page 12 and pages 3-5"), BTreeSet::from([3, 4, 5, 12]));
        assert_eq!(goal_pages("第12页和第3到5页"), BTreeSet::from([3, 4, 5, 12]));
        assert_eq!(goal_pages("第三季度 第2季度"), BTreeSet::new());
    }

    #[test]
    fn a_long_document_is_read_in_windows_up_to_the_page_limit() {
        let limits = DocumentLimits {
            dpi: 144,
            max_long_edge: 2048,
            max_pages: 1000,
            long_doc_pages: 20,
            direct_max_bytes: 1,
        };
        assert_eq!(limits.first_window_pages(), 200);
        assert!(limits.later_windows(200).is_empty());
        let windows = limits.later_windows(450);
        assert_eq!(windows.len(), 2);
        assert_eq!((windows[0][0], *windows[0].last().unwrap()), (201, 400));
        assert_eq!((windows[1][0], *windows[1].last().unwrap()), (401, 450));
        let capped = DocumentLimits { max_pages: 250, ..limits }.later_windows(450);
        assert_eq!(capped.len(), 1);
        assert_eq!(*capped[0].last().unwrap(), 250);
        assert!(DocumentLimits { max_pages: 50, ..limits }.later_windows(450).is_empty());
        let input = limits.pipeline_input("cyfile:a", Some("docx"), &[1], 1);
        assert_eq!(input["page_range"], json!([1]));
        assert_eq!(input["limits"]["max_pages"], 1);
        assert_eq!(input["format"], "docx");
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
    }
}
