//! NFSP `repr` / `get_repr` (frozen wire, preview iPhone media TODO D4).
//!
//! `repr`: `at` (an NFSP path / ref) or `args.source` (`{bucket: {bucket_id,
//! key, version?, access_token?}}` or `{obj_id}`) names the content; the
//! caller must be able to read it. `args.purpose` is `preview | thumbnail`,
//! `args.runtime` the Runtime's capability hints (`video_codecs.hevc` decides
//! remux over transcode), `retry` + `expected_attempt_id` the CAS of an
//! explicit Retry. Planning failures answer `state: "unsupported"` with a
//! `reason` (`no-pipeline` | `no-renderer` | `not-content`) and a `detail`;
//! execution failures are `state: "failed"` with an `error`. A document's
//! result is a PDF (`result_type: "document"`, accepted by a Runtime that
//! accepts `pdf`), a legacy spreadsheet's an xlsx (`sheet`), markup's one
//! self-contained HTML file (`html`). A thumbnail the content cannot give
//! (no embedded picture, an encrypted PDF) is `unsupported`, too.
//!
//! `get_repr`: `args.work_key` of a work this principal ensured (within an
//! hour); it never creates an attempt.

use super::convert::Profile;
use super::media::Kind;
use super::{tools, Input, Plan, Record};
use crate::acl::{Actor, Level};
use crate::auth::Credentials;
use crate::error::*;
use crate::fsutil::join_root;
use crate::namespace::Node;
use crate::state::SharedState;
use crate::types::Envelope;
use axum::http::HeaderMap;
use bucket_store::{BucketError, BucketId};
use ndn_lib::ObjId;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReprArgs {
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub source: Option<SourceArg>,
    #[serde(default)]
    pub runtime: RuntimeArg,
    #[serde(default)]
    pub retry: bool,
    #[serde(default)]
    pub expected_attempt_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceArg {
    #[serde(default)]
    pub bucket: Option<BucketSource>,
    #[serde(default)]
    pub obj_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BucketSource {
    pub bucket_id: String,
    pub key: String,
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub access_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct VideoCodecs {
    #[serde(default)]
    pub h264: bool,
    #[serde(default)]
    pub hevc: bool,
}

/// Capability hints; only `accept` and `video_codecs.hevc` change the plan.
#[derive(Debug, Default, Deserialize)]
pub struct RuntimeArg {
    #[serde(default)]
    pub accept: Option<Vec<String>>,
    #[serde(default)]
    pub image: Vec<String>,
    #[serde(default)]
    pub video: Vec<String>,
    #[serde(default)]
    pub audio: Vec<String>,
    #[serde(default)]
    pub video_codecs: VideoCodecs,
    #[serde(default)]
    pub heic: bool,
    #[serde(default)]
    pub pdf_inline: bool,
}

fn actor_key(actor: &Actor) -> String {
    match actor {
        Actor::Admin(u) => format!("admin:{}", u.as_deref().unwrap_or("-")),
        Actor::User(u) => format!("user:{u}"),
        Actor::Delegated { user, app, .. } => format!("app:{app}:{user}"),
        Actor::Anonymous => "anonymous".into(),
    }
}

fn unsupported(reason: &str, detail: impl Into<String>) -> Value {
    json!({"state": "unsupported", "reason": reason, "detail": detail.into()})
}

fn bucket_nfs_error(e: BucketError) -> NfsError {
    match e {
        BucketError::PermissionDenied(m) => NfsError::new(ErrorCode::PermissionDenied, m),
        BucketError::TokenInvalid => NfsError::new(ErrorCode::PermissionDenied, "the access token is invalid or expired"),
        BucketError::NoSuchKey(k) => not_found(format!("no such key {k}")),
        BucketError::BucketNotFound(b) => not_found(format!("no such bucket {b}")),
        BucketError::ContentEvicted(k) | BucketError::ContentNotAvailable(k) | BucketError::OriginUnavailable(k) => not_found(format!("content of {k} is not available")),
        BucketError::InvalidArgument(m) => invalid(m),
        other => internal(format!("bucket: {other}")),
    }
}

/// `(input, input ObjId)` of the named source, after the caller's read check.
async fn resolve_source(state: &SharedState, actor: &Actor, env: &Envelope, args: &ReprArgs, headers: &HeaderMap) -> NfsResult<Result<(Input, String), Value>> {
    let ndm = || async { state.share.store(&state.config).await.map(|s| s.clone()) };
    if let Some(at) = &env.at {
        let node = state.resolve_for(actor, at, Level::Read, "read")?;
        return Ok(match &node {
            Node::Native { root, rel, meta, .. } if meta.kind == "file" || meta.kind == "symlink" => {
                let cfg = state.config.root(root).ok_or_else(|| stale("root gone"))?;
                let full = join_root(&cfg.path, rel);
                if meta.kind == "symlink" {
                    state.require_symlink_target(actor, &full)?;
                }
                let id = super::native_obj_id(&state.db, &format!("{root}/{rel}"), &full).await?;
                Ok((Input::Path(full), id))
            }
            Node::Projection(p) => match &p.at {
                crate::bucket::projection::ProjAt::File { binding, .. } => {
                    let id = ObjId::new(binding.obj_id.as_deref().unwrap_or_default()).map_err(|e| invalid(e.to_string()))?;
                    let content = super::content_of(&ndm().await?, &id).await.map_err(|e| not_found(e.message))?;
                    Ok((Input::Object(id), content.to_string()))
                }
                _ => Err(unsupported("not-content", "folders and containers have no preview")),
            },
            _ => Err(unsupported("not-content", "folders and containers have no preview")),
        });
    }
    let source = args.source.as_ref().ok_or_else(|| invalid("repr requires at or args.source"))?;
    let id = if let Some(b) = &source.bucket {
        let bucket: BucketId = b.bucket_id.parse().map_err(|e: BucketError| invalid(e.to_string()))?;
        let creds = Credentials::from_headers(headers);
        let session = crate::bucket::session_from_headers(state, &creds).await.map_err(bucket_nfs_error)?;
        crate::bucket::spec::ensure(state, &bucket).await.map_err(bucket_nfs_error)?;
        let store = state.buckets.store(state).await.map_err(bucket_nfs_error)?.clone();
        let caller = crate::bucket::caller_of(state, &session, b.access_token.clone()).await;
        let (_, binding) = store.open_for_read(&bucket, &caller, &b.key, b.version).await.map_err(bucket_nfs_error)?;
        if binding.is_delete_marker {
            return Err(not_found(format!("{} was deleted", b.key)));
        }
        ObjId::new(binding.obj_id.as_deref().unwrap_or_default()).map_err(|e| invalid(e.to_string()))?
    } else if let Some(obj) = &source.obj_id {
        let id = ObjId::new(obj).map_err(|e| invalid(format!("bad obj_id: {e}")))?;
        let principal = crate::ndm::principal(state, headers).await.map_err(|_| NfsError::new(ErrorCode::PermissionDenied, "a valid session token is required"))?;
        if !crate::ndm::may_read(state, &principal, &id.to_string()).await {
            return Err(NfsError::new(ErrorCode::PermissionDenied, format!("not allowed to read {obj}")));
        }
        id
    } else {
        return Err(invalid("args.source needs bucket or obj_id"));
    };
    if !(id.is_file_object() || id.is_chunk() || id.is_chunk_list()) {
        return Ok(Err(unsupported("not-content", "the object holds no file content")));
    }
    let content = super::content_of(&ndm().await?, &id).await.map_err(|e| not_found(e.message))?;
    Ok(Ok((Input::Object(id), content.to_string())))
}

fn profile_for(kind: Kind, purpose: &str) -> Result<Profile, Value> {
    match (purpose, kind) {
        ("thumbnail", Kind::Audio) => Err(unsupported("no-renderer", "audio has no thumbnail")),
        ("thumbnail", Kind::Markup(reader)) if reader != "epub" => Err(unsupported("no-renderer", "markup has no thumbnail")),
        ("thumbnail", _) => Ok(Profile::Thumbnail),
        (_, Kind::Pdf) => Err(unsupported("no-pipeline", "a PDF is shown as it is")),
        (_, Kind::Heif | Kind::Image | Kind::Raster(_)) => Ok(Profile::PreviewImage),
        (_, Kind::Video) => Ok(Profile::PreviewVideo),
        (_, Kind::Audio) => Ok(Profile::PreviewAudio),
        (_, Kind::Document(ext)) if super::media::is_sheet(ext) => Ok(Profile::PreviewSheet),
        (_, Kind::Document(_)) => Ok(Profile::PreviewDocument),
        (_, Kind::Markup(_)) => Ok(Profile::PreviewHtml),
    }
}

/// The Runtime renderer (`runtime.accept`) a result needs.
fn renderer_of(profile: Profile) -> &'static str {
    match profile {
        Profile::PreviewImage | Profile::Thumbnail => "image",
        Profile::PreviewVideo => "video",
        Profile::PreviewAudio => "audio",
        Profile::PreviewDocument => "pdf",
        Profile::PreviewSheet => "sheet",
        Profile::PreviewHtml => "html",
    }
}

/// The wire record; completed results carry a fresh read URL.
pub async fn wire(state: &SharedState, rec: &Record) -> NfsResult<Value> {
    let mut out = json!({"work_key": rec.work_key, "state": rec.state, "attempt_id": rec.attempt_id});
    match rec.state.as_str() {
        "processing" => {
            out["retry_after_ms"] = json!(1000);
            if let Some(p) = state.preview.progress_of(&rec.work_key) {
                out["progress"] = json!({"completed": (p * 100.0).round() as u64, "total": 100, "message": "converting"});
            }
        }
        "completed" => {
            let r = rec.result.clone().unwrap_or_default();
            let key = r["key"].as_str().unwrap_or(&rec.work_key).to_string();
            let token = super::read_token(state, &key).await?;
            out["result"] = json!({
                "media_type": r["media_type"],
                "result_type": r["result_type"],
                "read_url": format!("/bucket/v1/{}/{}?token={}", super::bucket_id(), key, token),
                "result_obj_id": r["obj_id"],
                "source_version": rec.input_obj_id,
                "width": r["width"],
                "height": r["height"],
                "duration": r["duration"],
                "fidelity": r["fidelity"],
                "size": r["size"],
                "plan": r["plan"],
            });
        }
        _ if rec.error.as_ref().is_some_and(|e| e["code"] == super::convert::NO_THUMBNAIL) => {
            return Ok(unsupported("no-renderer", rec.error.as_ref().and_then(|e| e["message"].as_str()).unwrap_or_default()));
        }
        _ => out["error"] = rec.error.clone().unwrap_or_else(|| json!({"code": "PIPELINE_FAILED", "message": "the conversion failed", "retryable": true})),
    }
    Ok(out)
}

pub async fn op_repr(state: &SharedState, actor: &Actor, env: &Envelope, headers: &HeaderMap) -> NfsResult<Value> {
    let args: ReprArgs = serde_json::from_value(env.args.clone()).map_err(|e| invalid(format!("bad repr args: {e}")))?;
    let purpose = args.purpose.as_deref().unwrap_or("preview");
    if !matches!(purpose, "preview" | "thumbnail") {
        return Err(invalid(format!("unknown purpose '{purpose}'")));
    }
    if !crate::share::ShareMgr::store_configured(&state.config) {
        return Ok(unsupported("no-pipeline", "the named store is not configured"));
    }
    let probe = state.preview.refresh_async().await;
    if !probe.available() {
        return Ok(unsupported("no-pipeline", probe.missing_detail().unwrap_or_default()));
    }
    let (input, input_obj_id) = match resolve_source(state, actor, env, &args, headers).await? {
        Ok(found) => found,
        Err(planned) => return Ok(planned),
    };
    let head = super::sniff_bytes(state, &input).await.map_err(|e| not_found(e.message))?;
    let name = super::name_of(state, &input).await;
    let Some(kind) = super::media::sniff(&head, name.as_deref()) else {
        return Ok(unsupported("no-pipeline", "no converter for this content type"));
    };
    let profile = match profile_for(kind, purpose) {
        Ok(profile) => profile,
        Err(planned) => return Ok(planned),
    };
    let mut preview_pdf = None;
    if let (Profile::Thumbnail, Kind::Document(ext), Some(pdf)) = (profile, kind, &probe.pdf) {
        if let Some(soffice) = probe.soffice_for(super::media::office_of(ext)) {
            preview_pdf = super::cached_preview_pdf(state, soffice, &input_obj_id).await.map(|id| (id, tools::Converter::Pdftoppm(pdf.clone())));
        }
    }
    let (preview_pdf, converter) = match preview_pdf {
        Some((id, converter)) => (Some(id), converter),
        None => match probe.converter(profile, kind) {
            Ok(converter) => (None, converter),
            Err(detail) => return Ok(unsupported("no-pipeline", detail)),
        },
    };
    if let (Kind::Heif, tools::Converter::Ffmpeg(t)) = (kind, &converter) {
        if !t.heif {
            return Ok(unsupported("no-pipeline", format!("HEIF needs ffmpeg 7.1 or newer (found {})", t.ffmpeg.version)));
        }
    }
    if let Some(accept) = &args.runtime.accept {
        if !accept.iter().any(|a| a == renderer_of(profile)) {
            return Ok(unsupported("no-renderer", format!("the Runtime does not accept {} results", renderer_of(profile))));
        }
    }
    let plan = Plan { input, input_obj_id, kind, profile, hevc_playable: args.runtime.video_codecs.hevc, name, preview_pdf };
    let rec = super::ensure(state, &converter, plan, args.retry, args.expected_attempt_id.as_deref()).await?;
    state.preview.authorize(&actor_key(actor), &rec.work_key);
    wire(state, &rec).await
}

pub async fn op_get_repr(state: &SharedState, actor: &Actor, args: &Value) -> NfsResult<Value> {
    let key = args["work_key"].as_str().ok_or_else(|| invalid("get_repr requires work_key"))?;
    if !state.preview.is_authorized(&actor_key(actor), key) {
        return Err(NfsError::new(ErrorCode::PermissionDenied, "call repr for this content first").with("reason", json!("repr_required")));
    }
    let rec = super::get_record(&state.db, key)?.ok_or_else(|| not_found(format!("no preview work {key}")))?;
    wire(state, &rec).await
}
