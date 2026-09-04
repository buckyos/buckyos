use buckyos_api::{
    AiMessage, AiMethodStatus, AiRole, AiccError, AiccErrorCode, AiccExecutionMode,
    LlmChatInvokeRequest, LlmChatInvokeResponse, ProviderAddRequest,
};
use serde_json::{json, Value};

#[test]
fn llm_request_uses_canonical_schema_and_round_trips() {
    let mut request = LlmChatInvokeRequest::new(
        "gpt-test@provider-main",
        vec![AiMessage::text(AiRole::User, "hello")],
    );
    request.trace_id = Some("trace-1".to_owned());
    request.execution_mode = AiccExecutionMode::Stream;
    request.max_output_tokens = Some(32);
    request.idempotency_key = Some("idem-1".to_owned());

    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["exact_model"], "gpt-test@provider-main");
    assert_eq!(value["execution_mode"], "stream");
    assert!(value.get("stream").is_none());
    assert!(value.get("model").is_none());
    assert!(value.get("payload").is_none());
    assert_eq!(LlmChatInvokeRequest::from_json(value).unwrap(), request);

    assert!(LlmChatInvokeRequest::from_json(json!({
        "exact_model": "gpt-test@provider-main",
        "messages": [],
        "stream": true
    }))
    .is_err());
}

#[test]
fn llm_response_preserves_success_and_failure_payloads() {
    let success: LlmChatInvokeResponse = serde_json::from_value(json!({
        "task_id": "task-1",
        "status": "succeeded",
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": "done"}]
        },
        "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6}
    }))
    .unwrap();
    assert_eq!(success.status, AiMethodStatus::Succeeded);
    assert_eq!(success.message.unwrap().text_content(), "done");

    let failure: LlmChatInvokeResponse = serde_json::from_value(json!({
        "task_id": "task-2",
        "status": "failed",
        "error": {
            "code": "provider_start_failed",
            "message": "provider rejected request",
            "provider_code": "rate_limit_exceeded",
            "retriable": true,
            "details": {"status": 429}
        }
    }))
    .unwrap();
    let value = serde_json::to_value(failure).unwrap();
    assert_eq!(value["error"]["code"], "provider_start_failed");
    assert_eq!(value["error"]["provider_code"], "rate_limit_exceeded");
    assert_eq!(value["error"]["retriable"], true);
}

#[test]
fn aicc_errors_round_trip_through_krpc_and_task_data() {
    for code in [
        AiccErrorCode::InvalidRequest,
        AiccErrorCode::InvalidMethod,
        AiccErrorCode::SchemaValidationFailed,
        AiccErrorCode::UnsupportedExecutionMode,
        AiccErrorCode::InvalidModelName,
        AiccErrorCode::PolicyDenied,
        AiccErrorCode::ProviderStartFailed,
        AiccErrorCode::Timeout,
        AiccErrorCode::BudgetExceeded,
        AiccErrorCode::InternalError,
    ] {
        let mut expected = AiccError::new(code, format!("{} message", code.as_str()));
        expected.provider_code = Some("provider-code".to_owned());
        expected.retriable = matches!(
            code,
            AiccErrorCode::ProviderStartFailed | AiccErrorCode::Timeout
        );
        expected.details = Some(json!({"attempt": 2}));

        assert_eq!(
            AiccError::from_krpc_error(&expected.to_krpc_error()),
            Some(expected.clone())
        );
        assert_eq!(
            AiccError::from_task_data(&expected.to_task_data()),
            Some(expected)
        );
    }
}

#[test]
fn provider_add_accepts_only_the_current_locked_credential_schema() {
    let mut request = ProviderAddRequest::new(
        "t15-openai",
        "cloud_api",
        "openai",
        "http://127.0.0.1:18081/v1",
        json!({"api_token": {"locked": "mock-secret"}}),
    );
    request.protocol_adapter_id = Some("openai-responses".to_owned());
    request.auto_sync_models = Some(true);

    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["base_url"], "http://127.0.0.1:18081/v1");
    assert_eq!(ProviderAddRequest::from_json(value).unwrap(), request);
    assert!(ProviderAddRequest::from_json(json!({
        "provider_instance_name": "t15-openai",
        "provider_type": "cloud_api",
        "provider_profile_id": "openai",
        "endpoint": "http://127.0.0.1:18081/v1",
        "credentials": Value::Null,
        "enabled": true
    }))
    .is_err());
}
