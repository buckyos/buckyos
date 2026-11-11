//! The fixed Target Profiles (TODO D5) as ffmpeg / soffice invocations.
//!
//! - `preview-image`: JPEG q≈85, long edge ≤ 4096, BT.601 full range (what
//!   JPEG decoders assume), HEIF `irot`/`imir` baked into pixels, alpha
//!   flattened onto white.
//! - `preview-video`: H.264 High ≤ 1080p yuv420p + AAC 128k MP4 `+faststart`,
//!   rotation baked by ffmpeg's autorotate; a playable codec is only remuxed.
//! - `preview-audio`: AAC in MP4 for a container with no picture.
//! - `thumbnail`: JPEG, long edge ≤ 640 (a video's frame at min(1 s, 10 %)).
//! - `preview-document`: PDF by LibreOffice (Office import TODO §5.1, Q11).
//! - `preview-sheet`: xlsx by LibreOffice, for the viewer's sheet renderer.
//! - `preview-html`: one self-contained HTML file by `pandoc --sandbox`.
//! - Stills ffmpeg does not decode (RAW, XCF, JXL, …) go through ImageMagick
//!   with an explicit coder and resource limits; a PDF's thumbnail is its
//!   first page by `pdftoppm`, a document's the picture it carries inside.
//!
//! HDR (PQ / HLG) is tone-mapped to SDR BT.709 and noted in the fidelity.

use super::heif::{HeifInfo, Nclx, Transform};
use super::media::{Kind, MediaInfo, Stream};
use super::tools::{PdfTools, Tool, Toolchain};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    PreviewImage,
    PreviewVideo,
    PreviewAudio,
    Thumbnail,
    PreviewDocument,
    PreviewSheet,
    PreviewHtml,
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::PreviewImage => "preview-image",
            Profile::PreviewVideo => "preview-video",
            Profile::PreviewAudio => "preview-audio",
            Profile::Thumbnail => "thumbnail",
            Profile::PreviewDocument => "preview-document",
            Profile::PreviewSheet => "preview-sheet",
            Profile::PreviewHtml => "preview-html",
        }
    }
}

pub const IMAGE_MAX_EDGE: u32 = 4096;
pub const THUMBNAIL_MAX_EDGE: u32 = 640;
const VIDEO_LONG: u32 = 1920;
const VIDEO_SHORT: u32 = 1080;
const JPEG_QSCALE: &str = "3";
const THUMBNAIL_QSCALE: &str = "5";
pub const FIDELITY_HDR: &str = "HDR 已映射为 SDR";
pub const FIDELITY_HDR_RAW: &str = "HDR 未能映射为 SDR（缺少 zscale/tonemap 滤镜），颜色可能失真";
pub const FIDELITY_ALPHA: &str = "透明区域以白底显示";
pub const FIDELITY_DOCUMENT: &str = "由 LibreOffice 转换，字体与版式可能与原件有差异";
pub const FIDELITY_SHEET: &str = "由 LibreOffice 转换为表格显示，图表与宏不显示";
pub const FIDELITY_HTML: &str = "已转换为网页显示";
pub const FIDELITY_EMBEDDED: &str = "文档内嵌的缩略图";
/// The viewer's HTML renderer reads at most this much (TODO §3.4).
pub const HTML_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// A document's own thumbnail larger than this is not taken.
pub const EMBEDDED_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// A deterministic "this content has no thumbnail": answered as `unsupported`, not as a failure.
pub const NO_THUMBNAIL: &str = "NO_THUMBNAIL";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

#[derive(Debug, Clone)]
pub struct ConvertError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl ConvertError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ConvertError { code, message: message.into(), retryable: false }
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    pub path: PathBuf,
    pub media_type: &'static str,
    pub result_type: &'static str,
    pub plan: &'static str,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub fidelity: Vec<&'static str>,
}

pub struct Job<'a> {
    pub tools: &'a Toolchain,
    pub input: &'a Path,
    pub dir: &'a Path,
    pub kind: Kind,
    pub profile: Profile,
    /// The Runtime hinted it decodes HEVC (only remux, never transcode, then).
    pub hevc_playable: bool,
    pub heif: Option<HeifInfo>,
    pub progress: Arc<dyn Fn(f64) + Send + Sync>,
}

/// `scale` to fit a `max × max` box (never upscales), even dimensions.
fn fit(max: u32, extra: &str) -> String {
    format!("scale=w='min({max},iw)':h='min({max},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2{extra}")
}

/// 1920×1080 for landscape, 1080×1920 for portrait (after autorotate).
fn fit_video() -> String {
    format!(
        "scale=w='if(gte(iw,ih),min({VIDEO_LONG},iw),min({VIDEO_SHORT},iw))':h='if(gte(iw,ih),min({VIDEO_SHORT},ih),min({VIDEO_LONG},ih))':force_original_aspect_ratio=decrease:force_divisible_by=2"
    )
}

/// HDR → SDR BT.709; `input` names the source colour when frames carry none (HEIF tiles).
fn tonemap(input: Option<(&str, &str, &str, &str)>, out_matrix: &str, out_range: &str, out_format: &str) -> String {
    let src = input.map(|(t, p, m, r)| format!("tin={t}:pin={p}:min={m}:rin={r}:")).unwrap_or_default();
    format!(
        "zscale={src}t=linear:npl=100,format=gbrpf32le,zscale=p=709,tonemap=tonemap=hable:desat=0,zscale=t=709:m={out_matrix}:r={out_range},format={out_format}"
    )
}

fn zimg_transfer(t: u16) -> &'static str {
    match t {
        16 => "smpte2084",
        18 => "arib-std-b67",
        14 => "2020_10",
        15 => "2020_12",
        13 => "iec61966-2-1",
        6 => "601",
        _ => "709",
    }
}

fn zimg_primaries(p: u16) -> &'static str {
    match p {
        9 => "2020",
        12 => "smpte432",
        11 => "smpte431",
        5 | 6 => "170m",
        _ => "709",
    }
}

fn zimg_matrix(m: u16) -> &'static str {
    match m {
        9 => "2020_ncl",
        10 => "2020_cl",
        5 => "bt470bg",
        6 => "170m",
        0 => "gbr",
        _ => "bt709",
    }
}

fn sws_matrix(m: u16) -> &'static str {
    match m {
        1 => "bt709",
        9 | 10 => "bt2020",
        5 | 6 => "bt601",
        _ => "bt601",
    }
}

fn transform_filters(transforms: &[Transform]) -> Vec<&'static str> {
    transforms
        .iter()
        .flat_map(|t| match t {
            Transform::Rotate(1) => vec!["transpose=2"],
            Transform::Rotate(2) => vec!["hflip", "vflip"],
            Transform::Rotate(3) => vec!["transpose=1"],
            Transform::Rotate(_) => vec![],
            Transform::Mirror(0) => vec!["hflip"],
            Transform::Mirror(_) => vec!["vflip"],
        })
        .collect()
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Filter graph turning a HEIF primary image into `[out]` (JPEG-ready).
pub fn heif_graph(
    info: &MediaInfo,
    heif: &HeifInfo,
    tools: &Toolchain,
    max_edge: u32,
    fidelity: &mut Vec<&'static str>,
) -> Result<String, ConvertError> {
    let mut graph = String::new();
    if heif.grid {
        let g = info.primary_grid().ok_or_else(|| ConvertError::new("CONTENT_UNSUPPORTED", "the HEIF tile grid is not readable by this ffmpeg"))?;
        let crop = format!("crop={}:{}:{}:{}", g.width, g.height, g.h_offset, g.v_offset);
        if g.tiles.len() == 1 {
            graph.push_str(&format!("[0:{}]{crop}[img];", g.tiles[0].0));
        } else {
            let inputs: String = g.tiles.iter().map(|(i, _, _)| format!("[0:{i}]")).collect();
            let layout: Vec<String> = g.tiles.iter().map(|(_, x, y)| format!("{x}_{y}")).collect();
            graph.push_str(&format!("{inputs}xstack=inputs={}:layout={}:fill=black,{crop}[img];", g.tiles.len(), layout.join("|")));
        }
    } else {
        let p = info.primary_stream().ok_or_else(|| ConvertError::new("CONTENT_UNSUPPORTED", "the HEIF file has no image"))?;
        match info.alpha_for(p).filter(|_| heif.has_alpha) {
            Some(a) => {
                graph.push_str(&format!(
                    "[0:{}]format=yuva444p[c];[0:{}]format=gray[a];[c][a]alphamerge[ca];color=c=white:s={}x{},format=yuva444p[bg];[bg][ca]overlay=shortest=1:format=auto[img];",
                    p.index, a.index, p.width, p.height
                ));
                fidelity.push(FIDELITY_ALPHA);
            }
            None => graph.push_str(&format!("[0:{}]null[img];", p.index)),
        }
    }
    let mut chain: Vec<String> = transform_filters(&heif.transforms).into_iter().map(String::from).collect();
    let nclx = heif.nclx.unwrap_or(Nclx { primaries: 1, transfer: 13, matrix: 6, full_range: true });
    let range = if nclx.full_range { "full" } else { "limited" };
    if heif.hdr() {
        chain.push(fit(max_edge, ""));
        if tools.tonemap {
            chain.push(tonemap(Some((zimg_transfer(nclx.transfer), zimg_primaries(nclx.primaries), zimg_matrix(nclx.matrix), range)), "170m", "full", "yuvj420p"));
            fidelity.push(FIDELITY_HDR);
        } else {
            chain.push("format=yuvj420p".into());
            fidelity.push(FIDELITY_HDR_RAW);
        }
    } else {
        let sws_range = if nclx.full_range { "pc" } else { "tv" };
        chain.push(fit(max_edge, &format!(":in_color_matrix={}:out_color_matrix=bt601:in_range={sws_range}:out_range=pc", sws_matrix(nclx.matrix))));
        chain.push("format=yuvj420p".into());
    }
    graph.push_str(&format!("[img]{}[out]", chain.join(",")));
    Ok(graph)
}

/// A still image other than HEIF: alpha onto white, fit, JPEG.
pub fn image_graph(v: &Stream, max_edge: u32, fidelity: &mut Vec<&'static str>) -> String {
    let head = if v.has_alpha() && v.width > 0 && v.height > 0 {
        fidelity.push(FIDELITY_ALPHA);
        format!("color=c=white:s={}x{}[bg];[bg][0:{}]overlay=shortest=1:format=auto,", v.width, v.height, v.index)
    } else {
        format!("[0:{}]", v.index)
    };
    format!("{head}{},format=yuvj420p[out]", fit(max_edge, ":out_range=pc"))
}

fn video_color(v: &Stream, tools: &Toolchain, fidelity: &mut Vec<&'static str>, out_format: &str, jpeg: bool) -> String {
    if v.hdr() {
        if tools.tonemap {
            fidelity.push(FIDELITY_HDR);
            return tonemap(None, if jpeg { "170m" } else { "709" }, if jpeg { "full" } else { "limited" }, out_format);
        }
        fidelity.push(FIDELITY_HDR_RAW);
    }
    format!("format={out_format}")
}

const MP4_AUDIO: [&str; 2] = ["aac", "mp3"];

fn remuxable(v: &Stream, hevc_playable: bool) -> bool {
    let eight_bit_420 = matches!(v.pix_fmt.as_str(), "yuv420p" | "yuvj420p");
    match v.codec_name.as_str() {
        "h264" => eight_bit_420,
        "hevc" => hevc_playable && (eight_bit_420 || v.pix_fmt == "yuv420p10le"),
        _ => false,
    }
}

/// `(args after the input, output, plan)` for a profile.
#[allow(clippy::type_complexity)]
pub fn plan(job: &Job, info: &MediaInfo) -> Result<(Vec<OsString>, Output), ConvertError> {
    let mut fidelity = Vec::new();
    let jpeg = |name: &str, q: &str, graph: String, plan: &'static str, fidelity: Vec<&'static str>, size: Option<(u32, u32)>| {
        let out = job.dir.join(name);
        let mut args = os(&["-filter_complex"]);
        args.push(graph.into());
        args.extend(os(&["-map", "[out]", "-frames:v", "1", "-q:v", q, "-f", "image2", "-c:v", "mjpeg"]));
        args.push(out.clone().into());
        (args, Output { path: out, media_type: "image/jpeg", result_type: "image", plan, width: size.map(|s| s.0), height: size.map(|s| s.1), duration: None, fidelity })
    };
    let fitted = |(w, h): (u32, u32), max: u32| -> (u32, u32) {
        let scale = (max as f64 / w.max(h) as f64).min(1.0);
        let even = |v: f64| ((v / 2.0).floor() as u32 * 2).max(2);
        (even(w as f64 * scale), even(h as f64 * scale))
    };
    match (job.kind, job.profile) {
        (Kind::Heif, Profile::PreviewImage | Profile::Thumbnail) => {
            let heif = job.heif.as_ref().ok_or_else(|| ConvertError::new("CONTENT_UNSUPPORTED", "no HEIF primary image"))?;
            let max = if job.profile == Profile::Thumbnail { THUMBNAIL_MAX_EDGE } else { IMAGE_MAX_EDGE };
            let graph = heif_graph(info, heif, job.tools, max, &mut fidelity)?;
            let q = if job.profile == Profile::Thumbnail { THUMBNAIL_QSCALE } else { JPEG_QSCALE };
            Ok(jpeg("out.jpg", q, graph, "heif-jpeg", fidelity, Some(fitted(heif.display_size(), max))))
        }
        (Kind::Image, Profile::PreviewImage | Profile::Thumbnail) => {
            let v = info.video().ok_or_else(|| ConvertError::new("CONTENT_UNSUPPORTED", "the image has no picture ffmpeg can decode"))?;
            let max = if job.profile == Profile::Thumbnail { THUMBNAIL_MAX_EDGE } else { IMAGE_MAX_EDGE };
            let q = if job.profile == Profile::Thumbnail { THUMBNAIL_QSCALE } else { JPEG_QSCALE };
            let graph = image_graph(v, max, &mut fidelity);
            let size = (v.width > 0 && v.height > 0).then(|| fitted((v.width, v.height), max));
            Ok(jpeg("out.jpg", q, graph, "image-jpeg", fidelity, size))
        }
        (Kind::Video, Profile::Thumbnail) => {
            let v = info.video().ok_or_else(|| ConvertError::new("CONTENT_UNSUPPORTED", "the video has no picture"))?;
            let at = info.duration.map(|d| (d * 0.1).min(1.0)).unwrap_or(0.0);
            let color = video_color(v, job.tools, &mut fidelity, "yuvj420p", true);
            let graph = format!("[0:{}]{},{color}[out]", v.index, fit(THUMBNAIL_MAX_EDGE, ":out_range=pc"));
            let (mut args, mut out) = jpeg("out.jpg", THUMBNAIL_QSCALE, graph, "video-frame", fidelity, None);
            let mut seek = os(&["-ss", &format!("{at:.3}")]);
            seek.append(&mut args);
            out.duration = info.duration;
            Ok((seek, out))
        }
        (Kind::Video, Profile::PreviewVideo) => {
            let out = job.dir.join("out.mp4");
            let Some(v) = info.video() else {
                return audio_plan(job, info);
            };
            let mut args = os(&["-map", &format!("0:{}", v.index)]);
            if let Some(a) = info.audio() {
                args.extend(os(&["-map", &format!("0:{}", a.index)]));
            }
            args.extend(os(&["-sn", "-dn", "-map_metadata", "-1"]));
            let plan = if remuxable(v, job.hevc_playable) {
                args.extend(os(&["-c:v", "copy"]));
                if v.codec_name == "hevc" {
                    args.extend(os(&["-tag:v", "hvc1"]));
                }
                "remux"
            } else {
                if !job.tools.libx264 {
                    return Err(ConvertError::new("CONVERTER_UNAVAILABLE", format!("{} video needs the libx264 encoder, which this ffmpeg lacks", v.codec_name)));
                }
                let color = video_color(v, job.tools, &mut fidelity, "yuv420p", false);
                args.extend(os(&["-vf", &format!("{},{color}", fit_video())]));
                args.extend(os(&["-c:v", "libx264", "-profile:v", "high", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p"]));
                args.extend(os(&["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709"]));
                "transcode"
            };
            if let Some(a) = info.audio() {
                if plan == "remux" && MP4_AUDIO.contains(&a.codec_name.as_str()) {
                    args.extend(os(&["-c:a", "copy"]));
                } else {
                    args.extend(os(&["-c:a", "aac", "-b:a", "128k", "-ac", "2"]));
                }
            }
            args.extend(os(&["-movflags", "+faststart", "-max_muxing_queue_size", "1024", "-f", "mp4"]));
            args.push(out.clone().into());
            let rotated = (v.rotation / 90) % 2 != 0;
            let (w, h) = if rotated { (v.height, v.width) } else { (v.width, v.height) };
            let size = if plan == "remux" { (w, h) } else {
                let box_ = if w >= h { (VIDEO_LONG, VIDEO_SHORT) } else { (VIDEO_SHORT, VIDEO_LONG) };
                let scale = (box_.0 as f64 / w.max(1) as f64).min(box_.1 as f64 / h.max(1) as f64).min(1.0);
                (((w as f64 * scale) / 2.0).floor() as u32 * 2, ((h as f64 * scale) / 2.0).floor() as u32 * 2)
            };
            Ok((args, Output { path: out, media_type: "video/mp4", result_type: "video", plan, width: Some(size.0), height: Some(size.1), duration: info.duration, fidelity }))
        }
        (Kind::Audio, Profile::PreviewAudio) | (Kind::Video, Profile::PreviewAudio) => audio_plan(job, info),
        _ => Err(ConvertError::new("CONTENT_UNSUPPORTED", format!("no {} conversion for this content", job.profile.as_str()))),
    }
}

fn audio_plan(job: &Job, info: &MediaInfo) -> Result<(Vec<OsString>, Output), ConvertError> {
    let a = info.audio().ok_or_else(|| ConvertError::new("CONTENT_UNSUPPORTED", "the media has neither picture nor sound"))?;
    let out = job.dir.join("out.m4a");
    let mut args = os(&["-map", &format!("0:{}", a.index), "-vn", "-sn", "-dn", "-map_metadata", "-1"]);
    if a.codec_name == "aac" {
        args.extend(os(&["-c:a", "copy"]));
    } else {
        args.extend(os(&["-c:a", "aac", "-b:a", "128k", "-ac", "2"]));
    }
    args.extend(os(&["-movflags", "+faststart", "-f", "mp4"]));
    args.push(out.clone().into());
    Ok((args, Output { path: out, media_type: "audio/mp4", result_type: "audio", plan: "audio", width: None, height: None, duration: info.duration, fidelity: Vec::new() }))
}

/// D10: max(60 s, duration × 10).
pub fn timeout_for(duration: Option<f64>) -> Duration {
    Duration::from_secs_f64(duration.map(|d| d * 10.0).unwrap_or(0.0).max(60.0))
}

async fn drain<R: tokio::io::AsyncRead + Unpin>(mut r: R, keep: usize) -> String {
    let mut all = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                all.extend_from_slice(&buf[..n]);
                if all.len() > keep * 2 {
                    all.drain(..all.len() - keep);
                }
            }
        }
    }
    let start = all.len().saturating_sub(keep);
    String::from_utf8_lossy(&all[start..]).trim().to_string()
}

fn command(path: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(path);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    buckyos_api::hide_child_console_async(&mut cmd);
    cmd
}

/// `ffprobe` JSON of the input (stream groups when ffprobe ≥ 7.1 knows them).
pub async fn ffprobe(tools: &Toolchain, input: &Path) -> Result<MediaInfo, ConvertError> {
    let mut cmd = command(&tools.ffprobe.path);
    cmd.args(["-hide_banner", "-v", "error", "-print_format", "json", "-show_format", "-show_streams"]);
    if tools.heif {
        cmd.arg("-show_stream_groups");
    }
    cmd.arg("-i").arg(input);
    let out = tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| ConvertError { code: "CONVERT_TIMEOUT", message: "ffprobe timed out".into(), retryable: true })?
        .map_err(|e| ConvertError { code: "CONVERTER_UNAVAILABLE", message: format!("cannot run ffprobe: {e}"), retryable: true })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(ConvertError::new("CONTENT_CORRUPTED", format!("ffprobe cannot read the content: {}", err.trim())));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| ConvertError::new("CONTENT_CORRUPTED", format!("ffprobe output: {e}")))?;
    Ok(MediaInfo::parse(&json))
}

/// Runs the planned conversion; progress (0..1) follows `-progress` over the duration.
pub async fn run(job: &Job<'_>, info: &MediaInfo, args: Vec<OsString>, output: &Output) -> Result<(), ConvertError> {
    let mut cmd = command(&job.tools.ffmpeg.path);
    cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-y", "-progress", "pipe:1", "-nostats"]);
    if job.profile != Profile::Thumbnail || job.kind != Kind::Video {
        cmd.arg("-i").arg(job.input).args(&args);
    } else {
        // `-ss` before `-i` seeks the input.
        let (seek, rest) = args.split_at(2);
        cmd.args(seek).arg("-i").arg(job.input).args(rest);
    }
    log::debug!("preview ffmpeg {:?}", cmd.as_std());
    let mut child = cmd.spawn().map_err(|e| ConvertError { code: "CONVERTER_UNAVAILABLE", message: format!("cannot run ffmpeg: {e}"), retryable: true })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let duration = info.duration;
    let progress = job.progress.clone();
    let progress_task = tokio::spawn(async move {
        let Some(stdout) = stdout else { return };
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let (Some(d), Some(us)) = (duration, line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok())) {
                progress((us / 1_000_000.0 / d).clamp(0.0, 1.0));
            }
        }
    });
    let stderr_task = tokio::spawn(async move {
        match stderr {
            Some(s) => drain(s, 2048).await,
            None => String::new(),
        }
    });
    let limit = timeout_for(duration);
    let status = match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => status.map_err(|e| ConvertError { code: "CONVERT_FAILED", message: format!("ffmpeg: {e}"), retryable: true })?,
        Err(_) => {
            let _ = child.kill().await;
            progress_task.abort();
            return Err(ConvertError { code: "CONVERT_TIMEOUT", message: format!("the conversion took longer than {}s", limit.as_secs()), retryable: true });
        }
    };
    let _ = progress_task.await;
    let stderr = stderr_task.await.unwrap_or_default();
    if !status.success() {
        return Err(ConvertError::new("CONVERT_FAILED", format!("ffmpeg exited with {status}: {stderr}")));
    }
    match tokio::fs::metadata(&output.path).await {
        Ok(m) if m.len() > 0 => Ok(()),
        _ => Err(ConvertError::new("CONVERT_FAILED", format!("ffmpeg produced no output: {stderr}"))),
    }
}

/// `file://` URL of a local path (`-env:UserInstallation`), Windows drive paths included.
pub fn file_url(path: &Path) -> String {
    const PATH: &AsciiSet = &NON_ALPHANUMERIC.remove(b'/').remove(b':').remove(b'-').remove(b'_').remove(b'.').remove(b'~');
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text);
    let text = if text.starts_with('/') { text.to_string() } else { format!("/{text}") };
    format!("file://{}", utf8_percent_encode(&text, PATH))
}

/// soffice arguments: a private profile per attempt (a second instance on a
/// shared one exits silently and leaves a lock behind), the result next to the input.
pub fn document_args(profile: &Path, input: &Path, dir: &Path, to: &str) -> Vec<OsString> {
    let mut args = vec![OsString::from(format!("-env:UserInstallation={}", file_url(profile)))];
    args.extend(os(&["--headless", "--norestore", "--nologo", "--nolockcheck", "--nodefault", "--convert-to", to, "--outdir"]));
    args.push(dir.into());
    args.push(input.into());
    args
}

/// Q11: max(120 s, 10 s per MiB of input).
pub fn document_timeout(size: u64) -> Duration {
    Duration::from_secs(size.div_ceil(1024 * 1024).saturating_mul(10).max(120))
}

fn tail(b: &[u8]) -> String {
    String::from_utf8_lossy(&b[b.len().saturating_sub(2048)..]).trim().to_string()
}

fn timed_out(limit: Duration) -> ConvertError {
    ConvertError { code: "CONVERT_TIMEOUT", message: format!("the conversion took longer than {}s", limit.as_secs()), retryable: true }
}

/// Runs `cmd` to its end within `limit`; dropping the timed-out future kills it (`kill_on_drop`).
async fn output_within(mut cmd: tokio::process::Command, name: &str, limit: Duration) -> Result<std::process::Output, ConvertError> {
    log::debug!("preview {name} {:?}", cmd.as_std());
    tokio::time::timeout(limit, cmd.output())
        .await
        .map_err(|_| timed_out(limit))?
        .map_err(|e| ConvertError { code: "CONVERTER_UNAVAILABLE", message: format!("cannot run {name}: {e}"), retryable: true })
}

async fn non_empty(path: &Path) -> bool {
    matches!(tokio::fs::metadata(path).await, Ok(m) if m.len() > 0)
}

/// LibreOffice fetches what a document links to (measured: a linked picture in
/// a docx is requested over HTTP during a headless conversion). The private
/// profile turns that off ("block links from documents not among the trusted
/// locations").
const BLOCK_LINKS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="BlockUntrustedRefererLinks" oor:op="fuse"><value>true</value></prop></item>
</oor:items>
"#;

/// On Linux soffice also runs in a network namespace of its own when `unshare`
/// can make one (as root, or through a user namespace): the prefix to run it with.
async fn network_isolation() -> &'static [&'static str] {
    static PREFIX: tokio::sync::OnceCell<&'static [&'static str]> = tokio::sync::OnceCell::const_new();
    PREFIX
        .get_or_init(|| async {
            if !cfg!(target_os = "linux") {
                return &[][..];
            }
            const WAYS: [&[&str]; 2] = [&["unshare", "--net", "--"], &["unshare", "--user", "--map-root-user", "--net", "--"]];
            for way in WAYS {
                let mut cmd = tokio::process::Command::new(way[0]);
                cmd.args(&way[1..]).arg("true").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if matches!(cmd.status().await, Ok(s) if s.success()) {
                    log::info!("preview: soffice runs without network ({})", way.join(" "));
                    return way;
                }
            }
            log::warn!("preview: no network namespace for soffice (unshare is missing or not permitted); linked content is blocked by its profile only");
            &[][..]
        })
        .await
}

/// `preview-document` / `preview-sheet`: LibreOffice writes `<input stem>.pdf` (`.xlsx`) into `dir`.
pub async fn document(soffice: &Tool, input: &Path, dir: &Path, profile: &Path, sheet: bool) -> Result<Output, ConvertError> {
    let size = tokio::fs::metadata(input).await.map(|m| m.len()).unwrap_or(0);
    let to = if sheet { "xlsx" } else { "pdf" };
    let settings = profile.join("user");
    let prepared = async {
        tokio::fs::create_dir_all(&settings).await?;
        tokio::fs::write(settings.join("registrymodifications.xcu"), BLOCK_LINKS).await
    };
    prepared.await.map_err(|e| ConvertError { code: "CONVERT_FAILED", message: format!("soffice profile: {e}"), retryable: true })?;
    let mut cmd = match network_isolation().await {
        [] => command(&soffice.path),
        [unshare, rest @ ..] => {
            let mut cmd = command(Path::new(unshare));
            cmd.args(rest).arg(&soffice.path);
            cmd
        }
    };
    cmd.args(document_args(profile, input, dir, to));
    let out = output_within(cmd, "soffice", document_timeout(size)).await?;
    let path = dir.join(input.with_extension(to).file_name().unwrap_or_default());
    if !non_empty(&path).await {
        return Err(ConvertError::new("CONVERT_FAILED", format!("soffice exited with {} and produced no {to}: {} {}", out.status, tail(&out.stderr), tail(&out.stdout))));
    }
    Ok(Output {
        path,
        media_type: if sheet { XLSX } else { "application/pdf" },
        result_type: if sheet { "sheet" } else { "document" },
        plan: if sheet { "soffice-xlsx" } else { "soffice-pdf" },
        width: None,
        height: None,
        duration: None,
        fidelity: vec![if sheet { FIDELITY_SHEET } else { FIDELITY_DOCUMENT }],
    })
}

/// ImageMagick arguments: the coder is named (no format guessing, so no
/// delegate such as Ghostscript is reached), resources are capped on the
/// command line (the launcher fixes the policy path), only the first frame.
pub fn raster_args(coder: &str, input: &Path, output: &Path, max_edge: u32) -> Vec<OsString> {
    let mut args = os(&[
        "-limit", "memory", "1GiB", "-limit", "map", "2GiB", "-limit", "disk", "4GiB", "-limit", "width", "32KP", "-limit", "height", "32KP", "-limit", "area", "256MP",
        "-limit", "time", "120",
    ]);
    let mut source = OsString::from(format!("{coder}:"));
    source.push(input);
    source.push("[0]");
    args.push(source);
    args.extend(os(&["-auto-orient", "-background", "white", "-flatten", "-resize", &format!("{max_edge}x{max_edge}>"), "-colorspace", "sRGB", "-strip", "-quality", "85", "-print", "%w %h"]));
    let mut dest = OsString::from("jpg:");
    dest.push(output);
    args.push(dest);
    args
}

/// A still by ImageMagick: JPEG on white, long edge ≤ `max_edge`.
pub async fn raster(magick: &Tool, coder: &str, input: &Path, dir: &Path, max_edge: u32) -> Result<Output, ConvertError> {
    let path = dir.join("out.jpg");
    let mut cmd = command(&magick.path);
    cmd.args(raster_args(coder, input, &path, max_edge));
    let out = output_within(cmd, "magick", Duration::from_secs(150)).await?;
    if !out.status.success() || !non_empty(&path).await {
        return Err(ConvertError::new("CONVERT_FAILED", format!("magick exited with {}: {}", out.status, tail(&out.stderr))));
    }
    let printed = String::from_utf8_lossy(&out.stdout);
    let mut size = printed.split_whitespace().map(|v| v.parse::<u32>().ok());
    let (width, height) = (size.next().flatten(), size.next().flatten());
    Ok(Output { path, media_type: "image/jpeg", result_type: "image", plan: "magick-jpeg", width, height, duration: None, fidelity: Vec::new() })
}

/// Narrow-screen typography for pandoc's standalone page; wide tables and code scroll sideways.
const HTML_HEAD: &str = r#"<meta name="color-scheme" content="light dark">
<style>
html { -webkit-text-size-adjust: 100%; }
body { max-width: 46em; margin: 0 auto; padding: 16px; font: 16px/1.65 system-ui, -apple-system, "Segoe UI", "PingFang SC", "Noto Sans CJK SC", "Microsoft YaHei", sans-serif; overflow-wrap: break-word; }
img, svg, video { max-width: 100%; height: auto; }
pre { overflow-x: auto; padding: 12px; border-radius: 6px; background: rgba(127,127,127,.12); font-size: 13px; line-height: 1.5; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
table { display: block; max-width: 100%; overflow-x: auto; border-collapse: collapse; }
th, td { padding: 6px 10px; border: 1px solid rgba(127,127,127,.35); }
blockquote { margin: 1em 0; padding: 0 1em; border-left: 3px solid rgba(127,127,127,.4); }
math { overflow-x: auto; max-width: 100%; }
h1 { font-size: 1.6em; } h2 { font-size: 1.35em; } h3 { font-size: 1.15em; }
</style>
"#;

/// pandoc arguments: `--sandbox` allows no IO but the files named here; the
/// result embeds its images and styles.
pub fn markup_args(reader: &str, input: &Path, head: &Path, output: &Path, title: &str) -> Vec<OsString> {
    let mut args = os(&["+RTS", "-M1024m", "-RTS", "--sandbox", "-f", reader, "-t", "html5", "-s", "--embed-resources", "--math-method=mathml", "--metadata"]);
    args.push(format!("pagetitle={title}").into());
    args.push("-H".into());
    args.push(head.into());
    args.push("-o".into());
    args.push(output.into());
    args.push(input.into());
    args
}

/// `preview-html`: one self-contained HTML file.
pub async fn markup(pandoc: &Tool, reader: &str, input: &Path, dir: &Path, title: &str) -> Result<Output, ConvertError> {
    let head = dir.join("head.html");
    tokio::fs::write(&head, HTML_HEAD).await.map_err(|e| ConvertError { code: "CONVERT_FAILED", message: e.to_string(), retryable: true })?;
    let path = dir.join("out.html");
    let mut cmd = command(&pandoc.path);
    cmd.args(markup_args(reader, input, &head, &path, title));
    let out = output_within(cmd, "pandoc", Duration::from_secs(120)).await?;
    if !out.status.success() || !non_empty(&path).await {
        return Err(ConvertError::new("CONVERT_FAILED", format!("pandoc exited with {}: {}", out.status, tail(&out.stderr))));
    }
    let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    if size > HTML_MAX_BYTES {
        return Err(ConvertError::new("TOO_LARGE", format!("the page is {size} bytes, over the {HTML_MAX_BYTES}-byte limit of the HTML viewer")));
    }
    Ok(Output { path, media_type: "text/html; charset=utf-8", result_type: "html", plan: "pandoc-html", width: None, height: None, duration: None, fidelity: vec![FIDELITY_HTML] })
}

/// A PDF's first page as a JPEG, long edge `THUMBNAIL_MAX_EDGE`.
pub async fn pdf_thumbnail(tools: &PdfTools, input: &Path, dir: &Path) -> Result<Output, ConvertError> {
    let mut info = command(&tools.pdfinfo.path);
    info.arg(input);
    let info = output_within(info, "pdfinfo", Duration::from_secs(60)).await?;
    if !info.status.success() {
        let err = tail(&info.stderr);
        let code = if err.contains("Incorrect password") { NO_THUMBNAIL } else { "CONTENT_CORRUPTED" };
        return Err(ConvertError::new(code, if code == NO_THUMBNAIL { "the PDF is password protected".to_string() } else { format!("pdfinfo cannot read the PDF: {err}") }));
    }
    let mut cmd = command(&tools.pdftoppm.path);
    cmd.args(["-f", "1", "-l", "1", "-scale-to", &THUMBNAIL_MAX_EDGE.to_string(), "-jpeg", "-singlefile"]).arg(input).arg(dir.join("out"));
    let out = output_within(cmd, "pdftoppm", Duration::from_secs(60)).await?;
    let path = dir.join("out.jpg");
    if !out.status.success() || !non_empty(&path).await {
        return Err(ConvertError::new("CONVERT_FAILED", format!("pdftoppm exited with {}: {}", out.status, tail(&out.stderr))));
    }
    Ok(Output { path, media_type: "image/jpeg", result_type: "image", plan: "pdf-page", width: None, height: None, duration: None, fidelity: Vec::new() })
}

fn xml_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.match_indices(name).find(|(i, _)| tag[..*i].ends_with(char::is_whitespace) && tag[i + name.len()..].trim_start().starts_with('='))?.0;
    let rest = tag[at + name.len()..].trim_start().strip_prefix('=')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    rest[1..].split(quote).next()
}

fn xml_tags<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> {
    xml.split('<').filter(move |t| t.strip_prefix(name).is_some_and(|r| r.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>'))).map(|t| t.split('>').next().unwrap_or(t))
}

/// The cover image of an EPUB, as its zip entry name: `properties="cover-image"` (EPUB 3) or `<meta name="cover">` (EPUB 2).
pub fn epub_cover(container: &str, read_opf: impl FnOnce(&str) -> Option<String>) -> Option<String> {
    let opf_path = xml_tags(container, "rootfile").find_map(|t| xml_attr(t, "full-path"))?.to_string();
    let opf = read_opf(&opf_path)?;
    let strip = |t: &'_ str| t.rsplit_once(':').map(|(_, n)| n.to_string()).unwrap_or_else(|| t.to_string());
    let tags: Vec<&str> = opf.split('<').map(|t| t.split('>').next().unwrap_or(t)).collect();
    let named = |name: &str| tags.iter().copied().filter(|t| strip(t.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default()) == name).collect::<Vec<_>>();
    let items = named("item");
    let cover_id = named("meta").into_iter().find(|t| xml_attr(t, "name") == Some("cover")).and_then(|t| xml_attr(t, "content"));
    let item = items
        .iter()
        .find(|t| xml_attr(t, "properties").is_some_and(|p| p.split_whitespace().any(|w| w == "cover-image")))
        .or_else(|| items.iter().find(|t| cover_id.is_some() && xml_attr(t, "id") == cover_id))?;
    let href = percent_encoding::percent_decode_str(xml_attr(item, "href")?).decode_utf8_lossy().into_owned();
    let base = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or_default();
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in href.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

fn picture_ext(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        Some("png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("jpg")
    } else {
        None
    }
}

/// The picture a document carries for itself (blocking): ODF
/// `Thumbnails/thumbnail.png`, OOXML `docProps/thumbnail.jpeg`, iWork
/// `preview.jpg`, an EPUB's cover. Written to `dir/embedded.<ext>`.
pub fn embedded_thumbnail(input: &Path, dir: &Path) -> Result<PathBuf, ConvertError> {
    use std::io::Read;
    let none = || ConvertError::new(NO_THUMBNAIL, "the document carries no thumbnail");
    let file = std::fs::File::open(input).map_err(|e| ConvertError::new("CONTENT_UNAVAILABLE", e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| none())?;
    let text = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Option<String> {
        let mut out = String::new();
        zip.by_name(name).ok()?.take(4 * 1024 * 1024).read_to_string(&mut out).ok()?;
        Some(out)
    };
    const FIXED: &[&str] = &["Thumbnails/thumbnail.png", "docProps/thumbnail.jpeg", "docProps/thumbnail.png", "preview.jpg", "QuickLook/Thumbnail.jpg"];
    let names: Vec<String> = zip.file_names().map(String::from).collect();
    let cover = text(&mut zip, "META-INF/container.xml").and_then(|c| epub_cover(&c, |opf| text(&mut zip, opf)));
    let entry = FIXED.iter().map(|n| n.to_string()).chain(cover).find(|n| names.contains(n) && picture_ext(n).is_some()).ok_or_else(none)?;
    let ext = picture_ext(&entry).ok_or_else(none)?;
    let mut bytes = Vec::new();
    zip.by_name(&entry).map_err(|_| none())?.take(EMBEDDED_MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|e| ConvertError::new("CONTENT_CORRUPTED", e.to_string()))?;
    if bytes.is_empty() || bytes.len() as u64 > EMBEDDED_MAX_BYTES {
        return Err(none());
    }
    let path = dir.join(format!("embedded.{ext}"));
    std::fs::write(&path, bytes).map_err(|e| ConvertError { code: "CONVERT_FAILED", message: e.to_string(), retryable: true })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::heif::HeifInfo;
    use crate::preview::media::TileGrid;
    use crate::preview::tools::Tool;

    fn tools() -> Toolchain {
        let t = Tool::new("ffmpeg".into(), "7.1".into());
        Toolchain { ffmpeg: t.clone(), ffprobe: t, tonemap: true, libx264: true, heif: true }
    }

    #[test]
    fn heif_grid_portrait() {
        let info = MediaInfo {
            grids: vec![TileGrid { width: 1000, height: 700, h_offset: 0, v_offset: 0, tiles: vec![(0, 0, 0), (1, 512, 0), (2, 0, 512), (3, 512, 512)], default: true }],
            ..Default::default()
        };
        let heif = HeifInfo { width: 1000, height: 700, transforms: vec![Transform::Rotate(3)], grid: true, has_alpha: false, nclx: None };
        let mut fidelity = Vec::new();
        let graph = heif_graph(&info, &heif, &tools(), 4096, &mut fidelity).unwrap();
        assert!(graph.starts_with("[0:0][0:1][0:2][0:3]xstack=inputs=4:layout=0_0|512_0|0_512|512_512:fill=black,crop=1000:700:0:0[img];"), "{graph}");
        assert!(graph.contains("[img]transpose=1,scale="), "{graph}");
        assert!(graph.ends_with("format=yuvj420p[out]"));
        assert!(fidelity.is_empty());
    }

    #[test]
    fn heif_hdr_is_tonemapped() {
        let info = MediaInfo { streams: vec![Stream { index: 0, codec_type: "video".into(), width: 64, height: 64, default: true, ..Default::default() }], ..Default::default() };
        let heif = HeifInfo { width: 64, height: 64, nclx: Some(Nclx { primaries: 9, transfer: 16, matrix: 9, full_range: false }), ..Default::default() };
        let mut fidelity = Vec::new();
        let graph = heif_graph(&info, &heif, &tools(), 640, &mut fidelity).unwrap();
        assert!(graph.contains("zscale=tin=smpte2084:pin=2020:min=2020_ncl:rin=limited:t=linear"), "{graph}");
        assert_eq!(fidelity, vec![FIDELITY_HDR]);
    }

    #[test]
    fn remux_rules() {
        let v = |codec: &str, pix: &str| Stream { codec_name: codec.into(), pix_fmt: pix.into(), ..Default::default() };
        assert!(remuxable(&v("h264", "yuv420p"), false));
        assert!(!remuxable(&v("h264", "yuv422p10le"), true));
        assert!(!remuxable(&v("hevc", "yuv420p"), false));
        assert!(remuxable(&v("hevc", "yuv420p10le"), true));
        assert!(!remuxable(&v("prores", "yuv422p10le"), true));
    }

    #[test]
    fn timeouts() {
        assert_eq!(timeout_for(None).as_secs(), 60);
        assert_eq!(timeout_for(Some(30.0)).as_secs(), 300);
        assert_eq!(document_timeout(0).as_secs(), 120);
        assert_eq!(document_timeout(12 * 1024 * 1024).as_secs(), 120);
        assert_eq!(document_timeout(30 * 1024 * 1024 + 1).as_secs(), 310);
    }

    #[cfg(unix)]
    #[test]
    fn soffice_command() {
        let dir = Path::new("/data/preview-tmp/att_1");
        let args = document_args(Path::new("/data/preview-tmp/profile-att_1"), &dir.join("input.pptx"), dir, "pdf");
        let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            [
                "-env:UserInstallation=file:///data/preview-tmp/profile-att_1",
                "--headless",
                "--norestore",
                "--nologo",
                "--nolockcheck",
                "--nodefault",
                "--convert-to",
                "pdf",
                "--outdir",
                "/data/preview-tmp/att_1",
                "/data/preview-tmp/att_1/input.pptx",
            ]
        );
        assert_eq!(file_url(Path::new("/srv/数据 x/profile-1")), "file:///srv/%E6%95%B0%E6%8D%AE%20x/profile-1");
    }

    #[test]
    fn magick_names_the_coder() {
        let args: Vec<String> = raster_args("dng", Path::new("/t/input"), Path::new("/t/out.jpg"), 640).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.starts_with(&["-limit".into(), "memory".into(), "1GiB".into()]), "{args:?}");
        assert!(args.contains(&"dng:/t/input[0]".to_string()) && args.contains(&"640x640>".to_string()), "{args:?}");
        assert_eq!(args.last().unwrap(), "jpg:/t/out.jpg");
    }

    #[test]
    fn pandoc_is_sandboxed() {
        let args: Vec<String> = markup_args("epub", Path::new("/t/input"), Path::new("/t/head.html"), Path::new("/t/out.html"), "书 1.epub").iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--sandbox".to_string()) && args.contains(&"--embed-resources".to_string()), "{args:?}");
        assert!(args.contains(&"pagetitle=书 1.epub".to_string()), "{args:?}");
        assert_eq!(&args[args.len() - 3..], ["-o", "/t/out.html", "/t/input"]);
    }

    #[test]
    fn epub_covers() {
        let container = r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        let v3 = r#"<package><manifest><item id="nav" href="nav.xhtml" properties="nav"/><item href="images/cover%20art.jpg" id="c" media-type="image/jpeg" properties="cover-image"/></manifest></package>"#;
        assert_eq!(epub_cover(container, |p| (p == "OEBPS/content.opf").then(|| v3.to_string())).as_deref(), Some("OEBPS/images/cover art.jpg"));
        let v2 = r#"<opf:package><opf:metadata><opf:meta content='cov' name='cover'/></opf:metadata><opf:manifest><opf:item id="x" href="a.png"/><opf:item id="cov" href="../cover.png"/></opf:manifest></opf:package>"#;
        assert_eq!(epub_cover(container, |_| Some(v2.to_string())).as_deref(), Some("cover.png"));
        assert_eq!(epub_cover(container, |_| Some("<package><manifest><item id='a' href='a.xhtml'/></manifest></package>".into())), None);
        assert_eq!(epub_cover("<container/>", |_| panic!("no rootfile")), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_urls() {
        assert_eq!(file_url(Path::new(r"C:\buckyos\data\preview-tmp\profile-att_1")), "file:///C:/buckyos/data/preview-tmp/profile-att_1");
        assert_eq!(file_url(Path::new(r"\\?\D:\数据\p")), "file:///D:/%E6%95%B0%E6%8D%AE/p");
    }
}
