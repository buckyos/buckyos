use super::*;
use buckyos_api::{AiUsage, AiccCall, ApiType, AudioTextToSpeechRequest, VoiceSpec};
use bytes::Bytes;
use reqwest::{
    header::{HeaderMap, CONTENT_TYPE},
    StatusCode,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

fn response(value: Value) -> HttpResponse {
    HttpResponse {
        status: StatusCode::OK,
        headers: HeaderMap::new(),
        body: Bytes::from(serde_json::to_vec(&value).unwrap()),
        request_id: "review-test".into(),
        retry_after: None,
    }
}

fn registry() -> CodecRegistry {
    let mut registry = CodecRegistry::default();
    for (descriptor, codecs) in [
        doubao_media_adapter(),
        qwen_media_adapter(),
        glm_media_adapter(),
        minimax_media_adapter(),
        doubao_speech_adapter(),
        openai_responses_adapter(),
    ] {
        registry.register_codecs(descriptor, codecs).unwrap();
    }
    registry
}

fn cost(price: Value, usage: &AiUsage) -> f64 {
    crate::execution::PinnedPricingSnapshot::from_pricing(
        &serde_json::from_value(price).unwrap(),
        None,
        SystemTime::now(),
    )
    .unwrap()
    .unwrap()
    .completion_cost(usage)
    .unwrap()
    .amount
}

#[tokio::test]
async fn native_failures_preserve_business_errors_and_pending_poll_intervals() {
    let registry = registry();
    for (adapter, operation, failed, pending, code, interval) in [
        (
            "doubao-media",
            "ark.contents.generate",
            json!({"status":"failed", "error":{"code":"OutputVideoSensitiveContentDetected","message":"blocked media"}}),
            json!({"status":"running"}),
            "OutputVideoSensitiveContentDetected",
            3,
        ),
        (
            "qwen-media",
            "dashscope.video_synthesis",
            json!({"output":{"task_status":"FAILED","code":"DataInspectionFailed","message":"blocked media"}}),
            json!({"output":{"task_status":"RUNNING"}}),
            "DataInspectionFailed",
            5,
        ),
        (
            "glm-media",
            "videos.generate",
            json!({"task_status":"FAIL","error":{"code":"1301","message":"blocked media"}}),
            json!({"task_status":"PROCESSING"}),
            "1301",
            2,
        ),
        (
            "minimax-media",
            "video_generation.v2.create",
            json!({"task":{"status":"failed","error":{"code":"content_rejected","message":"blocked media"}}}),
            json!({"task":{"status":"running"}}),
            "content_rejected",
            2,
        ),
    ] {
        let error = registry
            .decode_native(
                adapter,
                operation,
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Status,
                response(failed),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::ProviderRejected);
        assert_eq!(error.provider_code.as_deref(), Some(code));
        assert_eq!(error.message, "blocked media");
        assert!(!error.retry_same_model());
        let NativeTaskOutput::Status { retry_after, .. } = registry
            .decode_native(
                adapter,
                operation,
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Status,
                response(pending.clone()),
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(retry_after, Some(Duration::from_secs(interval)));
        let mut pending = response(pending);
        pending.retry_after = Some(Duration::from_secs(17));
        let NativeTaskOutput::Status { retry_after, .. } = registry
            .decode_native(
                adapter,
                operation,
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Status,
                pending,
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(retry_after, Some(Duration::from_secs(17)));
    }
}

#[tokio::test]
async fn video_result_usage_matches_billable_dimensions() {
    let registry = registry();
    for (adapter, operation, value, price, expected) in [
        (
            "doubao-media",
            "ark.contents.generate",
            json!({"content":{"video_url":"https://example.com/result.mp4"},"usage":{"completion_tokens":100,"total_tokens":100}}),
            json!({"currency":"CNY","output_token":0.1}),
            10.0,
        ),
        (
            "qwen-media",
            "dashscope.video_synthesis",
            json!({"output":{"video_url":"https://example.com/result.mp4"},"usage":{"duration":5,"video_duration":0}}),
            json!({"currency":"CNY","unit":"video_second","amount":0.6}),
            3.0,
        ),
        (
            "minimax-media",
            "video_generation.v2.create",
            json!({"task":{"content":{"url":"https://example.com/result.mp4"},"usage":{"output_seconds":5}}}),
            json!({"currency":"USD","unit":"video_second","amount":0.2}),
            1.0,
        ),
    ] {
        let NativeTaskOutput::Result(output) = registry
            .decode_native(
                adapter,
                operation,
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Result,
                response(value),
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(cost(price, output.usage.as_ref().unwrap()), expected);
    }
    let NativeTaskOutput::Status { result_usage, .. } = registry
        .decode_native(
            "openai-responses",
            "videos.create",
            ApiType::VideoTextToVideo,
            NativeTaskOperation::Status,
            response(json!({"id":"video_1","status":"completed","seconds":"8"})),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(result_usage.unwrap().video_seconds, Some(8.0));
}

#[tokio::test]
async fn qwen_partial_images_and_seedream_images_are_billed_per_successful_image() {
    let registry = registry();
    let NativeTaskOutput::Result(output) = registry.decode_native("qwen-media", "dashscope.image_synthesis", ApiType::ImageTextToImage, NativeTaskOperation::Result, response(json!({"output":{"results":[{"url":"https://example.com/result.png"},{"code":"DataInspectionFailed","message":"blocked"}]}}))).await.unwrap() else { panic!() };
    assert_eq!(output.artifacts.len(), 1);
    assert_eq!(
        cost(
            json!({"currency":"CNY","unit":"image","amount":0.2}),
            output.usage.as_ref().unwrap()
        ),
        0.2
    );
    let ProtocolExecution::Immediate(output) = registry
        .decode(
            "doubao-media",
            "ark.images.generate",
            ApiType::ImageTextToImage,
            response(json!({"data":[{"url":"https://example.com/result.png"}]})),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(output.usage.unwrap().image_units, Some(1));
}

#[tokio::test]
async fn glm_query_does_not_require_submit_id_and_minimax_cancelled_is_terminal() {
    let registry = registry();
    let NativeTaskOutput::Status {
        state, result_ref, ..
    } = registry
        .decode_native(
            "glm-media",
            "videos.generate",
            ApiType::VideoTextToVideo,
            NativeTaskOperation::Status,
            response(json!({"request_id":"request_1","task_status":"SUCCESS"})),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(state, NativeTaskState::Succeeded);
    assert_eq!(result_ref, None);
    let NativeTaskOutput::Status { state, .. } = registry
        .decode_native(
            "minimax-media",
            "video_generation.v2.create",
            ApiType::VideoTextToVideo,
            NativeTaskOperation::Status,
            response(json!({"task":{"status":"cancelled"}})),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(state, NativeTaskState::Cancelled);
}

#[tokio::test]
async fn tts_uses_requested_format_final_usage_and_http_error_status() {
    let registry = registry();
    for (media_type, mime) in [
        ("audio/mpeg", "audio/mpeg"),
        ("audio/wav", "audio/wav"),
        ("audio/pcm", "audio/pcm"),
        ("audio/opus", "audio/ogg"),
    ] {
        let mut request =
            AudioTextToSpeechRequest::new("test", "hello".into(), VoiceSpec::default());
        request.output = Some(serde_json::from_value(json!({"media_type":media_type})).unwrap());
        let input = CodecInput {
            canonical_request: AiccCall::AudioTextToSpeech(request),
            resolved_parameters: BTreeMap::new(),
        };
        let mut wire = response(Value::Null);
        wire.headers
            .insert(CONTENT_TYPE, "text/plain".parse().unwrap());
        wire.body = Bytes::from_static(
            b"{\"code\":0,\"data\":\"SUQz\"}\n{\"code\":20000000,\"usage\":{\"text_words\":5}}\n",
        );
        let ProtocolExecution::Immediate(output) = registry
            .decode_with_input(
                "doubao-speech",
                "tts.unidirectional",
                ApiType::AudioTextToSpeech,
                wire,
                &input,
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(output.artifacts[0].mime.as_deref(), Some(mime));
        assert_eq!(output.usage.unwrap().characters, Some(5));
    }
    for (status, kind) in [
        (StatusCode::UNAUTHORIZED, ProtocolErrorKind::Authentication),
        (StatusCode::TOO_MANY_REQUESTS, ProtocolErrorKind::Transport),
        (StatusCode::BAD_GATEWAY, ProtocolErrorKind::Transport),
    ] {
        let mut wire = response(Value::Null);
        wire.status = status;
        wire.body = Bytes::from_static(b"<html>upstream error</html>");
        let error = registry
            .decode(
                "doubao-speech",
                "tts.unidirectional",
                ApiType::AudioTextToSpeech,
                wire,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.http_status, Some(status.as_u16()));
    }
}

fn context() -> CodecContext {
    CodecContext {
        base_url: "https://example.com/v1".into(),
        state_coordinate: buckyos_api::ProviderStateCoordinate {
            provider_profile_id: "test".into(),
            adapter_type: "test".into(),
            origin_provider: "test".into(),
            origin_model: "test".into(),
        },
        credential: Some(ResolvedCredential::bearer("secret://test", "test-key").unwrap()),
        resources: BTreeMap::new(),
        limits: CodecLimits {
            request_timeout: Duration::from_secs(10),
            max_request_bytes: 1048576,
            max_response_bytes: 1048576,
        },
    }
}

#[test]
fn media_request_lowering_uses_integer_duration_and_provider_sizes() {
    let registry = registry();
    let context = context();
    for (adapter, operation, model) in [
        (
            "doubao-media",
            "ark.contents.generate",
            "doubao-seedance-2.0",
        ),
        ("qwen-media", "dashscope.video_synthesis", "wan2.6-t2v"),
        ("glm-media", "videos.generate", "cogvideox-3"),
    ] {
        let mut request = buckyos_api::VideoTextToVideoRequest::new("ignored", "cat".into());
        request.duration_seconds = Some(5.0);
        if adapter == "qwen-media" {
            request.aspect_ratio = Some("16:9".into());
            request.resolution = Some("720p".into());
        }
        let mut input = CodecInput {
            canonical_request: AiccCall::VideoTextToVideo(request),
            resolved_parameters: BTreeMap::from([("provider_model_id".into(), json!(model))]),
        };
        let encode = |input: &CodecInput| {
            registry.encode_native(
                adapter,
                operation,
                ApiType::VideoTextToVideo,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Submit,
                    remote_task_id: None,
                    codec_input: Some(input),
                    resolved_parameters: &input.resolved_parameters,
                    context: &context,
                },
            )
        };
        let HttpBody::Json(body) = encode(&input).unwrap().body else {
            panic!()
        };
        if adapter == "qwen-media" {
            assert_eq!(body["parameters"]["duration"].as_u64(), Some(5));
            assert_eq!(body["parameters"]["size"], "1280*720");
            assert!(body["parameters"].get("ratio").is_none());
        } else {
            assert_eq!(body["duration"].as_u64(), Some(5));
        }
        if let AiccCall::VideoTextToVideo(request) = &mut input.canonical_request {
            request.duration_seconds = Some(5.5);
        }
        assert!(encode(&input).is_err());
    }
    let mut image = buckyos_api::TextToImageInvokeRequest::new("ignored", "cat");
    image.aspect_ratio = Some("16:9".into());
    let input = CodecInput {
        canonical_request: AiccCall::ImagesGenerate(image),
        resolved_parameters: BTreeMap::from([(
            "provider_model_id".into(),
            json!("doubao-seedream-5.0-lite"),
        )]),
    };
    let HttpBody::Json(body) = registry
        .encode(
            "doubao-media",
            "ark.images.generate",
            ApiType::ImageTextToImage,
            &input,
            &context,
        )
        .unwrap()
        .body
    else {
        panic!()
    };
    assert_eq!(body["size"], "2848x1600");
    let mut image = buckyos_api::TextToImageInvokeRequest::new("ignored", "cat");
    image.size = Some("1024x1024".into());
    let input = CodecInput {
        canonical_request: AiccCall::ImagesGenerate(image),
        resolved_parameters: BTreeMap::from([(
            "provider_model_id".into(),
            json!("wanx2.1-t2i-turbo"),
        )]),
    };
    let HttpBody::Json(body) = registry
        .encode_native(
            "qwen-media",
            "dashscope.image_synthesis",
            ApiType::ImageTextToImage,
            &NativeTaskInput {
                operation: NativeTaskOperation::Submit,
                remote_task_id: None,
                codec_input: Some(&input),
                resolved_parameters: &input.resolved_parameters,
                context: &context,
            },
        )
        .unwrap()
        .body
    else {
        panic!()
    };
    assert_eq!(body["parameters"]["size"], "1024*1024");
}
