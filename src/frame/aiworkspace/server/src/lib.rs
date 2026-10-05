//! aiworkspace service: kRPC method dispatch plus the upload/download routes
//! (design doc §5). All store work is synchronous and runs on the blocking pool;
//! each Workspace has exactly one writer behind its mutex.

pub mod auth;

use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::workspace::{CommitOpts, Workspace};
use aiworkspace_store::{Caller, Service};
use auth::Authenticator;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

pub const HTTP_PATH: &str = "/kapi/aiworkspace";

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_commit_bytes: usize,
    pub max_wait_ms: u64,
    pub max_asset_bytes: u64,
    pub max_package_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_commit_bytes: 8 * 1024 * 1024, max_wait_ms: 30_000, max_asset_bytes: 32 * 1024 * 1024, max_package_bytes: 2 << 30 }
    }
}

struct Upload {
    kind: &'static str,
    workspace_id: Option<String>,
    principal: String,
    path: PathBuf,
    received: Option<u64>,
}

/// A head or lock change of one Workspace: the payload of the wake-up hint.
#[derive(Debug, Clone)]
pub enum Wake {
    Head { workspace_id: String, epoch: String, head_seq: u64 },
    Locks { workspace_id: String, counter: u64 },
}

pub struct AppState {
    pub svc: Service,
    pub auth: Arc<dyn Authenticator>,
    pub limits: Limits,
    heads: Arc<Mutex<HashMap<String, watch::Sender<u64>>>>,
    uploads: Mutex<HashMap<String, Upload>>,
}

impl AppState {
    /// `wake` receives hints for an external event channel (kevent in system mode).
    pub fn new(mut svc: Service, auth: Arc<dyn Authenticator>, limits: Limits, wake: Option<tokio::sync::mpsc::UnboundedSender<Wake>>) -> Arc<AppState> {
        let heads: Arc<Mutex<HashMap<String, watch::Sender<u64>>>> = Arc::default();
        let hook_heads = heads.clone();
        svc.on_open = Some(Box::new(move |ws: &mut Workspace| {
            let tx = hook_heads
                .lock()
                .unwrap()
                .entry(ws.workspace_id.clone())
                .or_insert_with(|| watch::channel(ws.head_seq).0)
                .clone();
            let _ = tx.send(ws.head_seq);
            let (id, epoch, wake_head) = (ws.workspace_id.clone(), ws.epoch.clone(), wake.clone());
            // called only after the commit is durable and published in memory
            ws.on_commit = Some(Box::new(move |seq| {
                let _ = tx.send(seq);
                if let Some(w) = &wake_head {
                    let _ = w.send(Wake::Head { workspace_id: id.clone(), epoch: epoch.clone(), head_seq: seq });
                }
            }));
            let (id, wake_locks) = (ws.workspace_id.clone(), wake.clone());
            let counter = std::sync::atomic::AtomicU64::new(0);
            ws.on_lock_change = Some(Box::new(move || {
                if let Some(w) = &wake_locks {
                    let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let _ = w.send(Wake::Locks { workspace_id: id.clone(), counter: n });
                }
            }));
        }));
        Arc::new(AppState { svc, auth, limits, heads, uploads: Mutex::new(HashMap::new()) })
    }

    fn with_ws<T>(&self, id: &str, f: impl FnOnce(&mut Workspace) -> WsResult<T>) -> WsResult<T> {
        let handle = self.svc.workspace(id)?;
        let mut guard = match handle.lock() {
            Ok(g) => g,
            Err(_) => {
                // a panic left this writer in an unknown state: discard it and rebuild from the database
                self.svc.close(id);
                return Err(WsError::new(Code::WriterBusy, "workspace writer was reset; retry"));
            }
        };
        f(&mut guard)
    }

    fn staging_file(&self, name: &str) -> PathBuf {
        self.svc.data_dir.join("staging").join(name)
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let krpc_limit = state.limits.max_commit_bytes + 64 * 1024;
    Router::new()
        .route(HTTP_PATH, post(krpc))
        .route(&format!("{HTTP_PATH}/"), post(krpc))
        .layer(DefaultBodyLimit::max(krpc_limit))
        .route(&format!("{HTTP_PATH}/upload/{{upload_id}}"), put(upload).layer(DefaultBodyLimit::disable()))
        .route(&format!("{HTTP_PATH}/asset/{{workspace_id}}/{{object_id}}"), get(asset))
        .route(&format!("{HTTP_PATH}/export/{{workspace_id}}/{{export_id}}"), get(download_export))
        .route(&format!("{HTTP_PATH}/replica/{{workspace_id}}/{{replica_id}}"), get(download_replica))
        .route(&format!("{HTTP_PATH}/schemas/richtext.basic.v1.json"), get(schema))
        .route(&format!("{HTTP_PATH}/healthz"), get(|| async { "ok" }))
        .with_state(state)
}

async fn schema() -> Response {
    ([(header::CONTENT_TYPE, "application/json")], aiworkspace_core::richtext::SCHEMA_JSON).into_response()
}

// ---- kRPC ----

fn rpc_error(seq: u64, message: &str) -> Response {
    // the kRPC error channel is a single string and always HTTP 200
    Json(json!({ "error": message, "sys": [seq] })).into_response()
}

async fn krpc(State(state): State<Arc<AppState>>, body: bytes::Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(0, &format!("invalid request: {e}")),
    };
    let seq = req["sys"][0].as_u64().unwrap_or(0);
    let Some(method) = req["method"].as_str().map(str::to_string) else { return rpc_error(seq, "method required") };
    let Some(token) = req["sys"][1].as_str().filter(|t| !t.trim().is_empty()) else {
        return rpc_error(seq, "session token required");
    };
    let caller = match state.auth.authenticate(token).await {
        Ok(c) => c,
        Err(e) => return rpc_error(seq, &e),
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let result = if method == "doc.wait_changes" {
        wait_changes(&state, &params, &caller).await
    } else {
        let st = state.clone();
        let m = method.clone();
        match tokio::task::spawn_blocking(move || dispatch(&st, &m, &params, &caller)).await {
            Ok(r) => r,
            Err(e) => Err(WsError::io(format!("handler panicked: {e}"))),
        }
    };
    match result {
        Ok(v) => Json(json!({ "result": v, "sys": [seq] })).into_response(),
        Err(e) if e.detail == UNKNOWN_METHOD => rpc_error(seq, &format!("unknown method {method}")),
        // business failures are results, not kRPC errors: clients branch on `error.code`
        Err(e) => Json(json!({ "result": { "ok": false, "error": e.to_json() }, "sys": [seq] })).into_response(),
    }
}

const UNKNOWN_METHOD: &str = "\u{0}unknown-method";

fn s<'a>(p: &'a Value, key: &str) -> WsResult<&'a str> {
    p.get(key).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{key} required")))
}

fn strings(p: &Value, key: &str) -> WsResult<Vec<String>> {
    p.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .ok_or_else(|| WsError::invalid_op(format!("{key} must be an array of strings")))
}

fn okv(mut v: Value) -> Value {
    if v.get("ok").is_none() && v.get("status").is_none() {
        v["ok"] = json!(true);
    }
    v
}

/// Synchronous method dispatch. Every method authorizes against the grants of
/// the trusted `caller`.
fn dispatch(state: &Arc<AppState>, method: &str, p: &Value, caller: &Caller) -> WsResult<Value> {
    let svc = &state.svc;
    let ws_id = || s(p, "workspace_id");
    Ok(okv(match method {
        "ws.create" => svc.create_workspace(caller, p.get("title").and_then(Value::as_str).unwrap_or(""), p.get("workspace_id").and_then(Value::as_str))?,
        "ws.list" => svc.list_workspaces(caller)?,
        "ws.get_info" => state.with_ws(ws_id()?, |ws| ws.info(caller))?,
        "ws.delete" => svc.delete_workspace(caller, ws_id()?)?,
        "ws.grant" => state.with_ws(ws_id()?, |ws| {
            ws.grant(caller, s(p, "subject")?, p.get("scope_entity_id").and_then(Value::as_str), &strings(p, "capabilities")?)
        })?,
        "ws.revoke" => state.with_ws(ws_id()?, |ws| ws.revoke(caller, s(p, "subject")?, p.get("scope_entity_id").and_then(Value::as_str)))?,
        "ws.list_grants" => state.with_ws(ws_id()?, |ws| ws.list_grants(caller))?,
        "ws.fork" => svc.fork(caller, ws_id()?)?,
        "ws.begin_import" => begin_upload(state, "import", None, caller)?,
        "ws.import" => {
            let upload = take_upload(state, s(p, "upload_id")?, "import", caller)?;
            let r = svc.import(caller, &upload, s(p, "semantics")?, p.get("replace").and_then(Value::as_bool).unwrap_or(false));
            let _ = std::fs::remove_file(&upload);
            r?
        }
        "doc.outline" => state.with_ws(ws_id()?, |ws| ws.outline(caller))?,
        "doc.resolve" => state.with_ws(ws_id()?, |ws| ws.resolve(caller, p.get("reference"), p.get("path").and_then(Value::as_str)))?,
        "doc.read" => state.with_ws(ws_id()?, |ws| match p.get("targets").and_then(Value::as_array) {
            // batch form: each item succeeds or fails on its own
            Some(targets) => Ok(json!({ "ok": true, "head_seq": ws.head_seq, "results": targets.iter().map(|t| {
                match t.get("entity_id").and_then(Value::as_str) {
                    Some(id) => ws.read(caller, id, t.get("selector")).map(okv).unwrap_or_else(|e| json!({ "ok": false, "error": e.to_json() })),
                    None => json!({ "ok": false, "error": WsError::invalid_op("entity_id required").to_json() }),
                }
            }).collect::<Vec<_>>() })),
            None => ws.read(caller, s(p, "entity_id")?, p.get("selector")),
        })?,
        "doc.list_children" => state.with_ws(ws_id()?, |ws| {
            ws.list_children(caller, s(p, "entity_id")?, p.get("include_deleted").and_then(Value::as_bool).unwrap_or(false))
        })?,
        "doc.query" => state.with_ws(ws_id()?, |ws| ws.query(caller, p, &state.svc.sources))?,
        "doc.source_capabilities" => state.with_ws(ws_id()?, |ws| ws.source_capabilities(caller, s(p, "source_id")?, &state.svc.sources))?,
        "doc.get_collab_state" => state.with_ws(ws_id()?, |ws| ws.get_collab_state(caller, s(p, "entity_id")?))?,
        "doc.prepare" => state.with_ws(ws_id()?, |ws| Ok(ws.prepare(p, caller)))?,
        "doc.commit" => state.with_ws(ws_id()?, |ws| Ok(ws.commit(p, caller, &CommitOpts::default())))?,
        "doc.get_submission" => state.with_ws(ws_id()?, |ws| ws.get_submission(caller, s(p, "epoch")?, s(p, "idempotency_key")?))?,
        "doc.undo" => state.with_ws(ws_id()?, |ws| Ok(ws.undo(caller, p)))?,
        "doc.get_changes" => state.with_ws(ws_id()?, |ws| {
            let with_ops = p.get("filter").and_then(|f| f.get("detail")).and_then(Value::as_str) != Some("touched");
            ws.get_changes(caller, s(p, "epoch")?, p.get("after_seq").and_then(Value::as_u64).unwrap_or(0),
                           p.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize, with_ops)
        })?,
        "doc.checkpoint" => state.with_ws(ws_id()?, |ws| Ok(ws.checkpoint(caller)?.to_json()))?,
        "doc.export" => state.with_ws(ws_id()?, |ws| {
            let mut r = ws.export(caller, p.get("mode").and_then(Value::as_str).unwrap_or("share"),
                                  p.get("self_contained").and_then(Value::as_bool).unwrap_or(true))?;
            r.as_object_mut().unwrap().remove("path"); // server paths are not part of the protocol
            Ok(r)
        })?,
        "asset.begin_upload" => {
            let id = ws_id()?.to_string();
            let size = p.get("size").and_then(Value::as_u64).unwrap_or(0);
            if size > state.limits.max_asset_bytes {
                return Err(WsError::limit(format!("assets are limited to {} bytes in this phase", state.limits.max_asset_bytes)));
            }
            state.with_ws(&id, |ws| {
                let a = ws.require_ws_any(caller)?;
                use aiworkspace_core::access::Cap;
                if !a.ws_caps.has(Cap::Update) && !a.ws_caps.has(Cap::Structure) {
                    return Err(WsError::denied("update or structure capability required"));
                }
                Ok(())
            })?;
            begin_upload(state, "asset", Some(id), caller)?
        }
        "asset.finish_upload" => {
            let id = ws_id()?;
            let path = take_upload(state, s(p, "upload_id")?, "asset", caller)?;
            let data = std::fs::read(&path).map_err(|e| WsError::io(e.to_string()));
            let _ = std::fs::remove_file(&path);
            state.with_ws(id, |ws| ws.stage_asset(caller, &data?))?
        }
        "replica.bootstrap" => state.with_ws(ws_id()?, |ws| {
            let mut r = ws.replica_bootstrap(caller)?;
            r.as_object_mut().unwrap().remove("path");
            Ok(r)
        })?,
        "proc.start" => state.with_ws(ws_id()?, |ws| {
            ws.proc_start(caller, s(p, "program")?, p.get("params").unwrap_or(&Value::Null), s(p, "idempotency_key")?)
        })?,
        "proc.get" => state.with_ws(ws_id()?, |ws| ws.proc_get(caller, s(p, "run_id")?))?,
        "proc.apply" => state.with_ws(ws_id()?, |ws| ws.proc_apply(caller, s(p, "run_id")?, p.get("session_id").and_then(Value::as_str)))?,
        "proc.cancel" => state.with_ws(ws_id()?, |ws| ws.proc_cancel(caller, s(p, "run_id")?))?,
        "lock.acquire" => state.with_ws(ws_id()?, |ws| ws.lock_acquire(caller, &strings(p, "entity_ids")?, s(p, "session_id")?))?,
        "lock.renew" => state.with_ws(ws_id()?, |ws| ws.lock_renew(caller, &strings(p, "lock_ids")?))?,
        "lock.release" => state.with_ws(ws_id()?, |ws| ws.lock_release(caller, &strings(p, "lock_ids")?))?,
        "lock.break" => state.with_ws(ws_id()?, |ws| ws.lock_break(caller, s(p, "entity_id")?))?,
        "lock.list" => state.with_ws(ws_id()?, |ws| {
            let ids = p.get("entity_ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
            ws.lock_list(caller, ids.as_deref())
        })?,
        "diag.list_unretained" => state.with_ws(ws_id()?, |ws| ws.list_unretained(caller))?,
        "diag.verify_refs" => state.with_ws(ws_id()?, |ws| {
            ws.require_ws(caller, aiworkspace_core::access::Cap::Manage)?;
            ws.verify_refs()
        })?,
        _ => return Err(WsError::invalid_op(UNKNOWN_METHOD)),
    }))
}

/// `doc.wait_changes`: long poll; returns as soon as `head_seq > after_seq` or on timeout.
/// It only ever says "there is something new" — content comes from `get_changes`.
async fn wait_changes(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    let id = s(p, "workspace_id")?.to_string();
    let after = p.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
    let timeout = p.get("timeout_ms").and_then(Value::as_u64).unwrap_or(state.limits.max_wait_ms).min(state.limits.max_wait_ms);
    let (st, c, wid, epoch) = (state.clone(), caller.clone(), id.clone(), p.get("epoch").and_then(Value::as_str).map(str::to_string));
    let info = tokio::task::spawn_blocking(move || st.with_ws(&wid, |ws| ws.info(&c)))
        .await
        .map_err(|e| WsError::io(e.to_string()))??;
    if let Some(e) = &epoch {
        if info["epoch"] != json!(e) {
            return Err(WsError::new(Code::EpochMismatch, "the workspace history was replaced; resynchronize"));
        }
    }
    let mut rx = state.heads.lock().unwrap().get(&id).map(|tx| tx.subscribe()).ok_or_else(|| WsError::not_found("workspace not found"))?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
    loop {
        let head = *rx.borrow_and_update();
        if head > after {
            return Ok(json!({ "ok": true, "epoch": info["epoch"], "head_seq": head, "timed_out": false }));
        }
        match tokio::time::timeout_at(deadline, rx.changed()).await {
            Ok(Ok(())) => continue,
            _ => return Ok(json!({ "ok": true, "epoch": info["epoch"], "head_seq": head, "timed_out": true })),
        }
    }
}

// ---- uploads and downloads ----

fn begin_upload(state: &Arc<AppState>, kind: &'static str, workspace_id: Option<String>, caller: &Caller) -> WsResult<Value> {
    let upload_id = aiworkspace_store::workspace::random_id("up_");
    let path = state.staging_file(&format!("{upload_id}.part"));
    state.uploads.lock().unwrap().insert(upload_id.clone(), Upload { kind, workspace_id, principal: caller.principal.clone(), path, received: None });
    Ok(json!({ "ok": true, "upload_id": upload_id, "put": format!("{HTTP_PATH}/upload/{upload_id}") }))
}

/// Consume a finished upload of the expected kind belonging to the caller.
fn take_upload(state: &Arc<AppState>, upload_id: &str, kind: &str, caller: &Caller) -> WsResult<PathBuf> {
    let mut uploads = state.uploads.lock().unwrap();
    match uploads.get(upload_id) {
        Some(u) if u.kind == kind && u.principal == caller.principal && u.received.is_some() => Ok(uploads.remove(upload_id).unwrap().path),
        Some(u) if u.kind == kind && u.principal == caller.principal => Err(WsError::new(Code::DependencyUnavailable, "upload has not completed")),
        _ => Err(WsError::not_found("upload not found")),
    }
}

async fn bearer(state: &AppState, headers: &HeaderMap) -> Result<Caller, Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session token required").into_response())?;
    state.auth.authenticate(token).await.map_err(|e| (StatusCode::UNAUTHORIZED, e).into_response())
}

fn http_error(e: WsError) -> Response {
    let status = match e.code {
        Code::NotFound | Code::TargetDeleted => StatusCode::NOT_FOUND,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        Code::LimitExceeded => StatusCode::PAYLOAD_TOO_LARGE,
        Code::InvalidOperation | Code::InvalidSchema => StatusCode::BAD_REQUEST,
        Code::DependencyUnavailable => StatusCode::GONE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "ok": false, "error": e.to_json() }))).into_response()
}

/// `PUT /kapi/aiworkspace/upload/<upload_id>`: asset bytes or an import package, streamed to staging.
async fn upload(State(state): State<Arc<AppState>>, Path(upload_id): Path<String>, headers: HeaderMap, body: Body) -> Response {
    let caller = match bearer(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (path, limit) = {
        let uploads = state.uploads.lock().unwrap();
        match uploads.get(&upload_id) {
            Some(u) if u.principal == caller.principal && u.received.is_none() => {
                (u.path.clone(), if u.kind == "asset" { state.limits.max_asset_bytes } else { state.limits.max_package_bytes })
            }
            _ => return http_error(WsError::not_found("upload not found")),
        }
    };
    let result: Result<u64, WsError> = async {
        let mut file = tokio::fs::File::create(&path).await.map_err(|e| WsError::io(e.to_string()))?;
        let mut stream = body.into_data_stream();
        let mut total = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| WsError::io(format!("upload interrupted: {e}")))?;
            total += chunk.len() as u64;
            if total > limit {
                return Err(WsError::limit(format!("upload exceeds {limit} bytes")));
            }
            file.write_all(&chunk).await.map_err(|e| WsError::io(e.to_string()))?;
        }
        file.sync_all().await.map_err(|e| WsError::io(e.to_string()))?;
        Ok(total)
    }
    .await;
    match result {
        Ok(total) => {
            if let Some(u) = state.uploads.lock().unwrap().get_mut(&upload_id) {
                u.received = Some(total);
                let _ = &u.workspace_id;
            }
            Json(json!({ "ok": true, "upload_id": upload_id, "received": total })).into_response()
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            state.uploads.lock().unwrap().remove(&upload_id);
            http_error(e)
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> WsResult<T> + Send + 'static) -> WsResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| WsError::io(e.to_string()))?
}

/// `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>`: verified asset
/// bytes, authorized through an entity that references the asset.
async fn asset(State(state): State<Arc<AppState>>, Path((workspace_id, object_id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let caller = match bearer(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let st = state.clone();
    let result = blocking(move || {
        st.with_ws(&workspace_id, |ws| {
            if !ws.can_read_asset(&caller, &object_id)? {
                return Err(WsError::not_found("asset not found"));
            }
            let media = ws.asset_info(&object_id)?.map(|a| a.media_type).unwrap_or_else(|| "application/octet-stream".into());
            use aiworkspace_core::materialize::ObjectSource;
            Ok((media, ws.objects.get_file(&object_id)?)) // re-verified against its content id on every read
        })
    })
    .await;
    match result {
        Ok((media, data)) => {
            // user content is never served as active content from the service origin
            let safe = matches!(media.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "application/pdf" | "text/plain");
            let ctype = if safe { media } else { "application/octet-stream".to_string() };
            ([(header::CONTENT_TYPE, ctype), (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
              (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string())], data).into_response()
        }
        Err(e) => http_error(e),
    }
}

async fn serve_file(path: WsResult<PathBuf>, ctype: &'static str) -> Response {
    match path {
        Ok(p) => match tokio::fs::read(&p).await {
            Ok(data) => ([(header::CONTENT_TYPE, ctype)], data).into_response(),
            Err(e) => http_error(WsError::io(e.to_string())),
        },
        Err(e) => http_error(e),
    }
}

/// `GET /kapi/aiworkspace/export/<workspace_id>/<export_id>`
async fn download_export(State(state): State<Arc<AppState>>, Path((workspace_id, export_id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let caller = match bearer(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let st = state.clone();
    let path = blocking(move || {
        st.with_ws(&workspace_id, |ws| {
            ws.require_ws(&caller, aiworkspace_core::access::Cap::Export)?;
            ws.export_path(&export_id).ok_or_else(|| WsError::not_found("export not found"))
        })
    })
    .await;
    serve_file(path, "application/zip").await
}

/// `GET /kapi/aiworkspace/replica/<workspace_id>/<replica_id>`: the database built by `replica.bootstrap`.
async fn download_replica(State(state): State<Arc<AppState>>, Path((workspace_id, replica_id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let caller = match bearer(&state, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let st = state.clone();
    let path = blocking(move || {
        st.with_ws(&workspace_id, |ws| {
            let a = ws.require_ws_any(&caller)?;
            if !a.ws_caps.has(aiworkspace_core::access::Cap::Read) || !aiworkspace_core::id::is_prefixed_id("rp_", &replica_id) {
                return Err(WsError::not_found("replica not found"));
            }
            let p = ws.dir.join("staging").join("replicas").join(format!("{replica_id}.sqlite"));
            if p.exists() { Ok(p) } else { Err(WsError::not_found("replica not found")) }
        })
    })
    .await;
    serve_file(path, "application/vnd.sqlite3").await
}

pub async fn serve(state: Arc<AppState>, listen: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    log::info!("aiworkspace listening on http://{}{}", listener.local_addr()?, HTTP_PATH);
    axum::serve(listener, build_router(state)).await
}
