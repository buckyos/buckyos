//! Video generation contracts and plans.
//!
//! A model or channel declares what it can generate as a
//! [`VideoGenerationContract`] (`capabilities.video_generation`). Routing
//! checks a request's [`VideoGenerationRequirement`] against it, and lowering
//! turns the request plus the measured material facts into a [`VideoPlan`]:
//! the only thing a video codec reads to decide what it submits.

use crate::media::MediaFacts;
use buckyos_api::{
    parse_aspect_ratio, parse_video_prompt, reduce_ratio, render_video_prompt, video_error,
    AiccError, AiccErrorCode, KeyframePosition, VideoAudioMode, VideoCondition,
    VideoConditionKind, VideoExactSize, VideoGenerateRequest, VideoGenerationRequirement,
    VideoOutputSpec, VideoPromptSegment, VideoSize,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const VIDEO_CONTRACT_CAPABILITY: &str = "video_generation";
pub(crate) const VIDEO_PLAN_PARAMETER: &str = "video_plan";
/// Seconds a clip boundary may differ from the measured media and still count
/// as covering the whole clip.
const CLIP_TOLERANCE_SECONDS: f64 = 0.05;
const KEYFRAME_RATIO_TOLERANCE: f64 = 0.02;

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PromptTokenStyle {
    /// `reference image N`, `the first frame` (MiniMax H3 and OpenRouter).
    Minimax,
    /// `@图像N`, `首帧` (Volcengine Ark Seedance).
    Seedance,
    #[default]
    Generic,
}

impl PromptTokenStyle {
    fn token(self, input: &PlannedInput) -> String {
        let ordinal = input.ordinal;
        match (self, input.kind, input.position) {
            (Self::Seedance, VideoConditionKind::ReferenceImage, _) => format!("@图像{ordinal}"),
            (Self::Seedance, VideoConditionKind::ReferenceVideo, _) => format!("@视频{ordinal}"),
            (Self::Seedance, VideoConditionKind::ReferenceAudio, _) => format!("@音频{ordinal}"),
            (Self::Seedance, VideoConditionKind::Keyframe, Some(KeyframePosition::End)) => {
                "尾帧".into()
            }
            (Self::Seedance, VideoConditionKind::Keyframe, _) => "首帧".into(),
            (_, VideoConditionKind::Keyframe, Some(KeyframePosition::End)) => {
                "the last frame".into()
            }
            (_, VideoConditionKind::Keyframe, _) => "the first frame".into(),
            (Self::Minimax, kind, _) => format!("{} {ordinal}", kind_label(kind, true)),
            (Self::Generic, kind, _) => format!("{} {ordinal}", kind_label(kind, false)),
        }
    }
}

fn kind_label(kind: VideoConditionKind, reference: bool) -> &'static str {
    match (kind, reference) {
        (VideoConditionKind::ReferenceImage, true) => "reference image",
        (VideoConditionKind::ReferenceVideo, true) => "reference video",
        (VideoConditionKind::ReferenceAudio, true) => "reference audio",
        (VideoConditionKind::ReferenceImage, false) => "image",
        (VideoConditionKind::ReferenceVideo, false) => "video",
        (VideoConditionKind::ReferenceAudio, false) => "audio",
        (VideoConditionKind::Keyframe, _) => "frame",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VideoScenario {
    Text,
    Keyframes,
    References,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyframeContract {
    pub positions: BTreeSet<KeyframePosition>,
    /// Whether an end frame may be given without a start frame.
    #[serde(default)]
    pub end_only: bool,
    /// Every listed position must be given (dedicated first/last frame models).
    #[serde(default)]
    pub require_all: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CountLimit {
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClipLimit {
    pub max: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_seconds: Option<f64>,
    /// Video only: the model accepts a reference video whose audio it may use.
    #[serde(default)]
    pub include_audio: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReferenceContract {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<CountLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<ClipLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ClipLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total: Option<u32>,
    /// Audio references need at least one image or video reference.
    #[serde(default)]
    pub audio_requires_visual: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_edge: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edge: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_aspect: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_aspect: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DurationContract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub integer: bool,
}

impl DurationContract {
    fn allows(&self, seconds: f64) -> bool {
        if !self.values.is_empty() {
            return self
                .values
                .iter()
                .any(|value| (value - seconds).abs() < 1e-9);
        }
        (!self.integer || seconds.fract() == 0.0)
            && self.min.is_none_or(|min| seconds >= min)
            && self.max.is_none_or(|max| seconds <= max)
    }

    fn describe(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolutionTier {
    pub name: String,
    pub short_edge: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyframeAspectRatio {
    /// The output takes the keyframe image's ratio; a request ratio is only
    /// honored when it matches the measured keyframe.
    Input,
    #[default]
    Selectable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AudioOutput {
    Always,
    Never,
    Optional,
    /// Not documented: only `auto` is accepted.
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputRuleWhen {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<VideoConditionKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<VideoScenario>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputRule {
    pub when: OutputRuleWhen,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durations: Option<DurationContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratios: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputContract {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durations: Option<DurationContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aspect_ratios: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_aspect_ratio: Option<String>,
    #[serde(default)]
    pub keyframe_aspect_ratio: KeyframeAspectRatio,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<ResolutionTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fps: Vec<f64>,
    pub audio: AudioOutput,
    pub media_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<OutputRule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VideoGenerationContract {
    #[serde(default = "default_true")]
    pub prompt_required: bool,
    #[serde(default)]
    pub prompt_tokens: PromptTokenStyle,
    #[serde(default)]
    pub negative_prompt: bool,
    #[serde(default)]
    pub seed: bool,
    #[serde(default = "default_true")]
    pub text_to_video: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyframes: Option<KeyframeContract>,
    #[serde(default)]
    pub references: ReferenceContract,
    #[serde(default)]
    pub keyframes_with_references: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_limits: Option<ImageLimits>,
    pub output: OutputContract,
}

pub(crate) fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once('x')?;
    let width = width.parse::<u32>().ok().filter(|value| *value > 0)?;
    let height = height.parse::<u32>().ok().filter(|value| *value > 0)?;
    Some((width, height))
}

impl VideoGenerationContract {
    pub(crate) fn from_capabilities(capabilities: &BTreeMap<String, Value>) -> Option<Self> {
        serde_json::from_value(capabilities.get(VIDEO_CONTRACT_CAPABILITY)?.clone()).ok()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let output = &self.output;
        let durations = |contract: &DurationContract| -> Result<(), String> {
            if contract.values.iter().any(|value| !value.is_finite() || *value <= 0.0)
                || contract.min.is_some_and(|min| !min.is_finite() || min <= 0.0)
                || contract.max.is_some_and(|max| !max.is_finite() || max <= 0.0)
                || matches!((contract.min, contract.max), (Some(min), Some(max)) if min > max)
            {
                return Err("durations must be finite positive values".into());
            }
            Ok(())
        };
        if let Some(contract) = &output.durations {
            durations(contract)?;
        }
        if let Some(default) = output.default_duration_seconds {
            if !output
                .durations
                .as_ref()
                .is_some_and(|contract| contract.allows(default))
            {
                return Err("default_duration_seconds is outside durations".into());
            }
        }
        for ratio in &output.aspect_ratios {
            parse_aspect_ratio(ratio).ok_or_else(|| format!("aspect ratio `{ratio}` is invalid"))?;
        }
        if let Some(default) = &output.default_aspect_ratio {
            if !output.aspect_ratios.contains(default) {
                return Err("default_aspect_ratio is not listed".into());
            }
        }
        let mut names = BTreeSet::new();
        for tier in &output.resolutions {
            if tier.name.trim().is_empty() || tier.short_edge == 0 || !names.insert(&tier.name) {
                return Err("resolution tiers need unique names and short edges".into());
            }
        }
        if let Some(default) = &output.default_resolution {
            if !names.contains(default) {
                return Err("default_resolution is not listed".into());
            }
        }
        for size in &output.sizes {
            parse_size(size).ok_or_else(|| format!("size `{size}` is invalid"))?;
        }
        if output.fps.iter().any(|fps| !fps.is_finite() || *fps <= 0.0) {
            return Err("fps values must be finite and positive".into());
        }
        if output.media_types.is_empty()
            || output
                .media_types
                .iter()
                .any(|media| !media.starts_with("video/"))
        {
            return Err("media_types must list at least one video MIME type".into());
        }
        for rule in &output.rules {
            if let Some(contract) = &rule.durations {
                durations(contract)?;
            }
            for name in rule
                .when
                .resolutions
                .iter()
                .chain(rule.resolutions.iter().flatten())
            {
                if !names.contains(name) {
                    return Err(format!("rule resolution `{name}` is not listed"));
                }
            }
        }
        if let Some(keyframes) = &self.keyframes {
            if keyframes.positions.is_empty() {
                return Err("keyframes.positions must not be empty".into());
            }
        }
        if !self.text_to_video
            && self.keyframes.is_none()
            && self.references.image.is_none()
            && self.references.video.is_none()
            && self.references.audio.is_none()
        {
            return Err("contract allows no generation scenario".into());
        }
        Ok(())
    }

    fn resolution_for_edge(&self, short_edge: u32) -> Option<&ResolutionTier> {
        self.output
            .resolutions
            .iter()
            .find(|tier| tier.short_edge == short_edge)
    }

    /// A short edge maps to a resolution tier, or else to the one declared
    /// exact size with that short edge and the requested (or default) ratio.
    fn size_for(&self, short_edge: u32, aspect_ratio: Option<&str>) -> Option<SizeChoice> {
        if let Some(tier) = self.resolution_for_edge(short_edge) {
            return Some(SizeChoice::Tier(tier.name.clone()));
        }
        let ratio = aspect_ratio
            .or(self.output.default_aspect_ratio.as_deref())
            .and_then(parse_aspect_ratio)
            .map(|(w, h)| reduce_ratio(w, h));
        let matches = self
            .output
            .sizes
            .iter()
            .filter_map(|size| parse_size(size))
            .filter(|(width, height)| {
                (*width).min(*height) == short_edge
                    && ratio.is_none_or(|ratio| reduce_ratio(*width, *height) == ratio)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [(width, height)] => Some(SizeChoice::Exact(VideoExactSize {
                width: *width,
                height: *height,
            })),
            _ => None,
        }
    }
}

enum SizeChoice {
    Tier(String),
    Exact(VideoExactSize),
}

/// Measured durations of the clips a request references, in seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ClipSummary {
    pub video_seconds: Vec<f64>,
    pub audio_seconds: Vec<f64>,
    /// Reference videos whose audio counts towards the audio budget.
    pub video_audio_seconds: Vec<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MaterialTransform {
    /// A provider URL was downloaded and is submitted as AICC's own copy, so
    /// what was measured is what the model receives.
    Copy,
    StripAudio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct DerivedMaterial {
    pub transform: MaterialTransform,
    pub obj_id: String,
    pub source_digest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConditionFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<DerivedMaterial>,
}

/// Facts per condition id, measured before the final route.
pub(crate) type VideoMaterialFacts = BTreeMap<String, ConditionFacts>;

pub(crate) fn clip_summary(
    request: &VideoGenerateRequest,
    facts: &VideoMaterialFacts,
) -> ClipSummary {
    let mut summary = ClipSummary::default();
    for condition in &request.conditions {
        let Some(seconds) = clip_seconds(condition, facts) else {
            continue;
        };
        match condition.kind() {
            VideoConditionKind::ReferenceVideo => {
                summary.video_seconds.push(seconds);
                if condition.include_audio() {
                    summary.video_audio_seconds.push(seconds);
                }
            }
            VideoConditionKind::ReferenceAudio => summary.audio_seconds.push(seconds),
            _ => {}
        }
    }
    summary
}

fn clip_seconds(condition: &VideoCondition, facts: &VideoMaterialFacts) -> Option<f64> {
    if let Some(range) = condition.time_range() {
        return Some(range.end_seconds - range.start_seconds);
    }
    facts
        .get(condition.id())?
        .media
        .as_ref()?
        .duration_seconds
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Unmet {
    pub field: String,
    pub reason: String,
}

fn unmet(field: &str, reason: impl Into<String>) -> Unmet {
    Unmet {
        field: field.into(),
        reason: reason.into(),
    }
}

/// Every requirement the contract cannot satisfy. `clips` adds measured clip
/// lengths when they are known (the final route), and is empty before.
pub(crate) fn unmet_requirements(
    contract: &VideoGenerationContract,
    requirement: &VideoGenerationRequirement,
    clips: &ClipSummary,
) -> Vec<Unmet> {
    let mut reasons = Vec::new();
    if contract.prompt_required && !requirement.prompt {
        reasons.push(unmet("/prompt", "the model requires a prompt"));
    }
    if requirement.negative_prompt && !contract.negative_prompt {
        reasons.push(unmet("/negative_prompt", "the model has no negative prompt"));
    }
    if requirement.seed && !contract.seed {
        reasons.push(unmet("/seed", "the model does not accept a seed"));
    }
    let references = requirement.reference_count();
    let has_keyframes = !requirement.keyframes.is_empty();
    if !has_keyframes && references == 0 && !contract.text_to_video {
        reasons.push(unmet("/conditions", "the model needs at least one condition"));
    }
    if has_keyframes {
        match &contract.keyframes {
            None => reasons.push(unmet("/conditions", "the model does not take keyframes")),
            Some(keyframes) => {
                for position in &requirement.keyframes {
                    if !keyframes.positions.contains(position) {
                        reasons.push(unmet(
                            "/conditions",
                            format!("the model does not take a {position:?} keyframe"),
                        ));
                    }
                }
                if !requirement.keyframes.contains(&KeyframePosition::Start)
                    && !keyframes.end_only
                {
                    reasons.push(unmet(
                        "/conditions",
                        "the model needs a start keyframe alongside an end keyframe",
                    ));
                }
                if keyframes.require_all && requirement.keyframes != keyframes.positions {
                    reasons.push(unmet(
                        "/conditions",
                        format!("the model needs keyframes {:?}", keyframes.positions),
                    ));
                }
            }
        }
        if references > 0 && !contract.keyframes_with_references {
            reasons.push(unmet(
                "/conditions",
                "the model cannot combine keyframes with references",
            ));
        }
    }
    let limits = &contract.references;
    let count = |reasons: &mut Vec<Unmet>, kind: &str, wanted: u32, max: Option<u32>| {
        if wanted == 0 {
            return;
        }
        match max {
            None => reasons.push(unmet(
                "/conditions",
                format!("the model does not take {kind} references"),
            )),
            Some(max) if wanted > max => reasons.push(unmet(
                "/conditions",
                format!("the model takes at most {max} {kind} references, got {wanted}"),
            )),
            _ => {}
        }
    };
    count(
        &mut reasons,
        "image",
        requirement.reference_images,
        limits.image.as_ref().map(|limit| limit.max),
    );
    count(
        &mut reasons,
        "video",
        requirement.reference_videos,
        limits.video.as_ref().map(|limit| limit.max),
    );
    count(
        &mut reasons,
        "audio",
        requirement.reference_audios,
        limits.audio.as_ref().map(|limit| limit.max),
    );
    if let Some(max) = limits.max_total {
        if references > max {
            reasons.push(unmet(
                "/conditions",
                format!("the model takes at most {max} references in total, got {references}"),
            ));
        }
    }
    if limits.audio_requires_visual
        && requirement.reference_audios > 0
        && requirement.reference_images == 0
        && requirement.reference_videos == 0
    {
        reasons.push(unmet(
            "/conditions",
            "audio references need an image or video reference for this model",
        ));
    }
    if requirement.reference_video_audio
        && !limits.video.as_ref().is_some_and(|limit| limit.include_audio)
    {
        reasons.push(unmet(
            "/conditions",
            "the model cannot use a reference video's audio",
        ));
    }
    for (kind, seconds, limit) in [
        ("video", &clips.video_seconds, limits.video.as_ref()),
        ("audio", &clips.audio_seconds, limits.audio.as_ref()),
    ] {
        let Some(limit) = limit else {
            continue;
        };
        for value in seconds {
            if limit.min_seconds.is_some_and(|min| *value + CLIP_TOLERANCE_SECONDS < min)
                || limit.max_seconds.is_some_and(|max| *value > max + CLIP_TOLERANCE_SECONDS)
            {
                reasons.push(unmet(
                    "/conditions",
                    format!(
                        "{kind} reference of {value:.2}s is outside {:?}..{:?}s",
                        limit.min_seconds, limit.max_seconds
                    ),
                ));
            }
        }
        let mut total: f64 = seconds.iter().sum();
        if kind == "audio" {
            total += clips.video_audio_seconds.iter().sum::<f64>();
        }
        if let Some(max) = limit.max_total_seconds {
            if total > max + CLIP_TOLERANCE_SECONDS {
                reasons.push(unmet(
                    "/conditions",
                    format!("{kind} references total {total:.2}s, above {max}s"),
                ));
            }
        }
    }
    if let Some(output) = &requirement.output {
        reasons.extend(unmet_output(contract, output, scenario_of(requirement)));
    }
    reasons
}

fn scenario_of(requirement: &VideoGenerationRequirement) -> VideoScenario {
    if !requirement.keyframes.is_empty() {
        VideoScenario::Keyframes
    } else if requirement.reference_count() > 0 {
        VideoScenario::References
    } else {
        VideoScenario::Text
    }
}

fn unmet_output(
    contract: &VideoGenerationContract,
    output: &VideoOutputSpec,
    scenario: VideoScenario,
) -> Vec<Unmet> {
    let mut reasons = Vec::new();
    let spec = &contract.output;
    if let Some(duration) = output.duration_seconds {
        if !spec
            .durations
            .as_ref()
            .is_some_and(|contract| contract.allows(duration))
        {
            reasons.push(unmet(
                "/output/duration_seconds",
                format!("the model cannot produce {duration}s"),
            ));
        }
    }
    if let Some(ratio) = &output.aspect_ratio {
        let input_ratio = scenario == VideoScenario::Keyframes
            && spec.keyframe_aspect_ratio == KeyframeAspectRatio::Input;
        if !input_ratio && !spec.aspect_ratios.contains(ratio) {
            reasons.push(unmet(
                "/output/aspect_ratio",
                format!("the model cannot produce aspect ratio {ratio}"),
            ));
        }
    }
    match output.size {
        Some(VideoSize::ShortEdge(edge)) => {
            if contract
                .size_for(edge.short_edge, output.aspect_ratio.as_deref())
                .is_none()
            {
                reasons.push(unmet(
                    "/output/size",
                    format!("the model has no {}-pixel short edge output", edge.short_edge),
                ));
            }
        }
        Some(VideoSize::Exact(VideoExactSize { width, height })) => {
            if !spec
                .sizes
                .iter()
                .any(|size| parse_size(size) == Some((width, height)))
            {
                reasons.push(unmet(
                    "/output/size",
                    format!("the model cannot produce exactly {width}x{height}"),
                ));
            }
        }
        None => {}
    }
    if let Some(fps) = output.fps {
        if !spec.fps.iter().any(|value| (value - fps).abs() < 1e-6) {
            reasons.push(unmet("/output/fps", format!("the model cannot produce {fps} fps")));
        }
    }
    match (output.audio_mode(), spec.audio) {
        (VideoAudioMode::Generate, AudioOutput::Never) => reasons.push(unmet(
            "/output/audio",
            "the model never generates an audio track",
        )),
        (VideoAudioMode::None, AudioOutput::Always) => reasons.push(unmet(
            "/output/audio",
            "the model always generates an audio track",
        )),
        (VideoAudioMode::Generate | VideoAudioMode::None, AudioOutput::Unknown) => {
            reasons.push(unmet(
                "/output/audio",
                "the model's audio track is not declared; only `auto` is accepted",
            ))
        }
        _ => {}
    }
    if let Some(media_type) = &output.media_type {
        if !spec.media_types.contains(media_type) {
            reasons.push(unmet(
                "/output/media_type",
                format!("the model cannot produce {media_type}"),
            ));
        }
    }
    reasons
}

fn unsupported(reasons: Vec<Unmet>) -> AiccError {
    let field = reasons
        .first()
        .map(|reason| reason.field.clone())
        .unwrap_or_default();
    let message = reasons
        .iter()
        .map(|reason| reason.reason.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    video_error(
        AiccErrorCode::UnsupportedOperation,
        message,
        &field,
        None,
        Some(json!(reasons
            .iter()
            .map(|reason| json!({"field": reason.field, "reason": reason.reason}))
            .collect::<Vec<_>>())),
        None,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ValueSource {
    Request,
    Default,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PlannedInput {
    pub condition_id: String,
    pub kind: VideoConditionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<KeyframePosition>,
    /// 1-based order among conditions of the same kind, as submitted.
    pub ordinal: u32,
    pub token: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_audio: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts: Option<MediaFacts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<DerivedMaterial>,
}

/// What a video codec submits, decided before encoding and stored with the
/// task (`resolved_parameters.video_plan`) as the record of the request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VideoPlan {
    pub scenario: VideoScenario,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PlannedInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_source: Option<ValueSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    /// The output ratio follows the keyframe image rather than `aspect_ratio`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub aspect_ratio_from_input: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<VideoExactSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    pub audio: VideoAudioMode,
    /// Present when the protocol has an audio switch and the request decides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generate_audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_output: Option<VideoOutputSpec>,
}

impl VideoPlan {
    pub(crate) fn from_parameters(
        parameters: &BTreeMap<String, Value>,
    ) -> Result<Self, crate::protocol::ProtocolError> {
        let value = parameters.get(VIDEO_PLAN_PARAMETER).ok_or_else(|| {
            crate::protocol::ProtocolError::invalid_request("video plan is missing")
        })?;
        serde_json::from_value(value.clone()).map_err(|error| {
            crate::protocol::ProtocolError::invalid_request(format!("video plan is invalid: {error}"))
        })
    }

    pub(crate) fn inputs(&self, kind: VideoConditionKind) -> impl Iterator<Item = &PlannedInput> {
        self.inputs.iter().filter(move |input| input.kind == kind)
    }

    pub(crate) fn keyframe(&self, position: KeyframePosition) -> Option<&PlannedInput> {
        self.inputs
            .iter()
            .find(|input| input.position == Some(position))
    }

    /// Dimensions price rules can match on (`/video/...`).
    pub(crate) fn pricing_dimensions(&self) -> BTreeMap<String, Value> {
        let mut dimensions = BTreeMap::from([
            ("/video/scenario".to_string(), json!(self.scenario)),
            ("/video/audio".to_string(), json!(self.audio)),
            (
                "/video/reference_images".to_string(),
                json!(self.inputs(VideoConditionKind::ReferenceImage).count()),
            ),
        ]);
        if let Some(resolution) = &self.resolution {
            dimensions.insert("/video/resolution".into(), json!(resolution));
        }
        if let Some(duration) = self.duration_seconds {
            dimensions.insert("/video/duration_seconds".into(), json!(duration));
        }
        dimensions
    }
}

/// Checks `request` against `contract` with measured `facts` and decides
/// exactly what is submitted. Every refusal is `unsupported_operation`
/// (or `invalid_request` for contradictions with the measured media).
pub(crate) fn plan_video(
    contract: &VideoGenerationContract,
    request: &VideoGenerateRequest,
    facts: &VideoMaterialFacts,
) -> Result<VideoPlan, AiccError> {
    request.validate()?;
    for (index, condition) in request.conditions.iter().enumerate() {
        let measured = facts
            .get(condition.id())
            .and_then(|facts| facts.media.as_ref());
        if matches!(
            condition.kind(),
            VideoConditionKind::ReferenceVideo | VideoConditionKind::ReferenceAudio
        ) && measured.is_none()
        {
            return Err(video_error(
                AiccErrorCode::InternalError,
                "reference media was not measured before planning",
                &format!("/conditions/{index}/resource"),
                Some(condition.id()),
                None,
                None,
            ));
        }
        if let (Some(range), Some(media)) = (condition.time_range(), measured) {
            let duration = media.duration_seconds.unwrap_or_default();
            if range.end_seconds > duration + CLIP_TOLERANCE_SECONDS {
                return Err(video_error(
                    AiccErrorCode::InvalidRequest,
                    "time_range ends after the measured media",
                    &format!("/conditions/{index}/time_range"),
                    Some(condition.id()),
                    Some(json!({"max_end_seconds": duration})),
                    Some(json!(range)),
                ));
            }
            if range.start_seconds > CLIP_TOLERANCE_SECONDS
                || range.end_seconds + CLIP_TOLERANCE_SECONDS < duration
            {
                return Err(video_error(
                    AiccErrorCode::UnsupportedOperation,
                    "clip trimming is not available; time_range must cover the whole media",
                    &format!("/conditions/{index}/time_range"),
                    Some(condition.id()),
                    Some(json!({"start_seconds": 0, "end_seconds": duration})),
                    Some(json!(range)),
                ));
            }
        }
        if condition.include_audio() && measured.is_some_and(|media| !media.has_audio) {
            return Err(video_error(
                AiccErrorCode::ResourceInvalid,
                "include_audio requires a reference video with an audio track",
                &format!("/conditions/{index}/include_audio"),
                Some(condition.id()),
                None,
                None,
            ));
        }
        if let (Some(limits), Some(media)) = (&contract.image_limits, measured) {
            if matches!(
                condition.kind(),
                VideoConditionKind::ReferenceImage | VideoConditionKind::Keyframe
            ) {
                if let Some(reason) = image_limit_violation(limits, media) {
                    return Err(video_error(
                        AiccErrorCode::UnsupportedOperation,
                        reason,
                        &format!("/conditions/{index}/resource"),
                        Some(condition.id()),
                        Some(json!(limits)),
                        Some(json!({"width": media.width, "height": media.height})),
                    ));
                }
            }
        }
    }
    let requirement = request.requirement();
    let reasons = unmet_requirements(contract, &requirement, &clip_summary(request, facts));
    if !reasons.is_empty() {
        return Err(unsupported(reasons));
    }
    let scenario = scenario_of(&requirement);
    let output = request.output.clone().unwrap_or_default();
    let spec = &contract.output;

    let mut ordinals = BTreeMap::<VideoConditionKind, u32>::new();
    let mut inputs = Vec::new();
    for condition in &request.conditions {
        let ordinal = ordinals.entry(condition.kind()).or_default();
        *ordinal += 1;
        let condition_facts = facts.get(condition.id());
        let mut input = PlannedInput {
            condition_id: condition.id().to_string(),
            kind: condition.kind(),
            position: condition.keyframe_position(),
            ordinal: *ordinal,
            token: String::new(),
            include_audio: condition.include_audio(),
            facts: condition_facts.and_then(|facts| facts.media.clone()),
            derived: condition_facts.and_then(|facts| facts.derived.clone()),
        };
        input.token = contract.prompt_tokens.token(&input);
        inputs.push(input);
    }

    let (duration_seconds, duration_source) = match output.duration_seconds {
        Some(value) => (Some(value), Some(ValueSource::Request)),
        None => (
            spec.default_duration_seconds,
            spec.default_duration_seconds.map(|_| ValueSource::Default),
        ),
    };
    let (resolution, size) = match output.size {
        Some(VideoSize::ShortEdge(edge)) => {
            match contract.size_for(edge.short_edge, output.aspect_ratio.as_deref()) {
                Some(SizeChoice::Tier(name)) => (Some(name), None),
                Some(SizeChoice::Exact(size)) => (None, Some(size)),
                None => (None, None),
            }
        }
        Some(VideoSize::Exact(size)) => (None, Some(size)),
        None => (spec.default_resolution.clone(), None),
    };
    // A protocol that only takes exact sizes gets the first declared size with
    // the requested ratio; a ratio with no declared size cannot be honored.
    let size = match (&output.aspect_ratio, size, &resolution) {
        (Some(ratio), None, None) if spec.resolutions.is_empty() && !spec.sizes.is_empty() => {
            let wanted = parse_aspect_ratio(ratio).map(|(w, h)| reduce_ratio(w, h));
            let size = spec
                .sizes
                .iter()
                .filter_map(|size| parse_size(size))
                .find(|(width, height)| Some(reduce_ratio(*width, *height)) == wanted)
                .ok_or_else(|| {
                    unsupported(vec![unmet(
                        "/output/aspect_ratio",
                        format!("the model has no output size with aspect ratio {ratio}"),
                    )])
                })?;
            Some(VideoExactSize {
                width: size.0,
                height: size.1,
            })
        }
        (_, size, _) => size,
    };
    let aspect_ratio_from_input = scenario == VideoScenario::Keyframes
        && spec.keyframe_aspect_ratio == KeyframeAspectRatio::Input;
    if aspect_ratio_from_input {
        if let Some(ratio) = &output.aspect_ratio {
            let start = inputs
                .iter()
                .find(|input| input.kind == VideoConditionKind::Keyframe)
                .and_then(|input| input.facts.as_ref());
            let matches = start
                .and_then(|facts| Some((facts.width?, facts.height?)))
                .zip(parse_aspect_ratio(ratio))
                .is_some_and(|((width, height), (w, h))| {
                    let wanted = w as f64 / h as f64;
                    ((width as f64 / height as f64) - wanted).abs() / wanted
                        <= KEYFRAME_RATIO_TOLERANCE
                });
            if !matches {
                return Err(video_error(
                    AiccErrorCode::UnsupportedOperation,
                    "with keyframes this model takes the aspect ratio from the keyframe image; the requested ratio does not match the measured keyframe",
                    "/output/aspect_ratio",
                    None,
                    Some(json!(start.map(|facts| json!({"width": facts.width, "height": facts.height})))),
                    Some(json!(ratio)),
                ));
            }
        }
    }
    let aspect_ratio = match &output.aspect_ratio {
        Some(ratio) => Some(ratio.clone()),
        None if scenario == VideoScenario::Text => spec.default_aspect_ratio.clone(),
        None => None,
    };
    let audio = match (output.audio_mode(), spec.audio) {
        (_, AudioOutput::Always) => VideoAudioMode::Generate,
        (_, AudioOutput::Never) => VideoAudioMode::None,
        (mode, AudioOutput::Optional) => mode,
        (_, AudioOutput::Unknown) => VideoAudioMode::Auto,
    };
    let generate_audio = match (spec.audio, output.audio_mode()) {
        (AudioOutput::Optional, VideoAudioMode::Generate) => Some(true),
        (AudioOutput::Optional, VideoAudioMode::None) => Some(false),
        _ => None,
    };
    let media_type = output
        .media_type
        .clone()
        .or_else(|| spec.media_types.first().cloned());

    for rule in &spec.rules {
        let when = &rule.when;
        let applies = (when.resolutions.is_empty()
            || resolution
                .as_ref()
                .is_some_and(|name| when.resolutions.contains(name)))
            && (when.inputs.is_empty()
                || when
                    .inputs
                    .iter()
                    .all(|kind| inputs.iter().any(|input| input.kind == *kind)))
            && (when.scenarios.is_empty() || when.scenarios.contains(&scenario));
        if !applies {
            continue;
        }
        let mut reasons = Vec::new();
        if let (Some(contract), Some(seconds)) = (&rule.durations, duration_seconds) {
            if !contract.allows(seconds) {
                reasons.push(Unmet {
                    field: "/output/duration_seconds".into(),
                    reason: format!(
                        "this combination requires durations {}",
                        contract.describe()
                    ),
                });
            }
        }
        if let (Some(allowed), Some(name)) = (&rule.resolutions, &resolution) {
            if !allowed.contains(name) {
                reasons.push(unmet(
                    "/output/size",
                    format!("this combination requires resolutions {allowed:?}"),
                ));
            }
        }
        if let (Some(allowed), Some(ratio)) = (&rule.aspect_ratios, &aspect_ratio) {
            if !allowed.contains(ratio) {
                reasons.push(unmet(
                    "/output/aspect_ratio",
                    format!("this combination requires aspect ratios {allowed:?}"),
                ));
            }
        }
        if !reasons.is_empty() {
            return Err(unsupported(reasons));
        }
    }

    let segments = request
        .prompt
        .as_deref()
        .map(parse_video_prompt)
        .transpose()?;
    let submitted_prompt = segments.as_ref().map(|segments| {
        render_video_prompt(segments, |id| {
            inputs
                .iter()
                .find(|input| input.condition_id == id)
                .map(|input| input.token.clone())
                .unwrap_or_default()
        })
    });
    debug_assert!(segments.as_ref().is_none_or(|segments| segments.iter().all(|segment| {
        matches!(segment, VideoPromptSegment::Text(_))
            || matches!(segment, VideoPromptSegment::Reference(id) if request.condition(id).is_some())
    })));

    Ok(VideoPlan {
        scenario,
        prompt: request.prompt.clone(),
        submitted_prompt,
        negative_prompt: request.negative_prompt.clone(),
        seed: request.seed,
        inputs,
        duration_seconds,
        duration_source,
        aspect_ratio,
        aspect_ratio_from_input,
        resolution,
        size,
        fps: output.fps,
        audio,
        generate_audio,
        media_type,
        requested_output: request.output.clone().filter(|output| !output.is_empty()),
    })
}

fn image_limit_violation(limits: &ImageLimits, media: &MediaFacts) -> Option<String> {
    let (width, height) = (media.width?, media.height?);
    if limits
        .min_edge
        .is_some_and(|min| width.min(height) < min)
        || limits
            .max_edge
            .is_some_and(|max| width.max(height) > max)
    {
        return Some(format!(
            "image {width}x{height} is outside the {:?}..{:?} pixel range",
            limits.min_edge, limits.max_edge
        ));
    }
    let ratio = width as f64 / height as f64;
    if limits.min_aspect.is_some_and(|min| ratio < min)
        || limits.max_aspect.is_some_and(|max| ratio > max)
    {
        return Some(format!(
            "image aspect {ratio:.2} is outside {:?}..{:?}",
            limits.min_aspect, limits.max_aspect
        ));
    }
    None
}

/// Short edge of a vendor resolution name such as `720p`, `768P`, `2K`.
pub(crate) fn short_edge_of_resolution(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    if let Some(value) = lower.strip_suffix('p') {
        return value.parse().ok();
    }
    match lower.as_str() {
        "2k" => Some(1440),
        "4k" => Some(2160),
        _ => None,
    }
}

pub(crate) fn ratio_label(width: u32, height: u32) -> String {
    let (w, h) = reduce_ratio(width, height);
    format!("{w}:{h}")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::catalog::CatalogKind;

    fn builtin(kind: CatalogKind, catalog_id: &str) -> Value {
        let file = crate::settings::load_builtin_metadata()
            .unwrap()
            .into_iter()
            .find(|file| file.kind == kind && file.catalog_id == catalog_id)
            .unwrap_or_else(|| panic!("builtin metadata `{catalog_id}` is missing"));
        serde_json::from_slice(&file.contents).unwrap()
    }

    /// The vendor contract a model driver declares for `model`.
    pub(crate) fn model_contract(driver: &str, model: &str) -> VideoGenerationContract {
        let catalog = builtin(CatalogKind::ModelDriver, driver);
        let entry = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["id"] == model)
            .unwrap_or_else(|| panic!("model `{driver}/{model}` is missing"));
        serde_json::from_value(entry["capabilities"][VIDEO_CONTRACT_CAPABILITY].clone())
            .unwrap_or_else(|error| panic!("`{driver}/{model}` contract: {error}"))
    }

    /// The channel contract a provider rule declares for `id`.
    pub(crate) fn provider_contract(rules: &str, id: &str) -> VideoGenerationContract {
        let catalog = builtin(CatalogKind::ProviderRules, rules);
        let entry = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .chain(catalog["patterns"].as_array().unwrap())
            .find(|entry| entry["id"] == id || entry["match"] == id)
            .unwrap_or_else(|| panic!("provider rule `{rules}/{id}` is missing"));
        serde_json::from_value(entry[VIDEO_CONTRACT_CAPABILITY].clone())
            .unwrap_or_else(|error| panic!("`{rules}/{id}` contract: {error}"))
    }

    /// An OpenRouter channel contract as inventory builds it: the static
    /// provider rule with the `/videos/models` facts listed on 2026-10-22.
    pub(crate) fn openrouter_contract(model: &str) -> VideoGenerationContract {
        let mut capabilities = BTreeMap::from([(
            VIDEO_CONTRACT_CAPABILITY.to_string(),
            serde_json::to_value(provider_contract("openrouter", model)).unwrap(),
        )]);
        let max = model.ends_with("-max");
        apply_discovered_video_facts(
            &mut capabilities,
            &DiscoveredVideoFacts {
                durations: (5..=15).map(f64::from).collect(),
                resolutions: if max {
                    vec!["768p".into(), "480p".into()]
                } else {
                    vec!["2K".into()]
                },
                aspect_ratios: ["21:9", "16:9", "4:3", "1:1", "3:4", "9:16"]
                    .map(str::to_owned)
                    .to_vec(),
                sizes: Vec::new(),
                keyframes: vec![KeyframePosition::Start, KeyframePosition::End],
                generate_audio: Some(!max),
                seed: Some(false),
            },
        );
        VideoGenerationContract::from_capabilities(&capabilities).unwrap()
    }

    pub(crate) fn image_facts(width: u32, height: u32) -> ConditionFacts {
        ConditionFacts {
            media: Some(MediaFacts {
                container: crate::media::MediaContainer::Png,
                duration_seconds: None,
                width: Some(width),
                height: Some(height),
                fps: None,
                has_video: false,
                has_audio: false,
                fragmented: false,
            }),
            derived: None,
        }
    }

    pub(crate) fn clip_facts(seconds: f64, video: bool, audio: bool) -> ConditionFacts {
        ConditionFacts {
            media: Some(MediaFacts {
                container: if video {
                    crate::media::MediaContainer::Mp4
                } else {
                    crate::media::MediaContainer::Wav
                },
                duration_seconds: Some(seconds),
                width: video.then_some(1280),
                height: video.then_some(720),
                fps: video.then_some(24.0),
                has_video: video,
                has_audio: audio,
                fragmented: false,
            }),
            derived: None,
        }
    }

    /// Resolved parameters a codec receives for `request` on `model`.
    pub(crate) fn plan_parameters(
        contract: &VideoGenerationContract,
        request: &VideoGenerateRequest,
        facts: &VideoMaterialFacts,
        provider_model_id: &str,
    ) -> BTreeMap<String, Value> {
        let plan = plan_video(contract, request, facts).unwrap();
        BTreeMap::from([
            ("provider_model_id".to_string(), json!(provider_model_id)),
            (
                VIDEO_PLAN_PARAMETER.to_string(),
                serde_json::to_value(plan).unwrap(),
            ),
        ])
    }
}

/// Output facts a channel's live catalog reports for a video model, such as
/// OpenRouter's `/videos/models`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoveredVideoFacts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub durations: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolutions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aspect_ratios: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<KeyframePosition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generate_audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<bool>,
}

/// The channel catalog is the authority on output facts: it replaces the
/// output limits, keyframe positions, audio and seed of the static contract
/// (or forms a text/keyframe-only contract when none is declared). Reference
/// limits stay as the static channel contract declares them.
pub(crate) fn apply_discovered_video_facts(
    capabilities: &mut BTreeMap<String, Value>,
    facts: &DiscoveredVideoFacts,
) {
    let mut contract = VideoGenerationContract::from_capabilities(capabilities).unwrap_or(
        VideoGenerationContract {
            prompt_required: true,
            prompt_tokens: PromptTokenStyle::Generic,
            negative_prompt: false,
            seed: false,
            text_to_video: true,
            keyframes: None,
            references: ReferenceContract::default(),
            keyframes_with_references: false,
            image_limits: None,
            output: OutputContract {
                durations: None,
                default_duration_seconds: None,
                aspect_ratios: Vec::new(),
                default_aspect_ratio: None,
                keyframe_aspect_ratio: KeyframeAspectRatio::Input,
                resolutions: Vec::new(),
                default_resolution: None,
                sizes: Vec::new(),
                fps: Vec::new(),
                audio: AudioOutput::Unknown,
                media_types: vec!["video/mp4".to_string()],
                rules: Vec::new(),
            },
        },
    );
    let output = &mut contract.output;
    output.durations = (!facts.durations.is_empty()).then(|| DurationContract {
        values: facts.durations.clone(),
        ..Default::default()
    });
    if output
        .default_duration_seconds
        .is_some_and(|value| !facts.durations.contains(&value))
    {
        output.default_duration_seconds = None;
    }
    output.resolutions = facts
        .resolutions
        .iter()
        .filter_map(|name| {
            Some(ResolutionTier {
                name: name.clone(),
                short_edge: short_edge_of_resolution(name)?,
            })
        })
        .collect();
    let tiers = output
        .resolutions
        .iter()
        .map(|tier| tier.name.as_str())
        .collect::<BTreeSet<_>>();
    if output
        .default_resolution
        .as_deref()
        .is_some_and(|name| !tiers.contains(name))
    {
        output.default_resolution = None;
    }
    // A channel offering a single tier can only produce that tier.
    if output.default_resolution.is_none() && output.resolutions.len() == 1 {
        output.default_resolution = Some(output.resolutions[0].name.clone());
    }
    output.rules.retain(|rule| {
        rule.when
            .resolutions
            .iter()
            .chain(rule.resolutions.iter().flatten())
            .all(|name| tiers.contains(name.as_str()))
    });
    output.aspect_ratios = facts
        .aspect_ratios
        .iter()
        .filter(|ratio| parse_aspect_ratio(ratio).is_some())
        .cloned()
        .collect();
    if output
        .default_aspect_ratio
        .as_ref()
        .is_some_and(|ratio| !output.aspect_ratios.contains(ratio))
    {
        output.default_aspect_ratio = None;
    }
    output.sizes = facts
        .sizes
        .iter()
        .filter(|size| parse_size(size).is_some())
        .cloned()
        .collect();
    // `generate_audio: false` says the channel cannot generate sound, not that
    // the file has no audio track: OpenRouter's H3-Max returned one (T2,
    // 2026-10-22). Neither `audio: generate` nor `audio: none` is promised.
    output.audio = match facts.generate_audio {
        Some(true) => AudioOutput::Always,
        Some(false) | None => AudioOutput::Unknown,
    };
    contract.seed = facts.seed == Some(true);
    contract.keyframes = (!facts.keyframes.is_empty()).then(|| KeyframeContract {
        positions: facts.keyframes.iter().copied().collect(),
        end_only: contract
            .keyframes
            .as_ref()
            .is_none_or(|keyframes| keyframes.end_only),
        require_all: false,
    });
    if let Ok(value) = serde_json::to_value(&contract) {
        if contract.validate().is_ok() {
            capabilities.insert(VIDEO_CONTRACT_CAPABILITY.to_string(), value);
        } else {
            capabilities.remove(VIDEO_CONTRACT_CAPABILITY);
        }
    }
}
