//! The Zone's NDM gateway over this node's named store (share design D18/D19):
//! the browser/app upload protocol `/ndm/v1/*` (cyfs-ndn
//! `doc/NDM Protocol/named-data-mgr-zone-protocol.md`) and the trusted-process
//! proxy `/ndm/proxy/v1/*` (`named-data-mgr-proxy-protocol.md`).
//!
//! - `uploads`: tus sessions, owned by the verified caller and kept in filedb.
//! - `forward`: everything else is authorized here and then served by
//!   cyfs-ndn's own handlers.
//!
//! Every request needs a verified session. RBAC decides what a caller may do:
//!
//! | resource            | action | allows                                              |
//! | ------------------- | ------ | --------------------------------------------------- |
//! | `obj://ndm/uploads` | write  | upload chunks, register own objects, existence checks |
//! | `obj://ndm/objects` | read   | read any object or chunk                            |
//! | `obj://ndm/retain`  | write  | pin/unpin under the caller's own owner prefix       |
//! | `obj://ndm/manage`  | write  | delete, GC, fs anchors, edges, pins of any owner    |
//!
//! What a caller uploaded or registered stays readable to it (`ndm_holdings`)
//! and is held in the store by a lease for [`HOLD_TTL_SECS`], long enough for
//! the business that uses it (a message, a post, an app package) to take its
//! own retention.

pub mod forward;
pub mod import;
pub mod uploads;

use crate::auth::Credentials;
use crate::config::AuthMode;
use crate::error::NfsResult;
use crate::fsutil::unix_now;
use crate::state::AppState;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use named_store::{NamedDataMgr, NamedDataMgrNodeGateway, NamedDataMgrZoneGateway, NdmZoneGatewayConfig, PinScope};
use ndn_lib::ObjId;
use rusqlite::{params, OptionalExtension};
use std::path::PathBuf;
use std::sync::Arc;

pub const RES_UPLOADS: &str = "obj://ndm/uploads";
pub const RES_OBJECTS: &str = "obj://ndm/objects";
pub const RES_RETAIN: &str = "obj://ndm/retain";
pub const RES_MANAGE: &str = "obj://ndm/manage";

/// How long an upload or a registration holds what it brought into the store.
pub const HOLD_TTL_SECS: u64 = 24 * 3600;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ndm_uploads(
  session_id TEXT PRIMARY KEY,
  principal TEXT NOT NULL,
  upload_key TEXT NOT NULL,
  canonical_upload_id TEXT NOT NULL,
  logical_path TEXT NOT NULL,
  file_hash TEXT,
  chunk_index INTEGER NOT NULL,
  chunk_hash TEXT,
  chunk_size INTEGER NOT NULL,
  offset INTEGER NOT NULL,
  status TEXT NOT NULL,
  chunk_obj_id TEXT,
  raw_metadata TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ndm_uploads_key ON ndm_uploads(upload_key);
CREATE INDEX IF NOT EXISTS idx_ndm_uploads_principal ON ndm_uploads(principal, updated_at);
CREATE TABLE IF NOT EXISTS ndm_holdings(
  principal TEXT NOT NULL,
  obj_id TEXT NOT NULL,
  how TEXT NOT NULL,
  at INTEGER NOT NULL,
  PRIMARY KEY(principal, obj_id)
);
CREATE TABLE IF NOT EXISTS ndm_import_qcids(
  principal TEXT NOT NULL,
  qcid TEXT NOT NULL,
  content_id TEXT NOT NULL,
  size INTEGER NOT NULL,
  PRIMARY KEY(principal, qcid)
);
"#;

pub struct NdmMgr {
    pub staging: PathBuf,
    gateways: tokio::sync::OnceCell<(Arc<NamedDataMgrZoneGateway>, Arc<NamedDataMgrNodeGateway>)>,
    /// Sessions with a PATCH in flight (tus requires PATCHes to be serial).
    busy: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl NdmMgr {
    pub fn new(staging: PathBuf) -> NfsResult<NdmMgr> {
        std::fs::create_dir_all(&staging)?;
        Ok(NdmMgr { staging, gateways: tokio::sync::OnceCell::new(), busy: Default::default() })
    }

    pub(crate) fn try_busy(&self, session_id: &str) -> bool {
        self.busy.lock().unwrap().insert(session_id.to_string())
    }

    pub(crate) fn done(&self, session_id: &str) {
        self.busy.lock().unwrap().remove(session_id);
    }
}

/// The verified caller of an NDM request.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user: String,
    /// RBAC subject of the token's target (`app:<id>` / `system:<service>`).
    pub app_key: String,
    /// Canonical target (`app:<app>@<owner>` / `system:<service>`).
    pub app: String,
    token: Option<String>,
}

impl Principal {
    /// Owner of uploads and holdings: the same user through the same app.
    pub fn key(&self) -> String {
        format!("{}|{}", self.user, self.app)
    }

    /// Pin owners a caller with `obj://ndm/retain` may use.
    pub fn pin_prefix(&self) -> String {
        format!("{}/{}/", self.app_key, self.user)
    }

    fn hold_owner(&self) -> String {
        format!("ndm-hold:{}", self.key())
    }
}

pub fn ndm_error(status: StatusCode, code: &str, message: impl std::fmt::Display) -> Response {
    let body = serde_json::json!({"error": code, "message": message.to_string()});
    (status, [(header::CONTENT_TYPE, "application/json; charset=utf-8")], body.to_string()).into_response()
}

/// tus responses carry `Tus-Resumable` even when they fail.
pub fn tus_error(status: StatusCode, code: &str, message: impl std::fmt::Display) -> Response {
    let mut r = ndm_error(status, code, message);
    r.headers_mut().insert("tus-resumable", HeaderValue::from_static(uploads::TUS_VERSION));
    r
}

/// The caller's verified session; an explicit Bearer must be valid, cookies may be stale.
pub async fn principal(state: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    let test_key = match &state.config.auth {
        AuthMode::Disabled => {
            return Ok(Principal { user: "local".into(), app_key: "system:local".into(), app: "system:local".into(), token: None })
        }
        AuthMode::TestKey { public_key, .. } => Some(public_key.as_str()),
        AuthMode::Runtime => None,
    };
    let creds = Credentials::from_headers(headers);
    let candidates: Vec<&str> = match creds.bearer() {
        Some(bearer) => vec![bearer],
        None => creds.cookie_tokens().iter().map(String::as_str).collect(),
    };
    for token in candidates {
        if let Ok(claims) = buckyos_api::verify_nfs_session_token(token, test_key).await {
            return Ok(Principal {
                user: claims.subject,
                app_key: claims.authorization_key,
                app: claims.app_id,
                token: Some(token.to_string()),
            });
        }
    }
    Err(ndm_error(StatusCode::UNAUTHORIZED, "unauthenticated", "a valid session token is required"))
}

/// RBAC: the caller's user and its app must both be allowed `action` on `resource`.
pub async fn allowed(state: &AppState, p: &Principal, resource: &str, action: &str) -> bool {
    match &state.config.auth {
        AuthMode::Disabled => true,
        AuthMode::TestKey { .. } => rbac::enforce(&p.user, &p.app_key, resource, action, None).await,
        AuthMode::Runtime => {
            let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else { return false };
            let mut req = kRPC::RPCRequest::new("ndm.authorize", serde_json::Value::Null);
            req.token = p.token.clone();
            runtime.enforce(&req, action, resource).await.is_ok()
        }
    }
}

pub fn denied(what: &str) -> Response {
    ndm_error(StatusCode::FORBIDDEN, "permission_denied", format!("not allowed to {what}"))
}

pub async fn store(state: &AppState) -> Result<&NamedDataMgr, Response> {
    state
        .share
        .store(&state.config)
        .await
        .map_err(|e| ndm_error(StatusCode::SERVICE_UNAVAILABLE, "store_unavailable", e.message))
}

pub(crate) async fn gateways(state: &AppState) -> Result<&(Arc<NamedDataMgrZoneGateway>, Arc<NamedDataMgrNodeGateway>), Response> {
    let mgr = store(state).await?.clone();
    let cache_dir = state.ndm.staging.join("zone-gateway");
    Ok(state
        .ndm
        .gateways
        .get_or_init(|| async move {
            let mgr = Arc::new(mgr);
            let zone = NamedDataMgrZoneGateway::new(mgr.clone(), NdmZoneGatewayConfig { cache_dir, ..Default::default() });
            // Restricted calls are authorized here before they reach the proxy.
            (Arc::new(zone), Arc::new(NamedDataMgrNodeGateway::with_restricted(mgr)))
        })
        .await)
}

/// The caller brought `obj_id` into the store (an upload, a write or a registration).
pub fn record_holding(state: &AppState, p: &Principal, obj_id: &ObjId, how: &str) {
    let conn = state.db.lock();
    if let Err(e) = conn.execute(
        "INSERT INTO ndm_holdings(principal, obj_id, how, at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(principal, obj_id) DO UPDATE SET at=excluded.at",
        params![p.key(), obj_id.to_string(), how, unix_now()],
    ) {
        log::warn!("record ndm holding of {} failed: {}", obj_id, e);
    }
}

pub fn holds(state: &AppState, p: &Principal, obj_id: &str) -> bool {
    let conn = state.db.lock();
    conn.query_row(
        "SELECT 1 FROM ndm_holdings WHERE principal=?1 AND obj_id=?2",
        params![p.key(), obj_id],
        |_| Ok(()),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}

/// Keeps what a caller just brought in until its business takes over (lease, no cascade).
pub async fn lease(store: &NamedDataMgr, obj_id: &ObjId, owner: &str) {
    if let Err(e) = store.pin(obj_id, owner, PinScope::Lease, Some(std::time::Duration::from_secs(HOLD_TTL_SECS))).await {
        log::warn!("lease {} for {} failed: {}", obj_id, owner, e);
    }
}

pub async fn hold(state: &AppState, store: &NamedDataMgr, p: &Principal, obj_id: &ObjId, how: &str) {
    record_holding(state, p, obj_id, how);
    lease(store, obj_id, &p.hold_owner()).await;
}

pub async fn hold_checked(state: &AppState, store: &NamedDataMgr, p: &Principal, obj_id: &ObjId, how: &str) -> Result<(), Response> {
    store.pin(obj_id, &p.hold_owner(), PinScope::Recursive, Some(std::time::Duration::from_secs(HOLD_TTL_SECS))).await
        .map_err(|e| ndm_error(StatusCode::SERVICE_UNAVAILABLE, "retention_failed", e))?;
    state.db.lock().execute(
        "INSERT INTO ndm_holdings(principal, obj_id, how, at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(principal, obj_id) DO UPDATE SET at=excluded.at",
        params![p.key(), obj_id.to_string(), how, unix_now()],
    ).map_err(|e| ndm_error(StatusCode::SERVICE_UNAVAILABLE, "retention_failed", e))?;
    Ok(())
}

/// Read access to one object or chunk: the general read right, or the caller's own holding.
pub async fn may_read(state: &AppState, p: &Principal, obj_id: &str) -> bool {
    holds(state, p, obj_id) || allowed(state, p, RES_OBJECTS, "read").await
}

/// Once a minute: expire idle upload sessions and drop finished ones whose hold ran out.
pub async fn sweeper_loop(state: crate::state::SharedState) {
    uploads::remove_orphan_parts(&state);
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        uploads::sweep(&state).await;
    }
}
