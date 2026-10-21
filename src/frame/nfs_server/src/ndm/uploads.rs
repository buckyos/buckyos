//! tus 1.0.0 upload sessions of `/ndm/v1/uploads`, one per chunk (cyfs-ndn
//! `named-data-mgr-zone-protocol.md` §5). A session belongs to the caller that
//! created it: HEAD, PATCH and DELETE of anyone else's session answer 404.
//! Sessions and their staged bytes live in filedb and `ndm_uploads/`, so an
//! upload resumes across restarts; idle ones are discarded after
//! `ndm.session_ttl_secs`. A finished chunk goes into the named store with a
//! lease of this session (`ndm-upload:<id>`), which a termination releases —
//! never the chunk itself, which other business may also reference.

use super::{allowed, denied, hold, lease, principal, store, tus_error, Principal, HOLD_TTL_SECS, RES_UPLOADS};
use crate::fsutil::unix_now;
use crate::state::{AppState, SharedState};
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use ndn_lib::{ChunkHasher, ChunkId, ObjId};
use rusqlite::{params, OptionalExtension, Row as SqlRow};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;

pub const TUS_VERSION: &str = "1.0.0";
const MAX_CHUNK_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
struct Session {
    session_id: String,
    principal: String,
    canonical_upload_id: String,
    logical_path: String,
    file_hash: Option<String>,
    chunk_hash: Option<String>,
    chunk_size: u64,
    offset: u64,
    status: String,
    chunk_obj_id: Option<String>,
    raw_metadata: String,
    updated_at: i64,
}

const COLUMNS: &str = "session_id, principal, canonical_upload_id, logical_path, file_hash, chunk_hash, chunk_size, offset, status, chunk_obj_id, raw_metadata, updated_at";

fn row(r: &SqlRow) -> rusqlite::Result<Session> {
    Ok(Session {
        session_id: r.get(0)?,
        principal: r.get(1)?,
        canonical_upload_id: r.get(2)?,
        logical_path: r.get(3)?,
        file_hash: r.get(4)?,
        chunk_hash: r.get(5)?,
        chunk_size: r.get::<_, i64>(6)? as u64,
        offset: r.get::<_, i64>(7)? as u64,
        status: r.get(8)?,
        chunk_obj_id: r.get(9)?,
        raw_metadata: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

impl Session {
    fn unfinished(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "uploading")
    }

    fn expired(&self, state: &AppState) -> bool {
        self.unfinished() && unix_now() - self.updated_at > state.config.ndm.session_ttl_secs as i64
    }
}

fn part_path(state: &AppState, session_id: &str) -> std::path::PathBuf {
    state.ndm.staging.join(format!("{session_id}.part"))
}

fn load(state: &AppState, session_id: &str) -> Option<Session> {
    let conn = state.db.lock();
    conn.query_row(&format!("SELECT {COLUMNS} FROM ndm_uploads WHERE session_id=?1"), params![session_id], row)
        .optional()
        .ok()
        .flatten()
}

/// The caller's own session; anyone else's is reported as unknown.
fn owned(state: &AppState, p: &Principal, session_id: &str) -> Result<Session, Response> {
    match load(state, session_id) {
        Some(s) if s.principal == p.key() => Ok(s),
        _ => Err(tus_error(StatusCode::NOT_FOUND, "not_found", format!("upload {session_id} not found"))),
    }
}

/// Drops a session with its staged bytes and the lease it held (never the chunk itself).
async fn discard(state: &AppState, s: &Session, reason: &str) {
    let _ = tokio::fs::remove_file(part_path(state, &s.session_id)).await;
    if let Some(chunk) = s.chunk_obj_id.as_deref().and_then(|c| ObjId::new(c).ok()) {
        if let Ok(store) = store(state).await {
            if let Err(e) = store.unpin(&chunk, &lease_owner(&s.session_id)).await {
                log::debug!("release lease of upload {}: {}", s.session_id, e);
            }
        }
    }
    let conn = state.db.lock();
    let _ = conn.execute("DELETE FROM ndm_uploads WHERE session_id=?1", params![s.session_id]);
    log::info!("ndm upload {} ({}) discarded: {}", s.session_id, s.logical_path, reason);
}

fn lease_owner(session_id: &str) -> String {
    format!("ndm-upload:{session_id}")
}

fn check_tus(headers: &HeaderMap) -> Result<(), Response> {
    match headers.get("tus-resumable").and_then(|v| v.to_str().ok()) {
        Some(TUS_VERSION) => Ok(()),
        other => {
            let mut r = tus_error(
                StatusCode::PRECONDITION_FAILED,
                "precondition_failed",
                format!("unsupported Tus-Resumable {:?}, server supports {TUS_VERSION}", other),
            );
            r.headers_mut().insert("tus-version", HeaderValue::from_static(TUS_VERSION));
            Err(r)
        }
    }
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok())
}

/// `Upload-Metadata`: tus `key base64value` pairs, also `key=value`.
fn metadata(raw: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for pair in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((key, encoded)) = pair.split_once(' ') {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .unwrap_or_else(|| encoded.trim().to_string());
            fields.insert(key.trim().to_string(), decoded);
        } else if let Some((key, value)) = pair.split_once('=') {
            fields.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    fields
}

fn valid_logical_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains("..")
        && !path.starts_with('/')
        && path.chars().all(|c| c.is_ascii() && !c.is_ascii_control() && c != '\\')
}

fn http_date(at: i64) -> String {
    let days = at.div_euclid(86400);
    let secs = at.rem_euclid(86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const WDAY: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WDAY[days.rem_euclid(7) as usize],
        day,
        MON[(month - 1) as usize],
        year,
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn session_response(state: &AppState, status: StatusCode, s: &Session) -> Response {
    let mut r = status.into_response();
    let h = r.headers_mut();
    let set = |h: &mut HeaderMap, name: &'static str, value: String| {
        if let Ok(v) = HeaderValue::from_str(&value) {
            h.insert(name, v);
        }
    };
    set(h, "tus-resumable", TUS_VERSION.into());
    set(h, "location", format!("/ndm/v1/uploads/{}", s.session_id));
    set(h, "ndm-upload-id", s.canonical_upload_id.clone());
    set(h, "upload-offset", s.offset.to_string());
    set(h, "upload-length", s.chunk_size.to_string());
    set(h, "ndm-chunk-status", s.status.clone());
    if s.unfinished() {
        set(h, "upload-expires", http_date(s.updated_at + state.config.ndm.session_ttl_secs as i64));
    }
    if let Some(id) = &s.chunk_obj_id {
        set(h, "ndm-chunk-object-id", id.clone());
    }
    r
}

pub async fn options() -> Response {
    let mut r = StatusCode::NO_CONTENT.into_response();
    let h = r.headers_mut();
    h.insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
    h.insert("tus-version", HeaderValue::from_static(TUS_VERSION));
    h.insert("tus-extension", HeaderValue::from_static("creation,expiration,termination"));
    h.insert("tus-max-size", HeaderValue::from(MAX_CHUNK_SIZE));
    r
}

async fn uploader(state: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    let p = principal(state, headers).await.map_err(|mut r| {
        r.headers_mut().insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
        r
    })?;
    if !allowed(state, &p, RES_UPLOADS, "write").await {
        let mut r = denied("upload to the named store");
        r.headers_mut().insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
        return Err(r);
    }
    Ok(p)
}

/// `POST /ndm/v1/uploads`: create (or find again) the session for one chunk.
pub async fn create(State(state): State<SharedState>, headers: HeaderMap) -> Response {
    let p = match uploader(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = check_tus(&headers) {
        return r;
    }
    let Some(size) = header_u64(&headers, "upload-length") else {
        return tus_error(StatusCode::BAD_REQUEST, "invalid_param", "missing Upload-Length header");
    };
    if size > MAX_CHUNK_SIZE {
        return tus_error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", format!("chunk size {size} exceeds 32 MiB"));
    }
    let raw = headers.get("upload-metadata").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let meta = metadata(&raw);
    let Some(logical_path) = meta.get("logical_path").cloned().filter(|p| valid_logical_path(p)) else {
        return tus_error(StatusCode::BAD_REQUEST, "invalid_param", "Upload-Metadata needs a safe relative logical_path");
    };
    let file_hash = meta.get("file_hash").cloned();
    let chunk_index: i64 = meta.get("chunk_index").and_then(|v| v.parse().ok()).unwrap_or(0);
    let chunk_hash = meta.get("chunk_hash").cloned();
    let principal_key = p.key();
    let upload_key = hex::encode(Sha256::digest(
        format!("{principal_key}\n{logical_path}\n{}\n{chunk_index}", file_hash.as_deref().unwrap_or("")).as_bytes(),
    ));
    let canonical_upload_id = headers
        .get("ndm-upload-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| format!("path:{logical_path}"));

    // A new version of the same path replaces the caller's unfinished sessions for it.
    let stale: Vec<Session> = {
        let conn = state.db.lock();
        let mut stmt = match conn.prepare(&format!(
            "SELECT {COLUMNS} FROM ndm_uploads WHERE principal=?1 AND logical_path=?2 AND COALESCE(file_hash,'')<>?3 AND status IN ('pending','uploading')"
        )) {
            Ok(s) => s,
            Err(e) => return tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e),
        };
        stmt.query_map(params![principal_key, logical_path, file_hash.as_deref().unwrap_or("")], row)
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    for s in &stale {
        discard(&state, s, "replaced by another version of the file").await;
    }
    let existing = {
        let conn = state.db.lock();
        conn.query_row(&format!("SELECT {COLUMNS} FROM ndm_uploads WHERE upload_key=?1"), params![upload_key], row).optional().ok().flatten()
    };
    if let Some(s) = existing {
        if !s.expired(&state) {
            return session_response(&state, StatusCode::OK, &s);
        }
        discard(&state, &s, "expired").await;
    }

    let store = match store(&state).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let session_id = format!("us_{}", uuid::Uuid::new_v4().simple());
    let now = unix_now();
    let mut session = Session {
        session_id: session_id.clone(),
        principal: principal_key.clone(),
        canonical_upload_id,
        logical_path,
        file_hash,
        chunk_hash: chunk_hash.clone(),
        chunk_size: size,
        offset: 0,
        status: "pending".into(),
        chunk_obj_id: None,
        raw_metadata: raw,
        updated_at: now,
    };
    // The chunk is already here: nothing to upload.
    let known = chunk_hash.as_deref().and_then(|h| ChunkId::new(h).ok());
    if let Some(chunk) = known {
        if store.have_chunk(&chunk).await {
            let obj = chunk.to_obj_id();
            session.offset = size;
            session.status = "skipped".into();
            session.chunk_obj_id = Some(obj.to_string());
            if let Err(r) = insert(&state, &session, &upload_key, chunk_index) {
                return r;
            }
            hold(&state, store, &p, &obj, "upload").await;
            lease(store, &obj, &lease_owner(&session_id)).await;
            return session_response(&state, StatusCode::OK, &session);
        }
    }
    if let Err(r) = make_room(&state, &principal_key, size).await {
        return r;
    }
    if size == 0 {
        session.status = "completed".into();
    } else if let Err(e) = tokio::fs::File::create(part_path(&state, &session_id)).await {
        return tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", format!("create staging file: {e}"));
    }
    if let Err(r) = insert(&state, &session, &upload_key, chunk_index) {
        return r;
    }
    session_response(&state, StatusCode::CREATED, &session)
}

fn insert(state: &AppState, s: &Session, upload_key: &str, chunk_index: i64) -> Result<(), Response> {
    let conn = state.db.lock();
    conn.execute(
        "INSERT INTO ndm_uploads(session_id, principal, upload_key, canonical_upload_id, logical_path, file_hash, chunk_index, chunk_hash,
           chunk_size, offset, status, chunk_obj_id, raw_metadata, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
        params![
            s.session_id,
            s.principal,
            upload_key,
            s.canonical_upload_id,
            s.logical_path,
            s.file_hash,
            chunk_index,
            s.chunk_hash,
            s.chunk_size as i64,
            s.offset as i64,
            s.status,
            s.chunk_obj_id,
            s.raw_metadata,
            s.updated_at
        ],
    )
    .map(|_| ())
    .map_err(|e| tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))
}

/// Keeps the caller's staged bytes under its quota, evicting its oldest unfinished uploads.
async fn make_room(state: &AppState, principal_key: &str, size: u64) -> Result<(), Response> {
    let unfinished: Vec<Session> = {
        let conn = state.db.lock();
        let Ok(mut stmt) = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM ndm_uploads WHERE principal=?1 AND status IN ('pending','uploading') ORDER BY updated_at"
        )) else {
            return Ok(());
        };
        stmt.query_map(params![principal_key], row).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    };
    let quota = state.config.ndm.principal_quota;
    let mut in_use: u64 = unfinished.iter().map(|s| s.offset).sum();
    for s in &unfinished {
        if in_use + size <= quota {
            break;
        }
        in_use -= s.offset;
        discard(state, s, "evicted for quota").await;
    }
    if in_use + size > quota {
        return Err(tus_error(StatusCode::INSUFFICIENT_STORAGE, "quota_exceeded", format!("staged uploads would exceed {quota} bytes")));
    }
    Ok(())
}

/// `HEAD /ndm/v1/uploads/{id}`.
pub async fn head(State(state): State<SharedState>, AxPath(id): AxPath<String>, headers: HeaderMap) -> Response {
    let p = match uploader(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = check_tus(&headers) {
        return r;
    }
    let s = match owned(&state, &p, &id) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if s.expired(&state) {
        discard(&state, &s, "expired").await;
        return tus_error(StatusCode::NOT_FOUND, "not_found", format!("upload {id} expired"));
    }
    let mut r = session_response(&state, StatusCode::OK, &s);
    r.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&s.raw_metadata) {
        if !s.raw_metadata.is_empty() {
            r.headers_mut().insert("upload-metadata", v);
        }
    }
    r
}

/// `PATCH /ndm/v1/uploads/{id}`: append; the last byte stores the chunk.
pub async fn patch(State(state): State<SharedState>, AxPath(id): AxPath<String>, headers: HeaderMap, body: Body) -> Response {
    let p = match uploader(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = check_tus(&headers) {
        return r;
    }
    if headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/offset+octet-stream") {
        return tus_error(StatusCode::BAD_REQUEST, "invalid_param", "PATCH requires Content-Type: application/offset+octet-stream");
    }
    let Some(offset) = header_u64(&headers, "upload-offset") else {
        return tus_error(StatusCode::BAD_REQUEST, "invalid_param", "missing Upload-Offset header");
    };
    let s = match owned(&state, &p, &id) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !s.unfinished() {
        return session_response(&state, StatusCode::NO_CONTENT, &s);
    }
    if s.expired(&state) {
        discard(&state, &s, "expired").await;
        return tus_error(StatusCode::GONE, "session_expired", format!("upload {id} expired"));
    }
    if offset != s.offset {
        return tus_error(StatusCode::CONFLICT, "offset_conflict", format!("offset mismatch: client={offset}, server={}", s.offset));
    }
    if !state.ndm.try_busy(&id) {
        return tus_error(StatusCode::CONFLICT, "offset_conflict", "another PATCH of this upload is in progress");
    }
    let result = append(&state, &p, s, body).await;
    state.ndm.done(&id);
    result
}

async fn append(state: &AppState, p: &Principal, mut s: Session, body: Body) -> Response {
    let room = (s.chunk_size - s.offset) as usize;
    let data: Bytes = match axum::body::to_bytes(body, room + 1).await {
        Ok(d) => d,
        Err(e) => return tus_error(StatusCode::BAD_REQUEST, "invalid_param", format!("read body: {e}")),
    };
    if data.is_empty() {
        return tus_error(StatusCode::BAD_REQUEST, "invalid_param", "empty body");
    }
    if data.len() > room {
        return tus_error(StatusCode::BAD_REQUEST, "invalid_param", format!("data exceeds the chunk size {}", s.chunk_size));
    }
    let path = part_path(state, &s.session_id);
    let written = async {
        let mut file = tokio::fs::OpenOptions::new().append(true).open(&path).await?;
        file.write_all(&data).await?;
        file.flush().await
    }
    .await;
    if let Err(e) = written {
        return tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", format!("stage upload bytes: {e}"));
    }
    s.offset += data.len() as u64;
    s.updated_at = unix_now();
    s.status = if s.offset == s.chunk_size { "completed".into() } else { "uploading".into() };
    if s.status == "completed" {
        match finish(state, p, &s).await {
            Ok(chunk) => s.chunk_obj_id = Some(chunk),
            Err(r) => {
                discard(state, &s, "stored chunk failed").await;
                return r;
            }
        }
    }
    {
        let conn = state.db.lock();
        if let Err(e) = conn.execute(
            "UPDATE ndm_uploads SET offset=?2, status=?3, chunk_obj_id=?4, updated_at=?5 WHERE session_id=?1",
            params![s.session_id, s.offset as i64, s.status, s.chunk_obj_id, s.updated_at],
        ) {
            return tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e);
        }
    }
    if s.status == "completed" {
        let _ = tokio::fs::remove_file(&path).await;
    }
    session_response(state, StatusCode::NO_CONTENT, &s)
}

/// Stores the staged bytes as the chunk (the store checks the hash) and leases it.
async fn finish(state: &AppState, p: &Principal, s: &Session) -> Result<String, Response> {
    let store = store(state).await?;
    let path = part_path(state, &s.session_id);
    let chunk = match s.chunk_hash.as_deref() {
        Some(h) => ChunkId::new(h).map_err(|e| tus_error(StatusCode::BAD_REQUEST, "invalid_param", format!("chunk_hash: {e}")))?,
        None => {
            let data = tokio::fs::read(&path).await.map_err(|e| tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
            ChunkHasher::new(None)
                .and_then(|h| h.calc_mix_chunk_id_from_bytes(&data))
                .map_err(|e| tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?
        }
    };
    let file = tokio::fs::File::open(&path).await.map_err(|e| tus_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e))?;
    store
        .put_chunk_by_reader(&chunk, s.chunk_size, Box::pin(file))
        .await
        .map_err(|e| tus_error(StatusCode::BAD_REQUEST, "invalid_data", format!("chunk does not match {}: {e}", chunk.to_string())))?;
    let obj = chunk.to_obj_id();
    hold(state, store, p, &obj, "upload").await;
    lease(store, &obj, &lease_owner(&s.session_id)).await;
    Ok(obj.to_string())
}

/// `DELETE /ndm/v1/uploads/{id}` (tus termination): drop the session and what it staged.
pub async fn terminate(State(state): State<SharedState>, AxPath(id): AxPath<String>, headers: HeaderMap) -> Response {
    let p = match uploader(&state, &headers).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(r) = check_tus(&headers) {
        return r;
    }
    let s = match owned(&state, &p, &id) {
        Ok(s) => s,
        Err(r) => return r,
    };
    discard(&state, &s, "terminated by its owner").await;
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert("tus-resumable", HeaderValue::from_static(TUS_VERSION));
    r
}

/// tus clients in restricted environments tunnel PATCH/DELETE/HEAD over POST.
pub async fn post_override(state: State<SharedState>, id: AxPath<String>, headers: HeaderMap, body: Body) -> Response {
    match headers.get("x-http-method-override").and_then(|v| v.to_str().ok()).map(str::to_ascii_uppercase).as_deref() {
        Some("PATCH") => patch(state, id, headers, body).await,
        Some("DELETE") => terminate(state, id, headers).await,
        Some("HEAD") => head(state, id, headers).await,
        _ => tus_error(StatusCode::METHOD_NOT_ALLOWED, "unsupported", "POST on an upload needs X-HTTP-Method-Override"),
    }
}

/// Idle unfinished sessions go with their staged bytes; finished ones once their hold ran out.
pub async fn sweep(state: &AppState) {
    let now = unix_now();
    let due: Vec<Session> = {
        let conn = state.db.lock();
        let Ok(mut stmt) = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM ndm_uploads WHERE (status IN ('pending','uploading') AND updated_at < ?1) OR updated_at < ?2"
        )) else {
            return;
        };
        stmt.query_map(params![now - state.config.ndm.session_ttl_secs as i64, now - HOLD_TTL_SECS as i64], row)
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    };
    for s in due {
        discard(state, &s, if s.unfinished() { "expired" } else { "hold ended" }).await;
    }
}

/// Staged files no unfinished session points at (left by a crash between steps).
pub fn remove_orphan_parts(state: &AppState) {
    let Ok(entries) = std::fs::read_dir(&state.ndm.staging) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".part") else { continue };
        if load(state, id).is_none_or(|s| !s.unfinished()) {
            let _ = std::fs::remove_file(entry.path());
            log::info!("removed orphan ndm staging file {}", name);
        }
    }
}
