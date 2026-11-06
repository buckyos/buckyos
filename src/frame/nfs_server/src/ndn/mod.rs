//! `/ndn/<obj_id>` (bucket design §4.6.5, D22): the Zone's read-only, plain-HTTP
//! way to read a named object across Zones. Knowing an ObjId grants nothing: a
//! request reads only if its authorization root is provided publicly or a `bkt_`
//! token in the `hint` currently binds that root. Cookies and Bearer headers are
//! never read, so a page cannot borrow its visitor's session, and refusal is one
//! 404 with no bucket touched or created.
//!
//! `public`: the admission of the two public buckets (`bucket.provide_public`,
//! `bucket.cache_public`). `ambient`: pulling an ambient binding's content back from
//! its origin through this same protocol.

pub mod ambient;
pub mod public;

use crate::state::SharedState;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::Response;
use bucket_store::ndn_read::{ndn_bad_request, ndn_error, ndn_not_found};
use bucket_store::{BucketStore, NdnAccess, NdnRequest};
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn to_axum(r: ::http::Response<bucket_store::http::HttpBody>) -> Response {
    let (parts, body) = r.into_parts();
    Response::from_parts(parts, Body::new(body))
}

/// §4.6.5's order: `public-objects`, `public-cache`, then the token.
fn decide(store: &BucketStore, req: &NdnRequest) -> Option<NdnAccess> {
    for bucket in [buckyos_api::nfs_public_objects_bucket(), buckyos_api::nfs_public_cache_bucket()] {
        match store.ndn_bound_in(&bucket, &req.root) {
            Ok(true) => return Some(NdnAccess::Public),
            Ok(false) => {}
            Err(e) => log::warn!("ndn: public lookup in {bucket}: {e}"),
        }
    }
    let token = req.token.as_deref()?;
    store.ndn_token_covers(token, &req.root).then_some(NdnAccess::Token)
}

/// `GET | HEAD /ndn/{*path}`.
pub async fn handle(State(state): State<SharedState>, req: Request) -> Response {
    let head = match *req.method() {
        Method::GET => false,
        Method::HEAD => true,
        _ => {
            let mut r = to_axum(ndn_error(StatusCode::METHOD_NOT_ALLOWED, "/ndn/ is read-only; writes go through /ndm/"));
            r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return r;
        }
    };
    let parsed = match NdnRequest::parse(req.uri().path(), req.uri().query()) {
        Ok(parsed) => parsed,
        Err(e) => return to_axum(ndn_bad_request(e)),
    };
    let store = match state.buckets.store(&state).await {
        Ok(store) => store.clone(),
        Err(e) => {
            log::warn!("ndn: bucket service unavailable: {e}");
            return to_axum(ndn_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
        }
    };
    let Some(access) = decide(&store, &parsed) else { return to_axum(ndn_not_found()) };
    let response = to_axum(store.serve_ndn(&parsed, access, req.headers(), head).await);
    if access == NdnAccess::Public {
        return state.ndn_egress.limit(response);
    }
    response
}

/// Bytes per second all anonymous `/ndn/` responses of this server may send (D25).
/// A capability holder is someone's guest and is not limited.
pub struct EgressLimit {
    bytes_per_sec: u64,
    next_free: Mutex<Instant>,
}

/// How far ahead of the budget responses may run, so short reads stay instant.
const BURST: Duration = Duration::from_secs(1);

impl EgressLimit {
    pub fn new(bytes_per_sec: u64) -> Arc<Self> {
        Arc::new(EgressLimit { bytes_per_sec, next_free: Mutex::new(Instant::now()) })
    }

    /// How long a sender of `bytes` waits; reserves them.
    fn reserve(&self, bytes: usize) -> Duration {
        if self.bytes_per_sec == 0 {
            return Duration::ZERO;
        }
        let cost = Duration::from_secs_f64(bytes as f64 / self.bytes_per_sec as f64);
        let now = Instant::now();
        let mut next_free = self.next_free.lock().unwrap();
        let start = (*next_free).max(now);
        *next_free = start + cost;
        next_free.saturating_duration_since(now + BURST)
    }

    pub fn limit(self: &Arc<Self>, response: Response) -> Response {
        if self.bytes_per_sec == 0 {
            return response;
        }
        let (parts, body) = response.into_parts();
        let limit = self.clone();
        let stream = body.into_data_stream().then(move |frame| {
            let limit = limit.clone();
            async move {
                if let Ok(bytes) = &frame {
                    let wait = limit.reserve(bytes.len());
                    if !wait.is_zero() {
                        tokio::time::sleep(wait).await;
                    }
                }
                frame
            }
        });
        Response::from_parts(parts, Body::from_stream(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_charges_every_sender_after_a_burst() {
        let limit = EgressLimit::new(1000);
        assert!(limit.reserve(500).is_zero(), "within the burst");
        let wait = limit.reserve(1500);
        assert!(wait > Duration::from_millis(900) && wait <= Duration::from_secs(1), "{wait:?}");
        let wait = limit.reserve(1000);
        assert!(wait > Duration::from_millis(1900) && wait <= Duration::from_secs(2), "{wait:?}");
        assert!(EgressLimit::new(0).reserve(usize::MAX).is_zero(), "0 turns it off");
    }
}
