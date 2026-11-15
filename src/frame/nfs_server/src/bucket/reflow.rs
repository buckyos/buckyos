//! The reflow pages of the Agent Browser on `reflow.<zone>` (Agent Browser
//! Protocol §5.4): every path of that host is a key of the signed-in user's
//! reflow library, the bucket `user/control-panel/<user>/reflow`; a directory
//! answers with its `index.html`. The user is the one whose session
//! `/sso_callback` planted on this host (a Desktop session delegated to this
//! service, as for the pages of AI Workspace on `aiws-<id>.<zone>`).
//!
//! Unlike `/bucket/v1` (D24), a page keeps its type, shows in place and runs
//! on this origin: the library holds page code the system produced, served the
//! way an App's pages are (agent-runtime-browser TODO D27).

use super::http::to_axum;
use super::spec;
use crate::auth::{is_page_session, Credentials};
use crate::config::AuthMode;
use crate::fsutil::content_type_for;
use crate::state::{AppState, SharedState};
use axum::extract::{Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use bucket_store::http::error_response;
use bucket_store::{AllowAll, BucketError, BucketId, BucketResult, Caller};
use buckyos_api::{CONTROL_PANEL_REFLOW_BUCKET, CONTROL_PANEL_SERVICE_NAME, NFS_SERVER_SERVICE_NAME};
use std::sync::Arc;

const INDEX: &str = "index.html";

pub fn is_reflow_host(state: &AppState, host: &str) -> bool {
    match &state.config.share.zone_host {
        Some(zone) => host == format!("reflow.{zone}") || host == format!("reflow-{zone}"),
        None => false,
    }
}

/// Everything on `reflow.<zone>` is the reflow library: no other API answers there.
pub async fn host_route(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    match crate::share::serving::request_host(req.headers()) {
        Some(host) if is_reflow_host(&state, &host) => serve(state, req.into_parts().0).await,
        _ => next.run(req).await,
    }
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], text).into_response()
}

/// The user signed in on this host: the one whose page session (auth::is_page_session) the request carries.
/// Sessions of the Desktop, of Apps, Agents and services are not signed in here, whoever they act for.
async fn signed_in_user(state: &AppState, creds: &Credentials) -> Option<String> {
    let test_key = match &state.config.auth {
        AuthMode::Disabled => return super::user_of(&super::Session::Open).map(String::from),
        AuthMode::TestKey { public_key, .. } => Some(public_key.as_str()),
        AuthMode::Runtime => None,
    };
    for token in creds.bearer().into_iter().chain(creds.cookie_tokens().iter().map(String::as_str)) {
        if let Ok(claims) = buckyos_api::verify_nfs_session_token(token, test_key).await {
            if is_page_session(&claims) {
                return Some(claims.subject);
            }
        }
    }
    None
}

/// The library key a request path names: its decoded segments, a directory being its `index.html`.
fn key_of(path: &str) -> Option<String> {
    let mut segments = Vec::new();
    for segment in path.strip_prefix('/')?.split('/') {
        let name = percent_encoding::percent_decode_str(segment).decode_utf8().ok()?;
        if name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            return None;
        }
        segments.push(name.into_owned());
    }
    if let Some(last) = segments.last_mut().filter(|s| s.is_empty()) {
        *last = INDEX.to_string();
    }
    (!segments.iter().any(String::is_empty)).then(|| segments.join("/"))
}

/// The user reading their own library through this host: the route itself decided the access.
fn library_reader(user: &str) -> Caller {
    let mut caller = Caller::anonymous(None);
    caller.user = Some(user.to_string());
    caller.app = Some(NFS_SERVER_SERVICE_NAME.to_string());
    caller.identity = Arc::new(AllowAll);
    caller
}

/// A page of the library is served as what it is: not as a download, not sandboxed.
fn page_headers(response: &mut Response, key: &str) {
    let generic = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_none_or(|t| t.as_bytes().starts_with(ndn_lib::OCTET_STREAM.as_bytes()));
    if generic && response.status() != StatusCode::NOT_MODIFIED {
        response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type_for(key)));
    }
    response.headers_mut().remove(header::CONTENT_SECURITY_POLICY);
    response.headers_mut().remove(header::CONTENT_DISPOSITION);
}

async fn serve(state: SharedState, req: Parts) -> Response {
    let head = match req.method {
        Method::GET => false,
        Method::HEAD => true,
        _ => return plain(StatusCode::METHOD_NOT_ALLOWED, "the reflow library is read with GET"),
    };
    let Some(user) = signed_in_user(&state, &Credentials::from_headers(&req.headers)).await else {
        return plain(StatusCode::UNAUTHORIZED, "sign in to open this page");
    };
    let Some(key) = key_of(req.uri.path()) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    match page(&state, &user, &key, &req, head).await {
        Ok(response) => response,
        Err(e) => to_axum(error_response(&e)),
    }
}

async fn page(state: &SharedState, user: &str, key: &str, req: &Parts, head: bool) -> BucketResult<Response> {
    let store = state.buckets.store(state).await?.clone();
    let id = BucketId::user(CONTROL_PANEL_SERVICE_NAME, user, CONTROL_PANEL_REFLOW_BUCKET)?;
    spec::ensure(state, &id).await?;
    let reader = library_reader(user);
    match store.open_for_read(&id, &reader, key, None).await {
        Ok((bucket, binding)) => {
            let mut response = to_axum(store.serve_read_declared(&bucket, &binding, &req.headers, head).await);
            page_headers(&mut response, key);
            Ok(response)
        }
        // `/<site>` names the directory `/<site>/`: relative links of its index need the slash.
        Err(BucketError::NoSuchKey(missing)) => {
            if store.open_for_read(&id, &reader, &format!("{key}/{INDEX}"), None).await.is_err() {
                return Err(BucketError::NoSuchKey(missing));
            }
            let query = req.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
            let location = format!("{}/{query}", req.uri.path());
            Ok((StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, location), (header::CACHE_CONTROL, "no-store".into())]).into_response())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_library_keys_and_never_leave_the_library() {
        assert_eq!(key_of("/").as_deref(), Some("index.html"));
        assert_eq!(key_of("/www-example-com/").as_deref(), Some("www-example-com/index.html"));
        assert_eq!(key_of("/www-example-com").as_deref(), Some("www-example-com"));
        assert_eq!(key_of("/www-example-com/assets/app.js").as_deref(), Some("www-example-com/assets/app.js"));
        assert_eq!(key_of("/www-example-com/a%20b/%E4%B8%AD.json").as_deref(), Some("www-example-com/a b/中.json"));
        for path in [
            "",
            "www-example-com/",
            "/..",
            "/../x",
            "/www-example-com/../other-site/index.html",
            "/www-example-com/%2e%2e/other-site/",
            "/www-example-com/%2E%2E",
            "/www-example-com/..%2fother-site/",
            "/www-example-com/./index.html",
            "/www-example-com//index.html",
            "//evil.example/",
            "/%2f%2fevil.example",
            "/www-example-com/a%5cb",
            "/www-example-com/a%00b",
            "/www-example-com/%ff",
        ] {
            assert_eq!(key_of(path), None, "{path}");
        }
    }

    #[test]
    fn a_page_keeps_its_type_and_is_neither_a_download_nor_sandboxed() {
        let stored = |content_type: Option<&'static str>| {
            let mut response = StatusCode::OK.into_response();
            ndn_lib::apply_content_policy(response.headers_mut(), "text/html", Some("index.html"), false);
            match content_type {
                Some(t) => response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(t)),
                None => response.headers_mut().remove(header::CONTENT_TYPE),
            };
            response
        };
        for (declared, key, served) in [
            (Some("text/html; charset=utf-8"), "s/index.html", "text/html; charset=utf-8"),
            (Some("application/octet-stream"), "s/index.html", "text/html; charset=utf-8"),
            (None, "s/refresh.mjs", "text/javascript"),
            (Some("application/x-custom"), "s/data.json", "application/x-custom"),
        ] {
            let mut response = stored(declared);
            page_headers(&mut response, key);
            let headers = response.headers();
            assert_eq!(headers[header::CONTENT_TYPE], served, "{key}");
            assert!(!headers.contains_key(header::CONTENT_DISPOSITION) && !headers.contains_key(header::CONTENT_SECURITY_POLICY));
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        }
        let mut unchanged = StatusCode::NOT_MODIFIED.into_response();
        page_headers(&mut unchanged, "s/index.html");
        assert!(!unchanged.headers().contains_key(header::CONTENT_TYPE));
    }
}
