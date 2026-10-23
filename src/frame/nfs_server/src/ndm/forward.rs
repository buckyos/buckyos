//! `/ndm/v1/objects/lookup`, `/ndm/v1/store/{method}` and `/ndm/proxy/v1/*`:
//! authorized here, then served by cyfs-ndn's `NamedDataMgrZoneGateway` /
//! `NamedDataMgrNodeGateway` (the proxy with its restricted calls enabled,
//! because this layer decides them).
//!
//! Method classes (store API and proxy RPC share the names):
//! - existence checks need `uploads` or `objects` (they serve upload dedupe);
//! - reads need `objects`, or that the caller holds the object;
//! - `put_object` needs `uploads`, an object whose id matches its body, and
//!   children the caller holds (uploaded, written or registered) unless it may
//!   `manage`; `add_chunk_by_same_as` likewise needs the chunk list held;
//! - `pin`/`unpin`/`unpin_owner` need `manage`, or `retain` with an owner
//!   under the caller's own prefix;
//! - everything else (deletes, GC, fs anchors, edges, debug) needs `manage`.

use super::{allowed, denied, gateways, hold, may_read, ndm_error, principal, store, Principal, RES_MANAGE, RES_OBJECTS, RES_RETAIN, RES_UPLOADS};
use crate::state::{AppState, SharedState};
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use buckyos_http_server::{HttpServer, StreamInfo};
use http_body_util::BodyExt;
use ndn_lib::{ChunkList, KnownStandardObject, ObjId, OBJ_TYPE_CHUNK_LIST};
use serde_json::Value;
use rusqlite::OptionalExtension;

const MAX_CONTROL_BODY: usize = 1024 * 1024;
const MAX_CHUNK_BODY: usize = 32 * 1024 * 1024;

enum Need {
    Exists,
    Read(String),
    Register,
    SameAs,
    Retain,
    Manage,
}

fn classify(method: &str, args: &Value) -> Need {
    let field = |k: &str| args[k].as_str().unwrap_or_default().to_string();
    match method {
        "is_object_stored" | "is_object_exist" | "have_chunk" | "query_chunk_state" => Need::Exists,
        "get_object" | "open_object" | "query_object_by_id" => Need::Read(field("obj_id")),
        "get_dir_child" => Need::Read(field("dir_obj_id")),
        "put_object" => Need::Register,
        "add_chunk_by_same_as" => Need::SameAs,
        "pin" | "unpin" | "unpin_owner" => Need::Retain,
        _ => Need::Manage,
    }
}

/// The ids an object's body fixes, after checking that the body hashes to `obj_id`.
pub fn verified_children(obj_id: &ObjId, obj_data: &str) -> Result<Vec<ObjId>, String> {
    if obj_id.is_chunk() {
        return Err("put_object does not take chunk ids".into());
    }
    if obj_id.obj_type == OBJ_TYPE_CHUNK_LIST {
        let chunks: Vec<ndn_lib::ChunkId> = serde_json::from_str(obj_data).map_err(|e| format!("chunk list: {e}"))?;
        let (actual, _) = ChunkList::from_chunk_list(chunks.clone()).map_err(|e| e.to_string())?.gen_obj_id();
        if actual != *obj_id {
            return Err(format!("the chunk list hashes to {}, not {}", actual.to_string(), obj_id.to_string()));
        }
        return Ok(chunks.iter().map(|c| c.to_obj_id()).collect());
    }
    ndn_lib::verify_named_object_from_str(obj_id, obj_data).map_err(|e| e.to_string())?;
    Ok(KnownStandardObject::from_obj_data(obj_id, obj_data)
        .ok()
        .and_then(|known| known.get_child_objs().ok())
        .map(|children| children.into_iter().map(|(id, _)| id).collect())
        .unwrap_or_default())
}

/// What to remember once the call succeeded.
enum After {
    Nothing,
    Holding(ObjId, &'static str),
}

async fn authorize(state: &AppState, p: &Principal, method: &str, args: &Value) -> Result<After, Response> {
    let manage = || allowed(state, p, RES_MANAGE, "write");
    match classify(method, args) {
        Need::Exists => {
            if allowed(state, p, RES_UPLOADS, "write").await || allowed(state, p, RES_OBJECTS, "read").await {
                return Ok(After::Nothing);
            }
            Err(denied("query the named store"))
        }
        Need::Read(id) => {
            if may_read(state, p, &normalized(&id)).await {
                return Ok(After::Nothing);
            }
            Err(denied(&format!("read {id}")))
        }
        Need::Register => {
            if !allowed(state, p, RES_UPLOADS, "write").await {
                return Err(denied("register objects"));
            }
            let obj_id = ObjId::new(args["obj_id"].as_str().unwrap_or_default())
                .map_err(|e| ndm_error(StatusCode::BAD_REQUEST, "invalid_id", e))?;
            let children = verified_children(&obj_id, args["obj_data"].as_str().unwrap_or_default())
                .map_err(|e| ndm_error(StatusCode::BAD_REQUEST, "invalid_data", e))?;
            if let Some(missing) = children.iter().find(|c| !super::holds(state, p, &ObjId::to_string(c))) {
                if !manage().await {
                    return Err(ndm_error(
                        StatusCode::FORBIDDEN,
                        "permission_denied",
                        format!("{} references {}, which this session did not upload or register", obj_id.to_string(), missing.to_string()),
                    ));
                }
            }
            Ok(After::Holding(obj_id, "register"))
        }
        Need::SameAs => {
            if !allowed(state, p, RES_UPLOADS, "write").await {
                return Err(denied("register objects"));
            }
            let list = normalized(args["chunk_list_id"].as_str().unwrap_or_default());
            if !super::holds(state, p, &list) && !manage().await {
                return Err(denied(&format!("use chunk list {list}")));
            }
            let big = ObjId::new(args["big_chunk_id"].as_str().unwrap_or_default())
                .map_err(|e| ndm_error(StatusCode::BAD_REQUEST, "invalid_id", e))?;
            Ok(After::Holding(big, "register"))
        }
        Need::Retain => {
            if manage().await {
                return Ok(After::Nothing);
            }
            let owner = args["owner"].as_str().unwrap_or_default();
            if owner.starts_with(&p.pin_prefix()) && allowed(state, p, RES_RETAIN, "write").await {
                return Ok(After::Nothing);
            }
            Err(denied(&format!("change the retention of owner {owner:?}")))
        }
        Need::Manage => {
            if manage().await {
                return Ok(After::Nothing);
            }
            Err(denied(&format!("call {method}")))
        }
    }
}

fn normalized(id: &str) -> String {
    ObjId::new(id).map(|o| o.to_string()).unwrap_or_else(|_| id.to_string())
}

/// Hands a (buffered) request to a cyfs-ndn handler and its response back to axum.
async fn serve<S: HttpServer>(gw: &S, parts: http::request::Parts, body: Bytes) -> Response {
    let body = http_body_util::Full::new(body).map_err(|never| match never {}).boxed();
    let req = http::Request::from_parts(parts, body);
    match gw.serve_request(req, StreamInfo::new("127.0.0.1".into())).await {
        Ok(resp) => {
            let (parts, body) = resp.into_parts();
            Response::from_parts(parts, Body::new(body))
        }
        Err(e) => ndm_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", e.msg()),
    }
}

async fn buffered(req: Request, limit: usize) -> Result<(http::request::Parts, Bytes), Response> {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, limit)
        .await
        .map_err(|e| ndm_error(StatusCode::BAD_REQUEST, "invalid_data", format!("read body: {e}")))?;
    Ok((parts, bytes))
}

/// Buffers a small JSON body so it can be checked and then passed on unchanged.
async fn json_body(req: Request) -> Result<(http::request::Parts, Bytes, Value), Response> {
    let (parts, bytes) = buffered(req, MAX_CONTROL_BODY).await?;
    let args = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((parts, bytes, args))
}

async fn authorized_call<S: HttpServer>(state: &AppState, gw: &S, method: &str, req: Request) -> Response {
    let p = match principal(state, req.headers()).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (parts, bytes, args) = match json_body(req).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let after = match authorize(state, &p, method, &args).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let resp = serve(gw, parts, bytes).await;
    if resp.status().is_success() {
        if let After::Holding(id, how) = after {
            if let Ok(store) = store(state).await {
                hold(state, store, &p, &id, how).await;
            }
        }
    }
    resp
}

/// `GET /ndm/v1/objects/lookup`: upload dedupe.
pub async fn lookup(State(state): State<SharedState>, req: Request) -> Response {
    let p = match principal(&state, req.headers()).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !(allowed(&state, &p, RES_UPLOADS, "write").await || allowed(&state, &p, RES_OBJECTS, "read").await) {
        return denied("query the named store");
    }
    let scope = req.uri().query().unwrap_or_default().split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "scope").then(|| percent_encoding::percent_decode_str(value).decode_utf8_lossy().into_owned())
    });
    if !matches!(scope.as_deref(), Some("app" | "global")) {
        return ndm_error(StatusCode::BAD_REQUEST, "invalid_data", "scope must be app or global");
    }
    let id = req.uri().query().unwrap_or_default().split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "quick_hash").then(|| percent_encoding::percent_decode_str(value).decode_utf8_lossy().into_owned())
    });
    if let Some(id) = &id {
        let mapping: Option<(String, u64)> = state.db.lock().query_row(
            "SELECT content_id,size FROM ndm_import_qcids WHERE principal=?1 AND qcid=?2",
            rusqlite::params![p.key(), normalized(id)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().ok().flatten();
        if let Some((content, size)) = mapping {
            if let (Ok(store), Ok(content_id)) = (store(&state).await, ObjId::new(&content)) {
                if store.is_object_stored(&content_id, None).await.unwrap_or(false) {
                    let reusable = may_read(&state, &p, &content).await;
                    return axum::Json(serde_json::json!({"object_id": normalized(id), "content_id": content, "scope": scope, "state": "completed", "chunk_size": size, "reusable": reusable})).into_response();
                }
            }
        }
    }
    let reusable = match id {
        Some(id) => may_read(&state, &p, &normalized(&id)).await,
        None => false,
    };
    let (parts, bytes) = match buffered(req, MAX_CONTROL_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match gateways(&state).await {
        Ok((zone, _)) => {
            let response = serve(zone.as_ref(), parts, bytes).await;
            if !response.status().is_success() {
                return response;
            }
            let status = response.status();
            match axum::body::to_bytes(response.into_body(), MAX_CONTROL_BODY).await {
                Ok(body) => match serde_json::from_slice::<Value>(&body) {
                    Ok(mut value) if value.is_object() => {
                        value["reusable"] = Value::Bool(reusable);
                        (status, axum::Json(value)).into_response()
                    }
                    _ => ndm_error(StatusCode::BAD_GATEWAY, "invalid_data", "invalid lookup response"),
                },
                Err(e) => ndm_error(StatusCode::BAD_GATEWAY, "invalid_data", e),
            }
        }
        Err(r) => r,
    }
}

/// `POST /ndm/v1/store/{method}`.
pub async fn store_api(State(state): State<SharedState>, AxPath(method): AxPath<String>, req: Request) -> Response {
    match gateways(&state).await {
        Ok((zone, _)) => authorized_call(&state, zone.as_ref(), &method, req).await,
        Err(r) => r,
    }
}

/// `POST /ndm/proxy/v1/rpc/{method}`.
pub async fn proxy_rpc(State(state): State<SharedState>, AxPath(method): AxPath<String>, req: Request) -> Response {
    match gateways(&state).await {
        Ok((_, proxy)) => authorized_call(&state, proxy.as_ref(), &method, req).await,
        Err(r) => r,
    }
}

/// `POST /ndm/proxy/v1/read/{kind}`: readers need `objects` or a holding of what they open.
pub async fn proxy_read(State(state): State<SharedState>, req: Request) -> Response {
    let p = match principal(&state, req.headers()).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (parts, bytes, args) = match json_body(req).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let target = ["obj_id", "chunk_id", "chunk_list_id"].iter().find_map(|k| args[*k].as_str()).unwrap_or_default();
    if !may_read(&state, &p, &normalized(target)).await {
        return denied(&format!("read {target}"));
    }
    match gateways(&state).await {
        Ok((_, proxy)) => serve(proxy.as_ref(), parts, bytes).await,
        Err(r) => r,
    }
}

/// `PUT /ndm/proxy/v1/write/chunk/{chunk_id}`: an upload; the proxy checks the hash.
pub async fn proxy_write(State(state): State<SharedState>, AxPath(chunk_id): AxPath<String>, req: Request) -> Response {
    let p = match principal(&state, req.headers()).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if !allowed(&state, &p, RES_UPLOADS, "write").await {
        return denied("upload to the named store");
    }
    let Ok(chunk) = ObjId::new(&chunk_id) else {
        return ndm_error(StatusCode::BAD_REQUEST, "invalid_id", format!("invalid chunk id {chunk_id}"));
    };
    let (mut parts, bytes) = match buffered(req, MAX_CHUNK_BODY).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    // The handler reads the id from the raw path; hand it the decoded, canonical form.
    if let Ok(uri) = format!("/ndm/proxy/v1/write/chunk/{}", chunk.to_string()).parse() {
        parts.uri = uri;
    }
    let resp = match gateways(&state).await {
        Ok((_, proxy)) => serve(proxy.as_ref(), parts, bytes).await,
        Err(r) => return r,
    };
    if resp.status().is_success() {
        if let Ok(store) = store(&state).await {
            hold(&state, store, &p, &chunk, "write").await;
        }
    }
    resp
}
