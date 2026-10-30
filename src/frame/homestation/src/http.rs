//! HTTP surface: kRPC at `/kapi/homestation` (each user's own HomeStation, plus `portal.*`
//! for anyone), and the cross-node protocol paths under `/home/`: the zone index and
//! DID lookup at `/home/`, each user's home at `/home/<user>/...` (stream, entries, objects,
//! chunks, comment views, profile, inbox) and the zone feed at `/home/~zone/...`.

use crate::audience::Reader;
use crate::auth::reader_from_request;
use crate::delivery::HEADER_AUDIENCE;
use crate::error::{denied, HsError};
use crate::host::{Host, Viewer};
use crate::protocol::{normalize_obj_id, obj_type_of, OBJ_TYPE_FILE, ZONE_FEED};
use crate::Station;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub const KRPC_PATH: &str = "/kapi/homestation";
pub const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub host: Arc<Host>,
    /// Standalone mode only: this zone's stand-in verify-hub for other zones' logins.
    pub hub: Option<Arc<crate::standalone_hub::StandaloneHub>>,
}

/// Response header that carries an object's Card (base protocol §6.1).
const DID_DOCUMENT_HEADER: &str = "BuckyOS-DID-Document";

pub fn router(state: AppState) -> Router {
    let objects = Arc::new(crate::obj_provider::provider(state.clone()));
    let objects_root = objects.clone();
    let cards = objects.clone();
    Router::new()
        .route(crate::obj_provider::OBJ_PATH, any(move |req| crate::obj_provider::serve(objects_root.clone(), req)))
        .route(&format!("{}/{{*rest}}", crate::obj_provider::OBJ_PATH), any(move |req| crate::obj_provider::serve(objects.clone(), req)))
        .route("/kapi/verify-hub", post(verify_hub))
        .route(KRPC_PATH, post(krpc))
        .route(&format!("{KRPC_PATH}/"), post(krpc))
        .route("/home/", get(zone_index))
        .route("/home/{user}", get(profile))
        .route("/home/{user}/feed", get(feed))
        .route("/home/{user}/profile", get(profile))
        .route("/home/{user}/comments", get(comments))
        .route("/home/{user}/index/recent", get(collector_recent))
        .route("/home/{user}/objects/{id}", get(object))
        .route("/home/{user}/objects/{id}/content", get(object_content))
        .route("/home/{user}/objects/{id}/ref", get(object_ref))
        .route("/home/{user}/chunks/{id}", get(chunk))
        .route("/home/{user}/inbox", axum::routing::put(inbox_put).get(inbox_status))
        .route("/home/{user}/{ns}/@/{key}", get(entry_head))
        .route("/healthz", get(|| async { "ok" }))
        // A public read of a home object carries its Card, so the https link is the object too.
        .layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
            let cards = cards.clone();
            async move {
                let path = req.uri().path();
                let lookup = (req.method() == axum::http::Method::GET && path.starts_with("/home/") && path != "/home/")
                    .then(|| (path.to_string(), req.headers().clone()));
                let mut response = next.run(req).await;
                if let Some((path, headers)) = lookup.filter(|_| response.status().is_success()) {
                    if let Some(card) = crate::obj_provider::card_header(&cards, &path, &headers).await {
                        response.headers_mut().insert(DID_DOCUMENT_HEADER, card);
                    }
                }
                response
            }
        }))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
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

/// A failed call; a denial also carries its code and reason (contract §4.1).
fn rpc_failure(seq: u64, e: &HsError) -> Response {
    let mut body = json!({ "error": e.to_string(), "sys": [seq] });
    if let Some(denial) = e.denial() {
        body["code"] = json!(e.code());
        body["data"] = json!(denial);
    }
    Json(body).into_response()
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
            Err(HsError::Unauthorized(_)) if portal => None,
            Err(e) => return rpc_failure(seq, &e),
        },
        None if portal => None,
        None => return rpc_error(seq, "session token required"),
    };
    if method == "object.register" {
        let Some(token) = token else { return rpc_error(seq, "session token required") };
        if caller.as_ref().is_some_and(|caller| caller.restricted) {
            return rpc_failure(seq, &denied(buckyos_api::DenyReason::SessionKindNotAllowed, "object.register"));
        }
        // An upload bound in the caller's scratch bucket is checked by the station; any other
        // object must be readable by the caller's own NDM session.
        if params.get("key").is_none() {
            if let Err(error) = authorize_import(token, params.get("objId").and_then(Value::as_str).unwrap_or_default()).await {
                return rpc_failure(seq, &error);
            }
        }
    }
    match state.host.handle_rpc(caller.as_ref(), &method, params).await {
        Ok(v) => Json(json!({ "result": v, "sys": [seq] })).into_response(),
        Err(e) => rpc_failure(seq, &e),
    }
}

fn error_response(e: HsError) -> Response {
    let mut body = json!({ "error": e.code(), "message": e.message() });
    if let Some(denial) = e.denial() {
        body["data"] = json!(denial);
    }
    (e.status(), Json(body)).into_response()
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

/// Reader of one user's home: that user (owner view), another identity, or anonymous.
async fn reader(state: &AppState, station: &Station, headers: &HeaderMap, query: &HashMap<String, String>) -> Result<Reader, Response> {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    reader_from_request(
        state.host.auth.as_ref(),
        station.owner(),
        authorization,
        query.get("access").map(String::as_str),
    )
    .await
    .map_err(error_response)
}

/// Reader of the zone feed: no owner there, each listed entry is judged by its own station.
async fn viewer(state: &AppState, headers: &HeaderMap, query: &HashMap<String, String>) -> Result<Viewer, Response> {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    let reader = reader_from_request(
        state.host.auth.as_ref(),
        "",
        authorization,
        query.get("access").map(String::as_str),
    )
    .await
    .map_err(error_response)?;
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
pub(crate) async fn feed(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
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
pub(crate) async fn entry_head(State(state): State<AppState>, headers: HeaderMap, Path((user, ns, key)): Path<(String, String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
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

/// Single-range `bytes=` header: Ok(None) = whole body, Err = unsatisfiable.
fn byte_range(header: Option<&str>, len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = header.and_then(|h| h.strip_prefix("bytes=")) else { return Ok(None) };
    if spec.contains(',') {
        return Ok(None);
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    if len == 0 {
        return Err(());
    }
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => (len.saturating_sub(n.parse::<u64>().map_err(|_| ())?.max(1)), len - 1),
        (s, "") => (s.parse().map_err(|_| ())?, len - 1),
        (s, e) => (s.parse().map_err(|_| ())?, e.parse::<u64>().map_err(|_| ())?.min(len - 1)),
    };
    if start > end || start >= len {
        return Err(());
    }
    Ok(Some((start, end)))
}

/// `GET /home/<user>/objects/<file id>/content`: FileObject bytes, streamed with
/// Range, for media elements and downloads (session token may come as `?access=`).
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
    let opened = match station.store.open_file(&id).await {
        Ok(Some(opened)) => opened,
        Ok(None) if reader == Reader::Owner => match station.fetch_content_on_demand(&id).await {
            Ok(Some(_)) => match station.store.open_file(&id).await {
                Ok(Some(opened)) => opened,
                Ok(None) => return (StatusCode::NOT_FOUND, "content not available").into_response(),
                Err(e) => return error_response(e),
            },
            Ok(None) => return (StatusCode::NOT_FOUND, "content not available").into_response(),
            Err(e) => return error_response(e),
        },
        Ok(None) => return (StatusCode::NOT_FOUND, "content not held here").into_response(),
        Err(e) => return error_response(e),
    };
    let key = id.clone();
    let metadata = station.db.call(move |c| crate::objects::import_metadata(c, &key)).await.unwrap_or(Value::Null);
    let mime = metadata.get("mime").or_else(|| file.body.get("mime_type")).or_else(|| file.body.get("mime")).and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let (mut content, len) = opened;
    let range = match byte_range(headers.get(header::RANGE).and_then(|v| v.to_str().ok()), len) {
        Ok(r) => r,
        Err(()) => return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{len}"))]).into_response(),
    };
    let (start, end) = range.unwrap_or((0, len.saturating_sub(1)));
    if start > 0 {
        use tokio::io::AsyncReadExt;
        if let Err(e) = tokio::io::copy(&mut (&mut content).take(start), &mut tokio::io::sink()).await {
            return error_response(HsError::Unavailable(e.to_string()));
        }
    }
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut response = Response::new(axum::body::Body::new(named_store::chunk_reader_to_body(content, count)));
    if range.is_some() {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{len}")) {
            response.headers_mut().insert(header::CONTENT_RANGE, v);
        }
    }
    let h = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(count));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=86400, immutable"));
    response
}

/// `GET /home/<user>/objects/<file id>/ref`: where a reader of the object may
/// read it on the bucket data plane — its binding and a short-lived read token.
async fn object_ref(State(state): State<AppState>, headers: HeaderMap, Path((user, id)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>) -> Response {
    let id = normalize_obj_id(&id);
    if obj_type_of(&id).as_deref() != Some(OBJ_TYPE_FILE) {
        return (StatusCode::BAD_REQUEST, "not a file object").into_response();
    }
    match find_object(&state, &user, &headers, &q, &id).await {
        Ok(Some((station, _, _))) => match station.content_ref(&id).await {
            Ok(Some(found)) => no_store(Json(found).into_response()),
            Ok(None) => (StatusCode::NOT_FOUND, "not bound").into_response(),
            Err(e) => error_response(e),
        },
        Ok(None) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(r) => r,
    }
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

pub(crate) async fn profile(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
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

/// `POST /kapi/verify-hub` of a standalone zone: cross-zone logins (`standalone_hub.rs`).
async fn verify_hub(State(state): State<AppState>, body: Bytes) -> Response {
    match &state.hub {
        Some(hub) => no_store(Json(hub.rpc(&body).await).into_response()),
        None => not_found("no verify-hub here"),
    }
}

/// `PUT /home/<user>/inbox`: CYFS dispatch of one named object (§7.4). Only a session of this zone
/// delivers: another zone's station signs its owner in here first (跨Zone登录断言契约 §5); each
/// object still carries its publisher's signature.
async fn inbox_put(State(state): State<AppState>, Path(user): Path<String>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: Bytes) -> Response {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let target = state.host.home(&user).inbox();
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let authenticated = match token {
        Some(token) => state.host.auth.authenticate(token.trim()).await.is_ok(),
        None => false,
    };
    if !authenticated {
        return dispatch_response(crate::ingress::DispatchReply {
            http_status: 401,
            result: ndn_lib::CyfsDispatchResult::rejected(None, target, "unauthenticated", false),
            admission: None,
        });
    }
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
            .call(move |c| {
                Ok(rusqlite::OptionalExtension::optional(c.query_row(
                    "SELECT admission FROM inbox_receipts WHERE obj_id=?1 AND owner=?2",
                    rusqlite::params![id, c.owner],
                    |r| r.get::<_, Option<String>>(0),
                ))?)
            })
            .await
    }

    pub async fn object_restricted(&self, obj_id: &str) -> crate::error::HsResult<bool> {
        let id = obj_id.to_string();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT p.audience FROM object_grants g JOIN published p ON p.owner=g.owner AND p.entry=g.entry WHERE g.obj_id=?1 AND g.owner=?2",
                )?;
                let audiences = stmt.query_map(rusqlite::params![id, c.owner], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                Ok(!audiences.is_empty() && audiences.iter().all(|a| !a.contains("\"public\"")))
            })
            .await
    }
}

async fn authorize_import(token: &str, obj_id: &str) -> crate::error::HsResult<()> {
    let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| HsError::Unavailable(e.to_string()))?;
    let base = runtime.get_zone_service_url(buckyos_api::NFS_SERVER_SERVICE_NAME, false).await.map_err(|e| HsError::Unavailable(e.to_string()))?;
    let mut url = url::Url::parse(&base).map_err(|e| HsError::Unavailable(e.to_string()))?;
    url.set_path("/ndm/v1/store/get_object");
    let response = reqwest::Client::new().post(url).bearer_auth(token).json(&json!({ "obj_id": obj_id })).send().await.map_err(|e| HsError::Unavailable(e.to_string()))?;
    if !response.status().is_success() { return Err(HsError::Forbidden("attachment is not readable by this upload session".into())); }
    Ok(())
}
