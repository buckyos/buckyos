//! Media by Range (第四期规划 §7.3.2): `asset.stream_url { workspace_id, object_id }` hands out a short-lived address
//! `GET /kapi/aiworkspace/stream/<token>` a `<video>` / `<audio>` element plays from directly (it cannot send a bearer
//! header), with `Range` support so a follower seeks into a long video without fetching all of it.

use crate::{blocking, AppState, HTTP_PATH};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{WsError, WsResult};
use aiworkspace_store::workspace::random_id;
use aiworkspace_store::Caller;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;

/// A stream address lives this long; a player asks again when it expired.
const STREAM_TTL_MS: i64 = 6 * 3_600_000;

struct Grant {
    object_id: String,
    media_type: String,
    size: u64,
    expires_ms: i64,
}

#[derive(Default)]
pub struct Streams {
    map: Mutex<HashMap<String, Grant>>,
}

/// `asset.stream_url`: read access to the asset as for `GET …/asset/…`, then a token for its bytes.
pub async fn stream_url(state: &Arc<AppState>, caller: &Caller, p: &Value) -> WsResult<Value> {
    let ws = p.get("workspace_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("workspace_id required"))?.to_string();
    let object_id = p.get("object_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("object_id required"))?.to_string();
    let (st, c, o) = (state.clone(), caller.clone(), object_id.clone());
    let (media_type, size) = blocking(move || {
        st.with_ws(&ws, |w| {
            if !w.can_read_asset(&c, &o)? {
                return Err(WsError::not_found("asset not found"));
            }
            let info = w.asset_info(&o)?.ok_or_else(|| WsError::not_found("asset not found"))?;
            Ok((info.media_type, info.size))
        })
    })
    .await?;
    let now = (state.svc.clock)();
    let token = random_id("st_");
    let mut map = state.streams.map.lock().unwrap();
    map.retain(|_, g| g.expires_ms > now);
    map.insert(token.clone(), Grant { object_id, media_type: media_type.clone(), size, expires_ms: now + STREAM_TTL_MS });
    Ok(json!({ "ok": true, "url": format!("{HTTP_PATH}/stream/{token}"), "media_type": media_type, "size": size, "expires_at": format_utc_ms(now + STREAM_TTL_MS) }))
}

/// `bytes=a-b` / `bytes=a-` / `bytes=-n` within `size`.
fn range(headers: &HeaderMap, size: u64) -> Option<Result<(u64, u64), ()>> {
    let v = headers.get(header::RANGE)?.to_str().ok()?;
    let spec = v.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let r = match (a.parse::<u64>().ok(), b.parse::<u64>().ok()) {
        (Some(a), Some(b)) if a <= b => (a, b.min(size.saturating_sub(1))),
        (Some(a), None) => (a, size.saturating_sub(1)),
        (None, Some(n)) if n > 0 => (size.saturating_sub(n), size.saturating_sub(1)),
        _ => return Some(Err(())),
    };
    if size == 0 || r.0 >= size || r.0 > r.1 {
        return Some(Err(()));
    }
    Some(Ok(r))
}

pub async fn serve(State(state): State<Arc<AppState>>, Path(token): Path<String>, headers: HeaderMap) -> Response {
    let grant = {
        let map = state.streams.map.lock().unwrap();
        map.get(&token).filter(|g| g.expires_ms > (state.svc.clock)()).map(|g| (g.object_id.clone(), g.media_type.clone(), g.size))
    };
    let Some((object_id, media_type, size)) = grant else { return StatusCode::NOT_FOUND.into_response() };
    let (start, end, partial) = match range(&headers, size) {
        None => (0, size.saturating_sub(1), false),
        Some(Ok((a, b))) => (a, b, true),
        Some(Err(())) => return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{size}"))]).into_response(),
    };
    let Ok(id) = ndn_lib::ObjId::new(&object_id) else { return StatusCode::NOT_FOUND.into_response() };
    let ndm = state.svc.objects.named_store();
    let reader = match ndm.open_reader(&id, None).await {
        Ok((r, _)) => r,
        Err(e) => {
            log::warn!("stream {object_id}: {e}");
            return StatusCode::GONE.into_response();
        }
    };
    let len = if size == 0 { 0 } else { end - start + 1 };
    let body = futures_util::stream::unfold((reader, start, len), |(mut reader, mut skip, mut left)| async move {
        // the store's reader only reads forward: skip to the start of the range, then hand out the range
        let mut buf = vec![0u8; 64 * 1024];
        while skip > 0 {
            let n = (skip as usize).min(buf.len());
            match reader.read(&mut buf[..n]).await {
                Ok(0) | Err(_) => return None,
                Ok(k) => skip -= k as u64,
            }
        }
        if left == 0 {
            return None;
        }
        let n = (left as usize).min(buf.len());
        match reader.read(&mut buf[..n]).await {
            Ok(0) => None,
            Ok(k) => {
                left -= k as u64;
                buf.truncate(k);
                Some((Ok::<_, std::io::Error>(bytes::Bytes::from(buf)), (reader, 0, left)))
            }
            Err(e) => Some((Err(e), (reader, 0, 0))),
        }
    });
    let safe = media_type.starts_with("audio/") || media_type.starts_with("video/");
    let ctype = if safe && !media_type.contains(['+', ';']) { media_type } else { "application/octet-stream".to_string() };
    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, ctype)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "private, max-age=3600");
    builder = if partial {
        builder.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"))
    } else {
        builder.status(StatusCode::OK)
    };
    builder.body(Body::from_stream(body)).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
