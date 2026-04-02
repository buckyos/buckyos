mod common;

use aicc::claude::{ClaudeInstanceConfig, ClaudeProvider};
use aicc::gimini::{GoogleGiminiInstanceConfig, GoogleGiminiProvider};
use aicc::openai::{OpenAIInstanceConfig, OpenAIProvider};
use aicc::{InvokeCtx, Provider, ProviderStartResult, ResolvedRequest};
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

fn claude_provider(base_url: String, timeout_ms: u64) -> ClaudeProvider {
    ClaudeProvider::new(
        ClaudeInstanceConfig {
            instance_id: "claude-test".to_string(),
            provider_type: "claude".to_string(),
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
// 用例说明：
// - 验证场景：`adapter_openai_01_http_200_success` 用例，覆盖函数名对应的业务路径。
// - 输入参数：通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
        .await
        .expect("openai 200 should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text.as_deref(), Some("ok"));
            assert_eq!(summary.usage.as_ref().and_then(|u| u.total_tokens), Some(2));
        }
        _ => panic!("expected immediate summary"),
    }
}

#[tokio::test]
// 用例说明：
// - 验证场景：`adapter_openai_02_http_429_retryable` 用例，覆盖可重试错误分支、限流错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：错误被归类为可重试并触发对应策略。
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
// 用例说明：
// - 验证场景：`adapter_openai_03_http_503_retryable` 用例，覆盖可重试错误分支、服务不可用错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：错误被归类为可重试并触发对应策略。
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
// 用例说明：
// - 验证场景：`adapter_openai_04_http_400_fatal` 用例，覆盖致命错误分支、参数错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`adapter_openai_05_invalid_json_fatal` 用例，覆盖致命错误分支、非法 JSON 响应分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回成功结果，关键字段与断言一致；返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`adapter_openai_06_timeout_or_network_error_classified` 用例，覆盖超时/网络异常分类。
// - 输入参数：通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
// 用例说明：
// - 验证场景：`adapter_gimini_01_http_200_success` 用例，覆盖函数名对应的业务路径。
// - 输入参数：通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
        .await
        .expect("gimini 200 should succeed");
    match result {
        ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.text.as_deref(), Some("ok"));
            assert_eq!(summary.usage.as_ref().and_then(|u| u.total_tokens), Some(2));
        }
        _ => panic!("expected immediate summary"),
    }
}

#[tokio::test]
// 用例说明：
// - 验证场景：`adapter_gimini_02_http_429_retryable` 用例，覆盖可重试错误分支、限流错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：错误被归类为可重试并触发对应策略。
async fn adapter_gimini_02_http_429_retryable() {
    let base_url = spawn_fake_http_server(vec![MockHttpReply {
        status_code: 429,
        body: r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"too many requests"}}"#
            .to_string(),
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
// 用例说明：
// - 验证场景：`adapter_claude_01_http_200_success` 用例，覆盖函数名对应的业务路径。
// - 输入参数：通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回成功结果，关键字段与断言一致。
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
            assert_eq!(summary.text.as_deref(), Some("ok"));
            assert_eq!(summary.usage.as_ref().and_then(|u| u.input_tokens), Some(1));
            assert_eq!(summary.usage.as_ref().and_then(|u| u.output_tokens), Some(1));
        }
        _ => panic!("expected immediate summary"),
    }
}

#[tokio::test]
// 用例说明：
// - 验证场景：`adapter_claude_02_http_429_retryable` 用例，覆盖可重试错误分支、限流错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：错误被归类为可重试并触发对应策略。
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
// 用例说明：
// - 验证场景：`adapter_claude_03_http_400_fatal` 用例，覆盖致命错误分支、参数错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`adapter_claude_04_timeout_or_network_error_classified` 用例，覆盖超时/网络异常分类。
// - 输入参数：通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
async fn adapter_claude_04_timeout_or_network_error_classified() {
    let provider = claude_provider("http://127.0.0.1:9".to_string(), 80);
    let err = provider
        .start(
            InvokeCtx::default(),
            "claude-3-7-sonnet-20250219".to_string(),
            ResolvedRequest::new(base_request()),
            Arc::new(NoopSink),
        )
        .await
        .expect_err("must fail");
    assert!(err.is_retryable(), "assert failed in adapter_claude_04_timeout_or_network_error_classified: condition is false; check preconditions and expected branch outcome.");
}

#[tokio::test]
// 用例说明：
// - 验证场景：`adapter_gimini_03_http_503_retryable` 用例，覆盖可重试错误分支、服务不可用错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：错误被归类为可重试并触发对应策略。
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
// 用例说明：
// - 验证场景：`adapter_gimini_04_http_400_fatal` 用例，覆盖致命错误分支、参数错误分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`adapter_gimini_05_invalid_json_fatal` 用例，覆盖致命错误分支、非法 JSON 响应分类。
// - 输入参数：构造多个 provider 候选，并注入 Started/Queued/失败结果；通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：返回成功结果，关键字段与断言一致；返回拒绝或致命错误，错误码/错误消息符合预期。
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
// 用例说明：
// - 验证场景：`adapter_gimini_06_timeout_or_network_error_classified` 用例，覆盖超时/网络异常分类。
// - 输入参数：通过 mock HTTP 服务构造状态码/响应体/超时。
// - 处理流程：调用具体 provider adapter，请求 mock 服务并执行响应解析与错误分类。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
// 用例说明：
// - 验证场景：`proto_t2i_01_prompt_from_text` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
        .await
        .expect("text prompt should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.artifacts.len(), 1);
            match &summary.artifacts[0].resource {
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
// 用例说明：
// - 验证场景：`proto_t2i_04_artifact_url_format` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
// 用例说明：
// - 验证场景：`proto_t2i_03_prompt_from_options` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
        .await
        .expect("options prompt should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.artifacts.len(), 1);
            match &summary.artifacts[0].resource {
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
// 用例说明：
// - 验证场景：`proto_t2i_02_prompt_from_messages` 用例，覆盖函数名对应的业务路径。
// - 输入参数：构造协议字段、资源引用或 base64/url 输入。
// - 处理流程：走协议校验与任务执行路径，覆盖输入形态、资源处理与事件产出。
// - 预期输出：断言中的状态、错误码、路由选择或事件字段全部满足预期。
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
        .await
        .expect("message prompt should succeed");
    match res {
        aicc::ProviderStartResult::Immediate(summary) => {
            assert_eq!(summary.artifacts.len(), 1);
            match &summary.artifacts[0].resource {
                buckyos_api::ResourceRef::Url { url, .. } => {
                    assert_eq!(url, "https://example.com/a.png");
                }
                _ => panic!("expected url artifact"),
            }
        }
        _ => panic!("expected immediate"),
    }
}
