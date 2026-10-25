//! aiworkspace service: kRPC method dispatch plus the upload/download routes
//! (design doc §5). All store work is synchronous and runs on the blocking pool;
//! each Workspace has exactly one writer behind its mutex.

pub mod auth;
pub mod show;
pub mod source;
pub mod wish;

use aiworkspace_core::{Code, Deny, WsError, WsResult};
use aiworkspace_store::entries::{parse_entry_methods, split_entry_id, EntryState, ENTRY_DID_LOGIN, ENTRY_GUEST, ENTRY_PASSWORD, ENTRY_SELF_REGISTER};
use aiworkspace_store::service::require_may_own;
use aiworkspace_store::workspace::{CommitOpts, Workspace, CONTACT_PREFIX};
use aiworkspace_store::{Caller, CallerClass, Service};
use auth::{AuthError, Authenticator};
use buckyos_api::{AccessDenial, AuthMethod, DenyReason, EntryDescription, EntryMethods, EntryResource, SsoSettings, ACCESS_DENIED_ERROR_CODE};
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
    /// Logical bytes one Workspace may occupy (D19); `None` = no quota.
    pub workspace_quota_bytes: Option<u64>,
    /// Retention of history-only resources (D06), in milliseconds.
    pub history_retention_ms: i64,
    /// Bytes a Workspace's Sources may fetch (D24); `None` = unlimited.
    pub traffic_quota_bytes: Option<u64>,
    /// The Workspace's own origin (D21): `https://aiws-{workspace_id}.<zonehost>`; `None` = links are paths only.
    pub public_base: Option<String>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_commit_bytes: 8 * 1024 * 1024, max_wait_ms: 30_000, max_asset_bytes: 32 * 1024 * 1024, max_package_bytes: 2 << 30,
                 workspace_quota_bytes: None, history_retention_ms: aiworkspace_store::workspace::HISTORY_RETENTION_MS, traffic_quota_bytes: None, public_base: None }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Upload {
    kind: String,
    workspace_id: Option<String>,
    principal: String,
    path: PathBuf,
    received: Option<u64>,
    expires_at: i64,
    result: Option<Value>,
    finish_params: Option<Value>,
    #[serde(skip)]
    writing: bool,
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
    pub wish: Arc<wish::WishRuntime>,
    /// Non-public shows (third phase §11.3): in memory, ended by a restart.
    pub shows: show::Shows,
    heads: Arc<Mutex<HashMap<String, watch::Sender<u64>>>>,
    uploads: Mutex<HashMap<String, Upload>>,
    pub named_import: tokio::sync::OnceCell<(named_store::NamedDataMgr, String)>,
}

impl AppState {
    /// `wake` receives hints for an external event channel (kevent in system mode).
    pub fn new(mut svc: Service, auth: Arc<dyn Authenticator>, limits: Limits, wake: Option<tokio::sync::mpsc::UnboundedSender<Wake>>, wish: wish::WishConfig) -> Arc<AppState> {
        svc.quota = limits.workspace_quota_bytes;
        svc.history_retention_ms = limits.history_retention_ms;
        svc.traffic_quota = limits.traffic_quota_bytes;
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
        let mut uploads = HashMap::new();
        let staging = svc.data_dir.join("staging");
        if let Ok(entries) = std::fs::read_dir(&staging) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(id) = path.file_name().and_then(|s| s.to_str()).and_then(|s| s.strip_suffix(".upload.json")) else { continue };
                if !aiworkspace_core::id::is_prefixed_id("up_", id) { continue; }
                if let Some(upload) = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Upload>(&bytes).ok()) {
                    if upload.path != staging.join(format!("{id}.part")) { continue; }
                    if upload.expires_at <= (svc.clock)() {
                        let _ = std::fs::remove_file(&upload.path);
                        let _ = std::fs::remove_file(&path);
                    } else {
                        uploads.insert(id.to_string(), upload);
                    }
                }
            }
        }
        Arc::new(AppState { svc, auth, limits, wish: wish::WishRuntime::new(wish), shows: show::Shows::default(), heads, uploads: Mutex::new(uploads), named_import: tokio::sync::OnceCell::new() })
    }

    pub fn with_ws<T>(&self, id: &str, f: impl FnOnce(&mut Workspace) -> WsResult<T>) -> WsResult<T> {
        let handle = self.svc.workspace(id)?;
        let mut guard = match handle.lock() {
            Ok(g) => g,
            Err(_) => {
                // a panic left this writer in an unknown state: discard it and rebuild from the database
                self.svc.close(id);
                return Err(WsError::new(Code::WriterBusy, "workspace writer was reset; retry"));
            }
        };
        if !self.svc.is_current_workspace(id, &handle) {
            return Err(WsError::new(Code::WriterBusy, "workspace writer was replaced; retry"));
        }
        f(&mut guard)
    }

    fn staging_file(&self, name: &str) -> PathBuf {
        self.svc.data_dir.join("staging").join(name)
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let weak = Arc::downgrade(&state);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut ticks: u64 = 0;
        loop {
            timer.tick().await;
            let Some(state) = weak.upgrade() else { break };
            sweep_uploads(&state);
            ticks += 1;
            if ticks % 5 == 0 {
                // Sources on a schedule (D24)
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || source::tick(&st)).await;
            }
            if ticks % 60 == 0 {
                // the collector runs hourly (D06)
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || match st.svc.gc(false) {
                    Ok(r) => log::info!("gc: {r}"),
                    Err(e) => log::warn!("gc failed: {e}"),
                })
                .await;
            }
        }
    });
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
        .route(&format!("{HTTP_PATH}/wish-host/{{token}}/llm_map"), post(wish_llm_map))
        // publications (D26): by path on the service, and on the Workspace's own host (D21)
        .route(&format!("{HTTP_PATH}/pub/{{workspace_id}}/{{name}}"), get(source::pub_get))
        .route(&format!("{HTTP_PATH}/pub/{{workspace_id}}/{{name}}/referrers"), post(source::pub_referrer))
        .route("/", get(source::host_root))
        .route("/{workspace_name}", get(source::host_workspace_page))
        .route("/{workspace_name}/{name}", get(source::host_path))
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

/// A session this service refuses (D-4): the kRPC failure with `code: access_denied` and the denial as `data`.
fn rpc_denial(seq: u64, denial: &AccessDenial) -> Response {
    Json(json!({ "error": format!("access denied: {}", denial.reason.as_str()), "code": ACCESS_DENIED_ERROR_CODE, "data": denial, "sys": [seq] }))
        .into_response()
}

fn deny_reason(d: Deny) -> DenyReason {
    match d {
        Deny::SessionKindNotAllowed => DenyReason::SessionKindNotAllowed,
        Deny::ResourceGrantMissing => DenyReason::ResourceGrantMissing,
        Deny::EntryUnavailable => DenyReason::EntryUnavailable,
        Deny::ZoneMethodDisabled => DenyReason::ZoneMethodDisabled,
        Deny::AuthorizationUnavailable => DenyReason::AuthorizationUnavailable,
    }
}

fn class_name(caller: &Caller) -> &'static str {
    match caller.class {
        CallerClass::Account { restricted: false, .. } => "account",
        CallerClass::Account { restricted: true, .. } => "limited",
        CallerClass::External { .. } => "external",
        CallerClass::Guest => "guest",
        CallerClass::Service => "service",
    }
}

/// Give a business failure (`{ code, ..., denial?: { reason } }`) its D-4 form: `denial` becomes the full
/// `AccessDenial` (reason, who adjusts it, where). NOT_FOUND and PERMISSION_DENIED without a reason of their own are
/// `resource_grant_missing`; for a restricted caller their detail and extras are dropped, so a missing Workspace and
/// one not shared with it give the same answer (PERMISSION_DENIED only arises where it already holds a grant).
fn decorate_error(error: &mut Value, caller: Option<&Caller>, method: &str) {
    let code = error["code"].as_str().and_then(|c| Code::ALL.iter().copied().find(|x| x.as_str() == c));
    let explicit = error["denial"]["reason"].as_str().and_then(Deny::parse);
    let reason = match (explicit, code) {
        (Some(d), _) => d,
        (None, Some(Code::NotFound | Code::PermissionDenied)) if error.get("sub_code").is_none() => Deny::ResourceGrantMissing,
        _ => return,
    };
    let denial = AccessDenial::new(deny_reason(reason)).at("aiworkspace", method);
    if let Some(c) = caller {
        log::info!("denied {method} to {} ({}): {}", c.principal, class_name(c), denial.reason.as_str());
    }
    error["denial"] = serde_json::to_value(&denial).unwrap_or(Value::Null);
    if reason == Deny::ResourceGrantMissing && caller.is_some_and(Caller::is_restricted) {
        if let Some(o) = error.as_object_mut() {
            o.remove("data");
            o.remove("path");
            o.insert("detail".into(), json!("not found, or not shared with you"));
        }
    }
}

/// The failures a result carries inside (`error` of `{ ok: false }` / a rejected commit, `results[].error`).
fn decorate_result(v: &mut Value, caller: Option<&Caller>, method: &str) {
    if v.get("error").is_some_and(Value::is_object) {
        decorate_error(&mut v["error"], caller, method);
    }
    if let Some(items) = v.get_mut("results").and_then(Value::as_array_mut) {
        for item in items {
            if item.get("error").is_some_and(Value::is_object) {
                decorate_error(&mut item["error"], caller, method);
            }
        }
    }
}

fn failure_json(e: &WsError, caller: Option<&Caller>, method: &str) -> Value {
    let mut v = e.to_json();
    decorate_error(&mut v, caller, method);
    v
}

/// Settle the `group:` / `contact:` memberships the request can depend on, before any Workspace is locked: those
/// of the Workspace named in `params` (every Workspace for `ws.list`).
pub(crate) async fn prepare_caller(state: &Arc<AppState>, caller: &mut Caller, method: &str, workspace_id: Option<&str>) {
    if !matches!(caller.class, CallerClass::Account { .. } | CallerClass::External { .. }) {
        return;
    }
    let st = state.clone();
    let id = workspace_id.map(str::to_string);
    let list = method == "ws.list";
    let subjects = blocking(move || match (list, id) {
        (true, _) => st.svc.dynamic_subjects(),
        (false, Some(id)) => st.with_ws(&id, |ws| ws.dynamic_subjects()),
        (false, None) => Ok(Vec::new()),
    })
    .await
    .unwrap_or_default();
    if !subjects.is_empty() {
        state.auth.resolve_subjects(caller, &subjects).await;
    }
}

async fn krpc(State(state): State<Arc<AppState>>, body: bytes::Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(0, &format!("invalid request: {e}")),
    };
    let seq = req["sys"][0].as_u64().unwrap_or(0);
    let Some(method) = req["method"].as_str().map(str::to_string) else { return rpc_error(seq, "method required") };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    // public: the entry page describes a link before anyone signs in
    if method == "entry.describe" {
        return match describe_entry(&state, &params).await {
            Ok(v) => Json(json!({ "result": v, "sys": [seq] })).into_response(),
            Err(e) => Json(json!({ "result": { "ok": false, "error": failure_json(&e, None, &method) }, "sys": [seq] })).into_response(),
        };
    }
    // a prompter link carries the show's token instead of a session (third phase §11.1)
    let prompter = if method.starts_with("show.") { show::prompter_token(&params) } else { None };
    let mut caller = match (prompter.is_some(), req["sys"][1].as_str().filter(|t| !t.trim().is_empty())) {
        (true, _) => None,
        (false, None) => return rpc_error(seq, "session token required"),
        (false, Some(token)) if token.starts_with("rt_") => {
            // a run token (M3): the run's execution identity, for that run only
            let ws_id = params.get("workspace_id").and_then(Value::as_str).map(str::to_string);
            match run_token_caller(&state, ws_id.as_deref(), token, &method) {
                Ok(c) => Some(c),
                Err(e) => return rpc_error(seq, &e),
            }
        }
        (false, Some(token)) => match Caller::guest(token) {
            // a link's visitor: a guest token instead of a session (reaches `public` grants and guest entries only)
            Some(guest) => Some(guest),
            None => match state.auth.authenticate(token).await {
                Ok(c) => Some(c),
                Err(AuthError::Invalid(e)) => return rpc_error(seq, &e),
                Err(AuthError::Denied(d)) => {
                    log::info!("refused {method}: {}", d.reason.as_str());
                    return rpc_denial(seq, &d.at("aiworkspace", method.clone()));
                }
            },
        },
    };
    if let Some(c) = caller.as_mut() {
        // the Workspace the request is about: named, or the one an entry id names
        let workspace_id = params
            .get("workspace_id")
            .and_then(Value::as_str)
            .or_else(|| params.get("entry_id").and_then(Value::as_str).and_then(split_entry_id).map(|(ws, _)| ws));
        prepare_caller(&state, c, &method, workspace_id).await;
    }
    // naming a contact group in a grant is for its owner: the store compares the group's owner with the caller's DID
    let mut ready: WsResult<()> = Ok(());
    if method == "ws.grant" && params.get("subject").and_then(Value::as_str).is_some_and(|s| s.starts_with(CONTACT_PREFIX)) {
        if let Some(c) = caller.as_mut() {
            ready = state.auth.load_did(c).await.map_err(|_| {
                WsError::refused(Code::PermissionDenied, Deny::AuthorizationUnavailable, "the caller's DID could not be read; retry later")
            });
        }
    }
    let result = if let Err(e) = ready {
        Err(e)
    } else if let Some(token) = prompter {
        show::handle(&state, &method, &params, show::Auth::Token(token)).await
    } else if method.starts_with("show.") {
        show::handle(&state, &method, &params, show::Auth::Session(caller.clone().expect("authenticated above"))).await
    } else if method == "proc.tool" {
        let ws = params.get("workspace_id").and_then(Value::as_str).map(str::to_string);
        match ws {
            Some(ws) => wish::exec::tool(&state, caller.as_ref().expect("authenticated above"), &ws, &params).await,
            None => Err(WsError::invalid_op("workspace_id required")),
        }
    } else if method == "doc.wait_changes" {
        wait_changes(&state, &params, caller.as_ref().expect("authenticated above")).await
    } else if method == "ws.join_entry" {
        join_entry(&state, &params, caller.as_ref().expect("authenticated above")).await
    } else if method == "asset.import_object" {
        import_asset_object(&state, &params, caller.as_ref().expect("authenticated above"), req["sys"][1].as_str().unwrap_or_default()).await
    } else {
        let c = caller.clone().expect("authenticated above");
        let st = state.clone();
        let m = method.clone();
        match tokio::task::spawn_blocking(move || dispatch(&st, &m, &params, &c)).await {
            Ok(r) => r,
            Err(e) => Err(WsError::io(format!("handler panicked: {e}"))),
        }
    };
    match result {
        Ok(mut v) => {
            decorate_result(&mut v, caller.as_ref(), &method);
            Json(json!({ "result": v, "sys": [seq] })).into_response()
        }
        Err(e) if e.detail == UNKNOWN_METHOD => rpc_error(seq, &format!("unknown method {method}")),
        // the entry page (a sign-in flow, not the app) reads the shared kRPC wire: a refusal is `access_denied`
        Err(e) if method == "ws.join_entry" => {
            let error = failure_json(&e, caller.as_ref(), &method);
            match serde_json::from_value::<AccessDenial>(error["denial"].clone()) {
                Ok(denial) => rpc_denial(seq, &denial),
                Err(_) => rpc_error(seq, &e.to_string()),
            }
        }
        // business failures are results, not kRPC errors: clients branch on `error.code` (and `error.denial.reason`)
        Err(e) => Json(json!({ "result": { "ok": false, "error": failure_json(&e, caller.as_ref(), &method) }, "sys": [seq] })).into_response(),
    }
}

pub(crate) const UNKNOWN_METHOD: &str = "\u{0}unknown-method";

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

/// Every kRPC method and its level (内核架构改进 D11): `stable` methods change only with the protocol
/// version and each has a contract test (`server/tests/contract.rs`, `fixtures/protocol/`); `internal`
/// methods serve the Desktop and diagnostics and may change with it; `experimental` methods are not
/// frozen yet (`proc.*` become stable with M3). The README method table mirrors this list.
pub const METHOD_LEVELS: &[(&str, &str)] = &[
    ("ws.create", "stable"),
    ("ws.list", "stable"),
    ("ws.get_info", "stable"),
    ("ws.delete", "stable"),
    ("ws.fork", "stable"),
    ("ws.grant", "stable"),
    ("ws.revoke", "stable"),
    ("ws.list_grants", "stable"),
    ("ws.list_subjects", "stable"),
    ("ws.create_entry", "stable"),
    ("ws.list_entries", "stable"),
    ("ws.close_entry", "stable"),
    ("ws.join_entry", "stable"),
    ("entry.describe", "stable"),
    ("ws.get_user_state", "stable"),
    ("ws.set_user_state", "stable"),
    ("ws.set_wish_aicc", "stable"),
    ("ws.begin_import", "stable"),
    ("ws.import", "stable"),
    ("ws.release_retention", "stable"),
    ("ws.delete_checkpoint", "stable"),
    ("ws.purge_deleted", "stable"),
    ("ws.clear_staged", "stable"),
    ("doc.resolve", "stable"),
    ("doc.read", "stable"),
    ("doc.query", "stable"),
    ("doc.prepare", "stable"),
    ("doc.commit", "stable"),
    ("doc.get_submission", "stable"),
    ("doc.undo", "stable"),
    ("doc.get_changes", "stable"),
    ("doc.wait_changes", "stable"),
    ("asset.begin_upload", "stable"),
    ("asset.finish_upload", "stable"),
    ("asset.import_object", "stable"),
    ("lock.acquire", "stable"),
    ("lock.renew", "stable"),
    ("lock.release", "stable"),
    ("lock.break", "stable"),
    ("lock.list", "stable"),
    ("doc.outline", "internal"),
    ("doc.source_capabilities", "internal"),
    ("doc.get_collab_state", "internal"),
    ("replica.bootstrap", "internal"),
    ("show.start", "internal"),
    ("show.heartbeat", "internal"),
    ("show.publish", "internal"),
    ("show.command", "internal"),
    ("show.watch", "internal"),
    ("show.notes", "internal"),
    ("show.end", "internal"),
    ("diag.list_unretained", "internal"),
    ("diag.verify_refs", "internal"),
    ("diag.gc", "internal"),
    ("doc.list_children", "experimental"),
    ("doc.list_annotations", "experimental"),
    ("doc.freshness", "experimental"),
    ("doc.relations", "experimental"),
    ("doc.list_versions", "experimental"),
    ("doc.restore_version", "experimental"),
    ("doc.checkpoint", "experimental"),
    ("doc.export", "experimental"),
    ("proc.start", "experimental"),
    ("proc.get", "experimental"),
    ("proc.apply", "experimental"),
    ("proc.cancel", "experimental"),
    ("proc.list", "experimental"),
    ("proc.claim", "experimental"),
    ("proc.heartbeat", "experimental"),
    ("proc.report", "experimental"),
    ("proc.tool", "experimental"),
    ("proc.program", "experimental"),
    ("proc.files", "experimental"),
    ("ws.set_hosted_exec", "experimental"),
    ("ws.publish", "experimental"),
    ("ws.unpublish", "experimental"),
    ("ws.list_publications", "experimental"),
    ("ws.list_referrers", "experimental"),
];

/// Synchronous method dispatch. Every method authorizes against the grants of
/// the trusted `caller`.
fn dispatch(state: &Arc<AppState>, method: &str, p: &Value, caller: &Caller) -> WsResult<Value> {
    let svc = &state.svc;
    let ws_id = || s(p, "workspace_id");
    Ok(okv(match method {
        "ws.create" => svc.create_workspace(caller, p.get("title").and_then(Value::as_str).unwrap_or(""), p.get("workspace_id").and_then(Value::as_str))?,
        "ws.list" => svc.list_workspaces(caller)?,
        "ws.get_info" => {
            let mut v = state.with_ws(ws_id()?, |ws| ws.info(caller))?;
            v["wish_execution"] = json!({ "default": state.wish.config.default_execution, "service": state.wish.config.allow_service_execution });
            v["public_base"] = json!(source::public_base(state, ws_id()?));
            v["traffic"] = json!(state.with_ws(ws_id()?, |ws| ws.traffic())?);
            v
        }
        "ws.delete" => svc.delete_workspace(caller, ws_id()?)?,
        "ws.grant" => state.with_ws(ws_id()?, |ws| {
            ws.grant_with(caller, s(p, "subject")?, p.get("scope_entity_id").and_then(Value::as_str), &strings(p, "capabilities")?,
                          p.get("task_digest").and_then(Value::as_str), p.get("outputs").filter(|o| !o.is_null()))
        })?,
        "ws.revoke" => state.with_ws(ws_id()?, |ws| ws.revoke(caller, s(p, "subject")?, p.get("scope_entity_id").and_then(Value::as_str)))?,
        "ws.list_grants" => state.with_ws(ws_id()?, |ws| ws.list_grants(caller))?,
        "ws.get_user_state" => state.with_ws(ws_id()?, |ws| ws.get_user_state(caller))?,
        "ws.set_user_state" => state.with_ws(ws_id()?, |ws| ws.set_user_state(caller, p.get("entries").unwrap_or(&Value::Null), p.get("expect")))?,
        "ws.set_wish_aicc" => state.with_ws(ws_id()?, |ws| ws.set_wish_aicc_mode(caller, s(p, "mode")?))?,
        "ws.list_subjects" => {
            // zone-level: any account that may own Workspaces may ask (the list itself is as public as the zone makes it)
            require_may_own(caller)?;
            let subjects = tokio::runtime::Handle::current().block_on(state.auth.subjects());
            json!({ "ok": true, "subjects": subjects })
        }
        "ws.fork" => svc.fork(caller, ws_id()?)?,
        "ws.begin_import" => {
            require_may_own(caller)?;
            begin_upload(state, "import", None, caller)?
        }
        "ws.import" => {
            require_may_own(caller)?;
            if !matches!(s(p, "semantics")?, "new" | "restore") {
                return Err(WsError::invalid_op("semantics must be \"restore\" or \"new\""));
            }
            finish_upload(state, s(p, "upload_id")?, "import", None, caller, p, |path| {
                svc.import_upload(caller, path, s(p, "semantics")?, p.get("replace").and_then(Value::as_bool).unwrap_or(false), s(p, "upload_id")?)
            })?
        }
        "ws.create_entry" => state.with_ws(ws_id()?, |ws| {
            let caps = strings(p, "capabilities")?;
            let methods = parse_entry_methods(p.get("methods").unwrap_or(&Value::Null))?;
            let expires_at = match p.get("expires_at") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_i64().ok_or_else(|| WsError::invalid_op("expires_at must be unix seconds"))?),
            };
            ws.create_entry(caller, p.get("scope_entity_id").and_then(Value::as_str), &caps, methods, expires_at)
        })?,
        "ws.list_entries" => state.with_ws(ws_id()?, |ws| ws.list_entries(caller))?,
        "ws.close_entry" => {
            let (ws, local) = entry_of(p)?;
            state.with_ws(ws, |w| w.close_entry(caller, local))?
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
        "doc.list_annotations" => state.with_ws(ws_id()?, |ws| {
            let targets = match p.get("target_ids") {
                None | Some(Value::Null) => Vec::new(),
                Some(_) => strings(p, "target_ids")?,
            };
            ws.list_annotations(caller, &targets, p.get("parent_id").and_then(Value::as_str))
        })?,
        "doc.freshness" => state.with_ws(ws_id()?, |ws| ws.freshness(caller, &strings(p, "entity_ids")?))?,
        "doc.relations" => state.with_ws(ws_id()?, |ws| ws.relations(caller, s(p, "entity_id")?))?,
        "doc.list_versions" => state.with_ws(ws_id()?, |ws| ws.list_versions(caller, s(p, "entity_id")?))?,
        "doc.restore_version" => state.with_ws(ws_id()?, |ws| {
            ws.restore_version_plan(caller, s(p, "entity_id")?, p.get("content_rev").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("content_rev required"))?)
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
                                  p.get("self_contained").and_then(Value::as_bool).unwrap_or(true), p.get("include_notes").and_then(Value::as_bool))?;
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
            state.with_ws(id, |ws| {
                let access = ws.require_ws_any(caller)?;
                use aiworkspace_core::access::Cap;
                if !access.ws_caps.has(Cap::Update) && !access.ws_caps.has(Cap::Structure) {
                    return Err(WsError::denied("update or structure capability required"));
                }
                Ok(())
            })?;
            finish_upload(state, s(p, "upload_id")?, "asset", Some(id), caller, p, |path| {
                let data = std::fs::read(path).map_err(|e| WsError::io(e.to_string()))?;
                state.with_ws(id, |ws| ws.stage_asset(caller, &data))
            })?
        }
        "replica.bootstrap" => state.with_ws(ws_id()?, |ws| {
            let mut r = ws.replica_bootstrap(caller)?;
            r.as_object_mut().unwrap().remove("path");
            Ok(r)
        })?,
        "proc.start" if s(p, "program")? == aiworkspace_store::source::SOURCE_PROGRAM => {
            source::start(state, caller, ws_id()?, p.get("params").unwrap_or(&Value::Null), s(p, "idempotency_key")?)?
        }
        "ws.publish" => {
            let id = ws_id()?;
            let mut v = state.with_ws(id, |ws| ws.publish_entity(caller, s(p, "entity_id")?, p.get("name").and_then(Value::as_str)))?;
            if let Some(base) = source::public_base(state, id) {
                v["url"] = json!(format!("{base}{}", v["path"].as_str().unwrap_or("")));
            }
            v
        }
        "ws.unpublish" => state.with_ws(ws_id()?, |ws| ws.unpublish(caller, s(p, "entity_id")?))?,
        "ws.list_publications" => {
            let id = ws_id()?;
            let mut v = state.with_ws(id, |ws| ws.list_publications(caller))?;
            if let Some(base) = source::public_base(state, id) {
                for item in v["publications"].as_array_mut().into_iter().flatten() {
                    item["url"] = json!(format!("{base}{}", item["path"].as_str().unwrap_or("")));
                }
            }
            v
        }
        "ws.list_referrers" => state.with_ws(ws_id()?, |ws| ws.list_referrers(caller, s(p, "entity_id")?))?,
        "proc.start" if wish::is_wish(s(p, "program")?) => {
            wish::start(state, caller, ws_id()?, s(p, "program")?, p.get("params").unwrap_or(&Value::Null), s(p, "idempotency_key")?)?
        }
        "proc.start" => state.with_ws(ws_id()?, |ws| {
            ws.proc_start(caller, s(p, "program")?, p.get("params").unwrap_or(&Value::Null), s(p, "idempotency_key")?)
        })?,
        "proc.get" if is_wish_run(state, ws_id()?, s(p, "run_id")?) => wish::get(state, caller, ws_id()?, s(p, "run_id")?, p.get("choices").unwrap_or(&Value::Null))?,
        "proc.get" => state.with_ws(ws_id()?, |ws| ws.proc_get(caller, s(p, "run_id")?))?,
        "proc.apply" if is_wish_run(state, ws_id()?, s(p, "run_id")?) => wish::apply(state, caller, ws_id()?, p)?,
        "proc.apply" => state.with_ws(ws_id()?, |ws| ws.proc_apply(caller, s(p, "run_id")?, p.get("session_id").and_then(Value::as_str)))?,
        "proc.cancel" if is_wish_run(state, ws_id()?, s(p, "run_id")?) => wish::cancel(state, caller, ws_id()?, s(p, "run_id")?)?,
        "proc.cancel" => state.with_ws(ws_id()?, |ws| ws.proc_cancel(caller, s(p, "run_id")?))?,
        "proc.list" => wish::list(state, caller, ws_id()?, p)?,
        "proc.claim" => wish::exec::claim(state, caller, ws_id()?, p)?,
        "proc.heartbeat" => wish::exec::heartbeat(state, caller, ws_id()?, p)?,
        "proc.report" => wish::exec::report(state, caller, ws_id()?, p)?,
        "proc.program" => wish::exec::program(state, caller, ws_id()?, p)?,
        "proc.files" => wish::exec::files(state, caller, ws_id()?, p)?,
        "ws.set_hosted_exec" => state.with_ws(ws_id()?, |ws| ws.set_hosted_exec(caller, p.get("config").unwrap_or(&Value::Null)))?,
        "lock.acquire" => state.with_ws(ws_id()?, |ws| ws.lock_acquire(caller, &strings(p, "entity_ids")?, s(p, "session_id")?))?,
        "lock.renew" => state.with_ws(ws_id()?, |ws| ws.lock_renew(caller, &strings(p, "lock_ids")?))?,
        "lock.release" => state.with_ws(ws_id()?, |ws| ws.lock_release(caller, &strings(p, "lock_ids")?))?,
        "lock.break" => state.with_ws(ws_id()?, |ws| ws.lock_break(caller, s(p, "entity_id")?))?,
        "lock.list" => state.with_ws(ws_id()?, |ws| {
            let ids = p.get("entity_ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
            ws.lock_list(caller, ids.as_deref())
        })?,
        // the maintenance channel (D19): `manage`, accepted over quota
        "ws.release_retention" => state.with_ws(ws_id()?, |ws| {
            ws.require_ws(caller, aiworkspace_core::access::Cap::Manage)?;
            Ok(json!({ "ok": true, "released": ws.release_retention(s(p, "kind")?, s(p, "ref")?)? }))
        })?,
        "ws.delete_checkpoint" => state.with_ws(ws_id()?, |ws| ws.delete_checkpoint(caller, s(p, "snapshot_id")?))?,
        "ws.purge_deleted" => state.with_ws(ws_id()?, |ws| {
            let ids = match p.get("entity_ids") {
                None | Some(Value::Null) => None,
                Some(_) => Some(strings(p, "entity_ids")?),
            };
            ws.purge_deleted(caller, ids.as_deref(), p.get("session_id").and_then(Value::as_str))
        })?,
        "ws.clear_staged" => state.with_ws(ws_id()?, |ws| ws.clear_staged(caller))?,
        "diag.gc" => {
            require_may_own(caller)?;
            svc.gc(p.get("dry_run").and_then(Value::as_bool).unwrap_or(true))?
        }
        "diag.list_unretained" => state.with_ws(ws_id()?, |ws| ws.list_unretained(caller))?,
        "diag.verify_refs" => state.with_ws(ws_id()?, |ws| {
            ws.require_ws(caller, aiworkspace_core::access::Cap::Manage)?;
            ws.verify_refs()
        })?,
        _ => return Err(WsError::invalid_op(UNKNOWN_METHOD)),
    }))
}

/// Methods a run token may call: the run's own lifecycle and reads of the Workspace it runs in.
const RUN_TOKEN_METHODS: &[&str] = &["proc.get", "proc.heartbeat", "proc.report", "proc.tool", "proc.program", "proc.files", "proc.list", "ws.get_info", "doc.read", "doc.query", "doc.resolve", "doc.outline", "doc.list_children", "asset.begin_upload", "asset.finish_upload"];

fn run_token_caller(state: &Arc<AppState>, ws_id: Option<&str>, token: &str, method: &str) -> Result<Caller, String> {
    if !RUN_TOKEN_METHODS.contains(&method) {
        return Err(format!("a run token cannot call {method}"));
    }
    let ws_id = ws_id.ok_or_else(|| "workspace_id required".to_string())?;
    let found = state.with_ws(ws_id, |ws| ws.wish_run_by_token(token)).map_err(|e| e.to_string())?;
    let (run, _) = found.ok_or_else(|| "invalid run token".to_string())?;
    Ok(Caller::run_token(run["principal"].as_str().unwrap_or(""), run["run_id"].as_str().unwrap_or("")))
}

/// `entry_id` of `params` as (Workspace, entry within it); a `workspace_id` given beside it must name the same one.
fn entry_of(p: &Value) -> WsResult<(&str, &str)> {
    let not_found = || WsError::not_found("entry not found");
    let (ws, local) = split_entry_id(s(p, "entry_id")?).ok_or_else(not_found)?;
    match p.get("workspace_id").and_then(Value::as_str) {
        Some(w) if w != ws => Err(not_found()),
        _ => Ok((ws, local)),
    }
}

fn entry_methods(bits: u8) -> EntryMethods {
    EntryMethods {
        guest: bits & ENTRY_GUEST != 0,
        password: bits & ENTRY_PASSWORD != 0,
        self_register: bits & ENTRY_SELF_REGISTER != 0,
        did_login: bits & ENTRY_DID_LOGIN != 0,
    }
}

/// `entry.describe { entry_id }` (public, no session): what the entry page may show before anyone signs in —
/// the Workspace's title, its owner and the entry's own methods (the Control Panel intersects them with the Zone
/// switches). A missing, closed or expired entry is `state: unavailable` and nothing else.
async fn describe_entry(state: &Arc<AppState>, p: &Value) -> WsResult<Value> {
    let entry_id = s(p, "entry_id")?.to_string();
    let st = state.clone();
    let description = blocking(move || {
        let Some((ws_id, local)) = split_entry_id(&entry_id) else { return Ok(EntryDescription::unavailable()) };
        let found = st.with_ws(ws_id, |ws| {
            if ws.purpose().is_some() {
                return Ok(None);
            }
            Ok(ws.entry(local)?.filter(|e| e.state == EntryState::Active).map(|e| (e, ws.title(), ws.owner().ok().flatten())))
        });
        Ok(match found {
            Ok(Some((entry, title, owner))) => EntryDescription {
                state: buckyos_api::EntryState::Active,
                resource: Some(EntryResource { kind: "workspace".into(), title, owner_name: owner, path: format!("/workspace/{ws_id}") }),
                methods: entry_methods(entry.methods),
                expires_at: entry.expires_at.map(|e| e.max(0) as u64),
            },
            _ => EntryDescription::unavailable(),
        })
    })
    .await?;
    let mut v = serde_json::to_value(&description).map_err(|e| WsError::io(e.to_string()))?;
    v["ok"] = json!(true);
    Ok(v)
}

/// Whether an entry offering `bits` admits a session that signed in by `method`: the entry's methods ∩ the Zone
/// switches. Refused because the Zone closed the method → `zone_method_disabled`, else `entry_unavailable`.
fn entry_admits(bits: u8, settings: &SsoSettings, method: Option<AuthMethod>) -> WsResult<()> {
    let offered = entry_methods(bits);
    let (effective, _) = offered.effective(settings);
    if effective.allows_session(method) {
        Ok(())
    } else if offered.allows_session(method) {
        Err(WsError::refused(Code::PermissionDenied, Deny::ZoneMethodDisabled, "the Zone turned this way of signing in off"))
    } else {
        Err(WsError::refused(Code::PermissionDenied, Deny::EntryUnavailable, "the entry does not accept this way of signing in"))
    }
}

/// `ws.join_entry { entry_id }`: a signed-in visitor takes the entry's template as its own grant.
async fn join_entry(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    let unavailable = || WsError::refused(Code::NotFound, Deny::EntryUnavailable, "the entry is closed, expired or does not exist");
    let entry_id = s(p, "entry_id")?.to_string();
    let (ws_id, local) = split_entry_id(&entry_id).map(|(w, l)| (w.to_string(), l.to_string())).ok_or_else(unavailable)?;
    let settings = state.auth.sso_settings().await.map_err(|_| {
        WsError::refused(Code::PermissionDenied, Deny::AuthorizationUnavailable, "the Zone's sign-in settings could not be read; retry later")
    })?;
    let method = match caller.class {
        CallerClass::External { .. } => Some(AuthMethod::DidFederation),
        _ => caller.auth_method.as_deref().and_then(AuthMethod::parse),
    };
    let (st, c) = (state.clone(), caller.clone());
    let joined = blocking(move || st.with_ws(&ws_id, |ws| ws.join_entry(&c, &local, |bits| entry_admits(bits, &settings, method)))).await;
    let mut v = joined.map_err(|e| if e.code == Code::NotFound && e.denial.is_none() { unavailable() } else { e })?;
    if let Some(ws) = v["workspace_id"].as_str() {
        v["path"] = json!(format!("/workspace/{ws}"));
    }
    log::info!("{} ({}) joined {}", caller.principal, class_name(caller), v["entry_id"]);
    Ok(v)
}

/// Whether a run is a wish run (those are served by `wish::*`).
fn is_wish_run(state: &Arc<AppState>, ws_id: &str, run_id: &str) -> bool {
    state.with_ws(ws_id, |ws| Ok(ws.run_program(run_id))).ok().flatten().is_some_and(|p| wish::is_wish(&p))
}

/// `POST /kapi/aiworkspace/wish-host/<token>/llm_map`: a running wish program asks for per-item model
/// judgements. The token names the stage (and proves it); nothing else is reachable through it.
async fn wish_llm_map(State(state): State<Arc<AppState>>, Path(token): Path<String>, headers: HeaderMap, body: bytes::Bytes) -> Response {
    let Some(stage) = state.wish.stage_by_token(&token) else { return (StatusCode::NOT_FOUND, Json(json!({ "error": "no running stage" }))).into_response() };
    if headers.get("x-aiws-token").and_then(|v| v.to_str().ok()) != Some(token.as_str()) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "token mismatch" }))).into_response();
    }
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("invalid body: {e}") }))).into_response(),
    };
    Json(wish::llm::llm_map(&stage, &req).await).into_response()
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
        // the show lock rides along (第三期规划 §8.1): followers learn that writes are closed, and when they reopen
        let show_lock = || state.svc.show_locks.lock().unwrap().get(&id).filter(|l| l.expires_ms > (state.svc.clock)()).map(|l| l.to_json());
        if head > after {
            return Ok(json!({ "ok": true, "epoch": info["epoch"], "head_seq": head, "timed_out": false, "show_lock": show_lock() }));
        }
        match tokio::time::timeout_at(deadline, rx.changed()).await {
            Ok(Ok(())) => continue,
            _ => return Ok(json!({ "ok": true, "epoch": info["epoch"], "head_seq": head, "timed_out": true, "show_lock": show_lock() })),
        }
    }
}

// ---- uploads and downloads ----

fn begin_upload(state: &Arc<AppState>, kind: &'static str, workspace_id: Option<String>, caller: &Caller) -> WsResult<Value> {
    let upload_id = aiworkspace_store::workspace::random_id("up_");
    let path = state.staging_file(&format!("{upload_id}.part"));
    sweep_uploads(state);
    let mut uploads = state.uploads.lock().unwrap();
    let upload = Upload { kind: kind.into(), workspace_id, principal: caller.principal.clone(), path, received: None, expires_at: (state.svc.clock)() + 86_400_000, result: None, finish_params: None, writing: false };
    save_upload(state, &upload_id, &upload)?;
    uploads.insert(upload_id.clone(), upload);
    Ok(json!({ "ok": true, "upload_id": upload_id, "put": format!("{HTTP_PATH}/upload/{upload_id}") }))
}

fn sweep_uploads(state: &AppState) {
    state.uploads.lock().unwrap().retain(|id, upload| {
        if upload.writing || upload.expires_at > (state.svc.clock)() { return true; }
        let _ = std::fs::remove_file(&upload.path);
        let _ = std::fs::remove_file(state.staging_file(&format!("{id}.upload.json")));
        false
    });
}

async fn import_asset_object(state: &Arc<AppState>, params: &Value, caller: &Caller, token: &str) -> WsResult<Value> {
    let workspace = s(params, "workspace_id")?.to_string();
    let id = ndn_lib::ObjId::new(s(params, "obj_id")?).map_err(|e| WsError::invalid_op(e.to_string()))?;
    if !id.is_file_object() {
        return Err(WsError::invalid_op("an asset must be a FileObject; import folders as workspace containers"));
    }
    state.with_ws(&workspace, |ws| {
        let access = ws.require_ws_any(caller)?;
        use aiworkspace_core::access::Cap;
        if !access.ws_caps.has(Cap::Update) && !access.ws_caps.has(Cap::Structure) {
            return Err(WsError::denied("update or structure capability required"));
        }
        Ok(())
    })?;
    let (store, gateway) = state.named_import.get_or_try_init(|| async {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| WsError::io(e.to_string()))?;
        let gateway = runtime.get_zone_service_url("nfs-server", false).await.map_err(|e| WsError::io(e.to_string()))?;
        let store = runtime.get_local_named_store().await.map_err(|e| WsError::io(e.to_string()))?;
        Ok::<_, WsError>((store, gateway))
    }).await?;
    let mut url = reqwest::Url::parse(gateway).map_err(|e| WsError::io(e.to_string()))?;
    url.set_path("/ndm/v1/store/get_object");
    url.set_query(None);
    let response = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| WsError::io(e.to_string()))?
        .post(url).bearer_auth(token).json(&json!({"obj_id": id.to_string()})).send().await.map_err(|e| WsError::io(e.to_string()))?;
    if !response.status().is_success() {
        return Err(WsError::denied("the current session cannot read the imported object"));
    }
    if !store.is_object_stored(&id, None).await.map_err(|e| WsError::io(e.to_string()))? {
        return Err(WsError::new(Code::DependencyUnavailable, "imported object is incomplete"));
    }
    let (reader, size) = store.open_reader(&id, None).await.map_err(|e| WsError::io(e.to_string()))?;
    if size > state.limits.max_asset_bytes {
        return Err(WsError::limit(format!("workspace assets are limited to {} bytes", state.limits.max_asset_bytes)));
    }
    use tokio::io::AsyncReadExt;
    let mut data = Vec::new();
    reader.take(state.limits.max_asset_bytes + 1).read_to_end(&mut data).await.map_err(|e| WsError::io(e.to_string()))?;
    if data.len() as u64 != size {
        return Err(WsError::new(Code::DependencyUnavailable, "imported object size mismatch"));
    }
    let (st, c) = (state.clone(), caller.clone());
    blocking(move || st.with_ws(&workspace, |ws| {
        let mut result = ws.stage_asset(&c, &data)?;
        result["source_obj_id"] = json!(id.to_string());
        Ok(result)
    })).await
}

fn save_upload(state: &Arc<AppState>, id: &str, upload: &Upload) -> WsResult<()> {
    let bytes = serde_json::to_vec(upload).map_err(|e| WsError::io(e.to_string()))?;
    aiworkspace_store::objects::write_atomic(&state.staging_file(&format!("{id}.upload.json")), &bytes)
}

fn finish_upload(state: &Arc<AppState>, upload_id: &str, kind: &str, workspace_id: Option<&str>, caller: &Caller, params: &Value, finish: impl FnOnce(&std::path::Path) -> WsResult<Value>) -> WsResult<Value> {
    let mut uploads = state.uploads.lock().unwrap();
    let upload = uploads.get_mut(upload_id).filter(|u| u.kind == kind && u.principal == caller.principal && u.workspace_id.as_deref() == workspace_id && u.expires_at > (state.svc.clock)())
        .ok_or_else(|| WsError::not_found("upload not found for this workspace"))?;
    if upload.received.is_none() || upload.writing {
        return Err(WsError::new(Code::DependencyUnavailable, "upload has not completed"));
    }
    if upload.finish_params.as_ref().is_some_and(|intent| intent != params) {
        return Err(WsError::invalid_op("upload was already submitted with different parameters"));
    }
    if let Some(result) = &upload.result {
        save_upload(state, upload_id, upload)?;
        return Ok(result.clone());
    }
    upload.finish_params = Some(params.clone());
    save_upload(state, upload_id, upload)?;
    let result = finish(&upload.path)?;
    upload.result = Some(result.clone());
    save_upload(state, upload_id, upload)?;
    let _ = std::fs::remove_file(&upload.path);
    Ok(result)
}

/// The caller of an HTTP route, its memberships settled for `workspace_id` (when the route names one).
async fn bearer(state: &Arc<AppState>, headers: &HeaderMap, route: &str, workspace_id: Option<&str>) -> Result<Caller, Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session token required").into_response())?;
    if let Some(guest) = Caller::guest(token) {
        return Ok(guest);
    }
    let mut caller = state.auth.authenticate(token).await.map_err(|e| match e {
        AuthError::Invalid(e) => (StatusCode::UNAUTHORIZED, e).into_response(),
        AuthError::Denied(d) => {
            let d = d.at("aiworkspace", route);
            (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": { "code": ACCESS_DENIED_ERROR_CODE, "retryable": false, "detail": format!("access denied: {}", d.reason.as_str()), "denial": d } }))).into_response()
        }
    })?;
    prepare_caller(state, &mut caller, route, workspace_id).await;
    Ok(caller)
}

fn http_error(e: WsError, caller: Option<&Caller>, route: &str) -> Response {
    let status = match e.code {
        Code::NotFound | Code::TargetDeleted => StatusCode::NOT_FOUND,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        Code::LimitExceeded => StatusCode::PAYLOAD_TOO_LARGE,
        Code::InvalidOperation | Code::InvalidSchema => StatusCode::BAD_REQUEST,
        Code::DependencyUnavailable => StatusCode::GONE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "ok": false, "error": failure_json(&e, caller, route) }))).into_response()
}

/// `PUT /kapi/aiworkspace/upload/<upload_id>`: asset bytes or an import package, streamed to staging.
async fn upload(State(state): State<Arc<AppState>>, Path(upload_id): Path<String>, headers: HeaderMap, body: Body) -> Response {
    let caller = match bearer(&state, &headers, "upload", None).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (path, limit) = {
        let mut uploads = state.uploads.lock().unwrap();
        match uploads.get_mut(&upload_id) {
            Some(u) if u.principal == caller.principal && u.received.is_none() && !u.writing && u.expires_at > (state.svc.clock)() => {
                u.writing = true;
                (u.path.clone(), if u.kind == "asset" { state.limits.max_asset_bytes } else { state.limits.max_package_bytes })
            }
            _ => return http_error(WsError::not_found("upload not found"), Some(&caller), "upload"),
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
                u.writing = false;
                if let Err(e) = save_upload(&state, &upload_id, u) { return http_error(e, Some(&caller), "upload"); }
            }
            Json(json!({ "ok": true, "upload_id": upload_id, "received": total })).into_response()
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&path).await;
            if let Some(u) = state.uploads.lock().unwrap().get_mut(&upload_id) { u.writing = false; }
            http_error(e, Some(&caller), "upload")
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> WsResult<T> + Send + 'static) -> WsResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| WsError::io(e.to_string()))?
}

/// `GET /kapi/aiworkspace/asset/<workspace_id>/<object_id>`: verified asset
/// bytes, authorized through an entity that references the asset.
async fn asset(State(state): State<Arc<AppState>>, Path((workspace_id, object_id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let caller = match bearer(&state, &headers, "asset", Some(&workspace_id)).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (st, c) = (state.clone(), caller.clone());
    let result = blocking(move || {
        let caller = c;
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
        Err(e) => http_error(e, Some(&caller), "asset"),
    }
}

async fn serve_file(path: WsResult<PathBuf>, ctype: &'static str, caller: &Caller, route: &str) -> Response {
    match path {
        Ok(p) => match tokio::fs::read(&p).await {
            Ok(data) => ([(header::CONTENT_TYPE, ctype)], data).into_response(),
            Err(e) => http_error(WsError::io(e.to_string()), Some(caller), route),
        },
        Err(e) => http_error(e, Some(caller), route),
    }
}

/// `GET /kapi/aiworkspace/export/<workspace_id>/<export_id>`
async fn download_export(State(state): State<Arc<AppState>>, Path((workspace_id, export_id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let caller = match bearer(&state, &headers, "export", Some(&workspace_id)).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (st, c) = (state.clone(), caller.clone());
    let path = blocking(move || {
        let caller = c;
        st.with_ws(&workspace_id, |ws| {
            ws.require_ws(&caller, aiworkspace_core::access::Cap::Export)?;
            ws.export_path(&export_id).ok_or_else(|| WsError::not_found("export not found"))
        })
    })
    .await;
    serve_file(path, "application/zip", &caller, "export").await
}

/// `GET /kapi/aiworkspace/replica/<workspace_id>/<replica_id>`: the database built by `replica.bootstrap`.
async fn download_replica(State(state): State<Arc<AppState>>, Path((workspace_id, replica_id)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let caller = match bearer(&state, &headers, "replica", Some(&workspace_id)).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (st, c) = (state.clone(), caller.clone());
    let path = blocking(move || {
        let caller = c;
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
    serve_file(path, "application/vnd.sqlite3", &caller, "replica").await
}

pub async fn serve(state: Arc<AppState>, listen: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let addr = listener.local_addr()?;
    log::info!("aiworkspace listening on http://{}{}", addr, HTTP_PATH);
    // wish programs call back over loopback for `llm.map`
    let host = if addr.ip().is_unspecified() { format!("127.0.0.1:{}", addr.port()) } else { addr.to_string() };
    *state.wish.host_base.lock().unwrap() = Some(format!("http://{host}{HTTP_PATH}"));
    show::spawn_sweeper(state.clone());
    axum::serve(listener, build_router(state)).await
}
