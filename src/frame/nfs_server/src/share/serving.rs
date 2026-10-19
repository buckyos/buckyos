//! Visitor data plane (design §3.5), served only on `share.<zone>`:
//!
//!   GET  /s/{token}            page (server template, no SDK, no site cookies)
//!   GET  /s/{token}/meta?p=    JSON: title, kind, size, items, expiry, policy
//!   POST /s/{token}/unlock     {password} → `share_access` cookie (Path=/s/{token})
//!   GET  /s/{token}/d          the shared file (inline preview or ?download=1)
//!   GET  /s/{token}/d/{inner}  a file inside a shared folder
//!
//! Every request re-checks credential, expiry and share status; anything not
//! servable is a plain 404 (stopped, expired and unknown look the same). Only
//! snapshot credentials (`obj:` subtrees) open here, never live-subtree caps.

use super::db::{AccessEvent, CapRow, PolicyRow, ShareRow};
use super::snapshot::{self, Resolved};
use super::{ShareMgr, UNLOCK_TTL_SECS};
use crate::fsutil::unix_now;
use crate::state::SharedState;
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, Stream};
use serde_json::{json, Value};
use std::collections::HashMap;

const PAGE: &str = include_str!("page.html");
const COOKIE: &str = "share_access";

struct Link {
    share: ShareRow,
    cap: CapRow,
    policy: PolicyRow,
    token_hash: String,
}

fn not_found() -> Response {
    let mut r = (StatusCode::NOT_FOUND, "Not found").into_response();
    common_headers(r.headers_mut());
    r
}

fn common_headers(h: &mut HeaderMap) {
    // The token is in the URL: never leak it through Referer.
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
}

fn json_response(status: StatusCode, v: Value) -> Response {
    let mut r = (status, axum::Json(v)).into_response();
    common_headers(r.headers_mut());
    r
}

/// The host a request was made to: the gateway's `X-Forwarded-Host`, else `Host`.
pub fn request_host(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())?;
    let host = raw.split(',').next()?.trim();
    let host = host.rsplit_once(':').filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit())).map(|(h, _)| h).unwrap_or(host);
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

pub fn is_share_host(state: &SharedState, host: &str) -> bool {
    match &state.config.share.zone_host {
        Some(zone) => host == format!("share.{zone}") || host == format!("share-{zone}"),
        None => false,
    }
}

/// The share origin only serves `/s/*`: the file API never answers there.
pub async fn host_guard(State(state): State<SharedState>, req: Request, next: Next) -> Response {
    if let Some(host) = request_host(req.headers()) {
        if is_share_host(&state, &host) && !req.uri().path().starts_with("/s/") {
            return not_found();
        }
    }
    next.run(req).await
}

fn resolve_link(state: &SharedState, token: &str) -> Option<Link> {
    if token.len() > 64 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return None;
    }
    let token_hash = ShareMgr::token_hash(token);
    let grant = state.db.grant_by_token_hash(&token_hash).ok()??;
    if grant.revoked || grant.expires_at.is_some_and(|e| e <= unix_now()) || !grant.subtree.starts_with("obj:") {
        return None;
    }
    let share = state.db.share_by_cap(&grant.cap_id).ok()??;
    if share.status != "active" || share.kind != "link" {
        return None;
    }
    let policy = state.db.share_policy(&share.share_id).ok()??;
    let cap = CapRow { cap_id: grant.cap_id, subtree: grant.subtree, expires_at: grant.expires_at, revoked: false };
    Some(Link { share, cap, policy, token_hash })
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else { continue };
        for pair in value.split(';') {
            if let Some((n, v)) = pair.trim().split_once('=') {
                if n == name {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn unlocked(state: &SharedState, link: &Link, headers: &HeaderMap) -> bool {
    if link.policy.access != "password" {
        return true;
    }
    cookie_value(headers, COOKIE).is_some_and(|c| state.share.verify_unlock(&c, &link.share.share_id, link.policy.password_rev))
}

fn visitor(state: &SharedState, headers: &HeaderMap, at: i64) -> Option<String> {
    let ip = headers
        .get("x-real-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    Some(state.share.visitor_id(&ip, ua, at))
}

fn record(state: &SharedState, link: &Link, headers: &HeaderMap, kind: &'static str, status: StatusCode, bytes: u64, inner: Option<String>, start: bool) {
    let at = unix_now();
    state.share.record(AccessEvent {
        share_id: link.share.share_id.clone(),
        at,
        kind,
        status: status.as_u16(),
        bytes,
        visitor: visitor(state, headers, at),
        inner_path: inner,
        download_start: start,
    });
}

/// `a/b/c.txt` → decoded names, rejecting empty, `.` and `..` segments.
fn segments(path: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        let name = percent_encoding::percent_decode_str(seg).decode_utf8().ok()?.into_owned();
        if name == "." || name == ".." || name.contains('/') {
            return None;
        }
        out.push(name);
    }
    Some(out)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

// ---------- page ----------

pub async fn page(State(state): State<SharedState>, AxPath(token): AxPath<String>, headers: HeaderMap) -> Response {
    let Some(link) = resolve_link(&state, &token) else { return not_found() };
    let open = unlocked(&state, &link, &headers);
    let title = if open { link.share.title.clone() } else { String::new() };
    let boot = json!({"base": format!("/s/{token}"), "locked": !open});
    let body = PAGE
        .replace("{{BOOT}}", &boot.to_string().replace('<', "\\u003c"))
        .replace("{{TITLE}}", &html_escape(if title.is_empty() { "Shared content" } else { &title }));
    record(&state, &link, &headers, "page", StatusCode::OK, body.len() as u64, None, false);
    let mut r = (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response();
    let h = r.headers_mut();
    common_headers(h);
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    r
}

// ---------- meta ----------

pub async fn meta(
    State(state): State<SharedState>,
    AxPath(token): AxPath<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(link) = resolve_link(&state, &token) else { return not_found() };
    if !unlocked(&state, &link, &headers) {
        record(&state, &link, &headers, "meta", StatusCode::UNAUTHORIZED, 0, None, false);
        return json_response(StatusCode::UNAUTHORIZED, json!({"locked": true, "requires_password": true}));
    }
    let Ok(store) = state.share.store(&state.config).await else { return not_found() };
    let Some(segs) = segments(q.get("p").map(String::as_str).unwrap_or("")) else { return not_found() };
    let version = state.db.share_version_get(&link.share.obj_id).ok().flatten();
    let mut out = json!({
        "title": link.share.title,
        "name": version.as_ref().map(|v| v.root_name.clone()),
        "total_size": version.as_ref().map(|v| v.total_size),
        "item_count": version.as_ref().map(|v| v.item_count),
        "created_at": link.share.created_at,
        "expires_at": link.cap.expires_at,
        "requires_password": link.policy.access == "password",
        "allow_download": link.policy.allow_download,
        "path": segs.join("/"),
    });
    match snapshot::resolve(store, &link.share.obj_id, &segs).await {
        Ok(Resolved::Dir(dir)) => {
            out["kind"] = json!("folder");
            out["items"] = json!(snapshot::list_dir(store, &dir).await);
        }
        Ok(Resolved::File(file)) => {
            out["kind"] = json!("file");
            let mime = file.meta.get("mime").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| snapshot::base_mime(&file.name));
            out["file"] = json!({"name": file.name, "size": file.size, "mime": mime});
        }
        Err(_) => return not_found(),
    }
    record(&state, &link, &headers, "meta", StatusCode::OK, 0, None, false);
    json_response(StatusCode::OK, out)
}

// ---------- unlock ----------

pub async fn unlock(State(state): State<SharedState>, AxPath(token): AxPath<String>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(link) = resolve_link(&state, &token) else { return not_found() };
    if link.policy.access != "password" {
        return json_response(StatusCode::OK, json!({"ok": true}));
    }
    if !state.share.unlock_allowed(&link.token_hash) {
        return json_response(StatusCode::TOO_MANY_REQUESTS, json!({"ok": false, "reason": "rate_limited"}));
    }
    let password = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["password"].as_str().map(String::from))
        .or_else(|| {
            url_form(&body).remove("password")
        })
        .unwrap_or_default();
    // PBKDF2 takes a noticeable slice of CPU: keep it off the async workers.
    let hash = link.policy.password_hash.clone();
    let good = tokio::task::spawn_blocking(move || hash.is_some_and(|h| ShareMgr::verify_password(&password, &h)))
        .await
        .unwrap_or(false);
    if !good {
        record(&state, &link, &headers, "unlock_fail", StatusCode::FORBIDDEN, 0, None, false);
        return json_response(StatusCode::FORBIDDEN, json!({"ok": false, "reason": "wrong_password"}));
    }
    let now = unix_now();
    let exp = link.cap.expires_at.map_or(now + UNLOCK_TTL_SECS, |e| e.min(now + UNLOCK_TTL_SECS));
    let value = state.share.sign_unlock(&link.share.share_id, exp, link.policy.password_rev);
    let secure = state.config.share.base_url.as_deref().is_some_and(|b| b.starts_with("https://"));
    let cookie = format!(
        "{COOKIE}={value}; Path=/s/{token}; Max-Age={}; HttpOnly; SameSite=Strict{}",
        (exp - now).max(0),
        if secure { "; Secure" } else { "" }
    );
    let mut r = json_response(StatusCode::OK, json!({"ok": true}));
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

fn url_form(body: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(body);
    text.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let dec = |s: &str| percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
            (dec(k), dec(v))
        })
        .collect()
}

// ---------- content ----------

pub async fn download_root(
    State(state): State<SharedState>,
    AxPath(token): AxPath<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    serve(state, token, String::new(), q, headers).await
}

pub async fn download_inner(
    State(state): State<SharedState>,
    AxPath((token, inner)): AxPath<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    serve(state, token, inner, q, headers).await
}

/// Inline only for types a browser renders without running content.
fn inline_type(mime: &str) -> bool {
    let major = mime.split('/').next().unwrap_or("");
    matches!(major, "image" | "video" | "audio") || mime == "application/pdf" || mime == "text/plain"
}

async fn serve(state: SharedState, token: String, inner: String, q: HashMap<String, String>, headers: HeaderMap) -> Response {
    let Some(link) = resolve_link(&state, &token) else { return not_found() };
    if !unlocked(&state, &link, &headers) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"locked": true}));
    }
    let Some(segs) = segments(&inner) else { return not_found() };
    let Ok(store) = state.share.store(&state.config).await else { return not_found() };
    let file = match snapshot::resolve(store, &link.share.obj_id, &segs).await {
        Ok(Resolved::File(f)) => f,
        _ => return not_found(),
    };
    let want_download = q.contains_key("download");
    if want_download && !link.policy.allow_download {
        return json_response(StatusCode::FORBIDDEN, json!({"reason": "download_disabled"}));
    }
    let mime = file.meta.get("mime").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| snapshot::base_mime(&file.name));
    // HTML and other active types are never rendered on the share origin.
    let renders = inline_type(&mime);
    let content_type = if mime.starts_with("text/") && renders {
        format!("{mime}; charset=utf-8")
    } else if renders || !mime.contains("html") && !mime.contains("xml") && !mime.contains("javascript") {
        mime.clone()
    } else {
        "application/octet-stream".to_string()
    };
    let len = file.size;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, status) = match crate::server::parse_range(range, len) {
        Ok(Some((s, e))) => (s, e, StatusCode::PARTIAL_CONTENT),
        Ok(None) => (0, len.saturating_sub(1), StatusCode::OK),
        Err(_) => {
            let mut r = (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{}", len))]).into_response();
            common_headers(r.headers_mut());
            return r;
        }
    };
    let content_len = if len == 0 { 0 } else { end - start + 1 };
    let body = if content_len == 0 {
        Body::empty()
    } else {
        match snapshot::open_content(store, &file, start).await {
            Ok(reader) => Body::from_stream(reader_stream(reader, content_len)),
            Err(e) => {
                log::warn!("share {}: content unavailable: {}", link.share.share_id, e);
                return not_found();
            }
        }
    };
    let encoded = percent_encoding::utf8_percent_encode(&file.name, percent_encoding::NON_ALPHANUMERIC).to_string();
    let disposition = if want_download || !renders { "attachment" } else { "inline" };
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, content_len)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_DISPOSITION, format!("{disposition}; filename*=UTF-8''{encoded}"))
        .header(
            header::CONTENT_SECURITY_POLICY,
            // Browsers refuse to show PDFs in a sandboxed document.
            if mime == "application/pdf" { "default-src 'none'; object-src 'self'" } else { "sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'" },
        );
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, len));
    }
    let mut r = builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    common_headers(r.headers_mut());
    let inner_path = (!segs.is_empty()).then(|| segs.join("/"));
    // Previews on the page are inline fetches: only saved copies count as downloads.
    record(&state, &link, &headers, "download", status, content_len, inner_path, start == 0 && disposition == "attachment");
    r
}

fn reader_stream(reader: ndn_lib::ChunkReader, len: u64) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    stream::unfold((reader, len), |(mut reader, remaining)| async move {
        if remaining == 0 {
            return None;
        }
        use tokio::io::AsyncReadExt;
        let chunk = remaining.min(64 * 1024) as usize;
        let mut buf = vec![0u8; chunk];
        match reader.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), (reader, remaining - n as u64)))
            }
            Err(e) => Some((Err(e), (reader, 0))),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "Share.Example.COM:443".parse().unwrap());
        assert_eq!(request_host(&h).as_deref(), Some("share.example.com"));
        h.insert("x-forwarded-host", "share-zone.io".parse().unwrap());
        assert_eq!(request_host(&h).as_deref(), Some("share-zone.io"));
    }

    #[test]
    fn segment_rules() {
        assert_eq!(segments("a/b%20c.txt"), Some(vec!["a".into(), "b c.txt".into()]));
        assert_eq!(segments(""), Some(vec![]));
        assert!(segments("a/../b").is_none());
        assert!(segments("a/%2e%2e/b").is_none());
        assert!(segments("a%2Fb").is_none());
    }
}
