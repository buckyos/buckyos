//! The fixed Target Profiles (TODO D5) as ffmpeg invocations.
//!
//! - `preview-image`: JPEG q≈85, long edge ≤ 4096, BT.601 full range (what
//!   JPEG decoders assume), HEIF `irot`/`imir` baked into pixels, alpha
//!   flattened onto white.
//! - `preview-video`: H.264 High ≤ 1080p yuv420p + AAC 128k MP4 `+faststart`,
//!   rotation baked by ffmpeg's autorotate; a playable codec is only remuxed.
//! - `preview-audio`: AAC in MP4 for a container with no picture.
//! - `thumbnail`: JPEG, long edge ≤ 640 (a video's frame at min(1 s, 10 %)).
//!
//! HDR (PQ / HLG) is tone-mapped to SDR BT.709 and noted in the fidelity.

use super::heif::{HeifInfo, Nclx, Transform};
use super::media::{Kind, MediaInfo, Stream};
use super::tools::Toolchain;
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
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::PreviewImage => "preview-image",
            Profile::PreviewVideo => "preview-video",
            Profile::PreviewAudio => "preview-audio",
            Profile::Thumbnail => "thumbnail",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::heif::HeifInfo;
    use crate::preview::media::TileGrid;
    use crate::preview::tools::Tool;

    fn tools() -> Toolchain {
        let t = Tool { path: "ffmpeg".into(), version: "7.1".into() };
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
    }
}
