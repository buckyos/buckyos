mod common;

use aicc::gimini::{GoogleGiminiInstanceConfig, GoogleGiminiProvider};
use aicc::openai::{OpenAIInstanceConfig, OpenAIProvider};
use aicc::{InvokeCtx, Provider, ResolvedRequest};
use buckyos_api::Capability;
use common::*;
use std::collections::HashMap;
use std::sync::Arc;

fn openai_provider(base_url: String, timeout_ms: u64) -> OpenAIProvider {
    OpenAIProvider::new(
        OpenAIInstanceConfig {
            instance_id: "openai-test".to_string(),
            provider_type: "openai".to_string(),
            base_url,
            timeout_ms,
            models: vec!["gpt-4o-mini".to_string()],
            default_model: Some("gpt-4o-mini".to_string()),
            image_models: vec!["dall-e-3".to_string()],
            default_image_model: Some("dall-e-3".to_string()),
            features: vec!["plan".to_string()],
            alias_map: HashMap::new(),
        },
        "token".to_string(),
    )
    .expect("openai provider")
}

fn gimini_provider(base_url: String, timeout_ms: u64) -> GoogleGiminiProvider {
    GoogleGiminiProvider::new(
        GoogleGiminiInstanceConfig {
            instance_id: "gimini-test".to_string(),
            provider_type: "google-gimini".to_string(),
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
    .expect("gimini provider")
}

#[tokio::test]
async fn adapter_openai_01_http_200_success() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"id":"r1","status":"completed","output_text":"ok","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#.to_string(),
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
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
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
    assert!(err.is_retryable());
}

#[tokio::test]
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
    assert!(err.is_retryable());
}

#[tokio::test]
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
    assert!(!err.is_retryable());
}

#[tokio::test]
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
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn adapter_openai_06_timeout_or_network_error_classified() {
    let provider = openai_provider("http://127.0.0.1:9".to_string(), 80);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gpt-4o-mini".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn adapter_gimini_01_http_200_success() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gimini_provider(base_url, 500);
    let result = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn adapter_gimini_02_http_429_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 429,
        body: r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"too many requests"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gimini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn adapter_gimini_03_http_503_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 503,
        body: r#"{"error":{"status":"UNAVAILABLE","message":"service unavailable"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gimini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn adapter_gimini_04_http_400_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 400,
        body: r#"{"error":{"status":"INVALID_ARGUMENT","message":"bad request"}}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gimini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn adapter_gimini_05_invalid_json_fatal() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: "not-json".to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = gimini_provider(base_url, 500);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn adapter_gimini_06_timeout_or_network_error_classified() {
    let provider = gimini_provider("http://127.0.0.1:9".to_string(), 80);
    let err = provider
        .start(
            InvokeCtx::default(),
            "gemini-2.5-flash".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn proto_t2i_01_prompt_from_text() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Text2Image, "text2image.default");
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
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn proto_t2i_04_artifact_url_format() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Text2Image, "text2image.default");
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
            assert!(!summary.artifacts.is_empty());
            if let buckyos_api::ResourceRef::Url { url, .. } = &summary.artifacts[0].resource {
                assert!(url.starts_with("https://"));
            } else {
                panic!("expected url artifact");
            }
        }
        _ => panic!("expected immediate"),
    }
}

#[tokio::test]
async fn proto_t2i_03_prompt_from_options() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Text2Image, "text2image.default");
    req.payload.text = None;
    req.payload.options = Some(serde_json::json!({"prompt":"draw from options"}));
    let res = provider
        .start(
            InvokeCtx::default(),
            "dall-e-3".to_string(),
            ResolvedRequest::new(req),
            Arc::new(NoopSink),
        )
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn proto_t2i_02_prompt_from_messages() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 200,
        body: r#"{"data":[{"url":"https://example.com/a.png"}]}"#.to_string(),
        content_type: "application/json",
        delay_ms: 0,
    }])
    .await;
    let provider = openai_provider(base_url, 500);
    let mut req = base_request_for(Capability::Text2Image, "text2image.default");
    req.payload.text = None;
    req.payload.messages = vec![buckyos_api::AiMessage::new(
        "user".to_string(),
        "draw from message".to_string(),
    )];
    let res = provider
        .start(
            InvokeCtx::default(),
            "dall-e-3".to_string(),
            ResolvedRequest::new(req),
            Arc::new(NoopSink),
        )
        .await;
    assert!(res.is_ok());
}
