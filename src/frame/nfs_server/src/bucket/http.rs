//! Data plane `/bucket/v1/<bucket_id>/<key>` (design §6.2, §12.2): the caller's
//! session comes from a Bearer or the SSO cookies (so `<img src>` works), a
//! `bkt_` capability token from `?token=` or `Authorization: Bearer bkt_…`.

use super::{caller_of, session_from_headers, spec, Session};
use crate::acl::{Actor, Level};
use crate::auth::Credentials;
use crate::state::SharedState;
use axum::body::Body;
use axum::extract::{Path as AxPath, Query, Request, State};
use axum::http::{Method, StatusCode};
use axum::response::Response;
use bucket_store::http::{error_response, json_response, MAX_PUT_BYTES};
use bucket_store::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

fn to_axum(r: ::http::Response<bucket_store::http::HttpBody>) -> Response {
    let (parts, body) = r.into_parts();
    Response::from_parts(parts, Body::new(body))
}

fn fail(e: &BucketError) -> Response {
    to_axum(error_response(e))
}

fn bucket_token(q: &HashMap<String, String>, creds: &Credentials) -> Option<String> {
    q.get("token").cloned().or_else(|| creds.bearer().filter(|b| b.starts_with("bkt_")).map(String::from))
}

/// `GET | HEAD | PUT | DELETE /bucket/v1/{*path}`.
pub async fn handle(State(state): State<SharedState>, AxPath(path): AxPath<String>, Query(q): Query<HashMap<String, String>>, req: Request) -> Response {
    let (id, key) = match BucketId::split_path(&path) {
        Ok((id, key)) => (id, key.to_string()),
        Err(e) => return fail(&e),
    };
    let creds = Credentials::from_headers(req.headers());
    let session = match session_from_headers(&state, &creds).await {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    if let Err(e) = spec::ensure(&state, &id).await {
        return fail(&e);
    }
    let store = match state.buckets.store(&state).await {
        Ok(s) => s.clone(),
        Err(e) => return fail(&e),
    };
    let caller = caller_of(&state, &session, bucket_token(&q, &creds)).await;
    let version = match q.get("version").map(|v| v.parse::<u64>()) {
        Some(Ok(v)) => Some(v),
        Some(Err(_)) => return fail(&BucketError::InvalidArgument("version".into())),
        None => None,
    };
    let method = req.method().clone();
    match method {
        Method::GET | Method::HEAD => match store.open_for_read(&id, &caller, &key, version).await {
            Ok((bucket, binding)) => to_axum(store.serve_read(&bucket, &binding, req.headers(), method == Method::HEAD).await),
            Err(e) => fail(&e),
        },
        Method::PUT => {
            let headers = req.headers().clone();
            let bytes = match axum::body::to_bytes(req.into_body(), MAX_PUT_BYTES as usize).await {
                Ok(b) => b,
                Err(_) => return fail(&BucketError::ObjectTooLarge { size: MAX_PUT_BYTES + 1, limit: MAX_PUT_BYTES }),
            };
            match store.put_bytes(&id, &caller, &key, bytes, &headers).await {
                Ok(r) => {
                    let status = if r.created { StatusCode::CREATED } else { StatusCode::OK };
                    let v = json!({"bucket_id": id, "key": key, "obj_id": r.etag, "version": r.version, "etag": r.etag, "created": r.created});
                    to_axum(json_response(status, &v))
                }
                Err(e) => fail(&e),
            }
        }
        Method::DELETE => {
            let if_match = req.headers().get("if-match").and_then(|v| v.to_str().ok()).map(String::from);
            match store.delete(&id, &caller, &key, version, if_match).await {
                Ok(r) => to_axum(json_response(StatusCode::OK, &serde_json::to_value(r).unwrap_or_default())),
                Err(e) => fail(&e),
            }
        }
        _ => fail(&BucketError::InvalidArgument(format!("method {method} is not supported"))),
    }
}

/// `bucket.export`: materializes a data bucket snapshot's tree as a new
/// directory the user may write (`cyfs:///<root>/<path>`, must not exist).
pub async fn export(
    state: &SharedState,
    session: &Session,
    store: &Arc<BucketStore>,
    id: &BucketId,
    caller: &Caller,
    snapshot_id: &str,
    target: &str,
) -> BucketResult<Value> {
    let snap = store.snapshot_of(id, caller, snapshot_id).await?;
    if store.bucket(id)?.kind != BucketKind::Data {
        return Err(BucketError::KindMismatch("only data buckets export to files".into()));
    }
    let actor = match session {
        Session::Open | Session::Device { .. } => Actor::Admin(None),
        Session::User { user, app_key, app_session: false, .. } if app_key.starts_with("system:") => {
            if crate::auth::is_zone_owner(state, user).await {
                Actor::Admin(Some(user.clone()))
            } else {
                Actor::User(user.clone())
            }
        }
        _ => return Err(BucketError::PermissionDenied("export writes files; it needs the user's own session".into())),
    };
    let path = target
        .strip_prefix("cyfs:///")
        .ok_or_else(|| BucketError::InvalidArgument("target is a cyfs:/// path".into()))?
        .trim_end_matches('/');
    let (root, rel) = path.split_once('/').ok_or_else(|| BucketError::InvalidArgument("target needs a directory under an export root".into()))?;
    if rel.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err(BucketError::InvalidArgument("invalid target path".into()));
    }
    let root_cfg = state.config.root(root).ok_or_else(|| BucketError::InvalidArgument(format!("no export root {root}")))?;
    let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    if state.acls.level(&actor, root, parent) < Level::Write {
        return Err(BucketError::PermissionDenied(format!("cannot write cyfs:///{root}/{parent}")));
    }
    let dest = crate::fsutil::join_root(&root_cfg.path, rel);
    if dest.exists() {
        return Err(BucketError::Conflict(format!("{target} already exists")));
    }
    let tree = ndn_lib::ObjId::new(&snap.tree_obj_id).map_err(|e| BucketError::Internal(e.to_string()))?;
    store.ndm().extract_object(&tree, &dest).await.map_err(|e| BucketError::Store(e.to_string()))?;
    log::info!("bucket {id} snapshot {snapshot_id} exported to {target}");
    Ok(json!({"target": target, "snapshot_id": snapshot_id}))
}
