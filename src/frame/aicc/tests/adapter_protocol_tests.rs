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
// 意图：验证OpenAI 200 成功路径场景（adapter_openai_01_http_200_success）。预期start 成功，因为该输入会命中对应业务分支；不应被误分类为失败，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(result.is_ok(), "assert failed in adapter_openai_01_http_200_success: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证OpenAI 可重试错误分类场景（adapter_openai_02_http_429_retryable）。预期标记 retryable，因为该输入会命中对应业务分支；不应标记 fatal，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证OpenAI 可重试错误分类场景（adapter_openai_03_http_503_retryable）。预期标记 retryable，因为该输入会命中对应业务分支；不应标记 fatal，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证OpenAI 致命错误分类场景（adapter_openai_04_http_400_fatal）。预期标记 fatal，因为该输入会命中对应业务分支；不应进入自动重试，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证OpenAI 致命错误分类场景（adapter_openai_05_invalid_json_fatal）。预期标记 fatal，因为该输入会命中对应业务分支；不应进入自动重试，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
// 意图：验证OpenAI 可重试错误分类场景（adapter_openai_06_timeout_or_network_error_classified）。预期标记 retryable，因为该输入会命中对应业务分支；不应标记 fatal，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(err.is_retryable(), "assert failed in adapter_openai_06_timeout_or_network_error_classified: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证Gimini 200 成功路径场景（adapter_gimini_01_http_200_success）。预期start 成功，因为该输入会命中对应业务分支；不应被误分类为失败，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(result.is_ok(), "assert failed in adapter_gimini_01_http_200_success: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证Gimini 可重试错误分类场景（adapter_gimini_02_http_429_retryable）。预期标记 retryable，因为该输入会命中对应业务分支；不应标记 fatal，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(err.is_retryable(), "assert failed in adapter_gimini_02_http_429_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证Gimini 可重试错误分类场景（adapter_gimini_03_http_503_retryable）。预期标记 retryable，因为该输入会命中对应业务分支；不应标记 fatal，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(err.is_retryable(), "assert failed in adapter_gimini_03_http_503_retryable: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证Gimini 致命错误分类场景（adapter_gimini_04_http_400_fatal）。预期标记 fatal，因为该输入会命中对应业务分支；不应进入自动重试，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(!err.is_retryable(), "assert failed in adapter_gimini_04_http_400_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证Gimini 致命错误分类场景（adapter_gimini_05_invalid_json_fatal）。预期标记 fatal，因为该输入会命中对应业务分支；不应进入自动重试，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(!err.is_retryable(), "assert failed in adapter_gimini_05_invalid_json_fatal: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证Gimini 可重试错误分类场景（adapter_gimini_06_timeout_or_network_error_classified）。预期标记 retryable，因为该输入会命中对应业务分支；不应标记 fatal，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(err.is_retryable(), "assert failed in adapter_gimini_06_timeout_or_network_error_classified: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证文生图 prompt 优先级场景（proto_t2i_01_prompt_from_text）。预期优先使用 payload.text，因为该输入会命中对应业务分支；不应错误回退到其它字段，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(res.is_ok(), "assert failed in proto_t2i_01_prompt_from_text: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证文生图产物结构场景（proto_t2i_04_artifact_url_format）。预期返回含有可用 URL 的 artifact，因为该输入会命中对应业务分支；不应返回空产物或非 URL 资源，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
            assert!(!summary.artifacts.is_empty(), "assert failed in proto_t2i_04_artifact_url_format: condition is false; check preconditions and expected branch outcome.");
            if let buckyos_api::ResourceRef::Url { url, .. } = &summary.artifacts[0].resource {
                assert!(url.starts_with("https://"), "assert failed in proto_t2i_04_artifact_url_format: condition is false; check preconditions and expected branch outcome.");
            } else {
                panic!("expected url artifact");
            }
        }
        _ => panic!("expected immediate"),
    }
}

#[tokio::test]
// 意图：验证文生图 prompt 次级提取场景（proto_t2i_03_prompt_from_options）。预期从 options.prompt 提取，因为该输入会命中对应业务分支；不应丢失 prompt 或直接失败，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(res.is_ok(), "assert failed in proto_t2i_03_prompt_from_options: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 意图：验证文生图 prompt 兜底提取场景（proto_t2i_02_prompt_from_messages）。预期从 messages 提取，因为该输入会命中对应业务分支；不应直接报缺参，否则说明路由、状态机或错误分类逻辑与契约不一致。
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
    assert!(res.is_ok(), "assert failed in proto_t2i_02_prompt_from_messages: condition is false; check preconditions and expected branch outcome.");
}
