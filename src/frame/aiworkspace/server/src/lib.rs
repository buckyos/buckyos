//! aiworkspace service: kRPC method dispatch plus the asset and publication routes
//! (design doc §5). All store work is synchronous and runs on the blocking pool;
//! each Workspace has exactly one writer behind its mutex. Uploads and downloads go
//! through the zone's NDM and application object buckets (Durable Data Schema §3).

pub mod auth;
pub mod buckets;
pub mod control;
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
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    /// The NDM gateway (nfs-server origin) that judges whether a caller may read an object it asks to import.
    pub ndm_gateway: std::sync::OnceLock<String>,
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
        Arc::new(AppState { svc, auth, limits, wish: wish::WishRuntime::new(wish), shows: show::Shows::default(), heads, ndm_gateway: std::sync::OnceLock::new() })
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
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let weak = Arc::downgrade(&state);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut ticks: u64 = 0;
        loop {
            timer.tick().await;
            let Some(state) = weak.upgrade() else { break };
            ticks += 1;
            if ticks % 5 == 0 {
                // Sources on a schedule (D24)
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || source::tick(&st)).await;
            }
            if ticks % 60 == 0 {
                // what nothing retains any more is unbound hourly (D06)
                let st = state.clone();
                let _ = tokio::task::spawn_blocking(move || match st.svc.sweep() {
                    Ok(r) => log::info!("sweep: {r}"),
                    Err(e) => log::warn!("sweep failed: {e}"),
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
        .route(&format!("{HTTP_PATH}/asset/{{workspace_id}}/{{object_id}}"), get(asset))
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
    } else if method == "ws.import" {
        import_package(&state, &params, caller.as_ref().expect("authenticated above"), req["sys"][1].as_str().unwrap_or_default()).await
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
    ("control.list", "experimental"),
    ("control.describe", "experimental"),
    ("control.import", "experimental"),
    ("control.export", "experimental"),
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
        // Block-controls (脑图方案 §8.1): the catalogue needs a session only; import / export act in a Workspace
        "control.list" => control::list(),
        "control.describe" => control::describe(s(p, "kind")?)?,
        "control.import" => state.with_ws(ws_id()?, |ws| control::import(ws, caller, p))?,
        "control.export" => state.with_ws(ws_id()?, |ws| control::export(ws, caller, p))?,
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
            ws.export(caller, p.get("mode").and_then(Value::as_str).unwrap_or("share"),
                      p.get("self_contained").and_then(Value::as_bool).unwrap_or(true), p.get("include_notes").and_then(Value::as_bool))
        })?,
        "replica.bootstrap" => state.with_ws(ws_id()?, |ws| ws.replica_bootstrap(caller))?,
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
        "diag.list_unretained" => state.with_ws(ws_id()?, |ws| ws.list_unretained(caller))?,
        "diag.verify_refs" => state.with_ws(ws_id()?, |ws| {
            ws.require_ws(caller, aiworkspace_core::access::Cap::Manage)?;
            ws.verify_refs()
        })?,
        _ => return Err(WsError::invalid_op(UNKNOWN_METHOD)),
    }))
}

/// Methods a run token may call: the run's own lifecycle and reads of the Workspace it runs in.
const RUN_TOKEN_METHODS: &[&str] = &["proc.get", "proc.heartbeat", "proc.report", "proc.tool", "proc.program", "proc.files", "proc.list", "ws.get_info", "doc.read", "doc.query", "doc.resolve", "doc.outline", "doc.list_children"];

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

// ---- imports through NDM and buckets ----

/// An upload the caller bound itself (`ndm` finalize `auto_bind`, `PUT /bucket/v1/...`): it must sit in the
/// caller's own `name` bucket of this service; `bucket.get` proves the binding and names the object.
fn own_binding(state: &AppState, caller: &Caller, p: &Value, name: &str) -> WsResult<(String, String, String)> {
    let (bucket, key) = (s(p, "bucket_id")?, s(p, "key")?);
    if aiworkspace_store::objects::bucket_id(&caller.principal, name).ok().as_deref() != Some(bucket) {
        return Err(WsError::denied(format!("an upload is bound into the caller's own {name} bucket")));
    }
    let (obj_id, _) = state.svc.objects.binding(bucket, key)?.ok_or_else(|| WsError::not_found("no upload is bound under that key"))?;
    Ok((bucket.to_string(), key.to_string(), obj_id))
}

/// An unbound object the caller names (`obj_id`): the NDM gateway judges with the caller's own token whether it
/// may read it, before the service — which may read every object — touches it on its behalf.
async fn authorize_object(state: &Arc<AppState>, token: &str, id: &ndn_lib::ObjId) -> WsResult<()> {
    if !id.is_file_object() {
        return Err(WsError::invalid_op("expected a FileObject; import folders as workspace containers"));
    }
    let gateway = state.ndm_gateway.get().ok_or_else(|| WsError::new(Code::DependencyUnavailable, "this deployment has no NDM gateway"))?;
    let mut url = reqwest::Url::parse(gateway).map_err(|e| WsError::io(e.to_string()))?;
    url.set_path("/ndm/v1/store/get_object");
    url.set_query(None);
    let response = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| WsError::io(e.to_string()))?
        .post(url).bearer_auth(token).json(&json!({"obj_id": id.to_string()})).send().await.map_err(|e| WsError::io(e.to_string()))?;
    if !response.status().is_success() {
        return Err(WsError::denied("the current session cannot read the imported object"));
    }
    Ok(())
}

/// `ws.import { bucket_id, key | obj_id, semantics, replace? }`: a package the caller bound into its own `exports`
/// bucket — a successful import consumes it (the binding goes), so the same upload is never replayed — or an
/// unbound FileObject it may read (an Agent's upload).
async fn import_package(state: &Arc<AppState>, p: &Value, caller: &Caller, token: &str) -> WsResult<Value> {
    require_may_own(caller)?;
    let semantics = s(p, "semantics")?.to_string();
    if !matches!(semantics.as_str(), "new" | "restore") {
        return Err(WsError::invalid_op("semantics must be \"restore\" or \"new\""));
    }
    if p.get("bucket_id").is_none() {
        let id = ndn_lib::ObjId::new(s(p, "obj_id")?).map_err(|e| WsError::invalid_op(e.to_string()))?;
        authorize_object(state, token, &id).await?;
    }
    let (st, c, p) = (state.clone(), caller.clone(), p.clone());
    blocking(move || {
        let uploaded = match p.get("bucket_id") {
            Some(_) => Some(own_binding(&st, &c, &p, aiworkspace_store::objects::EXPORTS)?),
            None => None,
        };
        let obj_id = match &uploaded {
            Some((_, _, obj)) => obj.clone(),
            None => s(&p, "obj_id")?.to_string(),
        };
        if st.svc.objects.file_size(&obj_id)? > st.limits.max_package_bytes {
            return Err(WsError::limit("package is too large"));
        }
        let tmp = aiworkspace_store::export::TempFile::new("import.bcanvas")?;
        st.svc.objects.read_to_file(&obj_id, &tmp.0)?;
        let result = st.svc.import(&c, &tmp.0, &semantics, p.get("replace").and_then(Value::as_bool).unwrap_or(false))?;
        if let Some((bucket, key, _)) = uploaded {
            st.svc.objects.unbind(&bucket, &key)?;
        }
        Ok(result)
    })
    .await
}

/// `asset.import_object { workspace_id, bucket_id, key }` (the uploader bound the FileObject) or `{ workspace_id,
/// obj_id }` (an unbound upload: the NDM gateway judges with the caller's own token whether it may read it). Either
/// way the Workspace owner's `assets` bucket binds it under `<ws>/<object_id>`; nothing is copied.
async fn import_asset_object(state: &Arc<AppState>, params: &Value, caller: &Caller, token: &str) -> WsResult<Value> {
    let workspace = s(params, "workspace_id")?.to_string();
    if params.get("bucket_id").is_none() {
        let id = ndn_lib::ObjId::new(s(params, "obj_id")?).map_err(|e| WsError::invalid_op(e.to_string()))?;
        authorize_object(state, token, &id).await?;
    }
    let (st, c, p) = (state.clone(), caller.clone(), params.clone());
    blocking(move || {
        let uploaded = match p.get("bucket_id") {
            Some(_) => Some(own_binding(&st, &c, &p, aiworkspace_store::objects::ASSETS)?),
            None => None,
        };
        let obj_id = match &uploaded {
            Some((_, _, obj)) => obj.clone(),
            None => s(&p, "obj_id")?.to_string(),
        };
        let max = st.limits.max_asset_bytes;
        let result = st.with_ws(&workspace, |ws| ws.stage_object(&c, &obj_id, max))?;
        if let Some((bucket, key, _)) = uploaded {
            if result["bucket_id"].as_str() != Some(bucket.as_str()) || result["key"].as_str() != Some(key.as_str()) {
                st.svc.objects.unbind(&bucket, &key)?;
            }
        }
        Ok(result)
    })
    .await
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
            Ok((media, ws.objects.get_file(&object_id)?))
        })
    })
    .await;
    match result {
        Ok((media, data)) => {
            // user content is never served as active content from the service origin: pictures, sound and video are
            // passive, SVG and HTML are not
            let safe = matches!(media.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" | "image/bmp" | "application/pdf" | "text/plain")
                || (media.starts_with("audio/") || media.starts_with("video/")) && !media.contains(['+', ';']);
            let ctype = if safe { media } else { "application/octet-stream".to_string() };
            ([(header::CONTENT_TYPE, ctype), (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
              (header::CACHE_CONTROL, "private, max-age=31536000, immutable".to_string())], data).into_response()
        }
        Err(e) => http_error(e, Some(&caller), "asset"),
    }
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
