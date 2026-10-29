//! The Workspace's own origin (内核架构改进 D21, T4-8): on `aiws-<id>.<zonehost>` this service serves the AI Workspace
//! page itself — the Desktop build's `aiws.html` entry and its files — so a Workspace is a site of its own, with its
//! own login state (hybrid SSO: the host-only cookies `/sso_callback` sets there).
//!
//! Paths on that host (`notepads/aiworkspace-subdomain-todo.md` Q3/Q4):
//!   - a file of the Desktop build (`assets/…`, `aiworkspace/pdfjs/…`, `sw.js`, `favicon.svg`, …) → that file;
//!   - `/`, `/<workspace slug>` → the page (the Workspace opens on its default Surface);
//!   - `/<workspace slug>/<name>?id=<surface id>` → the page (the id decides, the names are for people and crawlers);
//!   - `/<workspace slug>/<name>` without `id`: a published data name answers the publication (JSON or HTML),
//!     else a Surface (by name, id or title slug) is redirected to its canonical `?id=` address, else the page.
//! `?block=` and any other query travel along. Without a web directory (standalone without `--web`) the page is 503.

use crate::source::{host_workspace, published};
use crate::AppState;
use aiworkspace_core::WsError;
use aiworkspace_store::source::pub_name;
use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The Desktop build's AI Workspace entry (vite.config.ts `build.rollupOptions.input.aiws`).
pub const ENTRY_FILE: &str = "aiws.html";

/// Where the Desktop build is, when this service knows it: `--web` (standalone), else `web/` beside the executable,
/// else the control-panel package's `web/` next to this service's package (both are kernel services of one node).
pub fn locate_web_dir(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = explicit {
        return Some(PathBuf::from(dir));
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [exe_dir.join("web"), exe_dir.join("..").join("control-panel").join("web")]
        .into_iter()
        .find(|d| d.join(ENTRY_FILE).is_file())
}

/// The page of a Workspace: `<public_base>/<workspace slug>[/<surface slug>?id=<surface id>]`; `None` without a public
/// base (then there is no origin to link to).
pub fn page_url(state: &Arc<AppState>, ws_id: &str, workspace_slug: &str, surface: Option<(&str, &str)>) -> Option<String> {
    let base = crate::source::public_base(state, ws_id)?;
    Some(match surface {
        Some((title, id)) => format!("{base}/{workspace_slug}/{}?id={}", surface_slug(title), percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC)),
        None => format!("{base}/{workspace_slug}"),
    })
}

/// A Surface's segment of a link: its title slugified, `surface` when that is empty.
pub fn surface_slug(title: &str) -> String {
    let s = pub_name(title);
    if s.is_empty() { "surface".into() } else { s }
}

/// Every GET the kRPC and publication routes did not take: a request on a Workspace host.
pub async fn any(State(state): State<Arc<AppState>>, OriginalUri(uri): OriginalUri, headers: HeaderMap) -> Response {
    let path = uri.path();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let decoded: Vec<String> = segments.iter().map(|s| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()).collect();
    if decoded.iter().any(|s| s == "." || s == ".." || s.contains('\\') || s.contains('\0')) {
        return StatusCode::NOT_FOUND.into_response();
    }
    // the service's own API namespace is never a page
    if segments.first().is_some_and(|s| *s == "kapi") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(dir) = state.limits.web_dir.as_deref() {
        if !decoded.is_empty() {
            if let Some(response) = static_file(dir, &decoded).await {
                return response;
            }
        }
    }
    let Some(ws_id) = host_workspace(&headers) else { return StatusCode::NOT_FOUND.into_response() };
    let query: HashMap<String, String> = uri.query().map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()).unwrap_or_default();
    match decoded.len() {
        0 | 1 => page(&state).await,
        2 if query.contains_key("id") => page(&state).await,
        2 => {
            let (st, w, n) = (state.clone(), ws_id.clone(), decoded[1].clone());
            let kind = tokio::task::spawn_blocking(move || {
                st.with_ws(&w, |ws| {
                    if ws.publication(&n)?.is_some() {
                        return Ok(json!({ "publication": true }));
                    }
                    Ok(json!({ "surface": ws.surface_by_name(&n)? }))
                })
            })
            .await
            .unwrap_or_else(|e| Err(WsError::io(e.to_string())));
            match kind {
                Ok(k) if k["publication"] == json!(true) => published(&state, &ws_id, &decoded[1], &query, &headers).await,
                Ok(k) => match k["surface"].as_str() {
                    Some(surface_id) => Redirect::temporary(&canonical(&uri, surface_id)).into_response(),
                    None => page(&state).await,
                },
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The same path with `id=<surface id>` added in front of the query it already carries (`block=` and the rest).
fn canonical(uri: &Uri, surface_id: &str) -> String {
    let mut query = format!("id={}", percent_encoding::utf8_percent_encode(surface_id, percent_encoding::NON_ALPHANUMERIC));
    if let Some(rest) = uri.query().filter(|q| !q.is_empty()) {
        query.push('&');
        query.push_str(rest);
    }
    format!("{}?{query}", uri.path())
}

async fn page(state: &Arc<AppState>) -> Response {
    let Some(dir) = state.limits.web_dir.as_deref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, [(header::CACHE_CONTROL, "no-store")], "the AI Workspace page files are not installed on this node").into_response();
    };
    match tokio::fs::read(dir.join(ENTRY_FILE)).await {
        Ok(body) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], body).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, [(header::CACHE_CONTROL, "no-store")], format!("{ENTRY_FILE} is missing from the page files")).into_response(),
    }
}

/// A file of the Desktop build, by its path segments; `None` when there is no such file (then the path means a page).
async fn static_file(dir: &Path, segments: &[String]) -> Option<Response> {
    let mut path = dir.to_path_buf();
    for s in segments {
        path.push(s);
    }
    let meta = tokio::fs::metadata(&path).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    // built files are content-hashed under assets/ (immutable); everything else is revalidated
    let cache = if segments.first().is_some_and(|s| s == "assets") { "public, max-age=31536000, immutable" } else { "no-cache" };
    let body = tokio::fs::read(&path).await.ok()?;
    Some(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(segments.last().map(String::as_str).unwrap_or("")))
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(body))
        .unwrap())
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" | "webmanifest" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
