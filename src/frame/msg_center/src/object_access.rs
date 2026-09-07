//! Authenticated object access for MessageHub attachments:
//!
//!   GET /kapi/msg-center/objects/{obj_id}          → named object JSON
//!   GET /kapi/msg-center/objects/{obj_id}/content  → FileObject bytes
//!
//! The zone gateway has no CYFS download route yet, so the message center
//! exposes the objects referenced by messages (`refs[].target.obj_id`,
//! `cyfs://<obj_id>` hints) to authenticated zone principals. The caller must
//! present a verified session token (`Authorization: Bearer` or `?token=`).

use crate::msg_center::MessageCenter;
use buckyos_api::get_buckyos_api_runtime;
use buckyos_http_server::ServerError;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use kRPC::RPCContext;
use ndn_lib::{load_named_obj, ChunkId, FileObject, ObjId};
use tokio::io::AsyncReadExt;

type Body = BoxBody<Bytes, ServerError>;

pub(crate) const OBJECTS_PATH: &str = "/kapi/msg-center/objects/";
/// Attachments are chat-sized; larger objects are refused instead of being
/// buffered in memory.
const MAX_CONTENT_BYTES: u64 = 64 * 1024 * 1024;

fn text_response(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("cache-control", "no-store")
        .body(
            Full::new(Bytes::from(message.to_string()))
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap()
}

fn bytes_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("cache-control", "private, max-age=3600")
        .body(
            Full::new(Bytes::from(body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap()
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key == name {
            percent_decode(value)
        } else {
            None
        }
    })
}

fn percent_decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode_filename(name: &str) -> String {
    let mut out = String::new();
    for byte in name.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

pub(crate) fn guess_mime(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Serve an object request. Returns `None` when the path is not an object
/// route so the caller can fall through to other handlers.
pub(crate) async fn serve(center: &MessageCenter, req: &Request<Body>) -> Option<Response<Body>> {
    let path = req.uri().path();
    let rest = path.strip_prefix(OBJECTS_PATH)?;
    if req.method() != http::Method::GET {
        return Some(text_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        ));
    }
    let (obj_id_raw, want_content) = match rest.strip_suffix("/content") {
        Some(id) => (id, true),
        None => (rest, false),
    };
    let obj_id_raw = percent_decode(obj_id_raw).unwrap_or_else(|| obj_id_raw.to_string());
    let obj_id = match ObjId::new(obj_id_raw.trim()) {
        Ok(id) => id,
        Err(_) => return Some(text_response(StatusCode::BAD_REQUEST, "invalid object id")),
    };

    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string())
        .or_else(|| query_param(req.uri().query().unwrap_or(""), "token"));
    let Some(token) = token else {
        return Some(text_response(StatusCode::UNAUTHORIZED, "unauthenticated"));
    };
    let ctx = RPCContext {
        token: Some(token),
        ..Default::default()
    };
    match center.caller_identity(&ctx).await {
        Ok(Some(_)) => {}
        Ok(None) => return Some(text_response(StatusCode::UNAUTHORIZED, "unauthenticated")),
        Err(_) => return Some(text_response(StatusCode::UNAUTHORIZED, "unauthenticated")),
    }

    let runtime = match get_buckyos_api_runtime() {
        Ok(runtime) => runtime,
        Err(_) => {
            return Some(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "named store unavailable",
            ))
        }
    };
    let named_store = match runtime.get_named_store().await {
        Ok(store) => store,
        Err(_) => {
            return Some(text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "named store unavailable",
            ))
        }
    };
    let obj_str = match named_store.get_object(&obj_id).await {
        Ok(value) => value,
        Err(error) => {
            let message = error.to_string();
            let status = if message.contains("NotFound") || message.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            };
            return Some(text_response(status, "object unavailable"));
        }
    };

    if !want_content {
        return Some(bytes_response(
            StatusCode::OK,
            "application/json; charset=utf-8",
            obj_str.into_bytes(),
        ));
    }

    if !obj_id.is_file_object() {
        return Some(text_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "object has no downloadable content",
        ));
    }
    let file_obj: FileObject = match load_named_obj(&obj_str) {
        Ok(value) => value,
        Err(_) => {
            return Some(text_response(
                StatusCode::BAD_GATEWAY,
                "invalid file object",
            ))
        }
    };
    if file_obj.size > MAX_CONTENT_BYTES {
        return Some(text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file is too large for inline download",
        ));
    }
    let chunk_id = match ChunkId::new(file_obj.content.trim()) {
        Ok(id) => id,
        Err(_) => {
            return Some(text_response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "file content is not a single chunk",
            ))
        }
    };
    let (mut reader, len) = match named_store.open_chunk_reader(&chunk_id, 0).await {
        Ok(value) => value,
        Err(_) => {
            return Some(text_response(
                StatusCode::NOT_FOUND,
                "file content unavailable",
            ))
        }
    };
    if len > MAX_CONTENT_BYTES {
        return Some(text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file is too large for inline download",
        ));
    }
    let mut data = Vec::with_capacity(len as usize);
    if reader.read_to_end(&mut data).await.is_err() {
        return Some(text_response(
            StatusCode::BAD_GATEWAY,
            "read file content failed",
        ));
    }
    let name = file_obj.content_obj.name.clone();
    let mime = file_obj
        .meta
        .get("mime_type")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| guess_mime(&name).to_string());
    let disposition_name = if name.trim().is_empty() {
        obj_id.to_string().replace(':', "_")
    } else {
        name
    };
    let mut response = bytes_response(StatusCode::OK, &mime, data);
    if let Ok(value) = http::HeaderValue::from_str(&format!(
        "inline; filename*=UTF-8''{}",
        percent_encode_filename(&disposition_name)
    )) {
        response.headers_mut().insert("content-disposition", value);
    }
    Some(response)
}
