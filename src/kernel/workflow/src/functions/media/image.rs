//! `media.fit_image` (LLM Understand Media §4.5.3), a pure native function:
//! header-only passthrough when the image already fits, otherwise EXIF
//! orientation, proportional downscale and PNG / JPEG re-encoding within the
//! byte limit.

use super::{function, tools, FIT_IMAGE};
use crate::native::{
    param_u64, ExecutionLane, NativeContext, NativeFunction, NativeOutcome, ToolEnvironment,
};
use async_trait::async_trait;
use buckyos_api::{error_codes, FunctionObject, ThunkExecutionError};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgb, RgbImage, RgbaImage};
use serde::Serialize;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Cursor;

const JPEG_QUALITY_LADDER: [u8; 3] = [85, 75, 60];
const MAX_SHRINK_ROUNDS: usize = 3;
const SHRINK_FACTOR: f64 = 0.75;
const DECODABLE_FORMATS: [ImageFormat; 4] = [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP, ImageFormat::Bmp];
const PASSTHROUGH_FORMATS: [ImageFormat; 3] = [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP];

pub fn function_object() -> FunctionObject {
    function(
        FIT_IMAGE,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "max_long_edge", "max_pixels", "max_bytes"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "max_long_edge": {"type": "integer", "minimum": 1},
                "max_pixels": {"type": "integer", "minimum": 1},
                "max_bytes": {"type": "integer", "minimum": 1}
            }
        }),
        "media.fitted_image",
        json!({
            "type": "object",
            "required": ["model_media_obj_id", "mime", "original", "sent", "actions"],
            "properties": {
                "model_media_obj_id": {"type": "string"},
                "mime": {"type": "string"},
                "original": image_info_schema(),
                "sent": image_info_schema(),
                "actions": {"type": "array", "items": {"type": "string"}}
            }
        }),
    )
}

fn image_info_schema() -> Value {
    json!({
        "type": "object",
        "required": ["mime", "width", "height", "bytes"],
        "properties": {
            "mime": {"type": "string"},
            "width": {"type": "integer"},
            "height": {"type": "integer"},
            "bytes": {"type": "integer"}
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageLimits {
    pub max_long_edge: u32,
    pub max_pixels: u64,
    pub max_bytes: usize,
    pub max_decode_pixels: u64,
}

impl ImageLimits {
    fn allows(&self, width: u32, height: u32, bytes: usize) -> bool {
        width.max(height) <= self.max_long_edge
            && u64::from(width) * u64::from(height) <= self.max_pixels
            && bytes <= self.max_bytes
    }

    fn scale_for(&self, width: u32, height: u32) -> f64 {
        let by_edge = f64::from(self.max_long_edge) / f64::from(width.max(height));
        let by_pixels = (self.max_pixels as f64 / (f64::from(width) * f64::from(height))).sqrt();
        by_edge.min(by_pixels).min(1.0)
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ImageInfo {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

pub struct FittedImage {
    pub mime: String,
    pub bytes: Vec<u8>,
    pub original: ImageInfo,
    pub sent: ImageInfo,
    pub actions: Vec<&'static str>,
}

pub enum Fit {
    Done(FittedImage),
    NeedsConversion(Vec<u8>),
}

pub fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    (width > 0 && height > 0).then_some((width, height))
}

/// Header-only `(format, width, height)` of a decodable image; `None` for
/// formats the image crate does not decode.
pub fn image_header(bytes: &[u8]) -> Result<Option<(ImageFormat, u32, u32)>, ThunkExecutionError> {
    let Some(format) = image::guess_format(bytes).ok().filter(|f| DECODABLE_FORMATS.contains(f)) else {
        return Ok(None);
    };
    let decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|err| decode_error(format!("read {} image header failed: {err}", format.to_mime_type())))?;
    let (w, h) = decoder.dimensions();
    Ok(Some((format, w, h)))
}

fn decode_error(message: String) -> ThunkExecutionError {
    ThunkExecutionError::new(error_codes::TOOL_FAILED, message).with_details(json!({"tool": "image"}))
}

pub fn fit_decodable(bytes: Vec<u8>, declared_mime: &str, limits: ImageLimits) -> Result<Fit, ThunkExecutionError> {
    if let Some((width, height)) = gif_dimensions(&bytes) {
        if !limits.allows(width, height, bytes.len()) {
            return Ok(Fit::NeedsConversion(bytes));
        }
        let info = ImageInfo { mime: "image/gif".to_string(), width, height, bytes: bytes.len() };
        return Ok(Fit::Done(passthrough(bytes, declared_mime, info)));
    }
    let Some(format) = image::guess_format(&bytes).ok().filter(|format| DECODABLE_FORMATS.contains(format)) else {
        return Ok(Fit::NeedsConversion(bytes));
    };
    let mime = format.to_mime_type();
    let mut decoder = ImageReader::with_format(Cursor::new(bytes.as_slice()), format)
        .into_decoder()
        .map_err(|err| decode_error(format!("read {mime} image header failed: {err}")))?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let original = ImageInfo { mime: mime.to_string(), width, height, bytes: bytes.len() };
    if PASSTHROUGH_FORMATS.contains(&format) && orientation == Orientation::NoTransforms && limits.allows(width, height, bytes.len()) {
        drop(decoder);
        return Ok(Fit::Done(passthrough(bytes, declared_mime, original)));
    }
    if u64::from(width) * u64::from(height) > limits.max_decode_pixels {
        return Err(ThunkExecutionError::new(
            error_codes::RESOURCE_LIMIT,
            format!("{mime} image {width}x{height} exceeds the decode limit of {} pixels", limits.max_decode_pixels),
        )
        .with_details(json!({"width": width, "height": height, "max_page_pixels": limits.max_decode_pixels})));
    }
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|err| decode_error(format!("decode {mime} image ({width}x{height}) failed: {err}")))?;
    drop(bytes);
    let mut actions = Vec::new();
    if orientation != Orientation::NoTransforms {
        image.apply_orientation(orientation);
        actions.push("rotated");
    }
    let prefer_png = matches!(format, ImageFormat::Png | ImageFormat::Bmp);
    let (sent_mime, encoded, sent_width, sent_height) = encode_within_limits(&image, prefer_png, limits)?;
    if sent_mime != original.mime {
        actions.insert(0, "converted");
    }
    if u64::from(sent_width) * u64::from(sent_height) < u64::from(image.width()) * u64::from(image.height()) {
        actions.push("downscaled");
    } else if actions.is_empty() {
        actions.push("recompressed");
    }
    Ok(Fit::Done(FittedImage {
        mime: sent_mime.to_string(),
        original,
        sent: ImageInfo { mime: sent_mime.to_string(), width: sent_width, height: sent_height, bytes: encoded.len() },
        actions,
        bytes: encoded,
    }))
}

fn passthrough(bytes: Vec<u8>, declared_mime: &str, info: ImageInfo) -> FittedImage {
    let actions = if declared_mime == info.mime { Vec::new() } else { vec!["mime_corrected"] };
    FittedImage {
        mime: info.mime.clone(),
        bytes,
        original: ImageInfo { mime: declared_mime.to_string(), ..info.clone() },
        sent: info,
        actions,
    }
}

fn encode_within_limits(image: &DynamicImage, prefer_png: bool, limits: ImageLimits) -> Result<(&'static str, Vec<u8>, u32, u32), ThunkExecutionError> {
    let (width, height) = (image.width(), image.height());
    let mut target = scaled_dims(width, height, limits.scale_for(width, height));
    for _ in 0..=MAX_SHRINK_ROUNDS {
        let resized = if target == (width, height) {
            Cow::Borrowed(image)
        } else {
            Cow::Owned(image.resize_exact(target.0, target.1, FilterType::CatmullRom))
        };
        if let Some((mime, encoded)) = encode_candidate(&resized, prefer_png, limits.max_bytes)? {
            return Ok((mime, encoded, target.0, target.1));
        }
        target = scaled_dims(target.0, target.1, SHRINK_FACTOR);
    }
    Err(ThunkExecutionError::new(
        error_codes::RESOURCE_LIMIT,
        format!("image is still larger than {} bytes after downscaling to {}x{}", limits.max_bytes, target.0, target.1),
    ))
}

pub fn scaled_dims(width: u32, height: u32, scale: f64) -> (u32, u32) {
    if scale >= 1.0 {
        return (width, height);
    }
    let scale_edge = |edge: u32| ((f64::from(edge) * scale + 1e-6).floor() as u32).clamp(1, edge);
    (scale_edge(width), scale_edge(height))
}

fn encode_candidate(image: &DynamicImage, prefer_png: bool, max_bytes: usize) -> Result<Option<(&'static str, Vec<u8>)>, ThunkExecutionError> {
    let rgba = image
        .color()
        .has_alpha()
        .then(|| image.to_rgba8())
        .filter(|rgba| rgba.pixels().any(|pixel| pixel[3] < u8::MAX));
    if rgba.is_some() || prefer_png {
        let mut png = Vec::new();
        let written = match &rgba {
            Some(rgba) => rgba.write_with_encoder(PngEncoder::new(&mut png)),
            None => image.to_rgb8().write_with_encoder(PngEncoder::new(&mut png)),
        };
        written.map_err(|err| decode_error(format!("encode PNG failed: {err}")))?;
        if png.len() <= max_bytes {
            return Ok(Some(("image/png", png)));
        }
    }
    let rgb = match &rgba {
        Some(rgba) => flatten_on_white(rgba),
        None => image.to_rgb8(),
    };
    for quality in JPEG_QUALITY_LADDER {
        let mut jpeg = Vec::new();
        rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, quality))
            .map_err(|err| decode_error(format!("encode JPEG failed: {err}")))?;
        if jpeg.len() <= max_bytes {
            return Ok(Some(("image/jpeg", jpeg)));
        }
    }
    Ok(None)
}

fn flatten_on_white(rgba: &RgbaImage) -> RgbImage {
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let alpha = u16::from(a);
        let blend = |c: u8| ((u16::from(c) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        Rgb([blend(r), blend(g), blend(b)])
    })
}

pub fn extension_of(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "bin",
    }
}

/// Converts a non-decodable image to PNG at full resolution with ffmpeg
/// (first frame of animations).
async fn convert_with_ffmpeg(ctx: &NativeContext, bytes: &[u8]) -> Result<Vec<u8>, ThunkExecutionError> {
    let ffmpeg = ctx.tool_path(tools::FFMPEG)?;
    let input = ctx.temp_dir.join("input.img");
    let output = ctx.temp_dir.join("converted.png");
    tokio::fs::write(&input, bytes)
        .await
        .map_err(|err| ThunkExecutionError::new(error_codes::INTERNAL, format!("write temp image: {err}")))?;
    let args = vec![
        "-v".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        input.to_string_lossy().into_owned(),
        "-frames:v".to_string(),
        "1".to_string(),
        output.to_string_lossy().into_owned(),
    ];
    ctx.run_tool(&ffmpeg, &args, 64 * 1024).await?.ok()?;
    tokio::fs::read(&output)
        .await
        .map_err(|err| ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("read converted image failed: {err}")))
}

pub struct FitImage;

#[async_trait]
impl NativeFunction for FitImage {
    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Image
    }

    fn fingerprint(&self, env: &ToolEnvironment) -> BTreeMap<String, String> {
        env.fingerprint_for(&[tools::FFMPEG])
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let content_id = crate::native::param_str(params, "content_id")?;
        let limits = ImageLimits {
            max_long_edge: u32::try_from(param_u64(params, "max_long_edge")?).unwrap_or(u32::MAX),
            max_pixels: param_u64(params, "max_pixels")?,
            max_bytes: usize::try_from(param_u64(params, "max_bytes")?).unwrap_or(usize::MAX),
            max_decode_pixels: ctx.limits.max_page_pixels,
        };
        let bytes = ctx.io.read_content(content_id, ctx.limits.max_input_bytes).await?;
        let declared = image::guess_format(&bytes)
            .map(|f| f.to_mime_type().to_string())
            .unwrap_or_else(|_| "application/octet-stream".to_string());
        let declared_for_fit = declared.clone();
        let fitted = match ctx.run_blocking(move |_| fit_decodable(bytes, &declared_for_fit, limits)).await? {
            Fit::Done(fitted) => fitted,
            Fit::NeedsConversion(bytes) => {
                let original_bytes = bytes.len();
                let png = convert_with_ffmpeg(ctx, &bytes).await?;
                drop(bytes);
                let Fit::Done(mut fitted) = ctx.run_blocking(move |_| fit_decodable(png, "image/png", limits)).await? else {
                    return Err(ThunkExecutionError::new(error_codes::TOOL_FAILED, "ffmpeg produced an unreadable PNG"));
                };
                fitted.original.mime = declared;
                fitted.original.bytes = original_bytes;
                if !fitted.actions.contains(&"converted") {
                    fitted.actions.insert(0, "converted");
                }
                fitted
            }
        };
        let name = format!("fit.{}", extension_of(&fitted.mime));
        let artifact = ctx
            .io
            .put_artifact(&ctx.artifact_key(&name), &name, &fitted.mime, fitted.bytes)
            .await?;
        Ok(NativeOutcome::new(json!({
            "model_media_obj_id": artifact.obj_id,
            "mime": fitted.mime,
            "original": fitted.original,
            "sent": fitted.sent,
            "actions": fitted.actions,
        }))
        .with_artifacts(vec![artifact.obj_id]))
    }
}
