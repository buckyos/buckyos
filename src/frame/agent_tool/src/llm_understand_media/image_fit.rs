//! 送进 vision 模型前的图片预处理（doc/llm_context/LLM Understand Media.md §4.5.3 的
//! `preprocess_image`）。
//!
//! 先只读文件头：尺寸、像素数、字节数、格式、EXIF 方向都合规的图片原样透传，不重新编码；
//! 否则固化 EXIF 方向、等比缩小、按需降质，输出 PNG（透明或无损源）或 JPEG。
//! `image` crate 只开了 jpeg/png/webp/bmp；其它格式（超限 GIF、TIFF、HEIC、AVIF…）先用
//! ffmpeg 转成 PNG 再走同一流程。

use std::borrow::Cow;
use std::io::Cursor;
use std::str::FromStr;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Rgb, RgbImage, RgbaImage};
use serde::Serialize;
use tokio::process::Command;

const DEFAULT_MAX_LONG_EDGE: u32 = 2048;
const DEFAULT_MAX_PIXELS: u64 = 2048 * 2048;
/// 3.5 MiB：base64 后约 4.67 MiB，低于常见厂商单图 5MB 上限。
const DEFAULT_MAX_BYTES: usize = 3 * 1024 * 1024 + 512 * 1024;
const DEFAULT_VIDEO_FRAME_MAX_LONG_EDGE: u32 = 1280;
const JPEG_QUALITY_LADDER: [u8; 3] = [85, 75, 60];
const MAX_SHRINK_ROUNDS: usize = 3;
const SHRINK_FACTOR: f64 = 0.75;
/// 解码后像素缓冲的上限，挡住文件头声明超大尺寸的解压炸弹。
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// 本进程用 `image` crate 解码的格式。
const DECODABLE_FORMATS: [ImageFormat; 4] = [
    ImageFormat::Png,
    ImageFormat::Jpeg,
    ImageFormat::WebP,
    ImageFormat::Bmp,
];
/// vision 厂商普遍直接接受、可原样透传的格式（GIF 另按文件头判断）。
const PASSTHROUGH_FORMATS: [ImageFormat; 3] =
    [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ImageLimits {
    pub max_long_edge: u32,
    pub max_pixels: u64,
    pub max_bytes: usize,
}

impl Default for ImageLimits {
    fn default() -> Self {
        Self {
            max_long_edge: DEFAULT_MAX_LONG_EDGE,
            max_pixels: DEFAULT_MAX_PIXELS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

impl ImageLimits {
    pub(super) fn from_env() -> Self {
        let default = Self::default();
        Self {
            max_long_edge: env_positive("LLM_UNDERSTAND_MEDIA_IMAGE_MAX_EDGE")
                .unwrap_or(default.max_long_edge),
            max_pixels: env_positive("LLM_UNDERSTAND_MEDIA_IMAGE_MAX_PIXELS")
                .unwrap_or(default.max_pixels),
            max_bytes: env_positive("LLM_UNDERSTAND_MEDIA_IMAGE_MAX_BYTES")
                .unwrap_or(default.max_bytes),
        }
    }

    fn allows(&self, width: u32, height: u32, bytes: usize) -> bool {
        width.max(height) <= self.max_long_edge
            && u64::from(width) * u64::from(height) <= self.max_pixels
            && bytes <= self.max_bytes
    }

    /// 同时满足长边与像素数的等比缩放系数，不放大。
    fn scale_for(&self, width: u32, height: u32) -> f64 {
        let by_edge = f64::from(self.max_long_edge) / f64::from(width.max(height));
        let by_pixels = (self.max_pixels as f64 / (f64::from(width) * f64::from(height))).sqrt();
        by_edge.min(by_pixels).min(1.0)
    }
}

/// 视频抽帧的长边上限；帧数最多 16，单帧比单图收得更紧。
pub(super) fn video_frame_max_long_edge() -> u32 {
    env_positive("LLM_UNDERSTAND_MEDIA_VIDEO_FRAME_MAX_EDGE")
        .unwrap_or(DEFAULT_VIDEO_FRAME_MAX_LONG_EDGE)
}

fn env_positive<T: FromStr + PartialOrd + Default>(key: &str) -> Option<T> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<T>().ok())
        .filter(|value| *value > T::default())
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct ImageInfo {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

/// 写进 `details.media_preprocess`，让调用方知道模型实际看到的是什么。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct ImagePreprocess {
    pub original: ImageInfo,
    pub sent: ImageInfo,
    /// `converted` / `rotated` / `downscaled` / `recompressed` / `mime_corrected`；空表示原样透传。
    pub actions: Vec<&'static str>,
}

impl ImagePreprocess {
    fn downscaled(&self) -> bool {
        self.actions.contains(&"downscaled")
    }

    /// 放在图片前给旁路模型的说明；只在会影响判断时出现。
    pub(super) fn model_note(&self) -> Option<String> {
        let mut notes = Vec::new();
        if self.downscaled() {
            notes.push(format!(
                "this image was downscaled from {}x{} to {}x{} pixels before analysis, so very small text or fine detail may be lost and any pixel coordinates refer to the downscaled image",
                self.original.width, self.original.height, self.sent.width, self.sent.height
            ));
        }
        if self.original.mime == "image/gif" && self.actions.contains(&"converted") {
            notes.push("only the first frame of the GIF is shown".to_string());
        }
        (!notes.is_empty()).then(|| format!("Image preprocessing note: {}.", notes.join("; ")))
    }

    /// 追加到工具 output 末尾给主干 agent 的说明。
    pub(super) fn caller_note(&self) -> Option<String> {
        self.downscaled().then(|| {
            format!(
                "Note: analyzed a downscaled copy ({}x{} -> {}x{}); for fine detail, crop the region of interest and analyze the crop.",
                self.original.width, self.original.height, self.sent.width, self.sent.height
            )
        })
    }
}

pub(super) struct FittedImage {
    pub mime: String,
    pub bytes: Vec<u8>,
    pub preprocess: ImagePreprocess,
}

enum Fit {
    Done(FittedImage),
    /// `image` crate 读不了，交给 ffmpeg。
    NeedsConversion(Vec<u8>),
}

pub(super) async fn fit_image_for_vision(
    bytes: Vec<u8>,
    declared_mime: &str,
    limits: ImageLimits,
) -> Result<FittedImage, String> {
    let declared = declared_mime.to_string();
    let bytes = {
        let declared = declared.clone();
        match run_blocking(move || fit_decodable(bytes, &declared, limits)).await? {
            Fit::Done(fitted) => return Ok(fitted),
            Fit::NeedsConversion(bytes) => bytes,
        }
    };

    let original_bytes = bytes.len();
    let png = convert_with_ffmpeg(&bytes, &declared).await?;
    drop(bytes);
    let Fit::Done(mut fitted) =
        run_blocking(move || fit_decodable(png, "image/png", limits)).await?
    else {
        return Err(format!(
            "ffmpeg converted `{declared}` image to an unreadable PNG"
        ));
    };
    let preprocess = &mut fitted.preprocess;
    preprocess.original.mime = declared;
    preprocess.original.bytes = original_bytes;
    if !preprocess.actions.contains(&"converted") {
        preprocess.actions.insert(0, "converted");
    }
    Ok(fitted)
}

async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| format!("image preprocessing task failed: {err}"))?
}

fn fit_decodable(bytes: Vec<u8>, declared_mime: &str, limits: ImageLimits) -> Result<Fit, String> {
    if let Some((width, height)) = gif_dimensions(&bytes) {
        if !limits.allows(width, height, bytes.len()) {
            return Ok(Fit::NeedsConversion(bytes));
        }
        let info = ImageInfo {
            mime: "image/gif".to_string(),
            width,
            height,
            bytes: bytes.len(),
        };
        return Ok(Fit::Done(passthrough(bytes, declared_mime, info)));
    }

    let Some(format) = image::guess_format(&bytes)
        .ok()
        .filter(|format| DECODABLE_FORMATS.contains(format))
    else {
        return Ok(Fit::NeedsConversion(bytes));
    };
    let mime = format.to_mime_type();
    let mut decoder = ImageReader::with_format(Cursor::new(bytes.as_slice()), format)
        .into_decoder()
        .map_err(|err| format!("read {mime} image header failed: {err}"))?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let original = ImageInfo {
        mime: mime.to_string(),
        width,
        height,
        bytes: bytes.len(),
    };
    if PASSTHROUGH_FORMATS.contains(&format)
        && orientation == Orientation::NoTransforms
        && limits.allows(width, height, bytes.len())
    {
        drop(decoder);
        return Ok(Fit::Done(passthrough(bytes, declared_mime, original)));
    }

    // image 的部分解码器不执行 Limits::max_alloc，解码前自己按缓冲大小判断。
    if decoder.total_bytes() > MAX_DECODE_BYTES {
        return Err(format!(
            "{mime} image {width}x{height} is too large to decode (needs {} MiB, limit {} MiB)",
            decoder.total_bytes() / (1024 * 1024),
            MAX_DECODE_BYTES / (1024 * 1024)
        ));
    }
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|err| format!("decode {mime} image ({width}x{height}) failed: {err}"))?;
    drop(bytes);
    let mut actions = Vec::new();
    if orientation != Orientation::NoTransforms {
        image.apply_orientation(orientation);
        actions.push("rotated");
    }
    let prefer_png = matches!(format, ImageFormat::Png | ImageFormat::Bmp);
    let (sent_mime, encoded, sent_width, sent_height) =
        encode_within_limits(&image, prefer_png, limits)?;
    if sent_mime != original.mime {
        actions.insert(0, "converted");
    }
    if u64::from(sent_width) * u64::from(sent_height)
        < u64::from(image.width()) * u64::from(image.height())
    {
        actions.push("downscaled");
    } else if actions.is_empty() {
        actions.push("recompressed");
    }
    Ok(Fit::Done(FittedImage {
        mime: sent_mime.to_string(),
        preprocess: ImagePreprocess {
            original,
            sent: ImageInfo {
                mime: sent_mime.to_string(),
                width: sent_width,
                height: sent_height,
                bytes: encoded.len(),
            },
            actions,
        },
        bytes: encoded,
    }))
}

fn passthrough(bytes: Vec<u8>, declared_mime: &str, info: ImageInfo) -> FittedImage {
    let actions = if declared_mime == info.mime {
        Vec::new()
    } else {
        vec!["mime_corrected"]
    };
    FittedImage {
        mime: info.mime.clone(),
        bytes,
        preprocess: ImagePreprocess {
            original: ImageInfo {
                mime: declared_mime.to_string(),
                ..info.clone()
            },
            sent: info,
            actions,
        },
    }
}

/// 等比缩到限制内后编码；字节仍超限就降 JPEG 质量，再不行每轮长边乘 0.75。
fn encode_within_limits(
    image: &DynamicImage,
    prefer_png: bool,
    limits: ImageLimits,
) -> Result<(&'static str, Vec<u8>, u32, u32), String> {
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
    Err(format!(
        "image is still larger than {} bytes after downscaling to {}x{}",
        limits.max_bytes, target.0, target.1
    ))
}

fn scaled_dims(width: u32, height: u32, scale: f64) -> (u32, u32) {
    if scale >= 1.0 {
        return (width, height);
    }
    // 加一点 epsilon 再取整，避免 2048.0 被浮点误差压成 2047。
    let scale_edge = |edge: u32| ((f64::from(edge) * scale + 1e-6).floor() as u32).clamp(1, edge);
    (scale_edge(width), scale_edge(height))
}

fn encode_candidate(
    image: &DynamicImage,
    prefer_png: bool,
    max_bytes: usize,
) -> Result<Option<(&'static str, Vec<u8>)>, String> {
    let rgba = image
        .color()
        .has_alpha()
        .then(|| image.to_rgba8())
        .filter(|rgba| rgba.pixels().any(|pixel| pixel[3] < u8::MAX));
    if rgba.is_some() || prefer_png {
        let mut png = Vec::new();
        let written = match &rgba {
            Some(rgba) => rgba.write_with_encoder(PngEncoder::new(&mut png)),
            None => image
                .to_rgb8()
                .write_with_encoder(PngEncoder::new(&mut png)),
        };
        written.map_err(|err| format!("encode PNG failed: {err}"))?;
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
            .map_err(|err| format!("encode JPEG failed: {err}"))?;
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

fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    (width > 0 && height > 0).then_some((width, height))
}

/// 原分辨率转成 PNG（动图取首帧），尺寸与编码交回 `fit_decodable`。
async fn convert_with_ffmpeg(bytes: &[u8], mime: &str) -> Result<Vec<u8>, String> {
    let temp_dir = super::media_temp_dir("llm-understand-image");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|err| format!("create image conversion temp dir failed: {err}"))?;
    let extension: String = mime
        .rsplit('/')
        .next()
        .unwrap_or("bin")
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    let input_path = temp_dir.join(format!("input.{extension}"));
    let output_path = temp_dir.join("output.png");
    let result = async {
        tokio::fs::write(&input_path, bytes)
            .await
            .map_err(|err| format!("write temporary image failed: {err}"))?;
        let output = buckyos_api::hide_child_console_async(&mut Command::new("ffmpeg"))
            .args(["-v", "error", "-y", "-i"])
            .arg(&input_path)
            .args(["-frames:v", "1"])
            .arg(&output_path)
            .output()
            .await
            .map_err(|err| {
                format!("`{mime}` image must be converted for vision models, but starting ffmpeg failed: {err}")
            })?;
        if !output.status.success() {
            return Err(format!(
                "convert `{mime}` image with ffmpeg failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        tokio::fs::read(&output_path)
            .await
            .map_err(|err| format!("read converted image failed: {err}"))
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn limits() -> ImageLimits {
        ImageLimits::default()
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        let mut out = Vec::new();
        image.write_with_encoder(PngEncoder::new(&mut out)).unwrap();
        out
    }

    fn jpeg(width: u32, height: u32, exif: Option<Vec<u8>>) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, _| Rgb([(x % 256) as u8, 30, 200]));
        let mut out = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut out, 90);
        if let Some(exif) = exif {
            encoder.set_exif_metadata(exif).unwrap();
        }
        image.write_with_encoder(encoder).unwrap();
        out
    }

    /// 只有一个 Orientation 条目的大端 TIFF/EXIF 块。
    fn exif_orientation(value: u16) -> Vec<u8> {
        let mut exif = b"MM\0\x2a\0\0\0\x08".to_vec();
        exif.extend_from_slice(&1u16.to_be_bytes());
        exif.extend_from_slice(&0x0112u16.to_be_bytes());
        exif.extend_from_slice(&3u16.to_be_bytes());
        exif.extend_from_slice(&1u32.to_be_bytes());
        exif.extend_from_slice(&value.to_be_bytes());
        exif.extend_from_slice(&[0, 0]);
        exif.extend_from_slice(&0u32.to_be_bytes());
        exif
    }

    fn fit(bytes: Vec<u8>, mime: &str, limits: ImageLimits) -> FittedImage {
        match fit_decodable(bytes, mime, limits).expect("fit image") {
            Fit::Done(fitted) => fitted,
            Fit::NeedsConversion(_) => panic!("image should not need ffmpeg"),
        }
    }

    fn sent_dimensions(fitted: &FittedImage) -> (u32, u32) {
        image::load_from_memory(&fitted.bytes)
            .map(|image| (image.width(), image.height()))
            .expect("sent image must decode")
    }

    #[test]
    fn compliant_image_is_passed_through_byte_for_byte() {
        let bytes = png(640, 480);
        let fitted = fit(bytes.clone(), "image/png", limits());
        assert_eq!(fitted.bytes, bytes);
        assert_eq!(fitted.mime, "image/png");
        assert!(fitted.preprocess.actions.is_empty());
        assert_eq!(fitted.preprocess.model_note(), None);
        assert_eq!(fitted.preprocess.caller_note(), None);
    }

    #[test]
    fn wrong_declared_mime_is_corrected_without_reencoding() {
        let bytes = jpeg(64, 64, None);
        let fitted = fit(bytes.clone(), "image/png", limits());
        assert_eq!(fitted.bytes, bytes);
        assert_eq!(fitted.mime, "image/jpeg");
        assert_eq!(fitted.preprocess.actions, vec!["mime_corrected"]);
    }

    #[test]
    fn long_thin_image_is_downscaled_to_the_long_edge() {
        let fitted = fit(png(5000, 100), "image/png", limits());
        assert_eq!(sent_dimensions(&fitted), (2048, 40));
        assert_eq!(
            (fitted.preprocess.sent.width, fitted.preprocess.sent.height),
            (2048, 40)
        );
        assert_eq!(fitted.preprocess.actions, vec!["downscaled"]);
        assert_eq!(fitted.mime, "image/png");
        let note = fitted.preprocess.model_note().expect("downscale note");
        assert!(note.contains("5000x100") && note.contains("2048x40"));
        assert!(fitted.preprocess.caller_note().is_some());
    }

    #[test]
    fn pixel_budget_shrinks_images_within_the_long_edge() {
        let fitted = fit(
            png(1600, 1600),
            "image/png",
            ImageLimits {
                max_pixels: 1000 * 1000,
                ..limits()
            },
        );
        let (width, height) = sent_dimensions(&fitted);
        assert_eq!((width, height), (1000, 1000));
    }

    #[test]
    fn exif_rotation_is_baked_into_pixels() {
        // Orientation 6 = 顺时针转 90°，宽高对调。
        let fitted = fit(
            jpeg(80, 40, Some(exif_orientation(6))),
            "image/jpeg",
            limits(),
        );
        assert_eq!(sent_dimensions(&fitted), (40, 80));
        assert_eq!(fitted.preprocess.original.width, 80);
        assert_eq!(fitted.preprocess.actions, vec!["rotated"]);
        assert_eq!(fitted.preprocess.model_note(), None);
    }

    #[test]
    fn downscaled_transparent_png_keeps_its_alpha() {
        let transparent =
            RgbaImage::from_fn(3000, 20, |x, _| image::Rgba([0, 0, 0, (x % 2 * 255) as u8]));
        let mut bytes = Vec::new();
        transparent
            .write_with_encoder(PngEncoder::new(&mut bytes))
            .unwrap();
        let fitted = fit(bytes, "image/png", limits());
        assert_eq!(fitted.mime, "image/png");
        assert!(image::load_from_memory(&fitted.bytes)
            .unwrap()
            .color()
            .has_alpha());
    }

    #[test]
    fn only_real_transparency_forces_png() {
        let opaque = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            64,
            64,
            image::Rgba([10, 20, 30, 255]),
        ));
        let (mime, _) = encode_candidate(&opaque, false, usize::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/jpeg");

        let translucent = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            64,
            64,
            image::Rgba([10, 20, 30, 128]),
        ));
        let (mime, _) = encode_candidate(&translucent, false, usize::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn jpeg_fallback_flattens_transparency_onto_white() {
        let rgba = RgbaImage::from_fn(3, 1, |x, _| match x {
            0 => image::Rgba([0, 0, 0, 0]),
            1 => image::Rgba([0, 0, 0, 255]),
            _ => image::Rgba([200, 100, 0, 128]),
        });
        let rgb = flatten_on_white(&rgba);
        assert_eq!(rgb.get_pixel(0, 0).0, [255, 255, 255]);
        assert_eq!(rgb.get_pixel(1, 0).0, [0, 0, 0]);
        assert_eq!(rgb.get_pixel(2, 0).0, [227, 177, 127]);
    }

    #[test]
    fn byte_budget_lowers_quality_then_shrinks() {
        let noisy = noise_rgb(1200, 1200);
        let mut lowest_quality = Vec::new();
        noisy
            .write_with_encoder(JpegEncoder::new_with_quality(
                &mut lowest_quality,
                *JPEG_QUALITY_LADDER.last().unwrap(),
            ))
            .unwrap();
        let max_bytes = lowest_quality.len() / 2;
        let mut bytes = Vec::new();
        noisy
            .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, 95))
            .unwrap();

        let fitted = fit(
            bytes,
            "image/jpeg",
            ImageLimits {
                max_bytes,
                ..limits()
            },
        );
        assert_eq!(fitted.mime, "image/jpeg");
        assert!(fitted.bytes.len() <= max_bytes);
        let (width, height) = sent_dimensions(&fitted);
        assert!(width < 1200 && width == height, "{width}x{height}");
        assert_eq!(fitted.preprocess.actions, vec!["downscaled"]);
    }

    #[test]
    fn decompression_bomb_header_is_rejected_before_decoding() {
        // IHDR 声明 60000x60000 RGB，解码缓冲约 10 GiB。
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&[0, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        bytes[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        let err = match fit_decodable(bytes, "image/png", limits()) {
            Ok(_) => panic!("bomb must not be accepted"),
            Err(err) => err,
        };
        assert!(
            err.contains("60000x60000") && err.contains("too large to decode"),
            "{err}"
        );
    }

    fn noise_rgb(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            let mut z = (u64::from(y) << 32 | u64::from(x)).wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            Rgb([z as u8, (z >> 8) as u8, (z >> 16) as u8])
        })
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn small_gif_passes_through_and_large_gif_needs_conversion() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&320u16.to_le_bytes());
        gif.extend_from_slice(&200u16.to_le_bytes());
        gif.extend_from_slice(&[0; 8]);
        let fitted = fit(gif.clone(), "image/gif", limits());
        assert_eq!(fitted.mime, "image/gif");
        assert_eq!(fitted.bytes, gif);

        gif[6..8].copy_from_slice(&4000u16.to_le_bytes());
        assert!(matches!(
            fit_decodable(gif, "image/gif", limits()),
            Ok(Fit::NeedsConversion(_))
        ));
    }

    #[test]
    fn formats_outside_the_image_crate_need_conversion() {
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec();
        assert!(matches!(
            fit_decodable(heic, "image/heic", limits()),
            Ok(Fit::NeedsConversion(_))
        ));
    }

    /// 未压缩 RGB 小端 TIFF；`image` 没开 tiff，只能走 ffmpeg。
    fn tiff_rgb(width: u32, height: u32) -> Vec<u8> {
        const SHORT: u16 = 3;
        const LONG: u16 = 4;
        let entries: [(u16, u16, u32, u32); 9] = [
            (256, LONG, 1, width),
            (257, LONG, 1, height),
            (258, SHORT, 3, 122),
            (259, SHORT, 1, 1),
            (262, SHORT, 1, 2),
            (273, LONG, 1, 128),
            (277, SHORT, 1, 3),
            (278, LONG, 1, height),
            (279, LONG, 1, width * height * 3),
        ];
        let mut tiff = b"II\x2a\0".to_vec();
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&kind.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        }
        tiff.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..3 {
            tiff.extend_from_slice(&8u16.to_le_bytes());
        }
        assert_eq!(tiff.len(), 128);
        tiff.extend(noise_rgb(width, height).into_raw());
        tiff
    }

    #[tokio::test]
    async fn other_formats_are_converted_with_ffmpeg_when_available() {
        if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("skip: ffmpeg not installed");
            return;
        }
        let tiff = tiff_rgb(3000, 1000);
        let original_bytes = tiff.len();
        let fitted = fit_image_for_vision(tiff, "image/tiff", limits())
            .await
            .expect("ffmpeg converts TIFF");
        assert_eq!(sent_dimensions(&fitted), (2048, 682));
        assert_eq!(fitted.preprocess.original.mime, "image/tiff");
        assert_eq!(fitted.preprocess.original.bytes, original_bytes);
        assert_eq!(
            (fitted.preprocess.original.width, fitted.preprocess.original.height),
            (3000, 1000)
        );
        assert_eq!(fitted.preprocess.actions[..2], ["converted", "downscaled"]);
        assert!(fitted.bytes.len() <= limits().max_bytes);
    }

    #[test]
    fn scaled_dims_hit_the_limit_exactly() {
        let limits = limits();
        let scale = limits.scale_for(6000, 4000);
        assert_eq!(scaled_dims(6000, 4000, scale), (2048, 1365));
        assert_eq!(
            scaled_dims(640, 480, limits.scale_for(640, 480)),
            (640, 480)
        );
    }
}
