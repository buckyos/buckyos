use super::test_support::{channel_contract, clip_facts, image_facts, model_contract};
use super::*;
use buckyos_api::{ResourceRef, TimeRange, VideoCondition, VideoGenerateRequest};

fn url(name: &str) -> ResourceRef {
    ResourceRef::url(format!("https://media.example/{name}"), None)
}

fn request(prompt: &str, conditions: Vec<VideoCondition>) -> VideoGenerateRequest {
    let mut request = VideoGenerateRequest::new("m@p", Some(prompt.into()));
    request.conditions = conditions;
    request
}

fn reference_video(id: &str, include_audio: bool, time_range: Option<TimeRange>) -> VideoCondition {
    VideoCondition::ReferenceVideo {
        id: id.into(),
        resource: url("clip.mp4"),
        time_range,
        include_audio,
    }
}

#[test]
fn every_builtin_video_contract_is_valid() {
    let mut checked = 0;
    for file in crate::settings::load_builtin_metadata().unwrap() {
        let value: Value = serde_json::from_slice(&file.contents).unwrap();
        let entries = value["models"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(value["patterns"].as_array().into_iter().flatten());
        for entry in entries {
            for contract in [
                entry
                    .get("capabilities")
                    .and_then(|capabilities| capabilities.get(VIDEO_CONTRACT_CAPABILITY)),
                entry.get(VIDEO_CONTRACT_CAPABILITY),
            ]
            .into_iter()
            .flatten()
            {
                let contract: VideoGenerationContract = serde_json::from_value(contract.clone())
                    .unwrap_or_else(|error| panic!("{} {}: {error}", file.catalog_id, entry["id"]));
                contract
                    .validate()
                    .unwrap_or_else(|error| panic!("{} {}: {error}", file.catalog_id, entry["id"]));
                checked += 1;
            }
            let declares_generate = entry["api_types"]
                .as_array()
                .is_some_and(|types| types.iter().any(|api| api == "video.generate"));
            assert!(
                !declares_generate
                    || entry["capabilities"]
                        .get(VIDEO_CONTRACT_CAPABILITY)
                        .is_some(),
                "{} {} declares video.generate without a contract",
                file.catalog_id,
                entry["id"]
            );
        }
    }
    assert!(checked >= 28, "only {checked} contracts were checked");
}

#[test]
fn plan_records_order_tokens_defaults_and_pricing_dimensions() {
    let contract = model_contract("minimax", "MiniMax-H3");
    let request = request(
        "@{b} meets @{a} and @{c}",
        vec![
            VideoCondition::ReferenceImage {
                id: "a".into(),
                resource: url("a.png"),
            },
            reference_video("v", true, None),
            VideoCondition::ReferenceImage {
                id: "b".into(),
                resource: url("b.png"),
            },
            VideoCondition::ReferenceImage {
                id: "c".into(),
                resource: url("c.png"),
            },
        ],
    );
    let facts = VideoMaterialFacts::from([("v".to_string(), clip_facts(4.0, true, true))]);
    let plan = plan_video(&contract, &request, &facts).unwrap();
    assert_eq!(plan.scenario, VideoScenario::References);
    assert_eq!(
        plan.submitted_prompt.as_deref(),
        Some("reference image 2 meets reference image 1 and reference image 3")
    );
    assert_eq!(
        plan.inputs
            .iter()
            .map(|input| (input.condition_id.as_str(), input.ordinal))
            .collect::<Vec<_>>(),
        vec![("a", 1), ("v", 1), ("b", 2), ("c", 3)]
    );
    assert_eq!(plan.duration_seconds, Some(4.0));
    assert_eq!(plan.duration_source, Some(ValueSource::Default));
    assert_eq!(plan.resolution.as_deref(), Some("768P"));
    assert_eq!(plan.aspect_ratio, None);
    assert_eq!(plan.audio, buckyos_api::VideoAudioMode::Generate);
    let dimensions = plan.pricing_dimensions();
    assert_eq!(dimensions["/video/resolution"], "768P");
    assert_eq!(dimensions["/video/reference_images"], 3);
    assert!(plan.inputs[1].include_audio);
}

#[test]
fn clips_are_checked_against_measured_lengths_and_audio() {
    let contract = model_contract("minimax", "MiniMax-H3");
    let short = request("a", vec![reference_video("v", false, None)]);
    let facts = VideoMaterialFacts::from([("v".to_string(), clip_facts(1.0, true, false))]);
    assert!(plan_video(&contract, &short, &facts)
        .unwrap_err()
        .message
        .contains("outside"));

    let audio = request("a", vec![reference_video("v", true, None)]);
    let silent = VideoMaterialFacts::from([("v".to_string(), clip_facts(5.0, true, false))]);
    assert_eq!(
        plan_video(&contract, &audio, &silent).unwrap_err().code,
        AiccErrorCode::ResourceInvalid
    );

    let mut voices = vec![reference_video("v", true, None)];
    voices.push(VideoCondition::ReferenceAudio {
        id: "s".into(),
        resource: url("s.wav"),
        time_range: None,
    });
    let budget = VideoMaterialFacts::from([
        ("v".to_string(), clip_facts(10.0, true, true)),
        ("s".to_string(), clip_facts(8.0, false, true)),
    ]);
    // The video's own audio counts towards the 15 s audio budget.
    assert!(plan_video(&contract, &request("a", voices), &budget)
        .unwrap_err()
        .message
        .contains("audio references total"));
}

#[test]
fn time_ranges_must_cover_the_whole_clip_until_trimming_exists() {
    let contract = model_contract("minimax", "MiniMax-H3");
    let facts = VideoMaterialFacts::from([("v".to_string(), clip_facts(6.0, true, false))]);
    let whole = request(
        "a",
        vec![reference_video(
            "v",
            false,
            Some(TimeRange {
                start_seconds: 0.0,
                end_seconds: 6.0,
            }),
        )],
    );
    assert!(plan_video(&contract, &whole, &facts).is_ok());
    let part = request(
        "a",
        vec![reference_video(
            "v",
            false,
            Some(TimeRange {
                start_seconds: 1.0,
                end_seconds: 4.0,
            }),
        )],
    );
    let error = plan_video(&contract, &part, &facts).unwrap_err();
    assert_eq!(error.code, AiccErrorCode::UnsupportedOperation);
    assert_eq!(error.details.unwrap()["condition_id"], "v");
    let past = request(
        "a",
        vec![reference_video(
            "v",
            false,
            Some(TimeRange {
                start_seconds: 0.0,
                end_seconds: 9.0,
            }),
        )],
    );
    assert_eq!(
        plan_video(&contract, &past, &facts).unwrap_err().code,
        AiccErrorCode::InvalidRequest
    );
}

#[test]
fn routing_requirements_and_output_rules_are_enforced() {
    let channel = channel_contract(Some(3));
    let mut requirement = VideoGenerationRequirement {
        prompt: true,
        reference_images: 4,
        ..Default::default()
    };
    assert!(!unmet_requirements(&channel, &requirement, &ClipSummary::default()).is_empty());
    requirement.reference_images = 3;
    assert!(unmet_requirements(&channel, &requirement, &ClipSummary::default()).is_empty());

    let mut tall = request("a", Vec::new());
    tall.output = Some(buckyos_api::VideoOutputSpec {
        size: Some(buckyos_api::VideoSize::short_edge(1080)),
        duration_seconds: Some(6.0),
        ..Default::default()
    });
    assert!(plan_video(&channel, &tall, &VideoMaterialFacts::new())
        .unwrap_err()
        .message
        .contains("requires durations"));

    let lite_only = VideoGenerationRequirement {
        prompt: true,
        keyframes: [KeyframePosition::End].into(),
        ..Default::default()
    };
    assert!(!unmet_requirements(&channel, &lite_only, &ClipSummary::default()).is_empty());

    let vidu = model_contract("glm", "vidu2-start-end");
    let start = VideoGenerationRequirement {
        keyframes: [KeyframePosition::Start].into(),
        ..Default::default()
    };
    assert!(!unmet_requirements(&vidu, &start, &ClipSummary::default()).is_empty());
    let both = VideoGenerationRequirement {
        keyframes: [KeyframePosition::Start, KeyframePosition::End].into(),
        ..Default::default()
    };
    assert!(unmet_requirements(&vidu, &both, &ClipSummary::default()).is_empty());
    let text = VideoGenerationRequirement {
        prompt: true,
        ..Default::default()
    };
    assert!(!unmet_requirements(&vidu, &text, &ClipSummary::default()).is_empty());
}

#[test]
fn exact_sizes_follow_short_edge_and_ratio() {
    let wan = model_contract("qwen", "wan2.6-t2v");
    let mut portrait = request("a", Vec::new());
    portrait.output = Some(buckyos_api::VideoOutputSpec {
        size: Some(buckyos_api::VideoSize::short_edge(1080)),
        aspect_ratio: Some("9:16".into()),
        ..Default::default()
    });
    let plan = plan_video(&wan, &portrait, &VideoMaterialFacts::new()).unwrap();
    assert_eq!(
        plan.size,
        Some(buckyos_api::VideoExactSize {
            width: 1080,
            height: 1920
        })
    );
    let mut ratio_only = request("a", Vec::new());
    ratio_only.output = Some(buckyos_api::VideoOutputSpec {
        aspect_ratio: Some("1:1".into()),
        ..Default::default()
    });
    assert_eq!(
        plan_video(&wan, &ratio_only, &VideoMaterialFacts::new())
            .unwrap()
            .size,
        Some(buckyos_api::VideoExactSize {
            width: 624,
            height: 624
        })
    );
    let mut unknown = request("a", Vec::new());
    unknown.output = Some(buckyos_api::VideoOutputSpec {
        size: Some(buckyos_api::VideoSize::short_edge(600)),
        ..Default::default()
    });
    assert!(plan_video(&wan, &unknown, &VideoMaterialFacts::new()).is_err());
}

#[test]
fn image_limits_apply_to_measured_images_only() {
    let contract = model_contract("minimax", "MiniMax-H3");
    let keyframe = request(
        "a",
        vec![VideoCondition::Keyframe {
            id: "k".into(),
            resource: url("k.png"),
            position: KeyframePosition::Start,
        }],
    );
    assert!(plan_video(&contract, &keyframe, &VideoMaterialFacts::new()).is_ok());
    let tiny = VideoMaterialFacts::from([("k".to_string(), image_facts(100, 100))]);
    assert_eq!(
        plan_video(&contract, &keyframe, &tiny).unwrap_err().code,
        AiccErrorCode::UnsupportedOperation
    );
    let wide = VideoMaterialFacts::from([("k".to_string(), image_facts(3000, 1000))]);
    assert!(plan_video(&contract, &keyframe, &wide).is_err());
}

#[test]
fn discovered_facts_replace_channel_output_and_unknown_tiers_are_dropped() {
    let mut capabilities = BTreeMap::new();
    apply_discovered_video_facts(
        &mut capabilities,
        &DiscoveredVideoFacts {
            durations: vec![5.0, 10.0],
            resolutions: vec!["720p".into(), "1K".into()],
            aspect_ratios: vec!["16:9".into(), "bogus".into()],
            sizes: vec!["1280x720".into()],
            keyframes: vec![KeyframePosition::Start],
            generate_audio: Some(false),
            seed: Some(true),
        },
    );
    let contract = VideoGenerationContract::from_capabilities(&capabilities).unwrap();
    assert_eq!(
        contract.output.resolutions,
        vec![ResolutionTier {
            name: "720p".into(),
            short_edge: 720
        }]
    );
    assert_eq!(contract.output.aspect_ratios, vec!["16:9".to_string()]);
    assert_eq!(contract.output.audio, AudioOutput::Unknown);
    assert!(contract.seed);
    assert!(contract.references.image.is_none());
    assert_eq!(
        contract.keyframes.unwrap().positions,
        [KeyframePosition::Start].into()
    );
    assert_eq!(short_edge_of_resolution("2K"), Some(1440));
    assert_eq!(short_edge_of_resolution("768P"), Some(768));
    assert_eq!(short_edge_of_resolution("1K"), None);
}

#[test]
fn audio_modes_follow_the_contract() {
    let seedance = model_contract("doubao", "doubao-seedance-2.0");
    let mut silent = request("a", Vec::new());
    silent.output = Some(buckyos_api::VideoOutputSpec {
        audio: Some(buckyos_api::VideoAudioMode::None),
        ..Default::default()
    });
    let plan = plan_video(&seedance, &silent, &VideoMaterialFacts::new()).unwrap();
    assert_eq!(plan.generate_audio, Some(false));
    let vidu = model_contract("glm", "viduq1-text");
    let mut text = request("a", Vec::new());
    assert!(plan_video(&vidu, &text, &VideoMaterialFacts::new()).is_ok());
    text.output = Some(buckyos_api::VideoOutputSpec {
        audio: Some(buckyos_api::VideoAudioMode::Generate),
        ..Default::default()
    });
    assert!(plan_video(&vidu, &text, &VideoMaterialFacts::new()).is_err());
}
