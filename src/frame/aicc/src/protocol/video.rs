//! Pieces every video codec shares: reading the lowered plan, finding a
//! planned condition's resource, and shaping `videos[]` output.

use super::{CodecInput, ProtocolError, ProtocolErrorKind, ProtocolOutput, ProtocolResultValue};
use crate::video::{PlannedInput, VideoPlan, VIDEO_PLAN_PARAMETER};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiUsage, AiccCall, GeneratedVideo, ResourceRef, VideoGenerateRequest, VideoMetadata,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) fn video_generate_input(
    input: &CodecInput,
) -> ProtocolResultValue<(&VideoGenerateRequest, VideoPlan)> {
    let AiccCall::VideoGenerate(request) = &input.canonical_request else {
        return Err(ProtocolError::invalid_request(
            "video codec received a request that is not video.generate",
        ));
    };
    Ok((
        request,
        VideoPlan::from_parameters(&input.resolved_parameters)?,
    ))
}

pub(crate) fn planned_resource<'a>(
    request: &'a VideoGenerateRequest,
    input: &PlannedInput,
) -> ProtocolResultValue<&'a ResourceRef> {
    request
        .condition(&input.condition_id)
        .map(|condition| condition.resource())
        .ok_or_else(|| {
            ProtocolError::invalid_request(format!(
                "video plan references unknown condition `{}`",
                input.condition_id
            ))
        })
}

/// Rejects resolved parameters other than the model id, the plan and
/// `allowed`, so provider rules cannot smuggle options past the plan.
pub(crate) fn require_video_parameters(
    parameters: &BTreeMap<String, Value>,
    protocol: &str,
    allowed: &[&str],
) -> ProtocolResultValue<()> {
    if let Some(name) = parameters.keys().find(|name| {
        !matches!(name.as_str(), "provider_model_id" | VIDEO_PLAN_PARAMETER)
            && !allowed.contains(&name.as_str())
    }) {
        return Err(ProtocolError::invalid_request(format!(
            "resolved {protocol} video parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

/// The plan's duration as whole seconds, which most video protocols require.
pub(crate) fn whole_seconds(plan: &VideoPlan) -> ProtocolResultValue<Option<u64>> {
    plan.duration_seconds
        .map(super::adapter::integer_seconds)
        .transpose()
}

pub(crate) fn unsupported(message: impl Into<String>) -> ProtocolError {
    ProtocolError::new(ProtocolErrorKind::UnsupportedOperation, message)
}

pub(crate) fn generated_video(
    resource: ResourceRef,
    media_type: Option<String>,
    provider_metadata: VideoMetadata,
) -> GeneratedVideo {
    let mut video = GeneratedVideo::new(resource, media_type);
    video.provider_metadata = provider_metadata;
    video
}

/// One finished video; a provider reporting more results than were asked for
/// fails instead of being truncated.
pub(crate) fn video_output(
    videos: Vec<GeneratedVideo>,
    usage: AiUsage,
) -> ProtocolResultValue<ProtocolOutput> {
    match videos.len() {
        0 => {
            return Err(ProtocolError::invalid_response(
                "video result contains no video",
            ))
        }
        1 => {}
        count => {
            return Err(ProtocolError::invalid_response(format!(
                "video result contains {count} videos but one was requested"
            )))
        }
    }
    let mut artifacts = Vec::new();
    for (index, video) in videos.iter().enumerate() {
        let name = if index == 0 {
            "video".to_string()
        } else {
            format!("video-{}", index + 1)
        };
        artifacts.push(AiArtifact {
            name,
            resource: video.resource.clone(),
            mime: video.media_type.clone(),
            metadata: None,
        });
        if let Some(last_frame) = &video.last_frame {
            artifacts.push(AiArtifact {
                name: format!("last-frame-{}", index + 1),
                resource: last_frame.clone(),
                mime: None,
                metadata: None,
            });
        }
    }
    Ok(ProtocolOutput {
        value: json!({ "videos": videos }),
        usage: Some(usage),
        artifacts,
    })
}

/// Short edge and width/height from a `WxH` string.
pub(crate) fn size_metadata(size: Option<&str>) -> (Option<u32>, Option<u32>) {
    match size.and_then(crate::video::parse_size) {
        Some((width, height)) => (Some(width), Some(height)),
        None => (None, None),
    }
}

/// Fills `measured_metadata` for videos whose bytes are inline in the output
/// (before they are stored) and records how each video deviates from the
/// explicitly requested output. URL results stay unmeasured.
pub(crate) fn finish_video_output(
    output: &mut ProtocolOutput,
    requested: Option<&buckyos_api::VideoOutputSpec>,
) {
    let Some(videos) = output.value.get_mut("videos").and_then(Value::as_array_mut) else {
        return;
    };
    for value in videos.iter_mut() {
        let Ok(mut video) = serde_json::from_value::<GeneratedVideo>(value.clone()) else {
            continue;
        };
        if let ResourceRef::Base64 { data_base64, .. } = &video.resource {
            if let Some(facts) = STANDARD
                .decode(data_base64)
                .ok()
                .and_then(|bytes| crate::media::probe(&bytes).ok())
                .filter(|facts| facts.has_video)
            {
                video.measured_metadata = Some(facts.metadata());
            }
        }
        video.record_deviations(requested);
        if let Ok(updated) = serde_json::to_value(&video) {
            *value = updated;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{VideoAudioMode, VideoMetadataSource, VideoOutputSpec, VideoSize};

    const VIDEO: &[u8] =
        include_bytes!("../../../../../test/jarvis_media_dv/assets/video_fresh.mp4");

    fn output_with(video: GeneratedVideo) -> ProtocolOutput {
        video_output(vec![video], AiUsage::default()).unwrap()
    }

    #[test]
    fn inline_video_is_measured_and_deviations_name_their_source() {
        let mut provider = VideoMetadata::default();
        provider.duration_seconds = Some(6.0);
        let mut output = output_with(generated_video(
            ResourceRef::base64("video/mp4".into(), STANDARD.encode(VIDEO)),
            Some("video/mp4".into()),
            provider,
        ));
        let requested = VideoOutputSpec {
            duration_seconds: Some(6.0),
            size: Some(VideoSize::short_edge(720)),
            audio: Some(VideoAudioMode::None),
            ..Default::default()
        };
        finish_video_output(&mut output, Some(&requested));
        let video: GeneratedVideo =
            serde_json::from_value(output.value["videos"][0].clone()).unwrap();
        let measured = video.measured_metadata.as_ref().unwrap();
        assert_eq!((measured.width, measured.height), (Some(960), Some(540)));
        assert_eq!(measured.has_audio, Some(true));
        assert!((measured.duration_seconds.unwrap() - 5.055).abs() < 0.01);
        // The Provider's own report stays as it was; the file decides.
        assert_eq!(video.provider_metadata.duration_seconds, Some(6.0));
        let deviation = |field: &str| {
            video
                .deviations
                .iter()
                .find(|deviation| deviation.field == field)
                .unwrap_or_else(|| panic!("no {field} deviation"))
        };
        assert_eq!(
            deviation("duration_seconds").source,
            VideoMetadataSource::Measured
        );
        assert_eq!(
            deviation("size").actual,
            json!({"width": 960, "height": 540})
        );
        assert_eq!(deviation("audio").actual, json!(true));
    }

    #[test]
    fn url_video_is_not_measured_and_unrequested_fields_are_not_deviations() {
        let mut provider = VideoMetadata::default();
        provider.width = Some(1280);
        provider.height = Some(720);
        let mut output = output_with(generated_video(
            ResourceRef::url("https://cdn.example.test/v.mp4".into(), None),
            None,
            provider,
        ));
        finish_video_output(&mut output, Some(&VideoOutputSpec::default()));
        let video: GeneratedVideo =
            serde_json::from_value(output.value["videos"][0].clone()).unwrap();
        assert!(video.measured_metadata.is_none());
        assert!(video.deviations.is_empty());
    }

    #[test]
    fn encoder_aligned_sizes_are_not_aspect_deviations() {
        // OpenRouter H3-Max returned 832x480 for 16:9 at 480p (T2, 2026-10-22).
        let mut provider = VideoMetadata::default();
        provider.width = Some(832);
        provider.height = Some(480);
        let mut video = generated_video(
            ResourceRef::url("https://cdn.example.test/v.mp4".into(), None),
            None,
            provider,
        );
        video.record_deviations(Some(&VideoOutputSpec {
            aspect_ratio: Some("16:9".into()),
            size: Some(VideoSize::short_edge(480)),
            ..Default::default()
        }));
        assert!(video.deviations.is_empty(), "{:?}", video.deviations);
        video.record_deviations(Some(&VideoOutputSpec {
            aspect_ratio: Some("4:3".into()),
            ..Default::default()
        }));
        assert_eq!(video.deviations[0].field, "aspect_ratio");
    }

    #[test]
    fn more_videos_than_requested_fail_instead_of_being_truncated() {
        let video = || {
            generated_video(
                ResourceRef::url("https://cdn.example.test/v.mp4".into(), None),
                None,
                VideoMetadata::default(),
            )
        };
        assert!(video_output(vec![video(), video()], AiUsage::default()).is_err());
        assert!(video_output(Vec::new(), AiUsage::default()).is_err());
    }
}
