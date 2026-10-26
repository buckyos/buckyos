//! The S3 compatible face (design §12.3) on `/s3/*` of the zone host and on
//! every path of `s3.<zone>`. cyfs-ndn `s3_gateway` speaks the protocol; this
//! module answers which `(user, app)` a key acts for and its RBAC.

use super::spec;
use crate::config::AuthMode;
use crate::state::{AppState, SharedState};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use bucket_store::{Action, BucketInfo, Caller, IdentityPolicy, S3Key};
use buckyos_http_server::{server_err, ServerErrorCode};
use http_body_util::BodyExt;
use s3_gateway::{S3Gateway, S3Host};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const PATH_PREFIX: &str = "/s3";

/// RBAC of a key's `(user, app)`: the same first layer as sessions, without a token.
struct KeyPolicy {
    state: SharedState,
    user: String,
    app_key: String,
    cache: Mutex<HashMap<(String, Action), bool>>,
}

#[async_trait::async_trait]
impl IdentityPolicy for KeyPolicy {
    async fn allowed(&self, bucket: &BucketInfo, key: &str, action: Action) -> bool {
        let resource = bucket.bucket_id.resource(key);
        if let Some(v) = self.cache.lock().unwrap().get(&(resource.clone(), action)) {
            return *v;
        }
        let allowed = match &self.state.config.auth {
            AuthMode::Disabled => true,
            AuthMode::TestKey { .. } => rbac::enforce(&self.user, &self.app_key, &resource, action.as_str(), None).await,
            AuthMode::Runtime => match buckyos_api::get_buckyos_api_runtime() {
                Ok(runtime) => runtime.enforce_principal(&self.user, &self.app_key, &resource, action.as_str()).await.unwrap_or(false),
                Err(_) => false,
            },
        };
        self.cache.lock().unwrap().insert((resource, action), allowed);
        allowed
    }

    async fn holds(&self, _obj_id: &str) -> bool {
        false
    }
}

struct Host {
    state: SharedState,
}

#[async_trait::async_trait]
impl S3Host for Host {
    async fn caller(&self, key: &S3Key) -> Caller {
        let (groups, zone_user) = self.state.buckets.user_groups(&self.state, &key.user_id).await;
        let mut caller = Caller::anonymous(None);
        caller.user = Some(key.user_id.clone());
        caller.app = Some(key.appid.clone());
        caller.groups = groups;
        caller.zone_user = zone_user;
        caller.identity = Arc::new(KeyPolicy {
            state: self.state.clone(),
            user: key.user_id.clone(),
            app_key: format!("app:{}", key.appid),
            cache: Default::default(),
        });
        caller
    }

    async fn sync_buckets(&self, appid: &str, user: &str) {
        if let Err(e) = spec::sync_app(&self.state, appid, user, false).await {
            log::debug!("s3: buckets of {appid} for {user}: {e}");
        }
    }
}

async fn serve(state: SharedState, req: Request, prefix: &str) -> Response {
    let store = match state.buckets.store(&state).await {
        Ok(s) => s.clone(),
        Err(e) => {
            let err = s3_gateway::S3Error::internal(e.to_string());
            let (parts, body) = s3_gateway::error_response(&err, req.uri().path(), "-").into_parts();
            return Response::from_parts(parts, Body::new(body));
        }
    };
    let (parts, body) = req.into_parts();
    let body = body.map_err(|e| server_err!(ServerErrorCode::StreamError, "{e}")).boxed_unsync();
    let gateway = S3Gateway::new(store);
    let (parts_out, body_out) = gateway.handle(&Host { state }, http::Request::from_parts(parts, body), prefix).await.into_parts();
    Response::from_parts(parts_out, Body::new(body_out))
}

/// `/s3` and `/s3/{*path}` on the zone host (path style, endpoint `https://<zone>/s3`).
pub async fn handle(State(state): State<SharedState>, req: Request) -> Response {
    serve(state, req, PATH_PREFIX).await
}

fn is_s3_host(state: &AppState, req: &Request) -> bool {
    let Some(zone) = &state.config.share.zone_host else { return false };
    crate::share::serving::request_host(req.headers()).is_some_and(|h| h == format!("s3.{zone}"))
}

/// Everything on `s3.<zone>` is S3 (path style, endpoint `https://s3.<zone>`).
pub async fn host_route(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    if is_s3_host(&state, &req) {
        return serve(state, req, "").await;
    }
    next.run(req).await
}

/// Aborts multipart uploads abandoned longer than their part hold.
pub async fn abort_stale_uploads(state: &SharedState) {
    if let Ok(store) = state.buckets.store(state).await {
        let n = S3Gateway::new(store.clone()).abort_stale_uploads().await;
        if n > 0 {
            log::info!("s3: aborted {n} abandoned multipart uploads");
        }
    }
}
