//! `ambient` content is someone else's that is nice to have (bucket design §4.5.2,
//! D20): only the description is held, and when the content is gone it comes back
//! from where the binding says it came from, through `/ndn/`, without credentials.
//! A source this Zone cannot read anonymously stays `origin_unavailable`.

use super::public::{anonymous_client, pull_verified, validate_source};
use crate::state::SharedState;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bucket_store::{Availability, Binding, BucketError, BucketResult, BucketStore, DataPolicy, ListRequest};
use ndn_lib::ObjId;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A failed source is not asked again for this long; readers get `origin_unavailable`.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct AmbientFetches {
    running: Mutex<HashMap<String, Instant>>,
    failed: Mutex<HashMap<String, Instant>>,
}

/// The plain `/ndn/` URL of `root` that `origin.source` names, when it names one:
/// `cyfs://<objid>.<zone>[/]` or `https://<zone>/ndn/<objid>`.
pub fn source_url(source: &str, root: &ObjId) -> Option<String> {
    let url = match buckyos_api::parse_object_url(source).ok()? {
        buckyos_api::ObjectUrl::CyfsObject { obj_id, source_zone, inner_path } if obj_id == *root && inner_path.trim_matches('/').is_empty() => {
            buckyos_api::ndn_read_url(&format!("https://{}", source_zone?), &root.to_string(), None)
        }
        buckyos_api::ObjectUrl::Web { url } => url,
        _ => return None,
    };
    url.starts_with("https://").then_some(url)
}

impl AmbientFetches {
    fn is_running(&self, root: &str) -> bool {
        self.running.lock().unwrap().contains_key(root)
    }

    fn failed_recently(&self, root: &str) -> bool {
        self.failed.lock().unwrap().get(root).is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE)
    }

    fn start(&self, root: &str) -> bool {
        let mut running = self.running.lock().unwrap();
        if running.contains_key(root) {
            return false;
        }
        running.insert(root.to_string(), Instant::now());
        true
    }

    fn finish(&self, root: &str, ok: bool) {
        self.running.lock().unwrap().remove(root);
        let mut failed = self.failed.lock().unwrap();
        if ok {
            failed.remove(root);
        } else {
            failed.insert(root.to_string(), Instant::now());
        }
    }
}

/// Starts the pull of a binding's content; `false` when nothing can be pulled.
fn trigger(state: &SharedState, store: &BucketStore, b: &Binding) -> bool {
    let (Some(root), Some(origin)) = (b.obj_id.as_deref().and_then(|o| ObjId::new(o).ok()), b.origin.as_ref()) else { return false };
    let id = root.to_string();
    let Some(source) = source_url(&origin.source, &root) else { return false };
    let Ok(host) = validate_source(&source, &root, state.config.share.zone_host.as_deref()) else { return false };
    if state.ambient.failed_recently(&id) {
        return false;
    }
    if !state.ambient.start(&id) {
        return true;
    }
    let (state, ndm) = (state.clone(), store.ndm().clone());
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        runtime.block_on(async move {
            let outcome = match anonymous_client(&host).await {
                Ok(client) => pull_verified(&client, &ndm, &root, &source, state.config.public_cache_app_bytes).await,
                Err(e) => Err(e),
            };
            if let Err(e) = &outcome {
                log::info!("ambient {id} from {host}: {e}");
            }
            state.ambient.finish(&id, outcome.is_ok());
        })
    });
    true
}

/// A read of an `ambient` key whose content is not here: `202 + Retry-After` while it
/// is on its way, `404 origin_unavailable` once it cannot be had. `None` when the content is here.
pub async fn miss_response(state: &SharedState, store: &BucketStore, b: &Binding) -> Option<Response> {
    let root = b.obj_id.as_deref().and_then(|o| ObjId::new(o).ok())?;
    if store.ndm().is_object_stored(&root, None).await.unwrap_or(false) {
        return None;
    }
    if !trigger(state, store, b) {
        let e = BucketError::OriginUnavailable(b.key.clone());
        let body = json!({"error": e.code(), "message": e.to_string()});
        return Some((StatusCode::NOT_FOUND, Json(body)).into_response());
    }
    let mut r = (StatusCode::ACCEPTED, Json(json!({"status": "fetching"}))).into_response();
    r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Some(r)
}

/// `bucket.head`: `remote` while nothing is happening, `fetching` while it is.
pub fn availability(state: &SharedState, availability: Availability, b: &Binding) -> Availability {
    match availability {
        Availability::Remote if b.obj_id.as_deref().is_some_and(|o| state.ambient.is_running(o)) => Availability::Fetching,
        other => other,
    }
}

/// `bucket.prefetch { bucket_id, prefix }`: starts the pull of what is missing under a prefix.
pub async fn prefetch(state: &SharedState, store: &BucketStore, id: &bucket_store::BucketId, caller: &bucket_store::Caller, prefix: String) -> BucketResult<serde_json::Value> {
    let bucket = store.bucket(id)?;
    if bucket.data_policy != DataPolicy::Ambient {
        return Ok(json!({"triggered": 0}));
    }
    let mut triggered = 0;
    let mut start_after = None;
    loop {
        let req = ListRequest { prefix: prefix.clone(), start_after, ..Default::default() };
        let page = store.list_objects(id, caller, &req).await?;
        for b in &page.items {
            let missing = match b.obj_id.as_deref().and_then(|o| ObjId::new(o).ok()) {
                Some(root) => !store.ndm().is_object_stored(&root, None).await.unwrap_or(false),
                None => false,
            };
            if missing && trigger(state, store, b) {
                triggered += 1;
            }
        }
        match page.next_start_after {
            Some(next) => start_after = Some(next),
            None => return Ok(json!({"triggered": triggered})),
        }
    }
}
