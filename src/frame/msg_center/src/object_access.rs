//! Authenticated object access for MessageHub attachments:
//!
//!   GET /kapi/msg-center/objects/{obj_id}?context_path=…          → named object JSON
//!   GET /kapi/msg-center/objects/{obj_id}/content?context_path=…  → FileObject bytes
//!
//! msg-center reads its zone `attachments` bucket on behalf of the members of
//! a conversation. `context_path` (`[<session address>/]<msg_id>`) names the
//! message the attachment is read through: the caller must be able to read
//! that message, and its binding (`<session>/<msg_id>/<n>`) must hold the
//! object. Without `context_path` the message is the newest one of the
//! caller's own mailboxes that references the object (an Agent's, for the App
//! an Agent runs as), so an Agent reads an attachment by its object id alone;
//! restricted sessions always name the message. A hosted group's message
//! objects come through the same route, and the files its displays show with
//! `context_path=<session address>/_display` (the caller reads that Session,
//! its display shows the file, and `<group address>/_display/<obj_id>` holds
//! it). The caller presents a verified session token (`Authorization: Bearer`
//! or `?token=`). What another Zone's message references is not bound yet the
//! first time: see `remote_attachment.rs` for what a read answers then.

use crate::msg_center::MessageCenter;
use crate::remote_attachment::RemoteAttachment;
use buckyos_api::{
    AccessDenial, DenyReason, SessionScope, ACCESS_DENIED_ERROR_CODE, MSG_CENTER_SERVICE_NAME,
};
use buckyos_http_server::ServerError;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use kRPC::RPCContext;
use named_store::NamedDataMgr;
use ndn_lib::{load_named_obj, FileObject, ObjId};
use tokio::io::AsyncReadExt;

type Body = BoxBody<Bytes, ServerError>;

pub(crate) const OBJECTS_PATH: &str = "/kapi/msg-center/objects/";
/// Attachments are chat-sized; larger objects are refused instead of being
/// buffered in memory.
pub(crate) const MAX_CONTENT_BYTES: u64 = 64 * 1024 * 1024;
/// How many of the newest messages of each of the caller's mailboxes a read
/// without `context_path` looks through; older ones are read by naming them.
const OWN_MESSAGES_SEARCHED: usize = 1000;

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

fn retry_later(message: &str, seconds: u64) -> Response<Body> {
    let mut response = text_response(StatusCode::SERVICE_UNAVAILABLE, message);
    response.headers_mut().insert("retry-after", http::HeaderValue::from(seconds));
    response
}

/// A read of an attachment that is not here. Desktop tells these apart by status and body.
fn remote_answer(outcome: RemoteAttachment) -> Response<Body> {
    let retry_after = outcome.retry_after_secs().unwrap_or_default();
    match outcome {
        RemoteAttachment::Bound | RemoteAttachment::NotRemote | RemoteAttachment::Refused => text_response(StatusCode::NOT_FOUND, "not-found"),
        RemoteAttachment::TooLarge => text_response(StatusCode::PAYLOAD_TOO_LARGE, "file is too large for inline download"),
        RemoteAttachment::Directory => text_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "attachment-directory-unsupported"),
        RemoteAttachment::AwaitingAcceptance => text_response(StatusCode::FORBIDDEN, "attachment-awaiting-acceptance"),
        RemoteAttachment::Corrupt => text_response(StatusCode::BAD_GATEWAY, "attachment-verification-failed"),
        RemoteAttachment::Fetching => retry_later("attachment-fetching", retry_after),
        RemoteAttachment::Unreachable => retry_later("attachment-source-unreachable", retry_after),
    }
}

/// A named object's JSON, kept here.
fn object_response(body: String) -> Response<Body> {
    bytes_response(StatusCode::OK, "application/json; charset=utf-8", body.into_bytes())
}

/// An object another Zone described: its bytes are not here, so a card must not ask for them,
/// and the answer changes once they are.
fn remote_object_response(body: String) -> Response<Body> {
    let mut response = object_response(body);
    let headers = response.headers_mut();
    headers.insert("x-attachment-remote", http::HeaderValue::from_static("1"));
    headers.insert("cache-control", http::HeaderValue::from_static("no-store"));
    response
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
    req.uri().path().strip_prefix(OBJECTS_PATH)?;
    let request = Box::pin(serve_object(center, req));
    crate::owner_session::serve_opened("objects.read", SessionScope::MsgGroup, request).await
}

async fn serve_object(center: &MessageCenter, req: &Request<Body>) -> Option<Response<Body>> {
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
    let caller = match center.caller_identity(&ctx).await {
        Ok(Some(caller)) => caller,
        Ok(None) => return Some(text_response(StatusCode::UNAUTHORIZED, "unauthenticated")),
        Err(error) => {
            if let Some(denial) = AccessDenial::from_rpc_error(&error) {
                return Some(denial_response(StatusCode::FORBIDDEN, denial));
            }
            return Some(text_response(StatusCode::UNAUTHORIZED, "unauthenticated"));
        }
    };
    let query = req.uri().query().unwrap_or("");
    let context = query_param(query, "context_path");
    let inner_path = query_param(query, "inner_path");

    if let Some(session) = context.as_deref().and_then(crate::buckets::display_context) {
        if !caller.restricted()
            && center
                .authorize_resource(&ctx, "obj://msg-center/objects", "read")
                .await
                .is_err()
        {
            return Some(text_response(StatusCode::FORBIDDEN, "object access denied"));
        }
        let actor = center.group_actor(&ctx).await.ok();
        let key = match &actor {
            Some(actor) => center.authorize_display_file(actor, &obj_id, session).await.ok(),
            None => None,
        };
        if let Some(key) = key {
            return Some(bound_key_response(center, &key, &obj_id, want_content, inner_path).await);
        }
        // A group joined from here: its host has the file and the say.
        if let Some(actor) = actor.as_ref().filter(|_| !caller.restricted()) {
            if let Some(fetch) = center.joined_display_file(actor, &obj_id, session).await {
                return Some(joined_display_response(center, fetch, &obj_id, want_content, inner_path).await);
            }
        }
        return Some(if caller.restricted() {
            grant_missing(&caller.user_id)
        } else {
            text_response(StatusCode::NOT_FOUND, "not-found")
        });
    }

    // A restricted session (Session Only guest) has no objects grant: it
    // reads a Session message, or an object one references, only through
    // that message's context, checked now.
    if caller.restricted() {
        let authorized = match context.as_deref() {
            Some(context) => center
                .authorize_session_object(&ctx, &obj_id, context)
                .await
                .is_ok(),
            None => false,
        };
        let msg_id = context.as_deref().map(crate::buckets::context_msg_id);
        let (true, Some(Ok(msg_id))) = (authorized, msg_id) else {
            return Some(grant_missing(&caller.user_id));
        };
        if let Ok(Some((_, body, jwt))) = center.groups.object(&obj_id).await {
            return Some(group_message_response(body, jwt));
        }
        return Some(bound_attachment_response(center, &ctx, &msg_id, &obj_id, want_content, inner_path).await);
    }

    if center
        .authorize_resource(&ctx, "obj://msg-center/objects", "read")
        .await
        .is_err()
    {
        return Some(text_response(StatusCode::FORBIDDEN, "object access denied"));
    }

    if let Ok(Some((_, body, jwt))) = center.groups.object(&obj_id).await {
        if center.authorize_message(&ctx, &obj_id).await.is_err() {
            return Some(text_response(StatusCode::NOT_FOUND, "not-found"));
        }
        return Some(group_message_response(body, jwt));
    }
    let msg_id = match context {
        Some(context) => center.authorize_attachment(&ctx, &obj_id, &context).await.ok(),
        None => center
            .own_message_referencing(&ctx, &obj_id, OWN_MESSAGES_SEARCHED)
            .await
            .ok()
            .flatten(),
    };
    let Some(msg_id) = msg_id else {
        return Some(text_response(StatusCode::NOT_FOUND, "not-found"));
    };
    Some(bound_attachment_response(center, &ctx, &msg_id, &obj_id, want_content, inner_path).await)
}

/// A stored group message: its signed form when it has one.
fn group_message_response(body: Option<String>, jwt: Option<String>) -> Response<Body> {
    let (mime, body) = match jwt {
        Some(jwt) => (ndn_lib::CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT, Some(jwt)),
        None => (ndn_lib::CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON, body),
    };
    match body {
        Some(body) => bytes_response(StatusCode::OK, mime, body.into_bytes()),
        None => text_response(StatusCode::NOT_FOUND, "not-found"),
    }
}

/// A refused read of a restricted visitor: missing and not granted read the
/// same, as a D-4 denial (`resource_grant_missing`).
fn grant_missing(subject: &str) -> Response<Body> {
    log::warn!("msg-center denied {} on objects.read: resource_grant_missing", subject);
    let denial = AccessDenial::new(DenyReason::ResourceGrantMissing)
        .at(MSG_CENTER_SERVICE_NAME, "objects.read");
    denial_response(StatusCode::NOT_FOUND, denial)
}

/// A D-4 denial as the kRPC wire carries it (`error`, `code`, `data`).
fn denial_response(status: StatusCode, denial: AccessDenial) -> Response<Body> {
    let body = serde_json::json!({
        "error": denial.reason.as_str(),
        "code": ACCESS_DENIED_ERROR_CODE,
        "data": denial,
    });
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(
            Full::new(Bytes::from(body.to_string()))
                .map_err(|never| match never {})
                .boxed(),
        )
        .unwrap()
}

/// Attachment `id` of message `msg_id`, the caller already authorized for
/// that message: served once the message's binding in `attachments` holds it.
/// Without a binding, `content` fetches it and a description asks the source only.
pub(crate) async fn bound_attachment_response(
    center: &MessageCenter,
    ctx: &RPCContext,
    msg_id: &ObjId,
    id: &ObjId,
    want_content: bool,
    inner_path: Option<String>,
) -> Response<Body> {
    let buckets = match center.open_buckets().await {
        Ok(Some(buckets)) => buckets,
        _ => return text_response(StatusCode::SERVICE_UNAVAILABLE, "bucket service unavailable"),
    };
    match center.bound_attachment(&buckets, msg_id, id).await {
        Ok(Some(_)) => fetch_object_path(&buckets.store, id, want_content, inner_path).await,
        Ok(None) => {
            let reader = center.message_reader(ctx, msg_id).await;
            if !want_content {
                return match center.describe_remote_attachment(msg_id, id, reader.as_ref()).await {
                    Ok(body) => remote_object_response(body),
                    Err(outcome) => remote_answer(outcome),
                };
            }
            match center.ensure_remote_attachment(&buckets, msg_id, id, reader.as_ref()).await {
                RemoteAttachment::Bound => fetch_object_path(&buckets.store, id, want_content, inner_path).await,
                outcome => remote_answer(outcome),
            }
        }
        Err(error) => {
            log::warn!("attachment {} of {} not checked: {}", id.to_string(), msg_id.to_string(), error);
            text_response(StatusCode::SERVICE_UNAVAILABLE, "bucket service unavailable")
        }
    }
}

/// A display file of a group joined from here: once fetched it is read like a hosted one.
async fn joined_display_response(
    center: &MessageCenter,
    fetch: crate::remote_attachment::Fetch,
    id: &ObjId,
    want_content: bool,
    inner_path: Option<String>,
) -> Response<Body> {
    let buckets = match center.open_buckets().await {
        Ok(Some(buckets)) => buckets,
        _ => return text_response(StatusCode::SERVICE_UNAVAILABLE, "bucket service unavailable"),
    };
    match MessageCenter::key_holds(&buckets, &fetch.key, id).await {
        Ok(true) => fetch_object_path(&buckets.store, id, want_content, inner_path).await,
        Ok(false) if want_content => match center.ensure_display_file(&buckets, fetch).await {
            RemoteAttachment::Bound => fetch_object_path(&buckets.store, id, want_content, inner_path).await,
            outcome => remote_answer(outcome),
        },
        Ok(false) => match center.describe_fetch(fetch).await {
            Ok(body) => remote_object_response(body),
            Err(outcome) => remote_answer(outcome),
        },
        Err(error) => {
            log::warn!("display file {} not checked: {}", id.to_string(), error);
            text_response(StatusCode::SERVICE_UNAVAILABLE, "bucket service unavailable")
        }
    }
}

/// Object `id` once `key` of `attachments` holds it.
async fn bound_key_response(
    center: &MessageCenter,
    key: &str,
    id: &ObjId,
    want_content: bool,
    inner_path: Option<String>,
) -> Response<Body> {
    let buckets = match center.open_buckets().await {
        Ok(Some(buckets)) => buckets,
        _ => return text_response(StatusCode::SERVICE_UNAVAILABLE, "bucket service unavailable"),
    };
    match MessageCenter::key_holds(&buckets, key, id).await {
        Ok(true) => fetch_object_path(&buckets.store, id, want_content, inner_path).await,
        Ok(false) => text_response(StatusCode::NOT_FOUND, "not-found"),
        Err(error) => {
            log::warn!("{key} not checked: {error}");
            text_response(StatusCode::SERVICE_UNAVAILABLE, "bucket service unavailable")
        }
    }
}

async fn fetch_object_path(named_store: &NamedDataMgr, root: &ObjId, want_content: bool, inner_path: Option<String>) -> Response<Body> {
    let mut obj_id = root.clone();
    if let Some(path) = inner_path {
        for name in path.split('/') {
            if name.is_empty() || name == "." || name == ".." || name.contains('\\') {
                return text_response(StatusCode::BAD_REQUEST, "invalid inner path");
            }
            obj_id = match named_store.get_dir_child(&obj_id, name).await {
                Ok(child) => child,
                Err(_) => return text_response(StatusCode::NOT_FOUND, "directory entry unavailable"),
            };
        }
    }
    let obj_str = match named_store.get_object(&obj_id).await {
        Ok(value) => value,
        Err(ndn_lib::NdnError::NotFound(_)) => return text_response(StatusCode::NOT_FOUND, "object unavailable"),
        Err(_) => return text_response(StatusCode::BAD_GATEWAY, "object unavailable"),
    };

    if !want_content {
        return object_response(obj_str);
    }

    if !obj_id.is_file_object() {
        return text_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "object has no downloadable content",
        );
    }
    let file_obj: FileObject = match load_named_obj(&obj_str) {
        Ok(value) => value,
        Err(_) => return text_response(StatusCode::BAD_GATEWAY, "invalid file object"),
    };
    if file_obj.size > MAX_CONTENT_BYTES {
        return text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file is too large for inline download",
        );
    }
    let (mut reader, len) = match named_store.open_reader(&obj_id, None).await {
        Ok(value) => value,
        Err(_) => return text_response(StatusCode::NOT_FOUND, "file content unavailable"),
    };
    if len > MAX_CONTENT_BYTES {
        return text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file is too large for inline download",
        );
    }
    let mut data = Vec::with_capacity(len as usize);
    if reader.read_to_end(&mut data).await.is_err() {
        return text_response(StatusCode::BAD_GATEWAY, "read file content failed");
    }
    let name = file_obj.content_obj.name.clone();
    let mime = file_obj
        .meta
        .get("mime")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| guess_mime(&name).to_string());
    let disposition_name = if name.trim().is_empty() {
        obj_id.to_string().replace(':', "_")
    } else {
        name
    };
    let mut response = bytes_response(StatusCode::OK, &mime, data);
    // The attachment route shares the Desktop origin: an attachment never runs here (D24).
    ndn_lib::apply_content_policy(response.headers_mut(), &mime, Some(&disposition_name), false);
    response
}
