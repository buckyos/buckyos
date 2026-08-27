mod common;

use aicc::claude::{ClaudeInstanceConfig, ClaudeProvider};
use aicc::fal::{FalInstanceConfig, FalProvider};
use aicc::gemini::{GoogleGeminiInstanceConfig, GoogleGeminiProvider};
use aicc::openai::{OpenAIInstanceConfig, OpenAIProvider};
use aicc::{
    InvokeCtx, Provider, ProviderStartResult, ResolvedRequest, TaskEventKind, TaskEventSinkFactory,
};
use buckyos_api::{ai_methods, AiResponse, Capability, ResourceRef};
use common::*;
use image::{DynamicImage, ImageFormat};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

fn png_image(width: u32, height: u32) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(width, height)
        .write_to(&mut output, ImageFormat::Png)
        .expect("encode test PNG");
    output.into_inner()
}

async fn wait_for_final_summary(sink_factory: &CollectingSinkFactory, task_id: &str) -> AiResponse {
    let final_event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(event) = sink_factory
                .events_for(task_id)
                .into_iter()
                .find(|event| matches!(event.kind, TaskEventKind::Final))
            {
                break event;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background task should emit final event");
    serde_json::from_value(
        final_event
            .data
            .and_then(|data| data.get("summary").cloned())
            .expect("final event should include summary"),
    )
    .expect("final event summary should be a valid AiResponse")
}

fn openai_provider(base_url: String, timeout_ms: u64) -> OpenAIProvider {
    OpenAIProvider::new(
        OpenAIInstanceConfig {
            provider_instance_name: "openai-test".to_string(),
            provider_type: "cloud_api".to_string(),
            provider_driver: "openai".to_string(),
            api_token: "token".to_string(),
            base_url,
            timeout_ms,
        },
        "token",
    )
    .expect("openai provider")
}

#[tokio::test]
async fn adapter_openai_vision_ocr_uses_multimodal_model() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"id":"r1","status":"completed","output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"first line\nsecond line","annotations":[]}]}],"usage":{"input_tokens":10,"output_tokens":4,"total_tokens":14}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(b"image"),
    });
    request.capability = Capability::Vision;
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({ "include_layout": true }));
    request.payload.options = None;

    let result = provider
        .start(
            InvokeCtx::default(),
            "gpt-5".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VISION_OCR, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("openai vision OCR should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text_content(), "first line\nsecond line");
            assert_eq!(
                summary
                    .extra
                    .as_ref()
                    .and_then(|extra| extra.pointer("/ocr/text"))
                    .and_then(|text| text.as_str()),
                Some("first line\nsecond line")
            );
        }
        other => panic!("expected immediate OCR response, got {:?}", other),
    }
}

#[tokio::test]
async fn adapter_openai_vision_caption_uses_multimodal_model() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"id":"r1","status":"completed","output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"A cat sitting by a window.","annotations":[]}]}],"usage":{"input_tokens":10,"output_tokens":6,"total_tokens":16}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(b"image"),
    });
    request.capability = Capability::Vision;
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({ "style": "short" }));
    request.payload.options = None;

    let result = provider
        .start(
            InvokeCtx::default(),
            "gpt-5".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VISION_CAPTION, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("openai vision caption should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text_content(), "A cat sitting by a window.");
            assert_eq!(
                summary
                    .extra
                    .as_ref()
                    .and_then(|extra| extra.pointer("/captions/text"))
                    .and_then(|text| text.as_str()),
                Some("A cat sitting by a window.")
            );
        }
        other => panic!("expected immediate caption response, got {:?}", other),
    }
}

#[tokio::test]
async fn adapter_openai_video_img2video_returns_downloaded_artifact() {
    let video_bytes = b"openai-video";
    let base_url = spawn_fake_http_server(vec![
        MockHttpReply {
            status_code: 200,
            body: r#"{"id":"video_1","status":"queued","model":"sora-2","progress":0}"#.to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: r#"{"id":"video_1","status":"completed","model":"sora-2","progress":100}"#
                .to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: String::from_utf8_lossy(video_bytes).to_string(),
            content_type: "video/mp4",
            delay_ms: 0,
        },
    ])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(png_image(320, 640).as_slice()),
    });
    request.payload.options = Some(serde_json::json!({ "response_format": "base64" }));
    let task_id = "openai-video-task";
    let sink_factory = Arc::new(CollectingSinkFactory::new());
    let sink = sink_factory.build(&InvokeCtx::default(), task_id);
    let result = provider
        .start(
            InvokeCtx {
                task_id: Some(task_id.to_string()),
                ..Default::default()
            },
            "sora-2".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VIDEO_IMG2VIDEO, request),
            sink,
        )
        .await
        .expect("openai img2video should succeed");
    assert!(matches!(result, ProviderStartResult::Started));

    let summary = wait_for_final_summary(sink_factory.as_ref(), task_id).await;
    let artifacts = summary.artifacts();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].mime.as_deref(), Some("video/mp4"));
    match &artifacts[0].resource {
        ResourceRef::Base64 { data_base64, .. } => {
            assert_eq!(data_base64, "b3BlbmFpLXZpZGVv");
        }
        other => panic!("unexpected video artifact: {:?}", other),
    }
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.request_units),
        Some(1)
    );
    assert_eq!(summary.cost.as_ref().map(|cost| cost.amount), Some(0.4));
}

#[tokio::test]
async fn adapter_gemini_video_img2video_returns_downloaded_artifact() {
    let video_bytes = b"gemini-video";
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![
        MockHttpReply {
            status_code: 200,
            body: r#"{"name":"operations/op1","done":false}"#.to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: r#"{"name":"operations/op1","done":true,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"video.mp4"}}]}}}"#.to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: String::from_utf8_lossy(video_bytes).to_string(),
            content_type: "video/mp4",
            delay_ms: 0,
        },
    ])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(b"image"),
    });
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({
        "prompt": "animate the image",
        "duration": 8,
        "output": { "resource_format": "named_object" }
    }));
    request.payload.options = Some(serde_json::json!({ "response_format": "base64" }));
    let task_id = "gemini-video-task";
    let sink_factory = Arc::new(CollectingSinkFactory::new());
    let sink = sink_factory.build(&InvokeCtx::default(), task_id);
    let result = provider
        .start(
            InvokeCtx {
                task_id: Some(task_id.to_string()),
                ..Default::default()
            },
            "veo-3.1-generate-preview".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VIDEO_IMG2VIDEO, request),
            sink,
        )
        .await
        .expect("gemini img2video should succeed");
    assert!(matches!(result, ProviderStartResult::Started));

    let request_body = captured_requests
        .lock()
        .expect("captured requests lock")
        .first()
        .cloned()
        .expect("video request should be captured");
    assert_eq!(
        request_body.pointer("/instances/0/prompt"),
        Some(&serde_json::json!("animate the image"))
    );
    assert_eq!(
        request_body.pointer("/instances/0/image"),
        Some(&serde_json::json!({
            "mimeType": "image/png",
            "bytesBase64Encoded": openai_b64(b"image")
        }))
    );
    assert_eq!(
        request_body.pointer("/parameters/durationSeconds"),
        Some(&serde_json::json!(8))
    );
    assert!(request_body.to_string().find("inlineData").is_none());

    let summary = wait_for_final_summary(sink_factory.as_ref(), task_id).await;
    let artifacts = summary.artifacts();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].mime.as_deref(), Some("video/mp4"));
    match &artifacts[0].resource {
        ResourceRef::Base64 { data_base64, .. } => {
            assert_eq!(data_base64, "Z2VtaW5pLXZpZGVv");
        }
        other => panic!("unexpected video artifact: {:?}", other),
    }
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.request_units),
        Some(1)
    );
    assert_eq!(summary.cost.as_ref().map(|cost| cost.amount), Some(0.5));
    assert_eq!(
        summary
            .extra
            .as_ref()
            .and_then(|extra| extra.get("continuation_handle"))
            .and_then(|value| value.as_str()),
        Some("video.mp4")
    );
}

#[tokio::test]
async fn adapter_gemini_video_extend_uses_generated_video_uri() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![
        MockHttpReply {
            status_code: 200,
            body: r#"{"name":"operations/extend","done":true,"response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"extended.mp4"}}]}}}"#.to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: "extended-video".to_string(),
            content_type: "video/mp4",
            delay_ms: 0,
        },
    ])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "video/mp4".to_string(),
        data_base64: openai_b64(b"previous-video"),
    });
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({
        "prompt": "continue into the garden",
        "continuation_handle": "https://generativelanguage.googleapis.com/v1beta/files/generated-video",
        "resolution": "720p"
    }));
    request.payload.options = Some(serde_json::json!({ "response_format": "base64" }));
    let task_id = "gemini-extend-task";
    let sink_factory = Arc::new(CollectingSinkFactory::new());
    let result = provider
        .start(
            InvokeCtx {
                task_id: Some(task_id.to_string()),
                ..Default::default()
            },
            "veo-3.1-generate-preview".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VIDEO_EXTEND, request),
            sink_factory.build(&InvokeCtx::default(), task_id),
        )
        .await
        .expect("gemini video extend should start");
    assert!(matches!(result, ProviderStartResult::Started));

    let request_body = captured_requests
        .lock()
        .expect("captured requests lock")
        .first()
        .cloned()
        .expect("extend request should be captured");
    assert_eq!(
        request_body.pointer("/instances/0/video"),
        Some(&serde_json::json!({
            "uri": "https://generativelanguage.googleapis.com/v1beta/files/generated-video"
        }))
    );
    assert!(request_body
        .pointer("/instances/0/continuation_handle")
        .is_none());
    assert_eq!(
        request_body.pointer("/parameters/resolution"),
        Some(&serde_json::json!("720p"))
    );
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(event) = sink_factory
                .events_for(task_id)
                .into_iter()
                .find(|event| matches!(event.kind, TaskEventKind::Final | TaskEventKind::Error))
            {
                break event;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("extend task should terminate");
    assert!(
        matches!(terminal.kind, TaskEventKind::Final),
        "extend task failed: {:?}",
        terminal.data
    );
}

#[tokio::test]
async fn adapter_gemini_video_edit_uses_interactions_protocol() {
    let output = openai_b64(b"edited-video");
    let (base_url, captured_requests) =
        spawn_fake_http_server_with_requests(vec![MockHttpReply {
            status_code: 200,
            body: format!(
                r#"{{"id":"interaction-edit","status":"completed","steps":[{{"type":"model_output","content":[{{"type":"video","mime_type":"video/mp4","data":"{}"}}]}}]}}"#,
                output
            ),
            content_type: "application/json",
            delay_ms: 0,
        }])
        .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "video/mp4".to_string(),
        data_base64: openai_b64(b"source-video"),
    });
    request.payload.text = Some("make the video warmer".to_string());
    request.payload.options = Some(serde_json::json!({
        "provider_options": { "protocol": "interactions" }
    }));

    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-omni-flash-preview".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VIDEO_VIDEO2VIDEO, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini video edit should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini video edit should complete immediately");
    };
    let artifacts = summary.artifacts();
    assert_eq!(artifacts.len(), 1);
    assert!(matches!(artifacts[0].resource, ResourceRef::Base64 { .. }));

    let request_body = captured_requests
        .lock()
        .expect("captured requests lock")
        .first()
        .cloned()
        .expect("interactions request should be captured");
    assert_eq!(
        request_body.pointer("/input/0/type"),
        Some(&serde_json::json!("user_input"))
    );
    assert_eq!(
        request_body.pointer("/input/0/content/0"),
        Some(&serde_json::json!({
            "type": "video",
            "mime_type": "video/mp4",
            "data": openai_b64(b"source-video")
        }))
    );
    assert_eq!(
        request_body.pointer("/input/0/content/1"),
        Some(&serde_json::json!({
            "type": "text",
            "text": "make the video warmer"
        }))
    );
    assert_eq!(
        request_body.pointer("/generation_config/video_config/task"),
        Some(&serde_json::json!("edit"))
    );
}

#[tokio::test]
async fn adapter_gemini_img2img_uses_generate_content_inline_data() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![MockHttpReply {
        status_code: 200,
        body: format!(
            r#"{{"candidates":[{{"content":{{"parts":[{{"inlineData":{{"mimeType":"image/png","data":"{}"}}}}]}}}}]}}"#,
            openai_b64(b"edited")
        ),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = request_with_resource(ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(b"image"),
    });
    request.capability = Capability::Image;
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({
        "prompt": "make it blue",
        "strength": 0.7,
        "output": { "media_type": "image/png" }
    }));
    request.payload.options = None;

    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-3-pro-image-preview".to_string(),
            ResolvedRequest::new_with_method(ai_methods::IMAGE_IMG2IMG, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini img2img should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini img2img should complete immediately");
    };
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.request_units),
        Some(1)
    );

    let request_body = captured_requests
        .lock()
        .expect("captured requests lock")
        .first()
        .cloned()
        .expect("img2img request should be captured");
    assert_eq!(
        request_body.pointer("/contents/0/parts/0/inlineData"),
        Some(&serde_json::json!({
            "mimeType": "image/png",
            "data": openai_b64(b"image")
        }))
    );
    assert!(request_body
        .pointer("/contents/0/parts/0/bytesBase64Encoded")
        .is_none());
    assert!(request_body
        .pointer("/contents/0/parts/1/text")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.contains("strength 0.7")));
}

#[tokio::test]
async fn adapter_gemini_vision_and_multimodal_embedding_use_content_parts() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![
        MockHttpReply {
            status_code: 200,
            body: r#"{"candidates":[{"content":{"parts":[{"text":"{\"text\":\"ok\"}"}]}}]}"#
                .to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: r#"{"embedding":{"values":[0.1,0.2]},"usageMetadata":{"promptTokenCount":3}}"#
                .to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
    ])
    .await;
    let provider = gemini_provider(base_url, 500);
    let resource = ResourceRef::Base64 {
        mime: "image/png".to_string(),
        data_base64: openai_b64(b"image"),
    };

    let mut vision_request = request_with_resource(resource.clone());
    vision_request.capability = Capability::Vision;
    vision_request.payload.text = None;
    vision_request.payload.options = None;
    provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VISION_OCR, vision_request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini vision should succeed");

    let mut embedding_request = request_with_resource(resource);
    embedding_request.capability = Capability::Embedding;
    embedding_request.payload.text = None;
    embedding_request.payload.options = None;
    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-embedding-2".to_string(),
            ResolvedRequest::new_with_method(ai_methods::EMBEDDING_MULTIMODAL, embedding_request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini multimodal embedding should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini multimodal embedding should complete immediately");
    };
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.input_tokens),
        Some(3)
    );

    let requests = captured_requests.lock().expect("captured requests lock");
    assert_eq!(
        requests[0].pointer("/contents/0/parts/0/inlineData/mimeType"),
        Some(&serde_json::json!("image/png"))
    );
    assert_eq!(
        requests[1].pointer("/content/parts/0/inlineData/mimeType"),
        Some(&serde_json::json!("image/png"))
    );
    assert!(!requests[0].to_string().contains("bytesBase64Encoded"));
    assert!(!requests[1].to_string().contains("bytesBase64Encoded"));
}

#[tokio::test]
async fn adapter_gemini_batch_embedding_preserves_all_items() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"embeddings":[{"values":[0.1,0.2]},{"values":[0.3,0.4]}],"usageMetadata":{"promptTokenCount":5}}"#
            .to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = base_request_for(Capability::Embedding, ai_methods::EMBEDDING_TEXT);
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({
        "items": [{ "text": "first" }, { "text": "second" }],
        "dimensions": 2
    }));
    request.payload.options = None;

    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-embedding-001".to_string(),
            ResolvedRequest::new_with_method(ai_methods::EMBEDDING_TEXT, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini batch embedding should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini batch embedding should complete immediately");
    };
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.input_tokens),
        Some(5)
    );
    assert_eq!(
        summary
            .extra
            .as_ref()
            .and_then(|extra| extra.pointer("/embedding/data"))
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );

    let requests = captured_requests.lock().expect("captured requests lock");
    assert_eq!(
        requests[0].pointer("/requests/0/model"),
        Some(&serde_json::json!("models/gemini-embedding-001"))
    );
    assert_eq!(
        requests[0].pointer("/requests/1/content/parts/0/text"),
        Some(&serde_json::json!("second"))
    );
    assert_eq!(
        requests[0].pointer("/requests/0/embedContentConfig/outputDimensionality"),
        Some(&serde_json::json!(2))
    );
}

#[tokio::test]
async fn adapter_gemini_tts_maps_text_voice_and_language() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![MockHttpReply {
        status_code: 200,
        body: format!(
            r#"{{"candidates":[{{"content":{{"parts":[{{"inlineData":{{"mimeType":"audio/L16;rate=24000","data":"{}"}}}}]}}}}]}}"#,
            openai_b64(b"audio")
        ),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = base_request_for(Capability::Audio, ai_methods::AUDIO_TTS);
    request.payload.text = None;
    request.payload.input_json = Some(serde_json::json!({
        "text": "你好，世界",
        "voice_id": "Kore",
        "language": "cmn-CN",
        "style": "calm",
        "speed": 0.9,
        "output": { "media_type": "audio/wav", "sample_rate": 24000 }
    }));
    request.payload.options = None;

    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash-preview-tts".to_string(),
            ResolvedRequest::new_with_method(ai_methods::AUDIO_TTS, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini tts should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini tts should complete immediately");
    };
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.request_units),
        Some(1)
    );

    let request_body = captured_requests
        .lock()
        .expect("captured requests lock")
        .first()
        .cloned()
        .expect("tts request should be captured");
    let prompt = request_body
        .pointer("/contents/0/parts/0/text")
        .and_then(|value| value.as_str())
        .expect("tts prompt");
    assert!(prompt.contains("你好，世界"));
    assert!(prompt.contains("Style: calm"));
    assert_eq!(
        request_body
            .pointer("/generationConfig/speechConfig/voiceConfig/prebuiltVoiceConfig/voiceName"),
        Some(&serde_json::json!("Kore"))
    );
    assert_eq!(
        request_body.pointer("/generationConfig/speechConfig/languageCode"),
        Some(&serde_json::json!("cmn-CN"))
    );
    assert_eq!(
        request_body.pointer("/generationConfig/responseFormat/audio"),
        Some(&serde_json::json!({
            "mimeType": "AUDIO_WAV",
            "sampleRate": 24000
        }))
    );
}

#[tokio::test]
async fn adapter_gemini_asr_maps_audio_and_transcript() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"candidates":[{"content":{"parts":[{"text":"{\"speech_detected\":true,\"transcript_confidence\":0.98,\"text\":\"hello world\",\"segments\":[{\"id\":\"seg-1\",\"start_seconds\":0.0,\"end_seconds\":1.2,\"text\":\"hello world\",\"speaker\":\"SPEAKER_0\",\"confidence\":0.98}]}"}]} }],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":8,"totalTokenCount":20}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = base_request_for(Capability::Audio, ai_methods::AUDIO_ASR);
    request.payload.resources = vec![ResourceRef::Base64 {
        mime: "audio/mpeg".to_string(),
        data_base64: openai_b64(b"audio"),
    }];
    request.payload.input_json = Some(serde_json::json!({
        "language": "en-US",
        "timestamps": "segment",
        "diarization": true
    }));
    request.payload.options = None;

    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new_with_method(ai_methods::AUDIO_ASR, request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini ASR should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini ASR should complete immediately");
    };
    assert_eq!(summary.text_content(), "hello world");
    assert_eq!(
        summary
            .extra
            .as_ref()
            .and_then(|extra| extra.pointer("/asr/segments/0/speaker")),
        Some(&serde_json::json!("SPEAKER_0"))
    );
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.total_tokens),
        Some(20)
    );

    let request_body = captured_requests
        .lock()
        .expect("captured requests lock")
        .first()
        .cloned()
        .expect("ASR request should be captured");
    assert_eq!(
        request_body.pointer("/contents/0/parts/0/inlineData/mimeType"),
        Some(&serde_json::json!("audio/mpeg"))
    );
    assert_eq!(
        request_body.pointer("/generationConfig/responseJsonSchema/additionalProperties"),
        Some(&serde_json::json!(false))
    );
    assert!(request_body
        .pointer("/generationConfig/responseSchema")
        .is_none());
    let prompt = request_body
        .pointer("/contents/0/parts/1/text")
        .and_then(serde_json::Value::as_str)
        .expect("ASR prompt");
    assert!(prompt.contains("en-US"));
    assert!(prompt.contains("segment"));
    assert!(prompt.contains("true"));
}

#[tokio::test]
async fn adapter_fal_video_upscale_runs_in_background() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"request_id":"fal-1","video":{"url":"https://example.com/upscaled.mp4","content_type":"video/mp4"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 100,
    }])
    .await;
    let provider = FalProvider::new(
        FalInstanceConfig {
            provider_instance_name: "fal-test".to_string(),
            provider_type: "cloud_api".to_string(),
            api_token: "token".to_string(),
            base_url,
            timeout_ms: 500,
            image_upscale_models: vec![],
            image_bg_remove_models: vec![],
            audio_enhance_models: vec![],
            video_upscale_models: vec!["fal-ai/video-upscaler".to_string()],
        },
        "token".to_string(),
    )
    .expect("fal provider");
    let request = request_with_resource(ResourceRef::Url {
        url: "https://example.com/input.mp4".to_string(),
        mime_hint: Some("video/mp4".to_string()),
    });
    let task_id = "fal-video-task";
    let sink_factory = Arc::new(CollectingSinkFactory::new());
    let sink = sink_factory.build(&InvokeCtx::default(), task_id);
    let result = provider
        .start(
            InvokeCtx {
                task_id: Some(task_id.to_string()),
                ..Default::default()
            },
            "fal-ai/video-upscaler".to_string(),
            ResolvedRequest::new_with_method(ai_methods::VIDEO_UPSCALE, request),
            sink,
        )
        .await
        .expect("fal video upscale should start");
    assert!(matches!(result, ProviderStartResult::Started));

    let summary = wait_for_final_summary(sink_factory.as_ref(), task_id).await;
    let artifacts = summary.artifacts();
    assert_eq!(artifacts.len(), 1);
    match &artifacts[0].resource {
        ResourceRef::Url { url, .. } => {
            assert_eq!(url, "https://example.com/upscaled.mp4");
        }
        other => panic!("unexpected video artifact: {:?}", other),
    }
}

fn gemini_provider(base_url: String, timeout_ms: u64) -> GoogleGeminiProvider {
    GoogleGeminiProvider::new(
        GoogleGeminiInstanceConfig {
            provider_instance_name: "gemini-test".to_string(),
            provider_type: "cloud_api".to_string(),
            provider_driver: "google-gemini".to_string(),
            api_token: "token".to_string(),
            base_url,
            timeout_ms,
            models: vec!["gemini-2.5-flash".to_string()],
            default_model: Some("gemini-2.5-flash".to_string()),
            image_models: vec!["gemini-2.5-flash-image-preview".to_string()],
            default_image_model: Some("gemini-2.5-flash-image-preview".to_string()),
            features: vec!["plan".to_string()],
            alias_map: HashMap::new(),
        },
        "token".to_string(),
    )
    .expect("gemini provider")
}

fn claude_provider(base_url: String, timeout_ms: u64) -> ClaudeProvider {
    ClaudeProvider::new(
        ClaudeInstanceConfig {
            provider_instance_name: "claude-test".to_string(),
            provider_type: "cloud_api".to_string(),
            provider_driver: "claude".to_string(),
            api_token: "token".to_string(),
            base_url,
            timeout_ms,
            models: vec!["claude-3-7-sonnet-20250219".to_string()],
            default_model: Some("claude-3-7-sonnet-20250219".to_string()),
            features: vec!["plan".to_string()],
            alias_map: HashMap::new(),
        },
        "token".to_string(),
    )
    .expect("claude provider")
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_01_http_200_success` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷淬€?
async fn adapter_openai_01_http_200_success() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"id":"r1","status":"completed","output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":"ok","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let result = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect("openai 200 should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text_content(), "ok");
            assert_eq!(summary.usage.as_ref().and_then(|u| u.total_tokens), Some(2));
        }
        _ => panic!("expected immediate summary"),
    }
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_02_http_429_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆侀檺娴侀敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
async fn adapter_openai_02_http_429_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 429,
        body: r#"{"error":{"code":"rate_limit","message":"too many requests"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_openai_02_http_429_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_03_http_503_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆佹湇鍔′笉鍙敤閿欒鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
async fn adapter_openai_03_http_503_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 503,
        body: r#"{"error":{"code":"unavailable","message":"service unavailable"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_openai_03_http_503_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_04_http_400_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€佸弬鏁伴敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴嫆缁濇垨鑷村懡閿欒锛岄敊璇爜/閿欒娑堟伅绗﹀悎棰勬湡銆?
async fn adapter_openai_04_http_400_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 400,
        body: r#"{"error":{"code":"invalid_request","message":"bad request"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable(), "assert failed in adapter_openai_04_http_400_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_05_invalid_json_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€侀潪娉?JSON 鍝嶅簲鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷达紱杩斿洖鎷掔粷鎴栬嚧鍛介敊璇紝閿欒鐮?閿欒娑堟伅绗﹀悎棰勬湡銆?
async fn adapter_openai_05_invalid_json_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: "not-json".to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable(), "assert failed in adapter_openai_05_invalid_json_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_06_timeout_or_network_error_classified` 鐢ㄤ緥锛岃鐩栬秴鏃?缃戠粶寮傚父鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn adapter_openai_06_timeout_or_network_error_classified() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: "{}".to_string(),
        content_type: "application/json",
        delay_ms: 200,
    }])
    .await;
    let provider = openai_provider(base_url, 20);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_openai_06_timeout_or_network_error_classified: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gemini_01_http_200_success` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷淬€?
async fn adapter_gemini_01_http_200_success() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini 200 should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text_content(), "ok");
            assert_eq!(summary.usage.as_ref().and_then(|u| u.total_tokens), Some(2));
        }
        _ => panic!("expected immediate summary"),
    }
}

#[tokio::test]
async fn adapter_gemini_retries_with_thinking_tokens_outside_completion_budget() {
    let (base_url, captured_requests) = spawn_fake_http_server_with_requests(vec![
        MockHttpReply {
            status_code: 200,
            body: r#"{"candidates":[{"content":{"parts":[{"text":"{"} ]},"finishReason":"MAX_TOKENS"}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":8,"thoughtsTokenCount":2000,"totalTokenCount":2108}}"#.to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
        MockHttpReply {
            status_code: 200,
            body: r#"{"candidates":[{"content":{"parts":[{"text":"complete"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":12,"thoughtsTokenCount":2000,"totalTokenCount":2112}}"#.to_string(),
            content_type: "application/json",
            delay_ms: 0,
        },
    ])
    .await;
    let provider = gemini_provider(base_url, 500);
    let mut request = base_request();
    request.payload.options = Some(serde_json::json!({
        "temperature": 0.2,
        "max_completion_tokens": 2048
    }));

    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(request),
            Arc::new(NoopSink),
        )
        .await
        .expect("gemini thinking retry should succeed");
    let ProviderStartResult::Immediate(summary) = result else {
        panic!("gemini thinking retry should complete immediately");
    };
    assert_eq!(summary.text_content(), "complete");
    assert_eq!(
        summary
            .extra
            .as_ref()
            .and_then(|extra| extra.get("thinking_tokens"))
            .and_then(|value| value.as_u64()),
        Some(4000)
    );
    assert_eq!(
        summary.usage.as_ref().and_then(|usage| usage.input_tokens),
        Some(200)
    );

    let requests = captured_requests.lock().expect("captured requests lock");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].pointer("/generationConfig/maxOutputTokens"),
        Some(&serde_json::json!(2048))
    );
    assert_eq!(
        requests[1].pointer("/generationConfig/maxOutputTokens"),
        Some(&serde_json::json!(4048))
    );
    assert!(requests[1]
        .pointer("/generationConfig/thinkingConfig")
        .is_none());
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gemini_02_http_429_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆侀檺娴侀敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
async fn adapter_gemini_02_http_429_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 429,
        body: r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"too many requests"}}"#
            .to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_gemini_02_http_429_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_claude_01_http_200_success` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷淬€?
async fn adapter_claude_01_http_200_success() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1},"stop_reason":"end_turn"}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = claude_provider(base_url, 500);
    let result = provider
        .start(
            InvokeCtx::default(),
            "claude-3-7-sonnet-20250219".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect("claude 200 should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text_content(), "ok");
            assert_eq!(summary.usage.as_ref().and_then(|u| u.input_tokens), Some(1));
            assert_eq!(
                summary.usage.as_ref().and_then(|u| u.output_tokens),
                Some(1)
            );
        }
        _ => panic!("expected immediate summary"),
    }
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_claude_02_http_429_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆侀檺娴侀敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
async fn adapter_claude_02_http_429_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 429,
        body: r#"{"error":{"message":"too many requests"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = claude_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "claude-3-7-sonnet-20250219".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_claude_02_http_429_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_claude_03_http_400_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€佸弬鏁伴敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴嫆缁濇垨鑷村懡閿欒锛岄敊璇爜/閿欒娑堟伅绗﹀悎棰勬湡銆?
async fn adapter_claude_03_http_400_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 400,
        body: r#"{"error":{"message":"bad request"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = claude_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "claude-3-7-sonnet-20250219".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable(), "assert failed in adapter_claude_03_http_400_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_claude_04_timeout_or_network_error_classified` 鐢ㄤ緥锛岃鐩栬秴鏃?缃戠粶寮傚父鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn adapter_claude_04_timeout_or_network_error_classified() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: "{}".to_string(),
        content_type: "application/json",
        delay_ms: 200,
    }])
    .await;
    let provider = claude_provider(base_url, 20);
    let err = provider
        .start(
            InvokeCtx::default(),
            "claude-3-7-sonnet-20250219".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(
        err.is_retryable(),
        "expected retryable timeout/network error, got: {err}"
    );
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gemini_03_http_503_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆佹湇鍔′笉鍙敤閿欒鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
async fn adapter_gemini_03_http_503_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 503,
        body: r#"{"error":{"status":"UNAVAILABLE","message":"service unavailable"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_gemini_03_http_503_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gemini_04_http_400_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€佸弬鏁伴敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴嫆缁濇垨鑷村懡閿欒锛岄敊璇爜/閿欒娑堟伅绗﹀悎棰勬湡銆?
async fn adapter_gemini_04_http_400_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 400,
        body: r#"{"error":{"status":"INVALID_ARGUMENT","message":"bad request"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable(), "assert failed in adapter_gemini_04_http_400_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gemini_05_invalid_json_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€侀潪娉?JSON 鍝嶅簲鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷达紱杩斿洖鎷掔粷鎴栬嚧鍛介敊璇紝閿欒鐮?閿欒娑堟伅绗﹀悎棰勬湡銆?
async fn adapter_gemini_05_invalid_json_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: "not-json".to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gemini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable(), "assert failed in adapter_gemini_05_invalid_json_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gemini_06_timeout_or_network_error_classified` 鐢ㄤ緥锛岃鐩栬秴鏃?缃戠粶寮傚父鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn adapter_gemini_06_timeout_or_network_error_classified() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: "{}".to_string(),
        content_type: "application/json",
        delay_ms: 200,
    }])
    .await;
    let provider = gemini_provider(base_url, 20);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_gemini_06_timeout_or_network_error_classified: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚proto_t2i_01_prompt_from_text` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲崗璁瓧娈点€佽祫婧愬紩鐢ㄦ垨 base64/url 杈撳叆銆?
// - 澶勭悊娴佺▼锛氳蛋鍗忚鏍￠獙涓庝换鍔℃墽琛岃矾寰勶紝瑕嗙洊杈撳叆褰㈡€併€佽祫婧愬鐞嗕笌浜嬩欢浜у嚭銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn proto_t2i_01_prompt_from_text() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Image, "text2image.default");
    req.payload.text = Some("draw a cat".to_string());
    req.payload.messages = vec![];
    req.payload.options = Some(serde_json::json!({"size":"1024x1024"}));
    let res = provider
        .start(
            InvokeCtx::default(),
            "dall-e-3".to_string(),
            ResolvedRequest::new(req),
            Arc::new(NoopSink),
        )
        .await
        .expect("text prompt should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.artifacts().len(), 1);
            match &summary.artifacts()[0].resource {
                buckyos_api::ResourceRef::Url { url, .. } => {
                    assert_eq!(url, "https://example.com/a.png");
                }
                _ => panic!("expected url artifact"),
            }
        }
        _ => panic!("expected immediate"),
    }
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚proto_t2i_04_artifact_url_format` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲崗璁瓧娈点€佽祫婧愬紩鐢ㄦ垨 base64/url 杈撳叆銆?
// - 澶勭悊娴佺▼锛氳蛋鍗忚鏍￠獙涓庝换鍔℃墽琛岃矾寰勶紝瑕嗙洊杈撳叆褰㈡€併€佽祫婧愬鐞嗕笌浜嬩欢浜у嚭銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn proto_t2i_04_artifact_url_format() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Image, "text2image.default");
    req.payload.text = Some("draw a cat".to_string());
    let res = provider
        .start(
            InvokeCtx::default(),
            "dall-e-3".to_string(),
            ResolvedRequest::new(req),
            Arc::new(NoopSink),
        )
        .await
        .expect("should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert!(!summary.artifacts().is_empty(), "assert failed in proto_t2i_04_artifact_url_format: condition is false; check preconditions and expected branch outcome.");
            if let buckyos_api::ResourceRef::Url { url, .. } = &summary.artifacts()[0].resource {
                assert!(url.starts_with("https://"), "assert failed in proto_t2i_04_artifact_url_format: condition is false; check preconditions and expected branch outcome.");
            } else {
                panic!("expected url artifact");
            }
        }
        _ => panic!("expected immediate"),
    }
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚proto_t2i_03_prompt_from_options` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲崗璁瓧娈点€佽祫婧愬紩鐢ㄦ垨 base64/url 杈撳叆銆?
// - 澶勭悊娴佺▼锛氳蛋鍗忚鏍￠獙涓庝换鍔℃墽琛岃矾寰勶紝瑕嗙洊杈撳叆褰㈡€併€佽祫婧愬鐞嗕笌浜嬩欢浜у嚭銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn proto_t2i_03_prompt_from_options() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Image, "text2image.default");
    req.payload.text = None;
    req.payload.options = Some(serde_json::json!({"prompt":"draw from options"}));
    let res = provider
        .start(
            InvokeCtx::default(),
            "dall-e-3".to_string(),
            ResolvedRequest::new(req),
            Arc::new(NoopSink),
        )
        .await
        .expect("options prompt should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.artifacts().len(), 1);
            match &summary.artifacts()[0].resource {
                buckyos_api::ResourceRef::Url { url, .. } => {
                    assert_eq!(url, "https://example.com/a.png");
                }
                _ => panic!("expected url artifact"),
            }
        }
        _ => panic!("expected immediate"),
    }
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚proto_t2i_02_prompt_from_messages` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲崗璁瓧娈点€佽祫婧愬紩鐢ㄦ垨 base64/url 杈撳叆銆?
// - 澶勭悊娴佺▼锛氳蛋鍗忚鏍￠獙涓庝换鍔℃墽琛岃矾寰勶紝瑕嗙洊杈撳叆褰㈡€併€佽祫婧愬鐞嗕笌浜嬩欢浜у嚭銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn proto_t2i_02_prompt_from_messages() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Image, "text2image.default");
    req.payload.text = None;
    req.payload.messages = vec![buckyos_api::AiMessage::text(
        buckyos_api::AiRole::User,
        "draw from message",
    )];
    let res = provider
        .start(
            InvokeCtx::default(),
            "dall-e-3".to_string(),
            ResolvedRequest::new(req),
            Arc::new(NoopSink),
        )
        .await
        .expect("message prompt should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.artifacts().len(), 1);
            match &summary.artifacts()[0].resource {
                buckyos_api::ResourceRef::Url { url, .. } => {
                    assert_eq!(url, "https://example.com/a.png");
                }
                _ => panic!("expected url artifact"),
            }
        }
        _ => panic!("expected immediate"),
    }
}
