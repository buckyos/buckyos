use buckyos_api::*;
use serde_json::{json, Value};

fn cases() -> Value {
    serde_json::from_str(include_str!("fixtures/video_generate_cases.json")).unwrap()
}

#[test]
fn prompt_reference_grammar_matches_shared_fixtures() {
    for case in cases()["prompt_cases"].as_array().unwrap() {
        let prompt = case["prompt"].as_str().unwrap();
        let parsed = parse_video_prompt(prompt);
        if case["error"].as_bool() == Some(true) {
            let error = parsed.expect_err(prompt);
            assert_eq!(error.code, AiccErrorCode::InvalidRequest, "{prompt}");
            assert_eq!(error.details.as_ref().unwrap()["field"], "/prompt");
            continue;
        }
        let segments = parsed
            .unwrap()
            .into_iter()
            .map(|segment| match segment {
                VideoPromptSegment::Text(text) => json!({"text": text}),
                VideoPromptSegment::Reference(id) => json!({"reference": id}),
            })
            .collect::<Vec<_>>();
        assert_eq!(Value::Array(segments), case["segments"], "{prompt}");
    }
}

#[test]
fn requests_and_requirements_match_shared_fixtures() {
    for case in cases()["requests"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let parsed = VideoGenerateRequest::from_json(case["request"].clone());
        if case["valid"].as_bool() != Some(true) {
            assert!(parsed.is_err(), "{name} must be rejected");
            continue;
        }
        let request = parsed.unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(
            request.requirement(),
            serde_json::from_value::<VideoGenerationRequirement>(case["requirement"].clone())
                .unwrap(),
            "{name}"
        );
        assert_eq!(
            VideoGenerateRequest::from_json(serde_json::to_value(&request).unwrap()).unwrap(),
            request,
            "{name} round trip"
        );
        let mut helper = case["request"].clone();
        let object = helper.as_object_mut().unwrap();
        object.remove("exact_model");
        object.insert("logical_model".into(), json!("video.generate"));
        let helper = VideoGenerateHelperRequest::from_json(helper).unwrap();
        assert_eq!(helper.requirement().unwrap(), request.requirement());
        assert_eq!(helper.to_exact("m@p"), request);
    }
}

#[test]
fn validation_errors_carry_field_and_condition_details() {
    let error = VideoGenerateRequest::new("m@p", Some("@{ghost}".into()))
        .validate()
        .unwrap_err();
    let details = error.details.unwrap();
    assert_eq!(details["field"], "/prompt");
    assert_eq!(details["condition_id"], "ghost");

    let mut request = VideoGenerateRequest::new("m@p", None);
    request.conditions = vec![
        VideoCondition::Keyframe {
            id: "a".into(),
            resource: ResourceRef::url("https://example.com/a.png".into(), None),
            position: KeyframePosition::End,
        },
        VideoCondition::Keyframe {
            id: "b".into(),
            resource: ResourceRef::url("https://example.com/b.png".into(), None),
            position: KeyframePosition::End,
        },
    ];
    let details = request.validate().unwrap_err().details.unwrap();
    assert_eq!(details["field"], "/conditions/1/position");
    assert_eq!(details["condition_id"], "b");
}

#[test]
fn helper_declared_requirements_tighten_but_never_relax() {
    let mut helper = VideoGenerateHelperRequest::new("video.generate", Some("a fox".into()));
    helper.output = Some(VideoOutputSpec {
        duration_seconds: Some(5.0),
        ..Default::default()
    });
    helper.requirements.video = Some(VideoGenerationRequirement {
        reference_images: 2,
        keyframes: [KeyframePosition::Start].into(),
        ..Default::default()
    });
    let requirement = helper.requirement().unwrap();
    assert!(requirement.prompt);
    assert_eq!(requirement.reference_images, 2);
    assert!(requirement.keyframes.contains(&KeyframePosition::Start));
    assert_eq!(requirement.output.unwrap().duration_seconds, Some(5.0));

    helper.requirements.video = Some(VideoGenerationRequirement {
        output: Some(VideoOutputSpec {
            duration_seconds: Some(8.0),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert!(helper.validate().is_err());
}

#[test]
fn video_methods_dispatch_and_legacy_methods_are_gone() {
    assert!(ai_methods::is_ai_method(ai_methods::VIDEO_GENERATE));
    assert!(ai_methods::is_ai_method(ai_methods::VIDEO_EDIT));
    assert!(ai_methods::is_aicc_core_method(
        ai_methods::HELPER_VIDEO_GENERATE
    ));
    for legacy in ["video.txt2video", "video.img2video", "video.video2video"] {
        assert!(!ai_methods::is_aicc_core_method(legacy));
        assert!(matches!(
            AiccCall::from_method_and_params(legacy, json!({"exact_model": "m@p", "prompt": "a"})),
            Err(kRPC::RPCErrors::UnknownMethod(_))
        ));
        assert!(serde_json::from_value::<ApiType>(json!(legacy)).is_err());
    }
    assert_eq!(
        serde_json::to_value(ApiType::VideoGenerate).unwrap(),
        json!("video.generate")
    );
    assert_eq!(
        ApiType::VideoGenerate.typed_method(),
        ai_methods::VIDEO_GENERATE
    );
    let call = AiccCall::from_method_and_params(
        ai_methods::HELPER_VIDEO_GENERATE,
        json!({"logical_model": "video.generate", "prompt": "a fox"}),
    )
    .unwrap();
    assert_eq!(call.api_type(), Some(ApiType::VideoGenerate));
    assert_eq!(call.exact_model(), None);
    assert!(VideoEditRequest::from_json(json!({
        "exact_model": "m@p",
        "video": {"kind": "url", "url": "https://example.com/a.mp4"},
        "prompt": " "
    }))
    .is_err());
}

#[test]
fn generated_video_reports_deviations_from_explicit_output() {
    let mut video = GeneratedVideo::new(
        ResourceRef::url("https://example.com/out.mp4".into(), None),
        Some("video/mp4".into()),
    );
    video.provider_metadata.duration_seconds = Some(6.0);
    video.measured_metadata = Some(VideoMetadata {
        duration_seconds: Some(6.04),
        width: Some(1280),
        height: Some(720),
        fps: Some(24.0),
        has_audio: Some(true),
    });
    video.record_deviations(Some(&VideoOutputSpec {
        duration_seconds: Some(6.0),
        size: Some(VideoSize::short_edge(1080)),
        audio: Some(VideoAudioMode::None),
        ..Default::default()
    }));
    let fields = video
        .deviations
        .iter()
        .map(|deviation| (deviation.field.as_str(), deviation.source))
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec![
            ("size", VideoMetadataSource::Measured),
            ("audio", VideoMetadataSource::Measured)
        ]
    );
    let value = serde_json::to_value(&video).unwrap();
    assert_eq!(
        serde_json::from_value::<GeneratedVideo>(value).unwrap(),
        video
    );
}
