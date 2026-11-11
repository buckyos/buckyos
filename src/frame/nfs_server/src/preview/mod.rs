//! Preview Pipeline: NFSP `repr` (ensure) / `get_repr` (query) for content the
//! viewer's Runtime cannot show directly (Preview PRD §23.6–23.7; preview
//! iPhone media TODO D1–D10).
//!
//! - Converters: ffmpeg / ffprobe (media), LibreOffice `soffice` (documents →
//!   PDF, legacy spreadsheets → xlsx), ImageMagick (RAW and other stills),
//!   poppler (PDF thumbnails) and pandoc (markup → HTML), spawned in-process,
//!   each group optional (`tools`); `hello` advertises `repr` while any one
//!   is found and lists the profiles it can produce.
//! - Records: filedb `preview_works`, one row per `work_key = sha256(input
//!   ObjId, converter@version, params)`; `processing` → `completed` | `failed`,
//!   a new attempt only by explicit Retry (CAS on `expected_attempt_id`),
//!   after the 10-minute negative cache, or when a result's content is gone.
//!   Restart turns orphaned `processing` rows into retryable failures.
//! - Results: FileObjects bound in the system bucket
//!   `zone/nfs-server/_/preview-cache` (key = work_key), read through the
//!   bucket data plane (Range) with a short-lived, read-only, key-scoped
//!   `bkt_` token signed only after the source's own ACL passed.
//! - Limits: 1–2 ffmpeg / magick / pandoc / pdftoppm conversions at once (by
//!   CPU count), ffmpeg's timeout max(60 s, 10× duration); one soffice at a time in its own slot with a private
//!   profile per attempt, timeout max(120 s, 10 s/MiB); callers of one work_key
//!   share the running attempt, a caller leaving never stops it; results unread
//!   for 30 days expire and the cache keeps under `preview_cache_mib` by least
//!   recent use.

pub mod convert;
pub mod heif;
pub mod media;
pub mod rpc;
pub mod tools;

use crate::error::{internal, NfsResult};
use crate::filedb::FileDb;
use crate::state::{AppState, SharedState};
use bucket_store::{Action, BucketError, BucketStore, Caller, PutRequest, TokenRequest};
use convert::{ConvertError, Job, Output, Profile};
use media::Kind;
use ndn_lib::ObjId;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;

pub const FEATURE: &str = "repr";
pub const PIPELINE_ID: &str = "nfs.preview";
pub const PIPELINE_VERSION: u32 = 1;
pub const PROFILES: &[Profile] =
    &[Profile::PreviewImage, Profile::PreviewVideo, Profile::PreviewAudio, Profile::Thumbnail, Profile::PreviewDocument, Profile::PreviewSheet, Profile::PreviewHtml];
/// A deterministic "no thumbnail" is not asked again until the record expires.
const NO_THUMBNAIL_TTL_MS: i64 = IDLE_EXPIRY_MS;
pub const NEGATIVE_TTL_MS: i64 = 10 * 60 * 1000;
pub const IDLE_EXPIRY_MS: i64 = 30 * 24 * 3600 * 1000;
const TOKEN_TTL_MS: i64 = 6 * 3600 * 1000;
const AUTHORIZED_TTL: Duration = Duration::from_secs(3600);
const MAINTENANCE_EVERY: Duration = Duration::from_secs(3600);
const HEAD_BYTES: usize = 1024 * 1024;
const OLE_DIRECTORY_BYTES: usize = 64 * 1024;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS preview_works(
  work_key TEXT PRIMARY KEY,
  state TEXT NOT NULL,
  attempt_id TEXT NOT NULL,
  input_obj_id TEXT NOT NULL,
  converter TEXT NOT NULL,
  params_json TEXT NOT NULL,
  params_hash TEXT NOT NULL,
  result_json TEXT,
  error_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  accessed_at INTEGER NOT NULL,
  expires_at INTEGER
);
CREATE INDEX IF NOT EXISTS preview_works_accessed ON preview_works(state, accessed_at);
CREATE TABLE IF NOT EXISTS preview_inputs(
  source TEXT PRIMARY KEY,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  obj_id TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
"#;

pub fn now_ms() -> i64 {
    bucket_store::now_ms()
}

pub fn bucket_id() -> bucket_store::BucketId {
    buckyos_api::nfs_preview_cache_bucket()
}

/// Where a source's bytes are: a readable local file, or a named object.
#[derive(Debug, Clone)]
pub enum Input {
    Path(PathBuf),
    Object(ObjId),
}

#[derive(Debug, Clone)]
pub struct Record {
    pub work_key: String,
    pub state: String,
    pub attempt_id: String,
    pub input_obj_id: String,
    pub converter: String,
    pub params: Value,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub created_at: i64,
    pub updated_at: i64,
    pub accessed_at: i64,
    pub expires_at: Option<i64>,
}

fn row_of(r: &rusqlite::Row) -> rusqlite::Result<Record> {
    let json = |s: Option<String>| s.and_then(|s| serde_json::from_str(&s).ok());
    Ok(Record {
        work_key: r.get(0)?,
        state: r.get(1)?,
        attempt_id: r.get(2)?,
        input_obj_id: r.get(3)?,
        converter: r.get(4)?,
        params: json(r.get(5)?).unwrap_or(Value::Null),
        result: json(r.get(6)?),
        error: json(r.get(7)?),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        accessed_at: r.get(10)?,
        expires_at: r.get(11)?,
    })
}

const COLUMNS: &str = "work_key, state, attempt_id, input_obj_id, converter, params_json, result_json, error_json, created_at, updated_at, accessed_at, expires_at";

pub fn get_record(db: &FileDb, key: &str) -> NfsResult<Option<Record>> {
    let conn = db.lock();
    Ok(conn.query_row(&format!("SELECT {COLUMNS} FROM preview_works WHERE work_key=?1"), params![key], row_of).optional()?)
}

fn put_record(db: &FileDb, r: &Record) -> NfsResult<()> {
    let conn = db.lock();
    conn.execute(
        "INSERT INTO preview_works(work_key, state, attempt_id, input_obj_id, converter, params_json, params_hash, result_json, error_json, created_at, updated_at, accessed_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(work_key) DO UPDATE SET state=excluded.state, attempt_id=excluded.attempt_id, result_json=excluded.result_json,
           error_json=excluded.error_json, updated_at=excluded.updated_at, accessed_at=excluded.accessed_at, expires_at=excluded.expires_at",
        params![
            r.work_key,
            r.state,
            r.attempt_id,
            r.input_obj_id,
            r.converter,
            r.params.to_string(),
            hex_sha256(r.params.to_string().as_bytes()),
            r.result.as_ref().map(Value::to_string),
            r.error.as_ref().map(Value::to_string),
            r.created_at,
            r.updated_at,
            r.accessed_at,
            r.expires_at,
        ],
    )?;
    Ok(())
}

/// Finishes an attempt only if it is still the record's current one.
fn finish_record(db: &FileDb, key: &str, attempt: &str, state: &str, result: Option<&Value>, error: Option<&Value>, expires_at: Option<i64>) -> NfsResult<bool> {
    let conn = db.lock();
    let now = now_ms();
    let n = conn.execute(
        "UPDATE preview_works SET state=?3, result_json=?4, error_json=?5, updated_at=?6, accessed_at=?6, expires_at=?7 WHERE work_key=?1 AND attempt_id=?2",
        params![key, attempt, state, result.map(Value::to_string), error.map(Value::to_string), now, expires_at],
    )?;
    Ok(n == 1)
}

fn delete_record(db: &FileDb, key: &str) -> NfsResult<()> {
    db.lock().execute("DELETE FROM preview_works WHERE work_key=?1", params![key])?;
    Ok(())
}

fn touch_record(db: &FileDb, key: &str) {
    let _ = db.lock().execute("UPDATE preview_works SET accessed_at=?2 WHERE work_key=?1", params![key, now_ms()]);
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn converter_id(converter: &tools::Converter) -> String {
    let tool = match converter {
        tools::Converter::Ffmpeg(t) => format!("ffmpeg-{}", t.ffmpeg.version),
        tools::Converter::Soffice(t) => format!("soffice-{}", t.version),
        tools::Converter::Magick(t) => format!("magick-{}", t.version),
        tools::Converter::Pandoc(t) => format!("pandoc-{}", t.version),
        tools::Converter::Pdftoppm(t) => format!("pdftoppm-{}", t.pdftoppm.version),
        tools::Converter::Embedded(_) => "embedded".to_string(),
    };
    format!("{PIPELINE_ID}/{PIPELINE_VERSION}@{tool}")
}

/// `params` is canonical JSON (serde_json maps are sorted).
pub fn work_key(input_obj_id: &str, converter: &str, params: &Value) -> String {
    hex_sha256(json!({"input": input_obj_id, "converter": converter, "params": params}).to_string().as_bytes())
}

pub struct PreviewMgr {
    dirs: Vec<PathBuf>,
    probe: Mutex<Option<(tools::Identity, tools::Probe)>>,
    gate: tokio::sync::Mutex<()>,
    running: Mutex<HashSet<String>>,
    progress: Mutex<HashMap<String, f64>>,
    permits: Arc<tokio::sync::Semaphore>,
    /// Q11: LibreOffice is memory-heavy, one at a time, apart from ffmpeg's slots.
    office_permits: Arc<tokio::sync::Semaphore>,
    authorized: Mutex<HashMap<(String, String), Instant>>,
    tokens: Mutex<HashMap<String, (String, i64)>>,
}

impl Default for PreviewMgr {
    fn default() -> Self {
        Self::with_dirs(tools::search_dirs())
    }
}

/// D10: two conversions on 8+ cores, otherwise one.
fn concurrency() -> usize {
    match std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) {
        n if n >= 8 => 2,
        _ => 1,
    }
}

impl PreviewMgr {
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        PreviewMgr {
            dirs,
            probe: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
            running: Default::default(),
            progress: Default::default(),
            permits: Arc::new(tokio::sync::Semaphore::new(concurrency())),
            office_permits: Arc::new(tokio::sync::Semaphore::new(1)),
            authorized: Default::default(),
            tokens: Default::default(),
        }
    }

    fn cached(&self) -> Result<tools::Probe, tools::Identity> {
        let found = tools::installed(&self.dirs);
        let identity = tools::identity(&found.iter().map(PathBuf::as_path).collect::<Vec<_>>());
        match self.probe.lock().unwrap().as_ref() {
            Some((id, probe)) if *id == identity => Ok(probe.clone()),
            _ => Err(identity),
        }
    }

    fn store(&self, identity: tools::Identity, probe: tools::Probe) -> tools::Probe {
        if let Some(t) = &probe.media {
            log::info!("preview converter: ffmpeg {} ({}), ffprobe {}, tonemap={}, libx264={}, heif={}", t.ffmpeg.version, t.ffmpeg.path.display(), t.ffprobe.version, t.tonemap, t.libx264, t.heif);
        }
        for o in &probe.office {
            log::info!("preview converter: soffice {} ({}, {}), applications {:?}", o.tool.version, o.tool.path.display(), o.tool.source, o.components);
        }
        for t in [&probe.magick, &probe.pandoc, &probe.pdf.as_ref().map(|p| p.pdftoppm.clone())].into_iter().flatten() {
            log::info!("preview converter: {} {} ({})", t.path.file_stem().unwrap_or_default().to_string_lossy(), t.version, t.source);
        }
        if probe.office.is_empty() {
            log::warn!("preview: soffice is not available, Office documents cannot be previewed (deployment defect: ext-core provides it)");
        }
        for app in probe.missing_office() {
            log::warn!("preview: no LibreOffice install has {}, its documents cannot be previewed (deployment defect)", app.name());
        }
        if let Some(detail) = probe.missing_detail() {
            log::info!("preview converter unavailable: {detail}");
        }
        *self.probe.lock().unwrap() = Some((identity, probe.clone()));
        probe
    }

    /// Re-probes when the binaries found differ from the cached ones (blocking).
    pub fn refresh(&self) -> tools::Probe {
        match self.cached() {
            Ok(probe) => probe,
            Err(identity) => self.store(identity, tools::probe(&self.dirs)),
        }
    }

    pub async fn refresh_async(&self) -> tools::Probe {
        let identity = match self.cached() {
            Ok(probe) => return probe,
            Err(identity) => identity,
        };
        let dirs = self.dirs.clone();
        let probe = tokio::task::spawn_blocking(move || tools::probe(&dirs)).await.unwrap_or(tools::Probe { missing: tools::NAMES.to_vec(), ..Default::default() });
        self.store(identity, probe)
    }

    pub fn authorize(&self, actor: &str, key: &str) {
        let mut map = self.authorized.lock().unwrap();
        map.retain(|_, at| at.elapsed() < AUTHORIZED_TTL);
        map.insert((actor.to_string(), key.to_string()), Instant::now());
    }

    pub fn is_authorized(&self, actor: &str, key: &str) -> bool {
        self.authorized.lock().unwrap().get(&(actor.to_string(), key.to_string())).is_some_and(|at| at.elapsed() < AUTHORIZED_TTL)
    }

    pub fn progress_of(&self, key: &str) -> Option<f64> {
        self.progress.lock().unwrap().get(key).copied()
    }

    pub fn is_running(&self, key: &str) -> bool {
        self.running.lock().unwrap().contains(key)
    }
}

/// One `repr` after planning: what to convert and how.
#[derive(Clone)]
pub struct Plan {
    pub input: Input,
    pub input_obj_id: String,
    pub kind: Kind,
    pub profile: Profile,
    pub hevc_playable: bool,
    /// The source's file name (a page title, never a path).
    pub name: Option<String>,
    /// A document's thumbnail taken from its cached `preview-document` PDF (that result's ObjId).
    pub preview_pdf: Option<ObjId>,
}

impl Plan {
    pub fn params(&self) -> Value {
        if self.profile == Profile::PreviewVideo {
            json!({"profile": self.profile.as_str(), "hevc": self.hevc_playable})
        } else if self.preview_pdf.is_some() {
            json!({"profile": self.profile.as_str(), "via": "preview-document"})
        } else {
            json!({"profile": self.profile.as_str()})
        }
    }
}

/// The PDF a completed `preview-document` of this input left in the cache.
pub async fn cached_preview_pdf(state: &AppState, soffice: &tools::Tool, input_obj_id: &str) -> Option<ObjId> {
    let converter = converter_id(&tools::Converter::Soffice(soffice.clone()));
    let key = work_key(input_obj_id, &converter, &json!({"profile": Profile::PreviewDocument.as_str()}));
    let rec = get_record(&state.db, &key).ok()??;
    if rec.state != "completed" || !result_available(state, &rec).await {
        return None;
    }
    ObjId::new(rec.result.as_ref()?["obj_id"].as_str()?).ok()
}

async fn cache_store(state: &AppState) -> Result<&Arc<BucketStore>, BucketError> {
    crate::bucket::spec::ensure(state, &bucket_id()).await?;
    state.buckets.store(state).await
}

/// Whether a completed result's binding still holds its content.
async fn result_available(state: &AppState, rec: &Record) -> bool {
    let Some(key) = rec.result.as_ref().and_then(|r| r["key"].as_str()) else { return false };
    let Ok(store) = cache_store(state).await else { return false };
    matches!(store.head(&bucket_id(), &Caller::system(), key, None).await, Ok((b, bucket_store::Availability::Local)) if !b.is_delete_marker)
}

/// A read token for a result key (reused while more than half its life remains).
pub async fn read_token(state: &AppState, key: &str) -> NfsResult<String> {
    let now = now_ms();
    if let Some((token, exp)) = state.preview.tokens.lock().unwrap().get(key).cloned() {
        if exp - now > TOKEN_TTL_MS / 2 {
            return Ok(token);
        }
    }
    let store = cache_store(state).await.map_err(|e| internal(format!("preview cache: {e}")))?;
    let req = TokenRequest {
        prefix: None,
        key: Some(key.to_string()),
        actions: [Action::Read].into_iter().collect(),
        expires_at: now + TOKEN_TTL_MS,
        max_uses: None,
        bind_user_id: None,
        bind_app_id: None,
    };
    let issued = store.create_access_token(&bucket_id(), &Caller::system(), req).await.map_err(|e| internal(format!("preview token: {e}")))?;
    let mut tokens = state.preview.tokens.lock().unwrap();
    tokens.retain(|_, (_, exp)| *exp > now);
    tokens.insert(key.to_string(), (issued.token.clone(), now + TOKEN_TTL_MS));
    Ok(issued.token)
}

fn new_attempt(plan: &Plan, key: &str, converter: &str, prev: Option<&Record>) -> Record {
    let now = now_ms();
    Record {
        work_key: key.to_string(),
        state: "processing".into(),
        attempt_id: format!("att_{}", uuid::Uuid::new_v4().simple()),
        input_obj_id: plan.input_obj_id.clone(),
        converter: converter.to_string(),
        params: plan.params(),
        result: None,
        error: None,
        created_at: prev.map(|p| p.created_at).unwrap_or(now),
        updated_at: now,
        accessed_at: now,
        expires_at: None,
    }
}

/// Atomic get-or-create of the work for `plan` (PRD §23.6 state rules).
pub async fn ensure(state: &SharedState, converter: &tools::Converter, plan: Plan, retry: bool, expected_attempt_id: Option<&str>) -> NfsResult<Record> {
    let converter_name = converter_id(converter);
    let key = work_key(&plan.input_obj_id, &converter_name, &plan.params());
    let _gate = state.preview.gate.lock().await;
    let current = get_record(&state.db, &key)?;
    let now = now_ms();
    let start = match &current {
        None => true,
        Some(r) if r.state == "processing" => !state.preview.is_running(&key),
        Some(r) if r.state == "completed" => {
            if result_available(state, r).await {
                touch_record(&state.db, &key);
                return Ok(r.clone());
            }
            log::info!("preview result of {key} is gone; converting again");
            true
        }
        // failed: the negative cache holds until `expires_at`; an explicit
        // Retry of the attempt the caller saw starts a new one at once.
        Some(r) => (retry && expected_attempt_id.is_none_or(|a| a == r.attempt_id)) || now >= r.expires_at.unwrap_or(0),
    };
    if !start {
        return Ok(current.unwrap());
    }
    let rec = new_attempt(&plan, &key, &converter_name, current.as_ref());
    put_record(&state.db, &rec)?;
    state.preview.running.lock().unwrap().insert(key.clone());
    let task_state = state.clone();
    let task_converter = converter.clone();
    let task_rec = rec.clone();
    tokio::spawn(async move { run_attempt(task_state, task_converter, task_rec, plan).await });
    Ok(rec)
}

async fn run_attempt(state: SharedState, converter: tools::Converter, rec: Record, plan: Plan) {
    let permits = match &converter {
        tools::Converter::Soffice(_) => &state.preview.office_permits,
        _ => &state.preview.permits,
    };
    let permit = permits.clone().acquire_owned().await;
    let dir = state.config.data_dir.join("preview-tmp").join(&rec.attempt_id);
    let started = Instant::now();
    let outcome = attempt(&state, &converter, &rec, &plan, &dir).await;
    drop(permit);
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let key = &rec.work_key;
    let finished = match &outcome {
        Ok(result) => {
            log::info!("preview {key} ({}) completed in {:?}: {}", plan.profile.as_str(), started.elapsed(), result);
            finish_record(&state.db, key, &rec.attempt_id, "completed", Some(result), None, None)
        }
        Err(e) => {
            log::warn!("preview {key} ({}) failed after {:?}: {} {}", plan.profile.as_str(), started.elapsed(), e.code, e.message);
            let until = now_ms() + if e.code == convert::NO_THUMBNAIL { NO_THUMBNAIL_TTL_MS } else { NEGATIVE_TTL_MS };
            let error = json!({"code": e.code, "message": e.message, "retryable": e.retryable, "retry_after": until});
            finish_record(&state.db, key, &rec.attempt_id, "failed", None, Some(&error), Some(until))
        }
    };
    if let Err(e) = finished {
        log::warn!("preview {key}: record update failed: {}", e.message);
    }
    state.preview.running.lock().unwrap().remove(key);
    state.preview.progress.lock().unwrap().remove(key);
    if outcome.is_ok() {
        if let Err(e) = enforce_capacity(&state).await {
            log::warn!("preview cache capacity: {}", e.message);
        }
    }
}

fn convert_err(code: &'static str, retryable: bool) -> impl Fn(String) -> ConvertError {
    move |message| ConvertError { code, message, retryable }
}

/// The content (chunk / chunk list) of a FileObject, or the id itself.
pub async fn content_of(ndm: &named_store::NamedDataMgr, id: &ObjId) -> Result<ObjId, ConvertError> {
    if !id.is_file_object() {
        return Ok(id.clone());
    }
    let file = bucket_store::content::load_file(ndm, id).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
    ObjId::new(&file.content).map_err(|e| ConvertError::new("CONTENT_CORRUPTED", e.to_string()))
}

/// First bytes of a source (planning).
pub async fn read_head(state: &AppState, input: &Input) -> Result<Vec<u8>, ConvertError> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    match input {
        Input::Path(p) => {
            let file = tokio::fs::File::open(p).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
            file.take(HEAD_BYTES as u64).read_to_end(&mut head).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
        }
        Input::Object(id) => {
            let ndm = state.share.store(&state.config).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.message))?;
            let content = content_of(ndm, id).await?;
            let (reader, _) = ndm.open_reader(&content, None).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
            reader.take(HEAD_BYTES as u64).read_to_end(&mut head).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
        }
    }
    Ok(head)
}

/// `len` bytes at `offset` of a source (an object is read up to there).
pub async fn read_range(state: &AppState, input: &Input, offset: u64, len: usize) -> Result<Vec<u8>, ConvertError> {
    use tokio::io::AsyncSeekExt;
    let unavailable = |e: String| ConvertError::new("CONTENT_UNAVAILABLE", e);
    let mut out = Vec::with_capacity(len);
    match input {
        Input::Path(p) => {
            let mut file = tokio::fs::File::open(p).await.map_err(|e| unavailable(e.to_string()))?;
            file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| unavailable(e.to_string()))?;
            file.take(len as u64).read_to_end(&mut out).await.map_err(|e| unavailable(e.to_string()))?;
        }
        Input::Object(id) => {
            let ndm = state.share.store(&state.config).await.map_err(|e| unavailable(e.message))?;
            let content = content_of(ndm, id).await?;
            let (mut reader, _) = ndm.open_reader(&content, None).await.map_err(|e| unavailable(e.to_string()))?;
            tokio::io::copy(&mut (&mut reader).take(offset), &mut tokio::io::sink()).await.map_err(|e| unavailable(e.to_string()))?;
            reader.take(len as u64).read_to_end(&mut out).await.map_err(|e| unavailable(e.to_string()))?;
        }
    }
    Ok(out)
}

/// What planning sniffs: the head, and for an OLE2 file whose directory lies
/// past it, the header followed by the directory read from where the header
/// says it is.
pub async fn sniff_bytes(state: &AppState, input: &Input) -> Result<Vec<u8>, ConvertError> {
    let head = read_head(state, input).await?;
    if !media::is_ole2(&head) || media::sniff(&head, None).is_some() {
        return Ok(head);
    }
    let Some(offset) = media::ole_directory_offset(&head).filter(|o| *o >= HEAD_BYTES as u64 || head.len() < HEAD_BYTES) else { return Ok(head) };
    let directory = read_range(state, input, offset, OLE_DIRECTORY_BYTES).await?;
    Ok([&head[..512.min(head.len())], &directory[..]].concat())
}

/// The source's file name: a hint for formats its bytes leave open (OLE2 without
/// its directory in the head), never the basis on its own.
pub async fn name_of(state: &AppState, input: &Input) -> Option<String> {
    match input {
        Input::Path(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
        Input::Object(id) if id.is_file_object() => {
            let ndm = state.share.store(&state.config).await.ok()?;
            bucket_store::content::load_file(ndm, id).await.ok().map(|f| f.name.clone())
        }
        Input::Object(_) => None,
    }
}

async fn materialize(state: &AppState, id: &ObjId, dest: &Path) -> Result<(), ConvertError> {
    let ndm = state.share.store(&state.config).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.message))?;
    let content = content_of(ndm, id).await?;
    let (mut reader, _) = ndm.open_reader(&content, None).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| ConvertError::new("CONVERT_FAILED", e.to_string()))?;
    tokio::io::copy(&mut reader, &mut file).await.map_err(|e| ConvertError { code: "CONTENT_UNAVAILABLE", message: e.to_string(), retryable: true })?;
    Ok(())
}

/// ffprobe, then the planned ffmpeg run.
async fn media_output(state: &SharedState, tools: &tools::Toolchain, rec: &Record, plan: &Plan, dir: &Path) -> Result<Output, ConvertError> {
    let input = local_input(state, &plan.input, dir, "input").await?;
    let heif = if plan.kind == Kind::Heif {
        let mut head = Vec::new();
        let file = tokio::fs::File::open(&input).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
        file.take(HEAD_BYTES as u64).read_to_end(&mut head).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
        Some(heif::parse(&head).ok_or_else(|| ConvertError::new("CONTENT_CORRUPTED", "the HEIF meta box is unreadable"))?)
    } else {
        None
    };
    let info = convert::ffprobe(tools, &input).await?;
    let key = rec.work_key.clone();
    let mgr_state = state.clone();
    let progress: Arc<dyn Fn(f64) + Send + Sync> = Arc::new(move |p| {
        mgr_state.preview.progress.lock().unwrap().insert(key.clone(), p);
    });
    let job = Job { tools, input: &input, dir, kind: plan.kind, profile: plan.profile, hevc_playable: plan.hevc_playable, heif, progress };
    let (mut args, output) = convert::plan(&job, &info)?;
    let seeks = plan.kind == Kind::Video && plan.profile == Profile::Thumbnail;
    match convert::run(&job, &info, args.clone(), &output).await {
        // A container that cannot seek gives no frame at the offset: its first frame, then.
        Err(e) if seeks && e.code == "CONVERT_FAILED" => {
            log::info!("preview {}: no frame at the thumbnail offset ({}), taking the first", rec.work_key, e.message);
            args[1] = "0".into();
            convert::run(&job, &info, args, &output).await?;
        }
        other => other?,
    }
    Ok(output)
}

/// The source as a local file: itself, or a copy of the object in `dir`.
async fn local_input(state: &AppState, input: &Input, dir: &Path, name: &str) -> Result<PathBuf, ConvertError> {
    match input {
        Input::Path(p) => Ok(p.clone()),
        Input::Object(id) => {
            let dest = dir.join(name);
            materialize(state, id, &dest).await?;
            Ok(dest)
        }
    }
}

/// LibreOffice on a copy named with the real extension (it picks the import
/// filter by it), with a private profile removed afterwards.
async fn document_output(state: &SharedState, soffice: &tools::Tool, rec: &Record, plan: &Plan, dir: &Path) -> Result<Output, ConvertError> {
    let Kind::Document(ext) = plan.kind else {
        return Err(ConvertError::new("CONTENT_UNSUPPORTED", "not a document"));
    };
    let input = dir.join(format!("input.{ext}"));
    match &plan.input {
        Input::Path(p) => {
            tokio::fs::copy(p, &input).await.map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
        }
        Input::Object(id) => materialize(state, id, &input).await?,
    }
    let profile = dir.with_file_name(format!("profile-{}", rec.attempt_id));
    let output = convert::document(soffice, &input, dir, &profile, plan.profile == Profile::PreviewSheet).await;
    let _ = tokio::fs::remove_dir_all(&profile).await;
    output
}

/// A document's own thumbnail, shrunk to the thumbnail size when a scaler is there.
async fn embedded_output(state: &SharedState, scaler: &Option<tools::Scaler>, rec: &Record, plan: &Plan, dir: &Path) -> Result<Output, ConvertError> {
    let input = local_input(state, &plan.input, dir, "input").await?;
    let out_dir = dir.to_path_buf();
    let picture = tokio::task::spawn_blocking(move || convert::embedded_thumbnail(&input, &out_dir))
        .await
        .map_err(|e| ConvertError { code: "CONVERT_FAILED", message: e.to_string(), retryable: true })??;
    let mut output = match scaler {
        Some(tools::Scaler::Ffmpeg(tools)) => {
            let image = Plan { input: Input::Path(picture), kind: Kind::Image, ..plan.clone() };
            media_output(state, tools, rec, &image, dir).await?
        }
        Some(tools::Scaler::Magick(magick)) => {
            let coder = if picture.extension().is_some_and(|e| e == "png") { "png" } else { "jpeg" };
            convert::raster(magick, coder, &picture, dir, convert::THUMBNAIL_MAX_EDGE).await?
        }
        None => {
            let png = picture.extension().is_some_and(|e| e == "png");
            Output { path: picture, media_type: if png { "image/png" } else { "image/jpeg" }, result_type: "image", plan: "embedded", width: None, height: None, duration: None, fidelity: Vec::new() }
        }
    };
    output.plan = "embedded";
    output.fidelity = vec![convert::FIDELITY_EMBEDDED];
    Ok(output)
}

async fn attempt(state: &SharedState, converter: &tools::Converter, rec: &Record, plan: &Plan, dir: &Path) -> Result<Value, ConvertError> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| convert_err("CONVERT_FAILED", true)(e.to_string()))?;
    let output = match (converter, plan.kind) {
        (tools::Converter::Ffmpeg(tools), _) => media_output(state, tools, rec, plan, dir).await?,
        (tools::Converter::Soffice(soffice), _) => document_output(state, soffice, rec, plan, dir).await?,
        (tools::Converter::Magick(magick), Kind::Raster(coder)) => {
            let input = local_input(state, &plan.input, dir, "input").await?;
            let max = if plan.profile == Profile::Thumbnail { convert::THUMBNAIL_MAX_EDGE } else { convert::IMAGE_MAX_EDGE };
            convert::raster(magick, coder, &input, dir, max).await?
        }
        (tools::Converter::Pandoc(pandoc), Kind::Markup(reader)) => {
            let input = local_input(state, &plan.input, dir, "input").await?;
            convert::markup(pandoc, reader, &input, dir, plan.name.as_deref().unwrap_or("Preview")).await?
        }
        (tools::Converter::Pdftoppm(pdf), _) => {
            let source = plan.preview_pdf.clone().map(Input::Object).unwrap_or_else(|| plan.input.clone());
            let input = local_input(state, &source, dir, "input.pdf").await?;
            convert::pdf_thumbnail(pdf, &input, dir).await?
        }
        (tools::Converter::Embedded(scaler), _) => embedded_output(state, scaler, rec, plan, dir).await?,
        _ => return Err(ConvertError::new("CONTENT_UNSUPPORTED", format!("no {} conversion for this content", plan.profile.as_str()))),
    };
    let ext = output.path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let name = format!("preview.{ext}");
    let ndm = state.share.store(&state.config).await.map_err(|e| ConvertError { code: "CONVERT_FAILED", message: e.message, retryable: true })?;
    let meta = tokio::fs::metadata(&output.path).await.map_err(|e| convert_err("CONVERT_FAILED", true)(e.to_string()))?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
    let progress = crate::share::snapshot::Progress::default();
    let (file, obj_id, _) = crate::share::snapshot::store_file(ndm, &output.path, &name, (meta.len(), mtime), &progress)
        .await
        .map_err(|e| ConvertError { code: "CONVERT_FAILED", message: format!("store result: {}", e.message), retryable: true })?;
    let store = cache_store(state).await.map_err(|e| ConvertError { code: "CONVERT_FAILED", message: format!("preview cache: {e}"), retryable: true })?;
    let req = PutRequest {
        key: rec.work_key.clone(),
        obj_id: obj_id.to_string(),
        content_type: Some(output.media_type.to_string()),
        metadata: [("profile".to_string(), plan.profile.as_str().to_string()), ("input".to_string(), plan.input_obj_id.clone())].into_iter().collect(),
        ..Default::default()
    };
    store.put(&bucket_id(), &Caller::system(), req).await.map_err(|e| ConvertError { code: "CONVERT_FAILED", message: format!("bind result: {e}"), retryable: true })?;
    let fidelity = (!output.fidelity.is_empty()).then(|| output.fidelity.join("；"));
    Ok(json!({
        "key": rec.work_key,
        "obj_id": obj_id.to_string(),
        "media_type": output.media_type,
        "result_type": output.result_type,
        "plan": output.plan,
        "size": file.size,
        "width": output.width,
        "height": output.height,
        "duration": output.duration,
        "fidelity": fidelity,
    }))
}

/// Removes a result: its binding first, then the record.
async fn drop_result(state: &AppState, rec: &Record) -> NfsResult<()> {
    if let Some(key) = rec.result.as_ref().and_then(|r| r["key"].as_str()) {
        let store = cache_store(state).await.map_err(|e| internal(format!("preview cache: {e}")))?;
        match store.delete(&bucket_id(), &Caller::system(), key, None, None).await {
            Ok(_) | Err(BucketError::NoSuchKey(_)) => {}
            Err(e) => return Err(internal(format!("preview cache delete: {e}"))),
        }
    }
    state.preview.tokens.lock().unwrap().remove(&rec.work_key);
    delete_record(&state.db, &rec.work_key)
}

fn completed_by_access(db: &FileDb) -> NfsResult<Vec<Record>> {
    let conn = db.lock();
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM preview_works WHERE state='completed' ORDER BY accessed_at ASC"))?;
    let rows = stmt.query_map([], row_of)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Least recently used results go until the cache is under `preview_cache_mib`.
pub async fn enforce_capacity(state: &AppState) -> NfsResult<u64> {
    let cap = state.config.preview_cache_mib.saturating_mul(1024 * 1024);
    let rows = completed_by_access(&state.db)?;
    let mut total: u64 = rows.iter().map(|r| r.result.as_ref().and_then(|v| v["size"].as_u64()).unwrap_or(0)).sum();
    let mut dropped = 0;
    for rec in rows {
        if total <= cap {
            break;
        }
        if state.preview.is_running(&rec.work_key) {
            continue;
        }
        total = total.saturating_sub(rec.result.as_ref().and_then(|v| v["size"].as_u64()).unwrap_or(0));
        drop_result(state, &rec).await?;
        dropped += 1;
    }
    Ok(dropped)
}

/// Results unread for 30 days expire; failures past their negative cache are forgotten.
pub async fn expire_idle(state: &AppState, now: i64) -> NfsResult<u64> {
    let mut dropped = 0;
    for rec in completed_by_access(&state.db)? {
        if rec.accessed_at > now - IDLE_EXPIRY_MS {
            break;
        }
        drop_result(state, &rec).await?;
        dropped += 1;
    }
    state.db.lock().execute("DELETE FROM preview_works WHERE state='failed' AND expires_at < ?1", params![now - NEGATIVE_TTL_MS])?;
    Ok(dropped)
}

/// Startup: no attempt survives the process; their rows become retryable failures.
pub fn recover(state: &AppState) {
    let now = now_ms();
    let error = json!({"code": "INTERRUPTED", "message": "the service restarted during the conversion", "retryable": true, "retry_after": now});
    match state.db.lock().execute(
        "UPDATE preview_works SET state='failed', error_json=?1, updated_at=?2, expires_at=?2 WHERE state='processing'",
        params![error.to_string(), now],
    ) {
        Ok(n) if n > 0 => log::info!("preview: {n} interrupted conversions marked for retry"),
        Ok(_) => {}
        Err(e) => log::warn!("preview recovery: {e}"),
    }
    let _ = std::fs::remove_dir_all(state.config.data_dir.join("preview-tmp"));
}

pub async fn maintenance_loop(state: SharedState) {
    let mut tick = tokio::time::interval(MAINTENANCE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        state.preview.refresh_async().await;
        match expire_idle(&state, now_ms()).await {
            Ok(n) if n > 0 => log::info!("preview cache: {n} idle results expired"),
            Ok(_) => {}
            Err(e) => log::warn!("preview cache expiry: {}", e.message),
        }
        if let Err(e) = enforce_capacity(&state).await {
            log::warn!("preview cache capacity: {}", e.message);
        }
    }
}

/// `sha256:<hex>` of a local file, cached per (path, size, mtime) in `preview_inputs`.
pub async fn native_obj_id(db: &FileDb, source: &str, path: &Path) -> NfsResult<String> {
    let meta = tokio::fs::metadata(path).await?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as i64).unwrap_or(0);
    let size = meta.len() as i64;
    {
        let conn = db.lock();
        let hit: Option<String> = conn
            .query_row("SELECT obj_id FROM preview_inputs WHERE source=?1 AND size=?2 AND mtime=?3", params![source, size, mtime], |r| r.get(0))
            .optional()?;
        if let Some(id) = hit {
            return Ok(id);
        }
    }
    let p = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::Read;
        let mut f = std::fs::File::open(p)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(hex::encode(h.finalize()))
    })
    .await
    .map_err(|e| internal(format!("hash task: {e}")))??;
    let id = format!("sha256:{hash}");
    db.lock().execute(
        "INSERT INTO preview_inputs(source, size, mtime, obj_id, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source) DO UPDATE SET size=excluded.size, mtime=excluded.mtime, obj_id=excluded.obj_id, updated_at=excluded.updated_at",
        params![source, size, mtime, id, now_ms()],
    )?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_keys_follow_input_converter_and_params() {
        let a = work_key("sha256:aa", "nfs.preview/1@ffmpeg-7.1", &json!({"profile": "preview-image"}));
        assert_eq!(a, work_key("sha256:aa", "nfs.preview/1@ffmpeg-7.1", &json!({"profile": "preview-image"})));
        assert_ne!(a, work_key("sha256:ab", "nfs.preview/1@ffmpeg-7.1", &json!({"profile": "preview-image"})));
        assert_ne!(a, work_key("sha256:aa", "nfs.preview/1@ffmpeg-7.2", &json!({"profile": "preview-image"})));
        assert_ne!(a, work_key("sha256:aa", "nfs.preview/1@ffmpeg-7.1", &json!({"profile": "thumbnail"})));
        let video = |hevc| Plan { input: Input::Path("/x".into()), input_obj_id: "x".into(), kind: Kind::Video, profile: Profile::PreviewVideo, hevc_playable: hevc, name: None, preview_pdf: None };
        assert_ne!(video(true).params(), video(false).params());
        let soffice = tools::Converter::Soffice(tools::Tool::new("soffice".into(), "25.2.3.2".into()));
        assert_eq!(converter_id(&soffice), "nfs.preview/1@soffice-25.2.3.2");
    }
}
