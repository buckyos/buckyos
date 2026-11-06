use super::image::{gif_dimensions, image_header};
use super::video::{ffprobe, VideoInfo};
use super::{function, mime_of_format, pdf_info, tools, PROBE};
use crate::native::{param_str, ExecutionLane, NativeContext, NativeFunction, NativeOutcome};
use async_trait::async_trait;
use buckyos_api::{FunctionObject, ThunkExecutionError};
use serde_json::{json, Value};

pub fn function_object() -> FunctionObject {
    function(
        PROBE,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "family"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "family": {"type": "string", "enum": ["image", "pdf", "video", "document"]}
            }
        }),
        "media.probe",
        json!({
            "type": "object",
            "required": ["kind", "mime", "size"],
            "properties": {
                "kind": {"type": "string"},
                "mime": {"type": "string"},
                "size": {"type": "integer"},
                "width": {"type": "integer"},
                "height": {"type": "integer"},
                "pages": {"type": "integer"},
                "duration_ms": {"type": "integer"},
                "has_audio": {"type": "boolean"},
                "has_video": {"type": "boolean"},
                "format": {"type": "string"}
            }
        }),
    )
}

pub struct Probe;

/// Container sniffing by magic bytes; never trusts file names.
pub fn sniff_format(head: &[u8]) -> &'static str {
    if head.starts_with(b"%PDF") {
        "pdf"
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpeg"
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        "gif"
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        "webp"
    } else if head.starts_with(b"BM") {
        "bmp"
    } else if head.starts_with(b"PK\x03\x04") {
        "zip"
    } else if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        "ole"
    } else if head.starts_with(b"{\\rtf") {
        "rtf"
    } else if head.len() >= 12 && &head[4..8] == b"ftyp" {
        "mp4"
    } else if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        "matroska"
    } else {
        "unknown"
    }
}

#[async_trait]
impl NativeFunction for Probe {
    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Light
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let content_id = param_str(params, "content_id")?;
        let family = param_str(params, "family")?;
        let result = match family {
            "image" => {
                let bytes = ctx.io.read_content(content_id, ctx.limits.max_input_bytes).await?;
                let size = bytes.len();
                let format = sniff_format(&bytes);
                let (mime, width, height) = ctx
                    .run_blocking(move |_| {
                        if let Some((w, h)) = gif_dimensions(&bytes) {
                            return Ok(("image/gif".to_string(), Some(w), Some(h)));
                        }
                        match image_header(&bytes) {
                            Ok(Some((format, w, h))) => Ok((format.to_mime_type().to_string(), Some(w), Some(h))),
                            Ok(None) => Ok(("application/octet-stream".to_string(), None, None)),
                            Err(err) => Err(err),
                        }
                    })
                    .await?;
                let mut out = json!({"kind": "image", "mime": mime, "size": size, "format": format});
                if let (Some(w), Some(h)) = (width, height) {
                    out["width"] = json!(w);
                    out["height"] = json!(h);
                }
                out
            }
            "pdf" => {
                ctx.require_tools(&[tools::PDFINFO])?;
                let (path, size) = ctx.materialize_input(content_id, "input.pdf").await?;
                let info = pdf_info(ctx, &path).await?;
                let mut out = json!({"kind": "pdf", "mime": "application/pdf", "size": size, "pages": info.pages, "format": "pdf"});
                if let Some((w, h)) = info.page_sizes.first() {
                    out["width"] = json!(w.round() as u64);
                    out["height"] = json!(h.round() as u64);
                }
                out
            }
            "video" => {
                ctx.require_tools(&[tools::FFPROBE])?;
                let input = ctx.open_tool_input(content_id, "input.video").await?;
                let info: VideoInfo = ffprobe(ctx, &input).await?;
                let mut out = json!({
                    "kind": "video",
                    "mime": info.mime(),
                    "size": input.size,
                    "duration_ms": info.duration_ms,
                    "has_audio": info.audio_codec.is_some(),
                    "has_video": info.width.is_some(),
                    "format": info.format_name,
                });
                if let (Some(w), Some(h)) = (info.width, info.height) {
                    out["width"] = json!(w);
                    out["height"] = json!(h);
                }
                out
            }
            _ => {
                let size = ctx
                    .io
                    .content_size(content_id)
                    .await?
                    .ok_or_else(|| ThunkExecutionError::new(buckyos_api::error_codes::INPUT_UNAVAILABLE, "content size unknown"))?;
                let head = ctx.io.read_content(content_id, size.min(64 * 1024)).await.or_else(|err| {
                    if err.code == buckyos_api::error_codes::RESOURCE_LIMIT {
                        Ok(Vec::new())
                    } else {
                        Err(err)
                    }
                })?;
                let format = sniff_format(&head);
                let mime = match format {
                    "zip" => "application/zip",
                    "ole" => "application/x-ole-storage",
                    other => mime_of_format(other),
                };
                json!({"kind": "document", "mime": mime, "size": size, "format": format})
            }
        };
        Ok(NativeOutcome::new(result))
    }
}
