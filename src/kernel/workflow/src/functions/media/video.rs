use super::{function, resource_limit, tool_output_error, tools, EXTRACT_AUDIO, EXTRACT_FRAMES, STDOUT_LIMIT};
use crate::native::{param_str, param_u64, ExecutionLane, NativeContext, NativeFunction, NativeOutcome};
use async_trait::async_trait;
use buckyos_api::{error_codes, FunctionObject, ThunkExecutionError};
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoInfo {
    pub format_name: String,
    pub duration_ms: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

impl VideoInfo {
    pub fn mime(&self) -> &'static str {
        let names: Vec<&str> = self.format_name.split(',').collect();
        if names.contains(&"mp4") || names.contains(&"mov") {
            "video/mp4"
        } else if names.contains(&"matroska") {
            "video/x-matroska"
        } else if names.contains(&"webm") {
            "video/webm"
        } else if names.contains(&"avi") {
            "video/x-msvideo"
        } else if names.contains(&"mpegts") {
            "video/mp2t"
        } else {
            "application/octet-stream"
        }
    }
}

pub fn parse_ffprobe(text: &str) -> Result<VideoInfo, ThunkExecutionError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|err| ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("ffprobe output is not JSON: {err}")).with_details(json!({"tool": "ffprobe"})))?;
    let mut info = VideoInfo {
        format_name: value.pointer("/format/format_name").and_then(Value::as_str).unwrap_or("").to_string(),
        ..Default::default()
    };
    let seconds = value
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    info.duration_ms = (seconds * 1000.0).round().max(0.0) as u64;
    for stream in value.get("streams").and_then(Value::as_array).into_iter().flatten() {
        let codec_type = stream.get("codec_type").and_then(Value::as_str).unwrap_or("");
        let codec = stream.get("codec_name").and_then(Value::as_str).map(str::to_string);
        match codec_type {
            "video" if info.video_codec.is_none() => {
                info.video_codec = codec;
                info.width = stream.get("width").and_then(Value::as_u64).and_then(|w| u32::try_from(w).ok());
                info.height = stream.get("height").and_then(Value::as_u64).and_then(|h| u32::try_from(h).ok());
                if info.duration_ms == 0 {
                    if let Some(d) = stream.get("duration").and_then(Value::as_str).and_then(|s| s.parse::<f64>().ok()) {
                        info.duration_ms = (d * 1000.0).round() as u64;
                    }
                }
            }
            "audio" if info.audio_codec.is_none() => info.audio_codec = codec,
            _ => {}
        }
    }
    Ok(info)
}

pub async fn ffprobe(ctx: &NativeContext, path: &Path) -> Result<VideoInfo, ThunkExecutionError> {
    let ffprobe = ctx.tool_path(tools::FFPROBE)?;
    let args = vec![
        "-v".to_string(),
        "error".to_string(),
        "-print_format".to_string(),
        "json".to_string(),
        "-show_streams".to_string(),
        "-show_format".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    let out = ctx.run_tool(&ffprobe, &args, STDOUT_LIMIT).await?;
    if !out.success {
        return Err(tool_output_error(&out, "probe failed"));
    }
    parse_ffprobe(&out.stdout_string())
}

/// Uniform sample times: the midpoint of each of `n` equal slices, so the
/// exact end of the stream is never requested.
pub fn uniform_timestamps(duration_ms: u64, n: u32) -> Vec<u64> {
    if n == 0 || duration_ms == 0 {
        return Vec::new();
    }
    let n = u64::from(n);
    (0..n).map(|i| (duration_ms * (2 * i + 1)) / (2 * n)).collect()
}

pub fn extract_frames_function_object() -> FunctionObject {
    function(
        EXTRACT_FRAMES,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "strategy", "max_frames", "frame_max_edge", "max_duration_ms"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "strategy": {"type": "string", "enum": ["uniform"]},
                "max_frames": {"type": "integer", "minimum": 1},
                "frame_max_edge": {"type": "integer", "minimum": 1},
                "max_duration_ms": {"type": "integer", "minimum": 1}
            }
        }),
        "media.frames",
        json!({
            "type": "object",
            "required": ["frames", "duration_ms"],
            "properties": {
                "frames": {"type": "array", "items": {
                    "type": "object",
                    "required": ["index", "time_ms", "status"],
                    "properties": {
                        "index": {"type": "integer"},
                        "time_ms": {"type": "integer"},
                        "image_obj_id": {"type": "string"},
                        "width": {"type": "integer"},
                        "height": {"type": "integer"},
                        "status": {"type": "string", "enum": ["ok", "failed"]},
                        "error": {"type": "object"}
                    }
                }},
                "duration_ms": {"type": "integer"}
            }
        }),
    )
}

pub struct ExtractFrames;

impl ExtractFrames {
    pub const TOOLS: &'static [&'static str] = &[tools::FFPROBE, tools::FFMPEG];
}

fn scale_filter(max_edge: u32) -> String {
    format!("scale=trunc(min(1\\,{max_edge}/max(iw\\,ih))*iw/2)*2:trunc(min(1\\,{max_edge}/max(iw\\,ih))*ih/2)*2")
}

#[async_trait]
impl NativeFunction for ExtractFrames {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Video
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let max_frames = u32::try_from(param_u64(params, "max_frames")?).unwrap_or(u32::MAX).min(ctx.limits.max_frames);
        let frame_max_edge = u32::try_from(param_u64(params, "frame_max_edge")?).unwrap_or(u32::MAX);
        let max_duration_ms = param_u64(params, "max_duration_ms")?;
        let (path, _) = ctx.materialize_input(content_id, "input.video").await?;
        let info = ffprobe(ctx, &path).await?;
        if info.width.is_none() {
            return Err(ThunkExecutionError::new(error_codes::TOOL_FAILED, "input has no video stream").with_details(json!({"tool": "ffprobe"})));
        }
        if info.duration_ms > max_duration_ms {
            return Err(resource_limit(
                format!("video duration {} ms exceeds the limit of {max_duration_ms} ms", info.duration_ms),
                json!({"duration_ms": info.duration_ms, "max_duration_ms": max_duration_ms}),
            ));
        }
        let ffmpeg = ctx.tool_path(tools::FFMPEG)?;
        let filter = scale_filter(frame_max_edge);
        let mut frames = Vec::new();
        let mut cacheable = true;
        for (index, time_ms) in uniform_timestamps(info.duration_ms, max_frames).into_iter().enumerate() {
            ctx.check_cancelled()?;
            let output = ctx.temp_dir.join(format!("frame-{index}.jpg"));
            let args = vec![
                "-v".to_string(),
                "error".to_string(),
                "-y".to_string(),
                "-ss".to_string(),
                format!("{:.3}", time_ms as f64 / 1000.0),
                "-i".to_string(),
                path.to_string_lossy().into_owned(),
                "-frames:v".to_string(),
                "1".to_string(),
                "-vf".to_string(),
                filter.clone(),
                "-q:v".to_string(),
                "3".to_string(),
                output.to_string_lossy().into_owned(),
            ];
            let out = ctx.run_tool(&ffmpeg, &args, 64 * 1024).await?;
            let result = if out.success && output.is_file() {
                let dims = image::ImageReader::open(&output)
                    .and_then(|r| r.into_dimensions().map_err(std::io::Error::other))
                    .map_err(|err| ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("read frame {index}: {err}")).retryable());
                match dims {
                    Ok((w, h)) => {
                        let name = format!("frame-{index}.jpg");
                        ctx.io
                            .put_artifact_file(&ctx.artifact_key(&name), &name, "image/jpeg", &output)
                            .await
                            .map(|artifact| (artifact.obj_id, w, h))
                    }
                    Err(err) => Err(err),
                }
            } else {
                Err(tool_output_error(&out, &format!("frame {index} at {time_ms} ms failed")).retryable())
            };
            let _ = tokio::fs::remove_file(&output).await;
            match result {
                Ok((id, w, h)) => frames.push(json!({"index": index, "time_ms": time_ms, "image_obj_id": id, "width": w, "height": h, "status": "ok"})),
                Err(err) if matches!(err.code.as_str(), error_codes::CANCELLED | error_codes::TIMEOUT | error_codes::INTERNAL) => return Err(err),
                Err(err) => {
                    if err.retryable {
                        cacheable = false;
                    }
                    frames.push(json!({"index": index, "time_ms": time_ms, "status": "failed", "error": {"code": err.code, "message": err.message, "retryable": err.retryable}}));
                }
            }
        }
        let refs = frames
            .iter()
            .filter_map(|f| f.get("image_obj_id").and_then(Value::as_str).map(str::to_string))
            .collect();
        let outcome = NativeOutcome::new(json!({"frames": frames, "duration_ms": info.duration_ms})).with_artifacts(refs);
        Ok(if cacheable { outcome } else { outcome.not_cacheable() })
    }
}

pub fn extract_audio_function_object() -> FunctionObject {
    function(
        EXTRACT_AUDIO,
        true,
        &["content_id"],
        json!({
            "type": "object",
            "required": ["content_id", "max_duration_ms"],
            "additionalProperties": false,
            "properties": {
                "content_id": {"type": "string"},
                "max_duration_ms": {"type": "integer", "minimum": 1}
            }
        }),
        "media.audio_track",
        json!({
            "type": "object",
            "required": ["status"],
            "properties": {
                "status": {"type": "string", "enum": ["ok", "absent"]},
                "audio_obj_id": {"type": "string"},
                "codec": {"type": "string"},
                "duration_ms": {"type": "integer"}
            }
        }),
    )
}

pub struct ExtractAudio;

impl ExtractAudio {
    pub const TOOLS: &'static [&'static str] = &[tools::FFPROBE, tools::FFMPEG];
}

#[async_trait]
impl NativeFunction for ExtractAudio {
    fn tools(&self) -> &'static [&'static str] {
        Self::TOOLS
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Video
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        ctx.require_tools(Self::TOOLS)?;
        let content_id = param_str(params, "content_id")?;
        let max_duration_ms = param_u64(params, "max_duration_ms")?;
        let (path, _) = ctx.materialize_input(content_id, "input.video").await?;
        let info = ffprobe(ctx, &path).await?;
        let Some(codec) = info.audio_codec.clone() else {
            return Ok(NativeOutcome::new(json!({"status": "absent", "duration_ms": info.duration_ms})));
        };
        if info.duration_ms > max_duration_ms {
            return Err(resource_limit(
                format!("audio duration {} ms exceeds the limit of {max_duration_ms} ms", info.duration_ms),
                json!({"duration_ms": info.duration_ms, "max_duration_ms": max_duration_ms}),
            ));
        }
        let ffmpeg = ctx.tool_path(tools::FFMPEG)?;
        let output = ctx.temp_dir.join("audio.mp3");
        let args = vec![
            "-v".to_string(),
            "error".to_string(),
            "-y".to_string(),
            "-i".to_string(),
            path.to_string_lossy().into_owned(),
            "-vn".to_string(),
            "-acodec".to_string(),
            "libmp3lame".to_string(),
            "-q:a".to_string(),
            "5".to_string(),
            output.to_string_lossy().into_owned(),
        ];
        ctx.run_tool(&ffmpeg, &args, 64 * 1024).await?.ok()?;
        let artifact = ctx
            .io
            .put_artifact_file(&ctx.artifact_key("audio.mp3"), "audio.mp3", "audio/mpeg", &output)
            .await?;
        Ok(NativeOutcome::new(json!({
            "status": "ok",
            "audio_obj_id": artifact.obj_id,
            "codec": codec,
            "duration_ms": info.duration_ms,
        }))
        .with_artifacts(vec![artifact.obj_id]))
    }
}
