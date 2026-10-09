//! HTTP surface: kRPC at `/kapi/homestation` (each user's own HomeStation, plus `portal.*`
//! for anyone), uploads, and the cross-node protocol paths under `/home/`: the zone index and
//! DID lookup at `/home/`, each user's home at `/home/<user>/...` (stream, entries, objects,
//! chunks, comment views, profile, inbox) and the zone feed at `/home/~zone/...`.

use crate::audience::Reader;
use crate::auth::{reader_from_request, Caller};
use crate::delivery::HEADER_AUDIENCE;
use crate::error::HsError;
use crate::host::{Host, Viewer};
use crate::protocol::{normalize_obj_id, obj_type_of, OBJ_TYPE_FILE, ZONE_FEED};
use crate::Station;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const KRPC_PATH: &str = "/kapi/homestation";
pub const MAX_UPLOAD: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub host: Arc<Host>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(KRPC_PATH, post(krpc))
        .route(&format!("{KRPC_PATH}/"), post(krpc))
        .route(&format!("{KRPC_PATH}/upload"), axum::routing::put(upload).post(upload))
        .route("/home/", get(zone_index))
        .route("/home/{user}/feed", get(feed))
        .route("/home/{user}/profile", get(profile))
        .route("/home/{user}/comments", get(comments))
        .route("/home/{user}/index/recent", get(collector_recent))
        .route("/home/{user}/objects/{id}", get(object))
        .route("/home/{user}/objects/{id}/content", get(object_content))
        .route("/home/{user}/chunks/{id}", get(chunk))
        .route("/home/{user}/inbox", axum::routing::put(inbox_put).get(inbox_status))
        .route("/home/{user}/{ns}/@/{key}", get(entry_head))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD + 1024))
        .with_state(state)
}

pub async fn serve(state: AppState, listen: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    log::info!("homestation listening on {listen}");
    serve_listener(state, listener).await
}

pub async fn serve_listener(state: AppState, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

fn rpc_error(seq: u64, message: &str) -> Response {
    Json(json!({ "error": message, "sys": [seq] })).into_response()
}

async fn krpc(State(state): State<AppState>, body: Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(0, &format!("invalid request: {e}")),
    };
    let seq = req["sys"][0].as_u64().unwrap_or(0);
    let Some(method) = req["method"].as_str().map(str::to_string) else { return rpc_error(seq, "method required") };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let portal = method.starts_with("portal.");
    let token = req["sys"][1].as_str().filter(|t| !t.trim().is_empty());
    let caller = match token {
        Some(token) => match state.host.auth.authenticate(token).await {
            Ok(c) => Some(c),
            // A stale session must not hide public pages: portal reads fall back to anonymous.
            Err(_) if portal => None,
            Err(e) => return rpc_error(seq, &format!("unauthorized: {e}")),
        },
        None if portal => None,
        None => return rpc_error(seq, "session token required"),
    };
    match state.host.handle_rpc(caller.as_ref(), &method, params).await {
        Ok(v) => Json(json!({ "result": v, "sys": [seq] })).into_response(),
        Err(e) => rpc_error(seq, &e.to_string()),
    }
}

fn error_response(e: HsError) -> Response {
    (e.status(), Json(json!({ "error": e.code(), "message": e.message() }))).into_response()
}

async fn caller_from_bearer(state: &AppState, headers: &HeaderMap, query: &HashMap<String, String>) -> Result<Caller, Response> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| query.get("access").cloned())
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session token required").into_response())?;
    state.host.auth.authenticate(&token).await.map_err(|e| (StatusCode::UNAUTHORIZED, e).into_response())
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found", "message": what }))).into_response()
}

/// The station serving `/home/<user>/...`.
async fn station_of(state: &AppState, user: &str) -> Result<Arc<Station>, Response> {
    match state.host.station(user).await {
        Ok(Some(station)) => Ok(station),
        Ok(None) => Err(not_found("no such home")),
        Err(e) => Err(error_response(e)),
    }
}

/// `PUT /kapi/homestation/upload?name=&mime=[&width=&height=&duration_ms=]`: store one
/// single-chunk FileObject (§9.7) and return its ObjId.
async fn upload(State(state): State<AppState>, headers: HeaderMap, Query(query): Query<HashMap<String, String>>, body: Bytes) -> Response {
    let caller = match caller_from_bearer(&state, &headers, &query).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let station = match state.host.station_for_caller(&caller).await {
        Ok(Some(station)) => station,
        Ok(None) => return (StatusCode::FORBIDDEN, "this account has no HomeStation here").into_response(),
        Err(e) => return error_response(e),
    };
    if body.is_empty() || body.len() > MAX_UPLOAD {
        return (StatusCode::PAYLOAD_TOO_LARGE, "file must be 1 byte to 32 MiB").into_response();
    }
    let name = query.get("name").cloned().unwrap_or_else(|| "file".into());
    let mime = query.get("mime").cloned().unwrap_or_else(|| "application/octet-stream".into());
    let mut meta = serde_json::Map::new();
    for key in ["width", "height", "duration_ms"] {
        if let Some(v) = query.get(key).and_then(|v| v.parse::<u64>().ok()) {
            meta.insert(key.into(), json!(v));
        }
    }
    match station.store_file(&name, &mime, body.to_vec(), Value::Object(meta)).await {
        Ok((obj_id, body)) => Json(json!({ "objId": obj_id, "file": crate::projection::file_view(&body), "object": body })).into_response(),
        Err(e) => error_response(e),
    }
}

/// Reader of one user's home: that user (owner view), another identity, or anonymous.
async fn reader(state: &AppState, station: &Station, headers: &HeaderMap, query: &HashMap<String, String>) -> Result<Reader, Response> {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    reader_from_request(
        state.host.auth.as_ref(),
        state.host.directory.as_ref(),
        station.owner(),
        &state.host.cfg.zone,
        authorization,
        query.get("access").map(String::as_str),
    )
    .await
    .map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized", "message": e }))).into_response())
}

/// Reader of the zone feed: no owner there, each listed entry is judged by its own station.
async fn viewer(state: &AppState, headers: &HeaderMap, query: &HashMap<String, String>) -> Result<Viewer, Response> {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let reader = reader_from_request(
        state.host.auth.as_ref(),
        state.host.directory.as_ref(),
        "",
        &state.host.cfg.zone,
        authorization,
        query.get("access").map(String::as_str),
    )
    .await
    .map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized", "message": e }))).into_response())?;
    Ok(Viewer::from_reader(&reader, ""))
}

/// `GET /home/`: the zone's default feed and zone feed; `?did=` locates a user's home (§4.4).
async fn zone_index(State(state): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Response {
    match q.get("did") {
        Some(did) => match state.host.locate(did).await {
            Some(home) => no_store(Json(home).into_response()),
            None => not_found("no HomeStation for this DID here"),
        },
        None => no_store(Json(state.host.zone_index().await).into_response()),
    }
}

/// `GET /home/<user>/feed?mode=display|changes` (§4.4), `GET /home/~zone/feed` (§4.6).
async fn feed(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let with_objects = q.get("objects").is_some_and(|v| v == "1" || v == "true");
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(20);
    let mode = q.get("mode").map(String::as_str).unwrap_or("display");
    if mode != "changes" && mode != "display" {
        return (StatusCode::BAD_REQUEST, "mode is display or changes").into_response();
    }
    let since = q.get("since").and_then(|v| v.parse().ok()).unwrap_or(0);
    let result = if user == ZONE_FEED {
        let viewer = match viewer(&state, &headers, &q).await {
            Ok(v) => v,
            Err(r) => return r,
        };
        if mode == "changes" {
            state.host.zone_changes(&viewer, since, limit.max(100), with_objects).await.map(|p| json!(p))
        } else {
            state.host.zone_display(&viewer, q.get("cursor").cloned(), limit, with_objects).await.map(|p| json!(p))
        }
    } else {
        let station = match station_of(&state, &user).await {
            Ok(s) => s,
            Err(r) => return r,
        };
        let reader = match reader(&state, &station, &headers, &q).await {
            Ok(r) => r,
            Err(r) => return r,
        };
        if mode == "changes" {
            station.read_changes(&reader, since, limit.max(100), with_objects).await.map(|p| json!(p))
        } else {
            station.read_display(&reader, q.get("kind").cloned(), q.get("cursor").cloned(), limit, with_objects).await.map(|p| json!(p))
        }
    };
    match result {
        Ok(v) => no_store(Json(v).into_response()),
        Err(e) => error_response(e),
    }
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(header::VARY, HeaderValue::from_static("authorization"));
    response
}

fn named_object_response(wire: String, content_type: &str, restricted: bool) -> Response {
    let mut response = (StatusCode::OK, wire).into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
    if restricted {
        response.headers_mut().insert(HEADER_AUDIENCE, HeaderValue::from_static("restricted"));
    }
    no_store(response)
}

/// `GET /home/<user>/<ns>/@/<key>`: the entry's current Head; 404 also when not visible (E11).
async fn entry_head(State(state): State<AppState>, headers: HeaderMap, Path((user, ns, key)): Path<(String, String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    let station = match station_of(&state, &user).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let reader = match reader(&state, &station, &headers, &q).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let entry = match station.entry_for_path(&ns, &key) {
        Ok(e) => e,
        Err(e) => return error_response(e),
    };
    match station.read_entry_head(&reader, &entry).await {
        Ok(Some((jwt, restricted))) => named_object_response(jwt, "application/cyfs-named-object+jwt", restricted),
        Ok(None) => (StatusCode::NOT_FOUND, "no such entry").into_response(),
        Err(e) => error_response(e),
    }
}

/// An object readable through `/home/<user>/objects/…`, or through the zone feed's listed entries.
async fn find_object(state: &AppState, user: &str, headers: &HeaderMap, q: &HashMap<String, String>, id: &str) -> Result<Option<(Arc<Station>, Reader, crate::objects::StoredObject)>, Response> {
    if user == ZONE_FEED {
        let viewer = viewer(state, headers, q).await?;
        return match state.host.zone_object(&viewer, id).await {
            Ok(Some((station, obj))) => {
                let reader = station.reader_for(&viewer);
                Ok(Some((station, reader, obj)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(error_response(e)),
        };
    }
    let station = station_of(state, user).await?;
    let reader = reader(state, &station, headers, q).await?;
    match station.read_object(&reader, id).await {
        Ok(Some(obj)) => Ok(Some((station, reader, obj))),
        Ok(None) => Ok(None),
        Err(e) => Err(error_response(e)),
    }
}

/// `GET /home/<user>/objects/<id>`: knowing an ObjId grants nothing (§4.5, A32).
async fn object(State(state): State<AppState>, headers: HeaderMap, Path((user, id)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    match find_object(&state, &user, &headers, &q, &id).await {
        Err(r) => r,
        Ok(Some((station, _, obj))) => {
            let restricted = station.object_restricted(&obj.obj_id).await.unwrap_or(false);
            let (wire, content_type) = obj.wire();
            named_object_response(wire, content_type, restricted)
        }
        Ok(None) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// `GET /home/<user>/objects/<file id>/content`: bytes of a single-chunk FileObject, for
/// media elements (session token may come as `?access=`).
async fn object_content(State(state): State<AppState>, headers: HeaderMap, Path((user, id)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    let id = normalize_obj_id(&id);
    if obj_type_of(&id).as_deref() != Some(OBJ_TYPE_FILE) {
        return (StatusCode::BAD_REQUEST, "not a file object").into_response();
    }
    let (station, reader, file) = match find_object(&state, &user, &headers, &q, &id).await {
        Ok(Some(found)) => found,
        Ok(None) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(r) => return r,
    };
    let Some(content) = file.body.get("content").and_then(Value::as_str) else { return (StatusCode::NOT_FOUND, "empty file").into_response() };
    let data = match station.chunks.get_chunk(content).await {
        Ok(Some(d)) => d,
        Ok(None) if reader == Reader::Owner => match station.fetch_content_on_demand(&id).await {
            Ok(Some(d)) => d,
            Ok(None) => return (StatusCode::NOT_FOUND, "content not available").into_response(),
            Err(e) => return error_response(e),
        },
        Ok(None) => return (StatusCode::NOT_FOUND, "content not held here").into_response(),
        Err(e) => return error_response(e),
    };
    let mime = file.body.get("mime").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let mut response = (StatusCode::OK, data).into_response();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        response.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=86400, immutable"));
    response
}

async fn chunk(State(state): State<AppState>, headers: HeaderMap, Path((user, id)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    let station = match station_of(&state, &user).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let reader = match reader(&state, &station, &headers, &q).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    match station.read_chunk(&reader, &id).await {
        Ok(Some(data)) => {
            let mut response = (StatusCode::OK, data).into_response();
            response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
            response
        }
        Ok(None) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => error_response(e),
    }
}

async fn comments(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let station = match station_of(&state, &user).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let reader = match reader(&state, &station, &headers, &q).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let Some(target) = q.get("target") else { return (StatusCode::BAD_REQUEST, "target required").into_response() };
    match station.read_comment_view(&reader, target, q.get("type").cloned()).await {
        Ok(Some(page)) => no_store(Json(page).into_response()),
        Ok(None) => (StatusCode::NOT_FOUND, "no view for this target").into_response(),
        Err(e) => error_response(e),
    }
}

async fn profile(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if user == ZONE_FEED {
        return match state.host.zone_profile().await {
            Ok(p) => no_store(Json(p).into_response()),
            Err(e) => error_response(e),
        };
    }
    let station = match station_of(&state, &user).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let reader = match reader(&state, &station, &headers, &q).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    match station.public_profile(&reader).await {
        Ok(p) => no_store(Json(p).into_response()),
        Err(e) => error_response(e),
    }
}

async fn collector_recent(State(state): State<AppState>, Path(user): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let station = match station_of(&state, &user).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let since = q.get("since").and_then(|v| v.parse().ok()).unwrap_or(0);
    let limit = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    match station.read_collector_recent(since, limit).await {
        Ok(p) => no_store(Json(p).into_response()),
        Err(e) => error_response(e),
    }
}

fn dispatch_response(reply: crate::ingress::DispatchReply) -> Response {
    let status = StatusCode::from_u16(reply.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, serde_json::to_string(&reply).unwrap_or_default()).into_response();
    reply.result.apply_headers(response.headers_mut());
    response
}

/// `PUT /home/<user>/inbox`: CYFS dispatch of one named object (§7.4).
async fn inbox_put(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: Bytes) -> Response {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let target = state.host.home(&user).inbox();
    let station = match state.host.station(&user).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return dispatch_response(crate::ingress::DispatchReply {
                http_status: 404,
                result: ndn_lib::CyfsDispatchResult::rejected(None, target, "no-handler", false),
                admission: None,
            })
        }
        Err(e) => return error_response(e),
    };
    if !q.is_empty() {
        return dispatch_response(crate::ingress::DispatchReply {
            http_status: 400,
            result: ndn_lib::CyfsDispatchResult::rejected(None, target, "query-not-allowed", false),
            admission: None,
        });
    }
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let claimed = headers.get(ndn_lib::CYFS_HEADER_OBJ_ID).and_then(|v| v.to_str().ok()).map(str::to_string);
    let tier = headers.get(HEADER_AUDIENCE).and_then(|v| v.to_str().ok()).map(str::to_string);
    let restricted = tier.as_deref().is_some_and(|t| t != "public");
    let path = station.home().path("inbox");
    let reply = station.receive_dispatch(&host, &path, &content_type, claimed.as_deref(), restricted, &body).await;
    if let (Some(tier), true) = (&tier, reply.result.status == ndn_lib::CyfsDispatchStatus::Accepted) {
        if let Some(entry) = entry_of_body(&body) {
            let tier = if tier == "restricted" { "dids" } else { tier.as_str() };
            let _ = station.set_entry_tier(&entry, tier).await;
        }
    }
    dispatch_response(reply)
}

fn entry_of_body(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    let claims = crate::sign::decode_unverified(text.trim()).ok()?.claims;
    claims.get("entry").and_then(Value::as_str).map(str::to_string)
}

/// `GET /home/<user>/inbox?dispatch-status=<ObjId>`: whether an object was accepted here.
async fn inbox_status(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let station = match station_of(&state, &user).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let target = station.home().inbox();
    let query = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let Ok(id) = ndn_lib::parse_cyfs_dispatch_status_query(&query) else {
        return (StatusCode::BAD_REQUEST, "dispatch-status=<ObjId> required").into_response();
    };
    let _ = headers;
    match station.inbox_receipt(&id.to_string()).await {
        Ok(Some(admission)) => {
            let mut result = ndn_lib::CyfsDispatchResult::new(Some(id), target, ndn_lib::CyfsDispatchStatus::Accepted);
            result.source = Some(ndn_lib::CyfsDispatchSource::Upstream);
            dispatch_response(crate::ingress::DispatchReply { http_status: 200, result, admission })
        }
        Ok(None) => {
            let mut response = (StatusCode::NOT_FOUND, Json(json!({ "target": target, "error": ndn_lib::CYFS_DISPATCH_ERROR_UNKNOWN }))).into_response();
            response.headers_mut().insert(ndn_lib::CYFS_HEADER_DISPATCH_ERROR, HeaderValue::from_static(ndn_lib::CYFS_DISPATCH_ERROR_UNKNOWN));
            response
        }
        Err(e) => error_response(e),
    }
}

impl Station {
    pub async fn inbox_receipt(&self, obj_id: &str) -> crate::error::HsResult<Option<Option<String>>> {
        let id = obj_id.to_string();
        self.db
            .call(move |c| Ok(rusqlite::OptionalExtension::optional(c.query_row("SELECT admission FROM inbox_receipts WHERE obj_id=?1", [id], |r| r.get::<_, Option<String>>(0)))?))
            .await
    }

    pub async fn object_restricted(&self, obj_id: &str) -> crate::error::HsResult<bool> {
        let id = obj_id.to_string();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare("SELECT p.audience FROM object_grants g JOIN published p ON p.entry=g.entry WHERE g.obj_id=?1")?;
                let audiences = stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                Ok(!audiences.is_empty() && audiences.iter().all(|a| !a.contains("\"public\"")))
            })
            .await
    }
}
