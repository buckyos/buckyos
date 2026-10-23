use crate::{
    validate_exact_model_name, validate_logical_model_name, AiTaskOptions, AiccError,
    AiccErrorCode, AiccExecutionMode, AiccRouteOverlay, HelperModelRequirement, ModelDisable,
    ResourceRef, RoutePolicy, TimeRange,
};
use ::kRPC::RPCErrors;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const VIDEO_MAX_CONDITIONS: usize = 32;
pub const VIDEO_CONDITION_ID_MAX_LEN: usize = 64;
pub const VIDEO_MAX_DURATION_SECONDS: f64 = 600.0;
pub const VIDEO_MAX_FPS: f64 = 240.0;
pub const VIDEO_MIN_EDGE: u32 = 16;
pub const VIDEO_MAX_EDGE: u32 = 8192;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KeyframePosition {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum VideoConditionKind {
    ReferenceImage,
    ReferenceVideo,
    ReferenceAudio,
    Keyframe,
}

impl VideoConditionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReferenceImage => "reference_image",
            Self::ReferenceVideo => "reference_video",
            Self::ReferenceAudio => "reference_audio",
            Self::Keyframe => "keyframe",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VideoCondition {
    ReferenceImage {
        id: String,
        resource: ResourceRef,
    },
    ReferenceVideo {
        id: String,
        resource: ResourceRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time_range: Option<TimeRange>,
        #[serde(default, skip_serializing_if = "is_false")]
        include_audio: bool,
    },
    ReferenceAudio {
        id: String,
        resource: ResourceRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time_range: Option<TimeRange>,
    },
    Keyframe {
        id: String,
        resource: ResourceRef,
        position: KeyframePosition,
    },
}

impl VideoCondition {
    pub fn id(&self) -> &str {
        match self {
            Self::ReferenceImage { id, .. }
            | Self::ReferenceVideo { id, .. }
            | Self::ReferenceAudio { id, .. }
            | Self::Keyframe { id, .. } => id,
        }
    }

    pub fn kind(&self) -> VideoConditionKind {
        match self {
            Self::ReferenceImage { .. } => VideoConditionKind::ReferenceImage,
            Self::ReferenceVideo { .. } => VideoConditionKind::ReferenceVideo,
            Self::ReferenceAudio { .. } => VideoConditionKind::ReferenceAudio,
            Self::Keyframe { .. } => VideoConditionKind::Keyframe,
        }
    }

    pub fn resource(&self) -> &ResourceRef {
        match self {
            Self::ReferenceImage { resource, .. }
            | Self::ReferenceVideo { resource, .. }
            | Self::ReferenceAudio { resource, .. }
            | Self::Keyframe { resource, .. } => resource,
        }
    }

    pub fn resource_mut(&mut self) -> &mut ResourceRef {
        match self {
            Self::ReferenceImage { resource, .. }
            | Self::ReferenceVideo { resource, .. }
            | Self::ReferenceAudio { resource, .. }
            | Self::Keyframe { resource, .. } => resource,
        }
    }

    pub fn time_range(&self) -> Option<&TimeRange> {
        match self {
            Self::ReferenceVideo { time_range, .. } | Self::ReferenceAudio { time_range, .. } => {
                time_range.as_ref()
            }
            _ => None,
        }
    }

    pub fn include_audio(&self) -> bool {
        matches!(
            self,
            Self::ReferenceVideo {
                include_audio: true,
                ..
            }
        )
    }

    pub fn keyframe_position(&self) -> Option<KeyframePosition> {
        match self {
            Self::Keyframe { position, .. } => Some(*position),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VideoShortEdge {
    pub short_edge: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VideoExactSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum VideoSize {
    ShortEdge(VideoShortEdge),
    Exact(VideoExactSize),
}

impl VideoSize {
    pub fn short_edge(short_edge: u32) -> Self {
        Self::ShortEdge(VideoShortEdge { short_edge })
    }

    pub fn exact(width: u32, height: u32) -> Self {
        Self::Exact(VideoExactSize { width, height })
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoAudioMode {
    #[default]
    Auto,
    Generate,
    None,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoOutputSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<VideoSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<VideoAudioMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl VideoOutputSpec {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn audio_mode(&self) -> VideoAudioMode {
        self.audio.unwrap_or_default()
    }

    pub fn validate(&self, pointer: &str) -> Result<(), AiccError> {
        if let Some(duration) = self.duration_seconds {
            if !duration.is_finite() || duration <= 0.0 || duration > VIDEO_MAX_DURATION_SECONDS {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "output duration_seconds must be a finite number in (0, 600]",
                    &format!("{pointer}/duration_seconds"),
                    None,
                    Some(json!({"min_exclusive": 0, "max": VIDEO_MAX_DURATION_SECONDS})),
                    Some(json!(duration)),
                ));
            }
        }
        let ratio = match &self.aspect_ratio {
            Some(value) => Some(parse_aspect_ratio(value).ok_or_else(|| {
                video_error(
                    AiccErrorCode::InvalidRequest,
                    "output aspect_ratio must be `W:H` with integers in 1..=100",
                    &format!("{pointer}/aspect_ratio"),
                    None,
                    None,
                    Some(json!(value)),
                )
            })?),
            None => None,
        };
        match self.size {
            Some(VideoSize::ShortEdge(VideoShortEdge { short_edge })) => {
                if !(VIDEO_MIN_EDGE..=VIDEO_MAX_EDGE).contains(&short_edge) {
                    return Err(edge_error(pointer, "short_edge", short_edge));
                }
            }
            Some(VideoSize::Exact(VideoExactSize { width, height })) => {
                if !(VIDEO_MIN_EDGE..=VIDEO_MAX_EDGE).contains(&width) {
                    return Err(edge_error(pointer, "width", width));
                }
                if !(VIDEO_MIN_EDGE..=VIDEO_MAX_EDGE).contains(&height) {
                    return Err(edge_error(pointer, "height", height));
                }
                if let Some((w, h)) = ratio {
                    if reduce_ratio(width, height) != reduce_ratio(w, h) {
                        return Err(video_error(
                            AiccErrorCode::InvalidRequest,
                            "output size conflicts with aspect_ratio",
                            &format!("{pointer}/size"),
                            None,
                            Some(json!(self.aspect_ratio)),
                            Some(json!(format!("{width}x{height}"))),
                        ));
                    }
                }
            }
            None => {}
        }
        if let Some(fps) = self.fps {
            if !fps.is_finite() || fps <= 0.0 || fps > VIDEO_MAX_FPS {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "output fps must be a finite number in (0, 240]",
                    &format!("{pointer}/fps"),
                    None,
                    None,
                    Some(json!(fps)),
                ));
            }
        }
        if let Some(media_type) = &self.media_type {
            if !valid_video_media_type(media_type) {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "output media_type must be a lowercase `video/*` MIME type",
                    &format!("{pointer}/media_type"),
                    None,
                    None,
                    Some(json!(media_type)),
                ));
            }
        }
        Ok(())
    }
}

fn edge_error(pointer: &str, field: &str, value: u32) -> AiccError {
    video_error(
        AiccErrorCode::InvalidRequest,
        format!("output size {field} must be within {VIDEO_MIN_EDGE}..={VIDEO_MAX_EDGE}"),
        &format!("{pointer}/size/{field}"),
        None,
        Some(json!({"min": VIDEO_MIN_EDGE, "max": VIDEO_MAX_EDGE})),
        Some(json!(value)),
    )
}

/// Parses `W:H` with both sides in `1..=100`.
pub fn parse_aspect_ratio(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once(':')?;
    let parse = |part: &str| {
        (!part.is_empty() && part.len() <= 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
            .filter(|value| (1..=100).contains(value))
    };
    Some((parse(width)?, parse(height)?))
}

pub fn reduce_ratio(width: u32, height: u32) -> (u32, u32) {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let divisor = gcd(width, height).max(1);
    (width / divisor, height / divisor)
}

fn valid_video_media_type(value: &str) -> bool {
    let Some(subtype) = value.strip_prefix("video/") else {
        return false;
    };
    !subtype.is_empty()
        && subtype.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-')
        })
}

pub fn validate_video_condition_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    id.len() <= VIDEO_CONDITION_ID_MAX_LEN
        && first.is_ascii_alphabetic()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoPromptSegment {
    Text(String),
    Reference(String),
}

/// Splits a prompt into literal text and `@{id}` references; `@@` is a
/// literal `@`, any other `@` stays as written.
pub fn parse_video_prompt(prompt: &str) -> Result<Vec<VideoPromptSegment>, AiccError> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut chars = prompt.char_indices().peekable();
    while let Some((offset, character)) = chars.next() {
        if character != '@' {
            text.push(character);
            continue;
        }
        match chars.peek().map(|(_, next)| *next) {
            Some('@') => {
                chars.next();
                text.push('@');
            }
            Some('{') => {
                chars.next();
                let mut id = String::new();
                let mut closed = false;
                for (_, next) in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    id.push(next);
                }
                if !closed {
                    return Err(video_error(
                        AiccErrorCode::InvalidRequest,
                        "prompt reference `@{` is not closed",
                        "/prompt",
                        None,
                        None,
                        Some(json!({"offset": offset})),
                    ));
                }
                if !validate_video_condition_id(&id) {
                    return Err(video_error(
                        AiccErrorCode::InvalidRequest,
                        "prompt reference contains an invalid condition id",
                        "/prompt",
                        None,
                        Some(json!(VIDEO_CONDITION_ID_PATTERN)),
                        Some(json!(id)),
                    ));
                }
                if !text.is_empty() {
                    segments.push(VideoPromptSegment::Text(std::mem::take(&mut text)));
                }
                segments.push(VideoPromptSegment::Reference(id));
            }
            _ => text.push('@'),
        }
    }
    if !text.is_empty() {
        segments.push(VideoPromptSegment::Text(text));
    }
    Ok(segments)
}

/// Renders parsed segments, mapping each reference through `token`.
pub fn render_video_prompt(
    segments: &[VideoPromptSegment],
    mut token: impl FnMut(&str) -> String,
) -> String {
    let mut output = String::new();
    for segment in segments {
        match segment {
            VideoPromptSegment::Text(text) => output.push_str(text),
            VideoPromptSegment::Reference(id) => output.push_str(&token(id)),
        }
    }
    output
}

pub const VIDEO_CONDITION_ID_PATTERN: &str = "^[A-Za-z][A-Za-z0-9_-]{0,63}$";

pub fn video_error(
    code: AiccErrorCode,
    message: impl Into<String>,
    field: &str,
    condition_id: Option<&str>,
    expected: Option<Value>,
    actual: Option<Value>,
) -> AiccError {
    let mut details = Map::from_iter([("field".to_string(), json!(field))]);
    if let Some(condition_id) = condition_id {
        details.insert("condition_id".into(), json!(condition_id));
    }
    if let Some(expected) = expected {
        details.insert("expected".into(), expected);
    }
    if let Some(actual) = actual {
        details.insert("actual".into(), actual);
    }
    AiccError {
        details: Some(Value::Object(details)),
        ..AiccError::new(code, message)
    }
}

fn validate_generation(
    prompt: Option<&str>,
    negative_prompt: Option<&str>,
    conditions: &[VideoCondition],
    output: Option<&VideoOutputSpec>,
) -> Result<(), AiccError> {
    let prompt_present = prompt.is_some_and(|prompt| !prompt.trim().is_empty());
    if !prompt_present && conditions.is_empty() {
        return Err(video_error(
            AiccErrorCode::InvalidRequest,
            "video generation requires a non-blank prompt or at least one condition",
            "/prompt",
            None,
            None,
            None,
        ));
    }
    if negative_prompt.is_some_and(|value| value.trim().is_empty()) {
        return Err(video_error(
            AiccErrorCode::InvalidRequest,
            "negative_prompt must not be blank when present",
            "/negative_prompt",
            None,
            None,
            None,
        ));
    }
    if conditions.len() > VIDEO_MAX_CONDITIONS {
        return Err(video_error(
            AiccErrorCode::InvalidRequest,
            format!("video generation accepts at most {VIDEO_MAX_CONDITIONS} conditions"),
            "/conditions",
            None,
            Some(json!(VIDEO_MAX_CONDITIONS)),
            Some(json!(conditions.len())),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut positions = BTreeSet::new();
    for (index, condition) in conditions.iter().enumerate() {
        let pointer = format!("/conditions/{index}");
        let id = condition.id();
        if !validate_video_condition_id(id) {
            return Err(video_error(
                AiccErrorCode::InvalidRequest,
                "condition id is invalid",
                &format!("{pointer}/id"),
                Some(id),
                Some(json!(VIDEO_CONDITION_ID_PATTERN)),
                Some(json!(id)),
            ));
        }
        if !ids.insert(id.to_ascii_lowercase()) {
            return Err(video_error(
                AiccErrorCode::InvalidRequest,
                "condition ids must be unique (compared case-insensitively)",
                &format!("{pointer}/id"),
                Some(id),
                None,
                Some(json!(id)),
            ));
        }
        if let Some(position) = condition.keyframe_position() {
            if !positions.insert(position) {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "at most one keyframe is allowed per position",
                    &format!("{pointer}/position"),
                    Some(id),
                    None,
                    Some(json!(position)),
                ));
            }
        }
        if let Some(range) = condition.time_range() {
            if !range.start_seconds.is_finite()
                || !range.end_seconds.is_finite()
                || range.start_seconds < 0.0
                || range.start_seconds >= range.end_seconds
            {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "time_range must be finite, non-negative and start before it ends",
                    &format!("{pointer}/time_range"),
                    Some(id),
                    None,
                    Some(json!(range)),
                ));
            }
        }
    }
    if let Some(prompt) = prompt {
        for segment in parse_video_prompt(prompt)? {
            if let VideoPromptSegment::Reference(id) = segment {
                if !conditions.iter().any(|condition| condition.id() == id) {
                    return Err(video_error(
                        AiccErrorCode::InvalidRequest,
                        "prompt references an unknown condition id",
                        "/prompt",
                        Some(&id),
                        None,
                        Some(json!(id)),
                    ));
                }
            }
        }
    }
    if let Some(output) = output {
        output.validate("/output")?;
    }
    Ok(())
}

fn generation_requirement(
    prompt: Option<&str>,
    negative_prompt: Option<&str>,
    seed: Option<u64>,
    conditions: &[VideoCondition],
    output: Option<&VideoOutputSpec>,
) -> VideoGenerationRequirement {
    let count = |kind: VideoConditionKind| {
        conditions
            .iter()
            .filter(|condition| condition.kind() == kind)
            .count() as u32
    };
    VideoGenerationRequirement {
        prompt: prompt.is_some_and(|prompt| !prompt.trim().is_empty()),
        negative_prompt: negative_prompt.is_some(),
        seed: seed.is_some(),
        reference_images: count(VideoConditionKind::ReferenceImage),
        reference_videos: count(VideoConditionKind::ReferenceVideo),
        reference_audios: count(VideoConditionKind::ReferenceAudio),
        keyframes: conditions
            .iter()
            .filter_map(VideoCondition::keyframe_position)
            .collect(),
        reference_video_audio: conditions.iter().any(VideoCondition::include_audio),
        time_ranges: conditions
            .iter()
            .any(|condition| condition.time_range().is_some()),
        output: output.filter(|output| !output.is_empty()).cloned(),
    }
}

/// Caller-visible video demands used by `route.resolve`, helpers and routing.
/// Server-measured facts (clip durations, audio tracks) are not part of it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoGenerationRequirement {
    #[serde(default, skip_serializing_if = "is_false")]
    pub prompt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub negative_prompt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub seed: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reference_images: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reference_videos: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reference_audios: u32,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub keyframes: BTreeSet<KeyframePosition>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reference_video_audio: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub time_ranges: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<VideoOutputSpec>,
}

impl VideoGenerationRequirement {
    pub fn reference_count(&self) -> u32 {
        self.reference_images + self.reference_videos + self.reference_audios
    }

    /// Adds `declared` (what a caller asked for) to the requirement derived
    /// from the request; a declaration can tighten, never relax.
    pub fn merge_declared(&mut self, declared: &Self) -> Result<(), AiccError> {
        self.prompt |= declared.prompt;
        self.negative_prompt |= declared.negative_prompt;
        self.seed |= declared.seed;
        self.reference_images = self.reference_images.max(declared.reference_images);
        self.reference_videos = self.reference_videos.max(declared.reference_videos);
        self.reference_audios = self.reference_audios.max(declared.reference_audios);
        self.keyframes.extend(declared.keyframes.iter().copied());
        self.reference_video_audio |= declared.reference_video_audio;
        self.time_ranges |= declared.time_ranges;
        match (&self.output, &declared.output) {
            (Some(own), Some(declared)) if own != declared => {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "requirements.video.output conflicts with the request output",
                    "/requirements/video/output",
                    None,
                    Some(json!(own)),
                    Some(json!(declared)),
                ));
            }
            (None, Some(declared)) => {
                declared.validate("/requirements/video/output")?;
                self.output = Some(declared.clone());
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoGenerateRequest {
    pub exact_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub execution_mode: AiccExecutionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<VideoCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<VideoOutputSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl VideoGenerateRequest {
    pub fn new(exact_model: impl Into<String>, prompt: Option<String>) -> Self {
        Self {
            exact_model: exact_model.into(),
            trace_id: None,
            execution_mode: AiccExecutionMode::Immediate,
            prompt,
            negative_prompt: None,
            conditions: Vec::new(),
            output: None,
            seed: None,
            idempotency_key: None,
            task_options: None,
            session_id: None,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse VideoGenerateRequest: {error}"))
        })?;
        request.validate().map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), AiccError> {
        validate_exact_model_name(&self.exact_model)?;
        validate_generation(
            self.prompt.as_deref(),
            self.negative_prompt.as_deref(),
            &self.conditions,
            self.output.as_ref(),
        )
    }

    pub fn requirement(&self) -> VideoGenerationRequirement {
        generation_requirement(
            self.prompt.as_deref(),
            self.negative_prompt.as_deref(),
            self.seed,
            &self.conditions,
            self.output.as_ref(),
        )
    }

    pub fn condition(&self, id: &str) -> Option<&VideoCondition> {
        self.conditions
            .iter()
            .find(|condition| condition.id() == id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoGenerateHelperRequest {
    pub logical_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub execution_mode: AiccExecutionMode,
    #[serde(default)]
    pub requirements: HelperModelRequirement,
    #[serde(default, skip_serializing_if = "is_default_disable")]
    pub disable: ModelDisable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<VideoCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<VideoOutputSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_overlay: Option<AiccRouteOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl VideoGenerateHelperRequest {
    pub fn new(logical_model: impl Into<String>, prompt: Option<String>) -> Self {
        Self {
            logical_model: logical_model.into(),
            trace_id: None,
            execution_mode: AiccExecutionMode::Immediate,
            requirements: HelperModelRequirement::default(),
            disable: ModelDisable::default(),
            policy: None,
            prompt,
            negative_prompt: None,
            conditions: Vec::new(),
            output: None,
            seed: None,
            idempotency_key: None,
            task_options: None,
            session_overlay: None,
            session_id: None,
        }
    }

    pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
        let request: Self = serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse VideoGenerateHelperRequest: {error}"
            ))
        })?;
        request.validate().map_err(|error| error.to_krpc_error())?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), AiccError> {
        validate_logical_model_name(&self.logical_model)?;
        validate_generation(
            self.prompt.as_deref(),
            self.negative_prompt.as_deref(),
            &self.conditions,
            self.output.as_ref(),
        )?;
        self.requirement().map(|_| ())
    }

    /// The request-derived requirement merged with `requirements.video`.
    pub fn requirement(&self) -> Result<VideoGenerationRequirement, AiccError> {
        let mut requirement = generation_requirement(
            self.prompt.as_deref(),
            self.negative_prompt.as_deref(),
            self.seed,
            &self.conditions,
            self.output.as_ref(),
        );
        if let Some(declared) = &self.requirements.video {
            requirement.merge_declared(declared)?;
        }
        Ok(requirement)
    }

    /// The exact-model request this helper call executes as.
    pub fn to_exact(&self, exact_model: impl Into<String>) -> VideoGenerateRequest {
        VideoGenerateRequest {
            exact_model: exact_model.into(),
            trace_id: self.trace_id.clone(),
            execution_mode: self.execution_mode,
            prompt: self.prompt.clone(),
            negative_prompt: self.negative_prompt.clone(),
            conditions: self.conditions.clone(),
            output: self.output.clone(),
            seed: self.seed,
            idempotency_key: self.idempotency_key.clone(),
            task_options: self.task_options.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoMetadataSource {
    Provider,
    Measured,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VideoDeviation {
    pub field: String,
    pub requested: Value,
    pub actual: Value,
    pub source: VideoMetadataSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GeneratedVideo {
    pub resource: ResourceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default)]
    pub provider_metadata: VideoMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_metadata: Option<VideoMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<ResourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deviations: Vec<VideoDeviation>,
}

impl GeneratedVideo {
    pub fn new(resource: ResourceRef, media_type: Option<String>) -> Self {
        Self {
            resource,
            media_type,
            provider_metadata: VideoMetadata::default(),
            measured_metadata: None,
            last_frame: None,
            deviations: Vec::new(),
        }
    }

    /// Records where the result differs from explicit request values.
    pub fn record_deviations(&mut self, requested: Option<&VideoOutputSpec>) {
        self.deviations.clear();
        let Some(requested) = requested else {
            return;
        };
        let sources = [
            (
                VideoMetadataSource::Measured,
                self.measured_metadata.clone(),
            ),
            (
                VideoMetadataSource::Provider,
                Some(self.provider_metadata.clone()),
            ),
        ];
        let mut seen = BTreeSet::new();
        for (source, metadata) in sources {
            let Some(metadata) = metadata else {
                continue;
            };
            let mut push = |field: &str, requested: Value, actual: Value| {
                if seen.insert(field.to_string()) {
                    self.deviations.push(VideoDeviation {
                        field: field.into(),
                        requested,
                        actual,
                        source,
                    });
                }
            };
            if let (Some(wanted), Some(actual)) =
                (requested.duration_seconds, metadata.duration_seconds)
            {
                if (wanted - actual).abs() > 0.5 {
                    push("duration_seconds", json!(wanted), json!(actual));
                }
            }
            if let (Some(width), Some(height)) = (metadata.width, metadata.height) {
                match requested.size {
                    Some(VideoSize::Exact(size))
                        if (size.width, size.height) != (width, height) =>
                    {
                        push(
                            "size",
                            json!(size),
                            json!({"width": width, "height": height}),
                        )
                    }
                    Some(VideoSize::ShortEdge(edge)) if width.min(height) != edge.short_edge => {
                        push(
                            "size",
                            json!(edge),
                            json!({"width": width, "height": height}),
                        )
                    }
                    _ => {}
                }
                if let Some((w, h)) = requested
                    .aspect_ratio
                    .as_deref()
                    .and_then(parse_aspect_ratio)
                {
                    let wanted = w as f64 / h as f64;
                    let actual = width as f64 / height as f64;
                    // Encoders align sizes to 16/32/64 pixels, so 832x480 is
                    // still the 16:9 a provider was asked for.
                    if (wanted - actual).abs() / wanted > 0.03 {
                        push(
                            "aspect_ratio",
                            json!(requested.aspect_ratio),
                            json!(format!("{width}:{height}")),
                        );
                    }
                }
            }
            if let (Some(wanted), Some(actual)) = (requested.fps, metadata.fps) {
                if (wanted - actual).abs() > 0.01 {
                    push("fps", json!(wanted), json!(actual));
                }
            }
            if let Some(actual) = metadata.has_audio {
                match requested.audio_mode() {
                    VideoAudioMode::Generate if !actual => {
                        push("audio", json!("generate"), json!(false))
                    }
                    VideoAudioMode::None if actual => push("audio", json!("none"), json!(true)),
                    _ => {}
                }
            }
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn is_default_disable(value: &ModelDisable) -> bool {
    value == &ModelDisable::default()
}
