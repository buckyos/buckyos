//! kRPC `bucket.*` at `/kapi/nfs-server` (design §12.1). Callers authenticate
//! with the kRPC session token; a `bkt_` capability token may ride in
//! `params.access_token`.

use super::{caller_of, is_admin, session_of, spec, user_of, Session};
use crate::state::SharedState;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bucket_store::*;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const KRPC_PATH: &str = "/kapi/nfs-server";

fn arg<T: DeserializeOwned>(p: &Value, key: &str) -> BucketResult<T> {
    serde_json::from_value(p.get(key).cloned().unwrap_or(Value::Null)).map_err(|e| BucketError::InvalidArgument(format!("{key}: {e}")))
}

fn opt<T: DeserializeOwned>(p: &Value, key: &str) -> BucketResult<Option<T>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| BucketError::InvalidArgument(format!("{key}: {e}"))),
    }
}

fn whole<T: DeserializeOwned>(p: &Value) -> BucketResult<T> {
    serde_json::from_value(p.clone()).map_err(|e| BucketError::InvalidArgument(e.to_string()))
}

fn to_value<T: serde::Serialize>(v: T) -> BucketResult<Value> {
    serde_json::to_value(v).map_err(BucketError::from)
}

fn rpc_error(seq: u64, e: &BucketError) -> Response {
    let mut body = json!({"error": format!("{}: {}", e.code(), e), "code": e.code(), "sys": [seq]});
    if let Some(data) = e.data() {
        body["data"] = data;
    }
    Json(body).into_response()
}

pub async fn krpc(State(state): State<SharedState>, body: Bytes) -> Response {
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(0, &BucketError::InvalidArgument(format!("invalid request: {e}"))),
    };
    let seq = req["sys"][0].as_u64().unwrap_or(0);
    let Some(method) = req["method"].as_str() else { return rpc_error(seq, &BucketError::InvalidArgument("method required".into())) };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let token = req["sys"][1].as_str().filter(|t| !t.trim().is_empty());
    let session = match session_of(&state, token).await {
        Ok(s) => s,
        Err(e) => return rpc_error(seq, &e),
    };
    match dispatch(&state, &session, method, &params).await {
        Ok(v) => Json(json!({"result": v, "sys": [seq]})).into_response(),
        Err(e) => {
            if !matches!(e, BucketError::NoSuchKey(_) | BucketError::PreconditionFailed(_)) {
                log::info!("bucket rpc {method} failed: {e}");
            }
            rpc_error(seq, &e)
        }
    }
}

async fn bucket_arg(state: &SharedState, p: &Value) -> BucketResult<BucketId> {
    let id: BucketId = arg::<String>(p, "bucket_id")?.parse()?;
    spec::ensure(state, &id).await?;
    Ok(id)
}

/// S3 keys are managed by the user from their own UI, for themselves (§12.3).
fn own_user(session: &Session) -> BucketResult<String> {
    match session {
        Session::Open => Ok("local".into()),
        Session::User { user, app_key, app_session: false, .. } if app_key.starts_with("system:") => Ok(user.clone()),
        _ => Err(BucketError::PermissionDenied("S3 access keys are managed from the user's own session".into())),
    }
}

fn require_admin(admin: bool) -> BucketResult<()> {
    if admin {
        Ok(())
    } else {
        Err(BucketError::PermissionDenied("admin only".into()))
    }
}

pub async fn dispatch(state: &SharedState, session: &Session, method: &str, p: &Value) -> BucketResult<Value> {
    let store = state.buckets.store(state).await?.clone();
    let caller = caller_of(state, session, opt::<String>(p, "access_token")?).await;
    let c = &caller;
    match method {
        "bucket.list" => {
            if let Some(user) = user_of(session) {
                spec::sync_user(state, user).await?;
            }
            spec::sync_zone(state).await?;
            let scope: Option<BucketScope> = opt(p, "scope")?;
            let appid: Option<String> = opt(p, "appid")?;
            let mut out = Vec::new();
            for b in store.list_buckets(None, appid.as_deref(), scope)? {
                if store.check_access(&b.bucket_id, c, Action::List, None, None).await?.allowed {
                    out.push(b);
                }
            }
            to_value(out)
        }
        "bucket.stat" => {
            let id = bucket_arg(state, p).await?;
            let info = store.stat(&id, c).await?;
            let mut v = to_value(&info)?;
            v["issues"] = json!(store.issues(&id)?);
            Ok(v)
        }
        "bucket.delete_bucket" => {
            let id = bucket_arg(state, p).await?;
            store.delete_bucket(&id, c).await?;
            Ok(json!({"deleted": true}))
        }
        "bucket.set_versioning" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.set_versioning(&id, c, arg(p, "enabled")?).await?)
        }
        "bucket.put" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.put(&id, c, whole(p)?).await?)
        }
        "bucket.get" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.get(&id, c, &arg::<String>(p, "key")?, opt(p, "version")?).await?)
        }
        "bucket.head" => {
            let id = bucket_arg(state, p).await?;
            let (binding, availability) = store.head(&id, c, &arg::<String>(p, "key")?, opt(p, "version")?).await?;
            Ok(json!({"binding": binding, "availability": availability}))
        }
        "bucket.delete" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.delete(&id, c, &arg::<String>(p, "key")?, opt(p, "version")?, opt(p, "if_match")?).await?)
        }
        "bucket.list_objects" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.list_objects(&id, c, &whole(p)?).await?)
        }
        "bucket.list_versions" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.list_versions(&id, c, &arg::<String>(p, "key")?).await?)
        }
        "bucket.copy" => {
            let req: CopyRequest = whole(p)?;
            spec::ensure(state, &req.src_bucket_id.parse()?).await?;
            spec::ensure(state, &req.dst_bucket_id.parse()?).await?;
            to_value(store.copy(c, req).await?)
        }
        "bucket.batch" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.batch(&id, c, arg(p, "ops")?).await?)
        }
        "bucket.set_metadata" => {
            let id = bucket_arg(state, p).await?;
            let metadata: Option<BTreeMap<String, String>> = opt(p, "metadata")?;
            let tags: Option<BTreeMap<String, String>> = opt(p, "tags")?;
            to_value(store.set_metadata(&id, c, &arg::<String>(p, "key")?, opt(p, "content_type")?, metadata, tags).await?)
        }
        "bucket.grant" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.grant(&id, c, whole(p)?).await?)
        }
        "bucket.revoke" => {
            store.revoke(c, &arg::<String>(p, "grant_id")?).await?;
            Ok(json!({"revoked": true}))
        }
        "bucket.list_grants" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.list_grants(&id, c).await?)
        }
        "bucket.create_access_token" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.create_access_token(&id, c, whole(p)?).await?)
        }
        "bucket.revoke_access_token" => {
            store.revoke_access_token(c, &arg::<String>(p, "token_id")?).await?;
            Ok(json!({"revoked": true}))
        }
        "bucket.list_access_tokens" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.list_access_tokens(&id, c).await?)
        }
        "bucket.check_access" => {
            let id = bucket_arg(state, p).await?;
            let key: Option<String> = opt(p, "key")?;
            let prefix: Option<String> = opt(p, "prefix")?;
            to_value(store.check_access(&id, c, arg(p, "action")?, key.as_deref(), prefix.as_deref()).await?)
        }
        "bucket.get_quota" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.get_quota(&id, c).await?)
        }
        "bucket.set_quota" => {
            let id = bucket_arg(state, p).await?;
            let admin = is_admin(state, session).await;
            to_value(store.set_quota(&id, c, whole(p)?, admin).await?)
        }
        "bucket.clear_quota" => {
            require_admin(is_admin(state, session).await)?;
            let id = bucket_arg(state, p).await?;
            store.clear_quota(&id)?;
            to_value(store.get_quota(&id, &Caller::system()).await?)
        }
        "bucket.get_pool_quota" => {
            let kind: PoolKind = arg(p, "kind")?;
            let id: String = arg(p, "id")?;
            let own = match (kind, user_of(session)) {
                (PoolKind::User, Some(u)) => id == u,
                (PoolKind::App, Some(u)) => id.rsplit_once('@').is_some_and(|(_, owner)| owner == u),
                _ => false,
            };
            if !own {
                require_admin(is_admin(state, session).await)?;
            }
            to_value(store.get_pool_quota(kind, &id)?)
        }
        "bucket.set_pool_quota" => {
            require_admin(is_admin(state, session).await)?;
            let kind: PoolKind = arg(p, "kind")?;
            to_value(store.set_pool_quota(kind, &arg::<String>(p, "id")?, opt(p, "max_bytes")?, &c.principal())?)
        }
        "bucket.usage_report" => {
            let scope: ReportScope = arg(p, "kind")?;
            let id: Option<String> = opt(p, "id")?;
            let days: u32 = opt(p, "days")?.unwrap_or(30);
            let own = scope == ReportScope::User && id.as_deref().is_some_and(|i| Some(i) == user_of(session));
            if !own {
                require_admin(is_admin(state, session).await)?;
            }
            to_value(store.usage_report(scope, id.as_deref(), days)?)
        }
        "bucket.recompute_usage" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.recompute_usage_checked(&id, c).await?)
        }
        "bucket.get_lifecycle" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.get_lifecycle(&id, c).await?)
        }
        "bucket.set_lifecycle" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.set_lifecycle(&id, c, arg(p, "rules")?).await?)
        }
        "bucket.snapshot" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.snapshot(&id, c, opt(p, "label")?, opt(p, "include_noncurrent")?.unwrap_or(false)).await?)
        }
        "bucket.list_snapshots" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.list_snapshots(&id, c).await?)
        }
        "bucket.delete_snapshot" => {
            let id = bucket_arg(state, p).await?;
            store.delete_snapshot(&id, c, &arg::<String>(p, "snapshot_id")?).await?;
            Ok(json!({"deleted": true}))
        }
        "bucket.restore_from_snapshot" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.restore_from_snapshot(&id, c, &arg::<String>(p, "root_obj_id")?).await?)
        }
        "bucket.export" => {
            let id = bucket_arg(state, p).await?;
            super::http::export(state, session, &store, &id, c, &arg::<String>(p, "snapshot_id")?, &arg::<String>(p, "target")?).await
        }
        "bucket.verify_references" => {
            let id = bucket_arg(state, p).await?;
            to_value(store.verify_references(&id, c, arg(p, "refs")?).await?)
        }
        "bucket.create_s3_key" => {
            let user = own_user(session)?;
            let scope: Option<S3KeyScope> = opt(p, "scope")?;
            to_value(store.create_s3_key(&user, &arg::<String>(p, "appid")?, scope, opt(p, "expires_at")?)?)
        }
        "bucket.list_s3_keys" => to_value(store.list_s3_keys(&own_user(session)?)?),
        "bucket.revoke_s3_key" => {
            store.revoke_s3_key(&own_user(session)?, &arg::<String>(p, "access_key_id")?)?;
            Ok(json!({"revoked": true}))
        }
        "bucket.prefetch" => Err(BucketError::InvalidArgument("bucket.prefetch is not available yet (P2)".into())),
        other => Err(BucketError::InvalidArgument(format!("unknown method {other}"))),
    }
}
