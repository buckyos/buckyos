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
            auth_mode: "bearer".to_string(),
            timeout_ms,
            models: vec!["gpt-4o-mini".to_string()],
            default_model: Some("gpt-4o-mini".to_string()),
            image_models: vec!["dall-e-3".to_string()],
            default_image_model: Some("dall-e-3".to_string()),
            features: vec!["plan".to_string()],
            alias_map: HashMap::new(),
        },
        "token",
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_openai_01_http_200_success` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷淬€?
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gimini_01_http_200_success` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷淬€?
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gimini_02_http_429_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆侀檺娴侀敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
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
            assert_eq!(summary.text.as_deref(), Some("ok"));
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gimini_03_http_503_retryable` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆佹湇鍔′笉鍙敤閿欒鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氶敊璇褰掔被涓哄彲閲嶈瘯骞惰Е鍙戝搴旂瓥鐣ャ€?
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gimini_04_http_400_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€佸弬鏁伴敊璇垎绫汇€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴嫆缁濇垨鑷村懡閿欒锛岄敊璇爜/閿欒娑堟伅绗﹀悎棰勬湡銆?
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gimini_05_invalid_json_fatal` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€侀潪娉?JSON 鍝嶅簲鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涢€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷达紱杩斿洖鎷掔粷鎴栬嚧鍛介敊璇紝閿欒鐮?閿欒娑堟伅绗﹀悎棰勬湡銆?
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
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚adapter_gimini_06_timeout_or_network_error_classified` 鐢ㄤ緥锛岃鐩栬秴鏃?缃戠粶寮傚父鍒嗙被銆?
// - 杈撳叆鍙傛暟锛氶€氳繃 mock HTTP 鏈嶅姟鏋勯€犵姸鎬佺爜/鍝嶅簲浣?瓒呮椂銆?
// - 澶勭悊娴佺▼锛氳皟鐢ㄥ叿浣?provider adapter锛岃姹?mock 鏈嶅姟骞舵墽琛屽搷搴旇В鏋愪笌閿欒鍒嗙被銆?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
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
