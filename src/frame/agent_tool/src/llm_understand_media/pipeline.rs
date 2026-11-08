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
use std::sync::atomic::{AtomicBool, Ordering};
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
/// `ocr_lang` of a window that chooses its OCR language itself.
pub const OCR_LANG_AUTO: &str = "auto";
/// A page's text is never cut below this many characters.
pub const PAGE_TEXT_FLOOR: usize = 1_500;
/// Steps that do the work of a conversion (the others take a moment).
const HEAVY_STEPS: [&str; 4] = ["to_pdf", "extract_pdf_text", "render_pdf_pages", "ocr_pages"];
const HEARTBEAT_SECS: u64 = 60;

static PROGRESS: AtomicBool = AtomicBool::new(false);

/// Progress lines go to stderr when the tool runs as a command: stdout ends
/// with the JSON result.
pub fn enable_progress() {
    PROGRESS.store(true, Ordering::SeqCst);
}

pub fn progress(line: impl AsRef<str>) {
    if PROGRESS.load(Ordering::SeqCst) {
        eprintln!("llm_understand_media: {}", line.as_ref());
    }
}

/// `2m23s` / `45s`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

fn step_doing(step: &str) -> &'static str {
    match step {
        "to_pdf" => "converting to PDF",
        "extract_pdf_text" => "extracting the text",
        "render_pdf_pages" => "rendering page images",
        "ocr_pages" => "recognizing text in page images (OCR)",
        "build_catalog" => "building the catalog",
        "extract_frames" => "sampling frames",
        "extract_audio" => "extracting the audio track",
        "fit_image" => "fitting the image",
        _ => "working",
    }
}

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
            long_doc_pages: env_positive("LLM_UNDERSTAND_MEDIA_LONG_DOC_PAGES").unwrap_or(20),
            direct_max_bytes: env_positive("LLM_UNDERSTAND_MEDIA_DOCUMENT_MAX_BYTES")
                .unwrap_or(20 * 1024 * 1024),
        }
    }

    /// `ocr_lang`: the language the first window chose, so that every window of
    /// a document is read the same way (`auto` lets the window choose).
    pub fn pipeline_input(
        &self,
        obj_id: &str,
        format: Option<&str>,
        page_range: &[u32],
        max_pages: u32,
        ocr_lang: &str,
    ) -> Value {
        let mut input = json!({
            "obj_id": obj_id,
            "dpi": self.dpi,
            "max_long_edge": self.max_long_edge,
            "page_range": page_range,
            "limits": {"max_pages": max_pages},
            "ocr_lang": ocr_lang,
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
    let content_id = ObjId::new(&content).map_err(|e| format!("content id: {e}"))?;
    let source = buckyos_api::RuntimeContentByteSource::new(&store, content_id, total);
    file.apply_media_meta(&ndn_lib::probe_media_meta(&source, name, Some(mime)).await);
    file.content = content;
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
        .get("mime")
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
    /// Progress of a document window: what it is called, and whether it is the
    /// first window (whose probe tells the page count).
    pub watch: Option<WindowWatch>,
}

impl PipelineRequest {
    pub fn new(workflow_id: &'static str, obj_id: &str, input: Value) -> Self {
        Self {
            workflow_id,
            obj_id: obj_id.to_string(),
            input,
            session_id: None,
            watch: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WindowWatch {
    pub label: String,
    pub first: bool,
    /// Pages a document can bring into its catalog.
    pub max_pages: u32,
}

/// What a waiting call says while a run works: the size of the conversion
/// once its probe knows, then a line a minute naming the step at work.
struct Watcher {
    watch: Option<WindowWatch>,
    started: Instant,
    last_look: Option<Instant>,
    last_line: Instant,
}

static ANNOUNCED: AtomicBool = AtomicBool::new(false);

impl Watcher {
    fn new(watch: Option<WindowWatch>) -> Self {
        Self { watch, started: Instant::now(), last_look: None, last_line: Instant::now() }
    }

    async fn look(&mut self, client: &WorkflowServiceClient, run_id: &str) {
        if !PROGRESS.load(Ordering::SeqCst) {
            return;
        }
        if self.last_look.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
            return;
        }
        self.last_look = Some(Instant::now());
        let Ok(graph) = client.get_run_graph(run_id).await else {
            return;
        };
        let state_of = |node: &str| {
            graph
                .node_states
                .get(node)
                .and_then(|s| serde_json::to_value(s).ok())
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        };
        let running: Option<&str> = HEAVY_STEPS.iter().copied().find(|s| state_of(s) == "running");
        if let (Some(watch), Some(_)) = (&self.watch, running) {
            let pages = graph.node_outputs.get("probe").and_then(|p| p.get("pages")).and_then(Value::as_u64);
            if let (true, Some(pages)) = (watch.first, pages) {
                if !ANNOUNCED.swap(true, Ordering::SeqCst) {
                    let pages = pages.min(u64::from(watch.max_pages));
                    let runs = pages.div_ceil(u64::from(CATALOG_WINDOW_PAGES)).max(1);
                    progress(format!(
                        "converting {pages} pages in {runs} run{} (first time for this file; later calls reuse the result)",
                        if runs == 1 { "" } else { "s" }
                    ));
                    self.last_line = Instant::now();
                }
            }
        }
        if let Some(step) = running {
            if self.last_line.elapsed() >= Duration::from_secs(HEARTBEAT_SECS) {
                let label = self.watch.as_ref().map(|w| format!("{}: ", w.label)).unwrap_or_default();
                progress(format!("{label}{} ({})", step_doing(step), format_elapsed(self.started.elapsed())));
                self.last_line = Instant::now();
            }
        }
    }
}

/// `pages 1-200 ready (2m23s)`, or `from cache` when no step had to run.
pub fn window_done_line(label: &str, trace: &PipelineTrace, elapsed: Duration) -> String {
    let executed = trace
        .steps
        .iter()
        .any(|(node, step)| HEAVY_STEPS.contains(&node.as_str()) && step.source.as_deref() != Some("cache"));
    if executed {
        format!("{label} ready ({})", format_elapsed(elapsed))
    } else {
        format!("{label} from cache")
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
    watcher: &mut Watcher,
) -> Result<Option<Result<WorkflowRunOutput, RunStatus>>, String> {
    let mut poll = Duration::from_millis(POLL_MIN_MS);
    loop {
        watcher.look(client, run_id).await;
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
    let mut watcher = Watcher::new(request.watch.clone());
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
        let waited = wait_terminal(&client, &current.run_id, deadline, &mut watcher).await?;
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
    /// Delivered pages whose text was cut to fit the budget.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_truncated: Vec<u64>,
    /// Delivered pages whose text is the OCR of the page image, not its text layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_ocr: Vec<u64>,
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
    /// The line the calling agent reads: what the model did not see, by reason,
    /// and how to read it (`catalog`: the document unpacked as a directory).
    pub fn uncovered_note(&self, catalog: Option<&std::path::Path>) -> Option<String> {
        if self.uncovered.is_empty() {
            return None;
        }
        let mut by_reason: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
        for item in &self.uncovered {
            by_reason.entry(item.reason.as_str()).or_default().push(item.location);
        }
        let parts = by_reason
            .iter()
            .map(|(reason, places)| format!("{reason}: {}", short_ranges(places, 120)))
            .collect::<Vec<_>>()
            .join("; ");
        if self.family == "video" {
            return Some(format!(
                "Note: {} of {} frames (by ms) were not shown to the model ({parts}).",
                self.uncovered.len(),
                self.requested.len()
            ));
        }
        let of = self.total.unwrap_or(self.requested.len() as u64);
        let first: Vec<u64> = self.uncovered.iter().map(|u| u.location).take(1).collect();
        let example_range = first_range(&self.uncovered.iter().map(|u| u.location).collect::<Vec<_>>());
        let page = first.first().copied().unwrap_or(1);
        let hint = match catalog {
            Some(dir) => {
                let dir = dir.display();
                format!(
                    " Every page is unpacked in {dir} (index.tsv lists them): `grep -il <word> {dir}/pages/*.txt` finds pages, \
                     `llm_understand_media {dir} --pages {example_range} --goal \"…\"` reads several, \
                     `llm_understand_media {dir}/pages/{} --goal \"…\"` one page with its text. Nothing is converted again.",
                    page_file_name(page, of, "png")
                )
            }
            None => format!(" Add `--pages {example_range}` to read pages; nothing is converted again."),
        };
        Some(format!(
            "Note: {} of {of} pages were not shown to the model ({parts}).{hint}",
            self.uncovered.len()
        ))
    }
}

/// `0101.png`: page numbers padded to the width of the page count (at least 4).
pub fn page_file_name(page: u64, total: u64, ext: &str) -> String {
    let width = total.to_string().len().max(4);
    format!("{page:0width$}.{ext}")
}

/// The first run of consecutive places, at most 32 long: `2-33`.
fn first_range(places: &[u64]) -> String {
    let mut sorted = places.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let Some(&start) = sorted.first() else {
        return "1".to_string();
    };
    let mut end = start;
    for p in sorted.iter().skip(1) {
        if *p != end + 1 || end - start >= 31 {
            break;
        }
        end = *p;
    }
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

/// [`format_ranges`] within about `max` characters: the first ranges, then
/// the count of the rest and the last place.
pub fn short_ranges(places: &[u64], max: usize) -> String {
    let full = format_ranges(places);
    if full.len() <= max {
        return full;
    }
    let mut shown = String::new();
    let mut count = 0usize;
    for part in full.split(", ") {
        if shown.len() + part.len() > max * 2 / 3 {
            break;
        }
        if !shown.is_empty() {
            shown.push_str(", ");
        }
        shown.push_str(part);
        count += 1;
    }
    let rest = full.split(", ").count() - count;
    let last = places.iter().max().copied().unwrap_or(0);
    format!("{} places: {shown}, … {rest} more ranges up to {last}", places.len())
}

/// `[1, 2, 3, 7]` -> `1-3, 7`.
pub fn format_ranges(places: &[u64]) -> String {
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

/// Which text of a page the catalog uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    Layer,
    Ocr,
}

impl TextSource {
    pub fn label(self) -> &'static str {
        match self {
            TextSource::Layer => "layer",
            TextSource::Ocr => "ocr",
        }
    }
}

/// The text a page is read by, whitespace normalized.
#[derive(Clone, Debug, PartialEq)]
pub struct PageText {
    pub text: String,
    pub source: TextSource,
}

/// Version of the rule that picks and normalizes page text: a catalog written
/// under another rule rewrites its texts.
pub const TEXT_RULE_VERSION: u32 = 2;
/// Below this agreement with its OCR (percent), a text layer is not what the
/// page shows: native layers agree 77% or more, dk10000.pdf's garbled one at
/// most 65%.
const OCR_OVER_LAYER_AGREEMENT: u8 = 70;
/// OCR this sure of itself replaces a disagreeing layer even when it has
/// fewer clean words (a page of numbers).
const OCR_CONFIDENT: u8 = 60;

/// The text of a page to read: the text layer, unless it disagrees with what
/// OCR reads on the page image and the OCR is cleaner or confident (doc §4.6.5).
pub fn choose_text(entry: &MediaEntry) -> Option<(String, TextSource)> {
    let quality = entry.text_quality.as_ref();
    match (&entry.text_obj_id, &entry.ocr_text_obj_id) {
        (Some(layer), Some(ocr)) => {
            let use_ocr = quality.is_some_and(|q| {
                q.agreement.is_some_and(|a| a < OCR_OVER_LAYER_AGREEMENT)
                    && (q.ocr_clean.unwrap_or(0) >= q.layer_clean.unwrap_or(0)
                        || q.ocr_confidence.unwrap_or(0) >= OCR_CONFIDENT)
            });
            Some(if use_ocr { (ocr.clone(), TextSource::Ocr) } else { (layer.clone(), TextSource::Layer) })
        }
        (Some(layer), None) => Some((layer.clone(), TextSource::Layer)),
        (None, Some(ocr)) => Some((ocr.clone(), TextSource::Ocr)),
        (None, None) => None,
    }
}

/// Layout whitespace out: lines trimmed, runs of blank lines kept as one, and
/// runs of whitespace inside a line as two spaces (a hint of columns).
pub fn normalize_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() / 2);
    let mut blank = false;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank {
                out.push('\n');
            }
        }
        blank = false;
        let mut spaces = 0usize;
        for c in line.chars() {
            if c.is_whitespace() {
                spaces += 1;
                continue;
            }
            match spaces {
                0 => {}
                1 => out.push(' '),
                _ => out.push_str("  "),
            }
            spaces = 0;
            out.push(c);
        }
    }
    out
}

pub struct PagePlan {
    pub page: u64,
    pub text: Option<PageText>,
    pub image_obj_id: Option<String>,
}

pub struct ContentPlan {
    /// Every page of the document, when the model will not see them all.
    pub index: Option<String>,
    pub texts: Vec<(u64, String)>,
    pub images: Vec<(u64, String)>,
    pub coverage: Coverage,
}

/// What the caller asks of a catalog.
pub struct PlanRequest<'a> {
    /// Search terms: the goal's, plus those translated into the document's language.
    pub terms: &'a [String],
    /// Pages the goal or `--pages` names.
    pub named: &'a BTreeSet<u64>,
    pub text_char_budget: usize,
    pub max_images: usize,
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
pub fn goal_terms(goal: &str) -> Vec<String> {
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

/// Lower case without whitespace: how terms and texts are compared.
fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// Score of every page for `terms`: the inverse document frequency of each
/// term it contains. A term on more than half of the pages says nothing.
pub fn page_scores(terms: &[String], texts: &BTreeMap<u64, PageText>) -> BTreeMap<u64, f64> {
    let terms: Vec<String> = terms.iter().map(|t| compact(t)).filter(|t| !t.is_empty()).collect();
    let pages: Vec<(u64, String)> = texts.iter().map(|(p, t)| (*p, compact(&t.text))).collect();
    let n = pages.len().max(1) as f64;
    let mut scores: BTreeMap<u64, f64> = BTreeMap::new();
    for term in &terms {
        let hits: Vec<u64> = pages.iter().filter(|(_, t)| t.contains(term.as_str())).map(|(p, _)| *p).collect();
        if hits.is_empty() || hits.len() as f64 > n / 2.0 && pages.len() > 2 {
            continue;
        }
        let weight = ((n + 1.0) / (hits.len() as f64 + 1.0)).ln().max(0.05);
        for page in hits {
            *scores.entry(page).or_default() += weight;
        }
    }
    scores
}

fn leading_number(text: &str) -> Option<(u64, &str)> {
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    if digits == 0 {
        return None;
    }
    Some((text[..digits].parse().ok()?, &text[digits..]))
}

/// `14, 50 和 70`, `3-5`, `3 to 5`: the numbers and ranges right after a page
/// marker, and what follows them.
fn page_list(text: &str) -> (Vec<(u64, u64)>, &str) {
    let mut items = Vec::new();
    let mut rest = text.trim_start();
    loop {
        let Some((first, after)) = leading_number(rest) else {
            break;
        };
        let trimmed = after.trim_start();
        let range = ["-", "–", "~", "到", "至", "to ", "through "]
            .iter()
            .find_map(|sep| trimmed.strip_prefix(sep))
            .and_then(|r| leading_number(r.trim_start()));
        let (last, after) = range.unwrap_or((first, after));
        items.push((first, last.max(first).min(first.saturating_add(u64::from(CATALOG_WINDOW_PAGES)))));
        rest = after;
        let trimmed = rest.trim_start();
        let next = [",", "，", "、", "和", "与", "及", "/", "&", "and ", "or "]
            .iter()
            .find_map(|sep| trimmed.strip_prefix(sep))
            .map(str::trim_start)
            .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()));
        match next {
            Some(more) => rest = more,
            None => break,
        }
    }
    (items, rest)
}

/// Pages the goal names: `page 12`, `pages 3-5`, `page 14, 50, 70`, `p14`,
/// `p. 12`, `pp. 3-5`, `第12页`, `第14、50、70页`, `第3到5页`, `物理页14`.
pub fn goal_pages(goal: &str) -> BTreeSet<u64> {
    let lower = goal.to_lowercase();
    let mut pages = BTreeSet::new();
    let mut add = |items: &[(u64, u64)]| {
        for (first, last) in items {
            pages.extend((*first..=*last).filter(|p| *p > 0));
        }
    };
    let chars: Vec<(usize, char)> = lower.char_indices().collect();
    for (i, (at, c)) in chars.iter().enumerate() {
        let before_ok = i == 0 || !chars[i - 1].1.is_ascii_alphanumeric();
        let rest = &lower[*at..];
        if *c == '第' {
            let (items, after) = page_list(&rest['第'.len_utf8()..]);
            if after.trim_start().starts_with('页') {
                add(&items);
            }
            continue;
        }
        if *c == '页' {
            let (items, _) = page_list(&rest['页'.len_utf8()..]);
            add(&items);
            continue;
        }
        if !before_ok || *c != 'p' {
            continue;
        }
        for marker in ["pages", "page", "pp.", "pp", "p.", "p"] {
            if let Some(after) = rest.strip_prefix(marker) {
                // `p` / `pp` only right before the number (`p14`, `pp 3-5`).
                if matches!(marker, "p" | "pp") && !after.trim_start_matches(' ').starts_with(|c: char| c.is_ascii_digit()) {
                    continue;
                }
                if marker == "p" && after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                    continue;
                }
                let (items, _) = page_list(after);
                add(&items);
                break;
            }
        }
    }
    pages
}

/// `--pages 1,5,10-20`.
pub fn parse_pages_arg(raw: &str) -> Result<BTreeSet<u64>, String> {
    let (items, rest) = page_list(raw);
    if items.is_empty() || !rest.trim().is_empty() {
        return Err(format!("--pages `{raw}` is not a list of pages such as 1,5,10-20"));
    }
    let mut pages = BTreeSet::new();
    for (first, last) in items {
        if first == 0 {
            return Err("pages are numbered from 1".to_string());
        }
        pages.extend(first..=last);
    }
    Ok(pages)
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

fn kind_of(plan: &PagePlan) -> &'static str {
    match (plan.text.as_ref().map(|t| t.source), plan.image_obj_id.is_some()) {
        (Some(TextSource::Layer), true) => "text+image",
        (Some(TextSource::Ocr), true) => "ocr+image",
        (Some(TextSource::Layer), false) => "text",
        (Some(TextSource::Ocr), false) => "ocr",
        (None, true) => "image",
        (None, false) => "nothing",
    }
}

fn first_line(plan: &PagePlan, chars: usize) -> String {
    plan.text
        .as_ref()
        .and_then(|t| t.text.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("")
        .chars()
        .take(chars)
        .collect()
}

/// The list of every page: one line each with the start of its text when that
/// fits `budget`; otherwise lines for pages spread over the document and the
/// other pages as ranges by kind.
fn render_index(pages: &BTreeMap<u64, PagePlan>, budget: usize) -> String {
    let mut out = String::from("Index of the whole document (what the pipeline has for each page):\n");
    let snippet_chars = (budget / pages.len().max(1)).saturating_sub(16).min(60);
    if snippet_chars >= 16 {
        for plan in pages.values() {
            out.push_str(&format!("{} [{}] {}\n", plan.page, kind_of(plan), first_line(plan, snippet_chars)));
        }
        return out;
    }
    // About 60 characters a listed line; a tenth of the budget for the ranges.
    let listed_count = (budget * 9 / 10 / 60).max(1);
    let all: Vec<u64> = pages.keys().copied().collect();
    let listed: BTreeSet<u64> = spread(&all, listed_count).into_iter().take(listed_count).collect();
    for page in &listed {
        let plan = &pages[page];
        out.push_str(&format!("{} [{}] {}\n", plan.page, kind_of(plan), first_line(plan, 40)));
    }
    let mut by_kind: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for plan in pages.values().filter(|p| !listed.contains(&p.page)) {
        by_kind.entry(kind_of(plan)).or_default().push(plan.page);
    }
    if !by_kind.is_empty() {
        out.push_str("Other pages:\n");
        for (name, places) in by_kind {
            out.push_str(&format!("{name}: {}\n", format_ranges(&places)));
        }
    }
    out
}

/// `text` cut to about `cap` characters at a line end, with a marker.
fn truncate_page(page: u64, text: &str, cap: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= cap {
        return (text.to_string(), false);
    }
    let cut: String = text.chars().take(cap).collect();
    let cut = match cut.rfind('\n') {
        Some(at) if at >= cut.len() * 4 / 5 => cut[..at].to_string(),
        _ => cut,
    };
    let kept = cut.chars().count();
    (format!("{cut}\n[page {page} text truncated: {kept} of {total} chars]"), true)
}

/// The most pages of `candidates`, spread over them, whose texts cut to
/// `max(budget / n, PAGE_TEXT_FLOOR)` fit `budget`; with that cap.
fn spread_fitting(candidates: &[u64], len: &dyn Fn(u64) -> usize, budget: usize) -> (Vec<u64>, usize) {
    let cap_of = |n: usize| (budget / n.max(1)).max(PAGE_TEXT_FLOOR);
    let fits = |n: usize| {
        n == 0 || spread(candidates, n).iter().take(n).map(|p| len(*p).min(cap_of(n))).sum::<usize>() <= budget
    };
    let (mut lo, mut hi) = (0usize, candidates.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let picked: Vec<u64> = spread(candidates, lo).into_iter().take(lo).collect();
    (picked, cap_of(lo))
}

/// Chooses what the model sees from a page manifest (doc §4.7).
///
/// Text: every page while they fit `text_char_budget`. Otherwise the named
/// pages first (each up to an even share of the budget), then pages that match
/// the terms (up to half of the budget, each up to an eighth of it), and the
/// rest of the budget over the other pages, spread evenly, each cut to the
/// same cap so that the spread pages all fit.
/// Images: named pages; up to half the budget for matching pages (their
/// figures are what text search cannot show); pages that only have an image;
/// then spread over the pages whose text was chosen, then over all pages.
pub fn plan_pages(
    request: &PlanRequest,
    manifest: &MediaManifest,
    texts: &BTreeMap<u64, PageText>,
    unreadable: &BTreeMap<u64, String>,
) -> ContentPlan {
    let budget = request.text_char_budget;
    let max_images = request.max_images;
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
        let plan = pages.entry(coord).or_insert(PagePlan { page: coord, text: None, image_obj_id: None });
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
        if let Some(text) = texts.get(&coord).filter(|t| !t.text.trim().is_empty()) {
            plan.text = Some(text.clone());
        }
        if let Some(image) = entry.image_obj_id.as_ref() {
            plan.image_obj_id = Some(image.clone());
        }
    }

    // Text selection: page -> cap.
    let text_pages: Vec<u64> = pages.values().filter(|p| p.text.is_some()).map(|p| p.page).collect();
    let text_len = |page: u64| pages[&page].text.as_ref().map(|t| t.text.chars().count()).unwrap_or(0);
    let total_chars: usize = text_pages.iter().map(|p| text_len(*p)).sum();
    let named: Vec<u64> = request.named.iter().copied().filter(|p| pages.contains_key(p)).collect();
    let page_texts: BTreeMap<u64, PageText> =
        pages.values().filter_map(|p| Some((p.page, p.text.clone()?))).collect();
    let scores = page_scores(request.terms, &page_texts);
    let mut matching: Vec<u64> = scores.keys().copied().filter(|p| !request.named.contains(p)).collect();
    matching.sort_by(|a, b| scores[b].total_cmp(&scores[a]).then(a.cmp(b)));
    let mut selected_text: BTreeMap<u64, usize> = BTreeMap::new();
    if !named.is_empty() {
        // Named pages are what the caller wants to see: those only.
        let named_text: Vec<u64> = named.iter().copied().filter(|p| text_len(*p) > 0).collect();
        let named_cap = (budget / named_text.len().max(1)).max(PAGE_TEXT_FLOOR);
        let mut used = 0usize;
        for page in &named_text {
            let take = text_len(*page).min(named_cap).min(budget.saturating_sub(used));
            if take < 200.min(text_len(*page)) {
                break;
            }
            used += take;
            selected_text.insert(*page, take);
        }
    } else if total_chars <= budget {
        for page in &text_pages {
            selected_text.insert(*page, usize::MAX);
        }
    } else {
        let mut used = 0usize;
        let others_left = text_pages.iter().any(|p| !selected_text.contains_key(p) && !matching.contains(p));
        let match_limit = if others_left { used + budget.saturating_sub(used) / 2 } else { budget };
        let match_cap = (budget / 8).max(PAGE_TEXT_FLOOR);
        let mut matched = false;
        for page in &matching {
            let take = text_len(*page).min(match_cap);
            let limit = if matched { match_limit } else { budget };
            if take == 0 || used + take > limit {
                continue;
            }
            used += take;
            matched = true;
            selected_text.insert(*page, take);
        }
        let rest: Vec<u64> = text_pages.iter().copied().filter(|p| !selected_text.contains_key(p)).collect();
        let (spread_pages, cap) = spread_fitting(&rest, &text_len, budget.saturating_sub(used));
        for page in spread_pages {
            selected_text.insert(page, cap);
        }
    }

    // Image selection.
    let has_image = |page: &u64| pages.get(page).is_some_and(|p| p.image_obj_id.is_some());
    let mut selected_images: Vec<u64> = Vec::new();
    let take = |page: u64, limit: usize, chosen: &mut Vec<u64>| {
        if chosen.len() < limit && !chosen.contains(&page) {
            chosen.push(page);
        }
    };
    let even_over = |pool: Vec<u64>, chosen: &mut Vec<u64>| {
        let pool: Vec<u64> = pool.into_iter().filter(|p| !chosen.contains(p)).collect();
        let room = max_images.saturating_sub(chosen.len());
        for page in spread(&pool, room).into_iter().take(room) {
            take(page, max_images, chosen);
        }
    };
    let named_images: Vec<u64> = named.iter().copied().filter(|p| has_image(p)).collect();
    even_over(named_images, &mut selected_images);
    let open = named.is_empty();
    for page in matching.iter().filter(|_| open).filter(|p| selected_text.contains_key(*p) && has_image(p)) {
        take(*page, (max_images / 2).max(selected_images.len()), &mut selected_images);
    }
    let image_only: Vec<u64> = pages
        .values()
        .filter(|p| p.text.is_none() && p.image_obj_id.is_some())
        .map(|p| p.page)
        .collect();
    if open {
        even_over(image_only, &mut selected_images);
        even_over(selected_text.keys().copied().filter(|p| has_image(p)).collect(), &mut selected_images);
        even_over(pages.values().filter(|p| p.image_obj_id.is_some()).map(|p| p.page).collect(), &mut selected_images);
    }
    selected_images.sort_unstable();

    let mut texts_out: Vec<(u64, String)> = Vec::new();
    for (page, cap) in &selected_text {
        let text = pages[page].text.as_ref().expect("selected page has text");
        let (shown, cut) = truncate_page(*page, &text.text, *cap);
        if cut {
            coverage.text_truncated.push(*page);
        }
        if text.source == TextSource::Ocr {
            coverage.text_ocr.push(*page);
        }
        texts_out.push((*page, shown));
    }
    let images_out: Vec<(u64, String)> = selected_images
        .iter()
        .map(|p| (*p, pages[p].image_obj_id.clone().unwrap_or_default()))
        .collect();
    coverage.text_delivered = selected_text.keys().copied().collect();
    coverage.image_delivered = selected_images.clone();
    for (coord, reason) in unreadable {
        coverage.uncovered.push(Uncovered {
            location: *coord,
            reason: format!("text unreadable: {reason}"),
        });
    }
    for (coord, plan) in &pages {
        let seen = selected_text.contains_key(coord) || selected_images.contains(coord);
        if seen {
            continue;
        }
        let reason = failed_reasons.get(coord).cloned().unwrap_or_else(|| {
            if !named.is_empty() {
                "not among the pages asked for".to_string()
            } else if plan.text.is_some() || plan.image_obj_id.is_some() {
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
    use buckyos_api::{MediaEntryError, MediaSourceRef, MediaTextQuality, MediaWhere};

    fn entry(page: u32, status: MediaEntryStatus, text: bool, image: bool) -> MediaEntry {
        MediaEntry {
            kind: MediaEntryKind::Page,
            location: MediaWhere { source: MediaSourceRef::Original, page: Some(page), time_ms: None, index: None },
            text_obj_id: text.then(|| format!("cyfile:t{page}")),
            image_obj_id: image.then(|| format!("cyfile:i{page}")),
            audio_obj_id: None,
            ocr_text_obj_id: None,
            text_quality: None,
            status,
            error: (status == MediaEntryStatus::Failed).then(|| MediaEntryError { code: "tool_failed".into(), message: "render".into(), retryable: true }),
        }
    }

    fn layer(texts: BTreeMap<u64, String>) -> BTreeMap<u64, PageText> {
        texts.into_iter().map(|(p, text)| (p, PageText { text, source: TextSource::Layer })).collect()
    }

    fn plan_for(goal: &str, extra: &[u64], manifest: &MediaManifest, texts: &BTreeMap<u64, PageText>, unreadable: &BTreeMap<u64, String>, budget: usize, images: usize) -> ContentPlan {
        let terms = goal_terms(goal);
        let mut named = goal_pages(goal);
        named.extend(extra.iter().copied());
        plan_pages(&PlanRequest { terms: &terms, named: &named, text_char_budget: budget, max_images: images }, manifest, texts, unreadable)
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
        let texts = layer(BTreeMap::from([
            (1u64, "introduction about cats".to_string()),
            (2u64, "the answer is on the last page".to_string()),
            (5u64, "final answer: forty-two".to_string()),
        ]));
        let none = BTreeMap::new();
        let plan = plan_for("what is the final answer", &[], &manifest, &texts, &none, 25, 2);
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

        let all = plan_for("anything", &[], &manifest, &texts, &none, 10_000, 16);
        assert_eq!(all.texts.len(), 3);
        assert_eq!(all.images.len(), 4);
        assert_eq!(all.coverage.uncovered.len(), 1);
    }

    fn long_document(pages: u32) -> (MediaManifest, BTreeMap<u64, PageText>) {
        let mut manifest = MediaManifest::new("pdf", "sha256:x");
        manifest.range.total = Some(u64::from(pages));
        manifest.range.requested = (1..=u64::from(pages)).collect();
        manifest.entries = (1..=pages).map(|p| entry(p, MediaEntryStatus::Ok, true, true)).collect();
        manifest.recompute_coverage();
        let texts = layer((1..=u64::from(pages)).map(|p| (p, format!("ordinary filler {p:04}"))).collect());
        (manifest, texts)
    }

    #[test]
    fn a_chinese_goal_finds_the_page_that_answers_it() {
        let (manifest, mut texts) = long_document(120);
        texts.insert(97, PageText { text: "第三季度 营收 为 四十二 亿元".to_string(), source: TextSource::Layer });
        let plan = plan_for("第三季度的营收是多少", &[], &manifest, &texts, &BTreeMap::new(), 20, 4);
        assert_eq!(plan.coverage.text_delivered, vec![97]);
        assert!(plan.images.iter().any(|(p, _)| *p == 97));
    }

    #[test]
    fn a_named_page_is_read_and_a_goal_without_matches_spreads_over_the_document() {
        let (manifest, texts) = long_document(100);
        let plan = plan_for("translate 第88页", &[], &manifest, &texts, &BTreeMap::new(), 100, 2);
        assert_eq!(plan.coverage.text_delivered, vec![88], "named pages only");
        assert_eq!(plan.coverage.image_delivered, vec![88]);
        assert!(plan.index.is_some(), "the rest of the document as an index");
        let plan2 = plan_for("第14、50、70页讲什么", &[], &manifest, &texts, &BTreeMap::new(), 60_000, 16);
        assert_eq!(plan2.coverage.text_delivered, vec![14, 50, 70]);
        assert_eq!(plan2.coverage.image_delivered, vec![14, 50, 70]);
        let many = plan_for("translate this", &(1..=40).collect::<Vec<_>>(), &manifest, &texts, &BTreeMap::new(), 60_000, 4);
        assert_eq!(many.coverage.text_delivered.len(), 40);
        assert_eq!(many.coverage.image_delivered, vec![1, 11, 21, 31], "images spread over the named pages");

        let plan = plan_for("zzz", &[], &manifest, &texts, &BTreeMap::new(), 200, 0);
        let delivered = &plan.coverage.text_delivered;
        assert!(delivered.len() >= 5);
        assert!(delivered.iter().any(|p| *p > 80), "{delivered:?}");
        assert!(delivered.iter().any(|p| *p < 20), "{delivered:?}");
    }

    /// dk10000.pdf: 402 text pages of about a thousand characters, one of 48k.
    #[test]
    fn a_huge_page_is_cut_and_the_chosen_pages_spread_over_the_book() {
        let mut manifest = MediaManifest::new("pdf", "sha256:x");
        manifest.range.total = Some(403);
        manifest.range.requested = (1..=403).collect();
        manifest.entries = (1..=403).map(|p| entry(p, MediaEntryStatus::Ok, p != 383, true)).collect();
        manifest.recompute_coverage();
        let mut texts: BTreeMap<u64, PageText> = (1..=403u64)
            .filter(|p| *p != 383)
            .map(|p| (p, PageText { text: format!("{p:04} {}", "word ".repeat(200)), source: TextSource::Ocr }))
            .collect();
        texts.insert(101, PageText { text: format!("0101 {}", "x".repeat(48_000)), source: TextSource::Ocr });
        let plan = plan_for("总结一下这本书", &[], &manifest, &texts, &BTreeMap::new(), 60_000, 16);
        let pages = &plan.coverage.text_delivered;
        assert!(pages.len() >= 50, "{} pages of text", pages.len());
        let used: usize = plan.texts.iter().map(|(_, t)| t.chars().count()).sum();
        assert!(used <= 60_000 + 200, "{used}");
        let average = 403.0 / pages.len() as f64;
        for pair in pages.windows(2) {
            assert!(((pair[1] - pair[0]) as f64) <= 2.0 * average, "gap {pair:?} > 2 x {average}");
        }
        assert!(pages[0] <= 2 * average as u64 && *pages.last().unwrap() >= 403 - 2 * average as u64);
        if pages.contains(&101) {
            assert_eq!(plan.coverage.text_truncated, vec![101]);
            assert!(plan.texts.iter().any(|(p, t)| *p == 101 && t.contains("[page 101 text truncated:")));
        }
        assert_eq!(plan.coverage.text_ocr.len(), pages.len());
        // M4: the image budget is used, over the whole book.
        let images = &plan.coverage.image_delivered;
        assert_eq!(images.len(), 16);
        assert!(images.contains(&383), "the page with only an image");
        assert!(images.first().unwrap() < &40 && images.last().unwrap() > &360, "{images:?}");
        // M5: the index lists pages over the whole book, the rest by range.
        let index = plan.index.unwrap();
        assert!(index.lines().filter(|l| l.contains("[ocr+image]")).count() >= 50, "{index}");
        assert!(index.contains("Other pages:") && index.contains("ocr+image: "), "{index}");
        assert!(index.contains("\n383 [image]") || index.contains("image: ") , "{index}");
        assert!(index.len() <= INDEX_CHARS + 600, "{}", index.len());
    }

    #[test]
    fn unread_text_is_reported_and_the_note_names_ranges() {
        let (manifest, texts) = long_document(30);
        let unreadable = BTreeMap::from([(7u64, "gone".to_string())]);
        let plan = plan_for("zzz", &[], &manifest, &texts, &unreadable, 60, 0);
        assert!(plan.coverage.uncovered.iter().any(|u| u.location == 7 && u.reason.contains("gone")));
        let note = plan.coverage.uncovered_note(None).unwrap();
        assert!(note.contains("were not shown to the model"), "{note}");
        assert!(note.contains("omitted by the budget"), "{note}");
        assert!(note.contains("--pages "), "{note}");
        let dir = std::path::Path::new("/c/cyfile-ab");
        let note = plan.coverage.uncovered_note(Some(dir)).unwrap();
        assert!(note.contains("llm_understand_media /c/cyfile-ab --pages "), "{note}");
        assert!(note.contains("/c/cyfile-ab/pages/0002.png") || note.contains("/c/cyfile-ab/pages/0001.png"), "{note}");
        assert_eq!(format_ranges(&[7, 1, 2, 3, 9, 10]), "1-3, 7, 9-10");
        let gappy: Vec<u64> = (1..=403).filter(|p| p % 7 != 0).collect();
        let short = short_ranges(&gappy, 120);
        assert!(short.len() < 140 && short.starts_with("346 places: 1-6, 8-13") && short.ends_with("up to 403"), "{short}");
        assert_eq!(short_ranges(&[1, 2, 3], 120), "1-3");
        assert_eq!(first_range(&[2, 3, 4, 9]), "2-4");
        assert_eq!(page_file_name(7, 403, "png"), "0007.png");
        assert_eq!(page_file_name(7, 12000, "txt"), "00007.txt");
    }

    #[test]
    fn goal_terms_and_pages() {
        assert!(goal_terms("Q3 revenue").contains(&"q3".to_string()));
        assert!(goal_terms("Q3 revenue").contains(&"revenue".to_string()));
        assert!(!goal_terms("is a 3").contains(&"3".to_string()));
        assert_eq!(goal_terms("营收多少"), vec!["多少", "收多", "营收"]);
        let set = |v: &[u64]| v.iter().copied().collect::<BTreeSet<u64>>();
        assert_eq!(goal_pages("see page 12 and pages 3-5"), set(&[3, 4, 5, 12]));
        assert_eq!(goal_pages("第12页和第3到5页"), set(&[3, 4, 5, 12]));
        assert_eq!(goal_pages("第三季度 第2季度"), set(&[]));
        assert_eq!(goal_pages("page 14, 50, 70"), set(&[14, 50, 70]));
        assert_eq!(goal_pages("第14、50、70页讲什么"), set(&[14, 50, 70]));
        assert_eq!(goal_pages("看一下PDF物理页14、50、70、104、148、162"), set(&[14, 50, 70, 104, 148, 162]));
        assert_eq!(goal_pages("compare p14 with pp. 3-5"), set(&[3, 4, 5, 14]));
        assert_eq!(goal_pages("p. 12 and page 7 and 9"), set(&[7, 9, 12]));
        assert_eq!(goal_pages("pages 3 to 5"), set(&[3, 4, 5]));
        assert_eq!(goal_pages("the top 10 photos, step 3, php 5"), set(&[]));
        assert_eq!(parse_pages_arg("1,5,10-12").unwrap(), set(&[1, 5, 10, 11, 12]));
        assert_eq!(parse_pages_arg(" 7 ").unwrap(), set(&[7]));
        assert!(parse_pages_arg("1,x").is_err() && parse_pages_arg("0").is_err() && parse_pages_arg("").is_err());
    }

    #[test]
    fn page_text_is_normalized_and_the_layer_gives_way_to_better_ocr() {
        let raw = "   44.2      HEAD   \n\n\n\n        brain          sinus\t\t \n  \n   palate";
        assert_eq!(normalize_text(raw), "44.2  HEAD\n\nbrain  sinus\n\npalate");
        assert_eq!(normalize_text("a b\nc"), "a b\nc");
        let mut e = entry(1, MediaEntryStatus::Ok, true, true);
        assert_eq!(choose_text(&e), Some(("cyfile:t1".to_string(), TextSource::Layer)));
        e.ocr_text_obj_id = Some("cyfile:o1".into());
        e.text_quality = Some(MediaTextQuality { layer_clean: Some(48), ocr_clean: Some(80), agreement: Some(42), ocr_confidence: Some(69), ocr: None });
        assert_eq!(choose_text(&e).unwrap().1, TextSource::Ocr);
        e.text_quality = Some(MediaTextQuality { layer_clean: Some(55), ocr_clean: Some(57), agreement: Some(97), ocr_confidence: Some(90), ocr: None });
        assert_eq!(choose_text(&e).unwrap().1, TextSource::Layer, "a layer OCR agrees with stays");
        // dk10000.pdf p.356, cardinal numbers: OCR has digits, the layer garbage.
        e.text_quality = Some(MediaTextQuality { layer_clean: Some(51), ocr_clean: Some(39), agreement: Some(21), ocr_confidence: Some(85), ocr: None });
        assert_eq!(choose_text(&e).unwrap().1, TextSource::Ocr);
        // p.392, an index page: most words right, still garbled.
        e.text_quality = Some(MediaTextQuality { layer_clean: Some(59), ocr_clean: Some(67), agreement: Some(61), ocr_confidence: Some(91), ocr: None });
        assert_eq!(choose_text(&e).unwrap().1, TextSource::Ocr);
        e.text_quality = Some(MediaTextQuality { layer_clean: Some(60), ocr_clean: Some(40), agreement: Some(50), ocr_confidence: Some(30), ocr: None });
        assert_eq!(choose_text(&e).unwrap().1, TextSource::Layer, "unsure OCR with fewer clean words loses");
        e.text_obj_id = None;
        assert_eq!(choose_text(&e).unwrap().1, TextSource::Ocr);
        e.ocr_text_obj_id = None;
        assert_eq!(choose_text(&e), None);
    }

    #[test]
    fn idf_ignores_terms_on_most_pages() {
        let texts = layer(BTreeMap::from([
            (1u64, "this book is about cooking".to_string()),
            (2u64, "this page is about trains".to_string()),
            (3u64, "this page is about ships".to_string()),
            (4u64, "this is the index".to_string()),
        ]));
        let scores = page_scores(&["this".into(), "cooking verbs".into(), "cooking".into()], &texts);
        assert_eq!(scores.keys().copied().collect::<Vec<_>>(), vec![1]);
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
        let input = limits.pipeline_input("cyfile:a", Some("docx"), &[1], 1, "auto");
        assert_eq!(input["page_range"], json!([1]));
        assert_eq!(input["limits"]["max_pages"], 1);
        assert_eq!(input["format"], "docx");
        assert_eq!(input["ocr_lang"], "auto");
        assert_eq!(format_elapsed(Duration::from_secs(143)), "2m23s");
        assert_eq!(format_elapsed(Duration::from_secs(45)), "45s");
        let mut trace = PipelineTrace::default();
        trace.steps.insert("render_pdf_pages".into(), StepTrace { state: "completed".into(), source: Some("cache".into()), attempts: 1, error: None });
        assert_eq!(window_done_line("pages 1-200", &trace, Duration::from_secs(3)), "pages 1-200 from cache");
        trace.steps.insert("ocr_pages".into(), StepTrace { state: "completed".into(), source: Some("executed".into()), attempts: 1, error: None });
        assert_eq!(window_done_line("pages 1-200", &trace, Duration::from_secs(143)), "pages 1-200 ready (2m23s)");
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
