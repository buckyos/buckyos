use super::{
    AdapterDescriptor, AdapterStatus, ClaudeMessagesCodec, CodecCall, CodecInput,
    CodecRegistration, ExecutionMode, HttpBody, HttpRequest, HttpResponse, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolEvent, ProtocolExecution,
    ProtocolOutput, ProtocolResultValue, ProtocolStream, StreamingHttpResponse,
    CLAUDE_MESSAGES_ADAPTER_ID,
};
use async_trait::async_trait;
use buckyos_api::{AiccCall, ApiType};
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const MINIMAX_MESSAGES_ADAPTER_ID: &str = "minimax-messages";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MiniMaxMessagesDialectContract {
    pub base_adapter_id: &'static str,
    pub override_points: BTreeSet<&'static str>,
    pub unsupported_parameters: BTreeSet<&'static str>,
    pub unsupported_capabilities: BTreeSet<&'static str>,
}

pub(crate) fn minimax_messages_dialect_contract() -> MiniMaxMessagesDialectContract {
    MiniMaxMessagesDialectContract {
        base_adapter_id: CLAUDE_MESSAGES_ADAPTER_ID,
        override_points: BTreeSet::from([
            "request_parameter_validation",
            "base_resp_error",
            "provider_state_namespace",
        ]),
        unsupported_parameters: BTreeSet::new(),
        unsupported_capabilities: BTreeSet::new(),
    }
}

pub(crate) fn minimax_messages_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let base = ClaudeMessagesCodec::new();
    let operation = base.descriptor().clone();
    let descriptor = AdapterDescriptor {
        protocol_family_id: "claude".to_owned(),
        protocol_adapter_id: MINIMAX_MESSAGES_ADAPTER_ID.to_owned(),
        interface_generation: "messages-2023-06-01-minimax".to_owned(),
        base_adapter_id: Some(CLAUDE_MESSAGES_ADAPTER_ID.to_owned()),
        status: AdapterStatus::Stable,
        operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
    };
    (
        descriptor,
        CodecRegistration {
            operation_codecs: vec![Arc::new(MiniMaxMessagesCodec {
                descriptor: operation,
                base: Arc::new(base),
            })],
            native_task_codecs: Vec::new(),
        },
    )
}

struct MiniMaxMessagesCodec {
    descriptor: OperationDescriptor,
    base: Arc<dyn OperationCodec>,
}

#[async_trait]
impl OperationCodec for MiniMaxMessagesCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::Llm
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate, ExecutionMode::Stream])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        validate_minimax_request(call)?;
        let AiccCall::ChatCompletionsCreate(request) = &call.input.canonical_request else {
            unreachable!("MiniMax request validation checked the canonical method")
        };
        let Some(temperature) = request.temperature.filter(|value| *value > 1.0) else {
            return self.base.encode(call);
        };
        let mut canonical_request = call.input.canonical_request.clone();
        let AiccCall::ChatCompletionsCreate(request) = &mut canonical_request else {
            unreachable!("MiniMax request validation checked the canonical method")
        };
        request.temperature = None;
        let input = CodecInput {
            canonical_request,
            resolved_parameters: call.input.resolved_parameters.clone(),
        };
        let mut encoded = self.base.encode(&CodecCall {
            api_type: call.api_type,
            input: &input,
            context: call.context,
        })?;
        let HttpBody::Json(Value::Object(body)) = &mut encoded.body else {
            return Err(ProtocolError::invalid_configuration(
                "Claude Messages base codec did not produce a JSON object",
            ));
        };
        body.insert("temperature".to_owned(), Value::from(temperature));
        Ok(encoded)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        validate_minimax_response(&response)?;
        let execution = self.base.decode(response).await?;
        Ok(rewrite_execution_namespace(execution))
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        if !response.status.is_success() {
            let response = response
                .into_bounded_error_response(self.descriptor.max_response_bytes)
                .await?;
            return Err(minimax_http_error(&response));
        }
        let stream = self.base.decode_stream(response).await?;
        Ok(ProtocolStream {
            events: Box::pin(
                stream
                    .events
                    .map(|event| event.map(rewrite_event_namespace)),
            ),
        })
    }
}

fn validate_minimax_request(call: &CodecCall<'_>) -> ProtocolResultValue<()> {
    let AiccCall::ChatCompletionsCreate(request) = &call.input.canonical_request else {
        return Err(ProtocolError::invalid_request(
            "MiniMax Messages only accepts chat.completions.create",
        ));
    };
    if request
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        return Err(ProtocolError::invalid_request(
            "MiniMax Messages temperature must be between 0 and 2",
        ));
    }
    Ok(())
}

fn validate_minimax_response(response: &HttpResponse) -> ProtocolResultValue<()> {
    if !response.status.is_success() {
        return Err(minimax_http_error(response));
    }
    let Ok(value) = serde_json::from_slice::<Value>(&response.body) else {
        return Ok(());
    };
    let Some(base_resp) = value.get("base_resp") else {
        return Ok(());
    };
    let status_code = base_resp
        .get("status_code")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            ProtocolError::invalid_response("MiniMax base_resp.status_code must be an integer")
                .with_request_id(Some(response.request_id.clone()))
        })?;
    if status_code == 0 {
        return Ok(());
    }
    let message = base_resp
        .get("status_msg")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or("MiniMax request failed");
    let kind = match status_code {
        1004 => ProtocolErrorKind::Authentication,
        1001 => ProtocolErrorKind::Timeout,
        1026 | 1027 | 1039 | 1042 | 2013 => ProtocolErrorKind::InvalidRequest,
        _ => ProtocolErrorKind::Transport,
    };
    Err(
        ProtocolError::new(kind, format!("MiniMax error {status_code}: {message}"))
            .with_request_id(Some(response.request_id.clone()))
            .with_retry_after(response.retry_after),
    )
}

fn minimax_http_error(response: &HttpResponse) -> ProtocolError {
    let parsed = serde_json::from_slice::<Value>(&response.body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|value| value.pointer("/base_resp/status_msg"))
                .and_then(Value::as_str)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or("MiniMax request failed");
    let kind = match response.status {
        reqwest::StatusCode::BAD_REQUEST
        | reqwest::StatusCode::NOT_FOUND
        | reqwest::StatusCode::METHOD_NOT_ALLOWED => ProtocolErrorKind::InvalidRequest,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ProtocolErrorKind::Authentication
        }
        reqwest::StatusCode::REQUEST_TIMEOUT | reqwest::StatusCode::GATEWAY_TIMEOUT => {
            ProtocolErrorKind::Timeout
        }
        _ => ProtocolErrorKind::Transport,
    };
    ProtocolError::new(
        kind,
        format!(
            "MiniMax request failed with status {}: {message}",
            response.status
        ),
    )
    .with_request_id(Some(response.request_id.clone()))
    .with_retry_after(response.retry_after)
}

fn rewrite_execution_namespace(execution: ProtocolExecution) -> ProtocolExecution {
    match execution {
        ProtocolExecution::Immediate(mut output) => {
            rewrite_output_namespace(&mut output);
            ProtocolExecution::Immediate(output)
        }
        ProtocolExecution::Stream(stream) => ProtocolExecution::Stream(stream),
        ProtocolExecution::NativeTask(task) => ProtocolExecution::NativeTask(task),
    }
}

fn rewrite_event_namespace(event: ProtocolEvent) -> ProtocolEvent {
    match event {
        ProtocolEvent::Delta(mut value) => {
            rewrite_value_namespace(&mut value);
            ProtocolEvent::Delta(value)
        }
        ProtocolEvent::Progress(mut value) => {
            rewrite_value_namespace(&mut value);
            ProtocolEvent::Progress(value)
        }
        ProtocolEvent::Final(mut output) => {
            rewrite_output_namespace(&mut output);
            ProtocolEvent::Final(output)
        }
    }
}

fn rewrite_output_namespace(output: &mut ProtocolOutput) {
    rewrite_value_namespace(&mut output.value);
}

fn rewrite_value_namespace(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                rewrite_value_namespace(value);
            }
        }
        Value::Object(values) => {
            if values.get("provider").and_then(Value::as_str) == Some("claude") {
                values.insert("provider".to_owned(), Value::String("minimax".to_owned()));
            }
            for value in values.values_mut() {
                rewrite_value_namespace(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, ResolvedCredential, CLAUDE_MESSAGES_OPERATION_ID,
    };
    use buckyos_api::{AiContent, AiMessage, AiRole, LlmChatInvokeRequest};
    use bytes::Bytes;
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;
    use serde_json::json;
    use std::time::Duration;

    fn input(stop: Vec<String>, temperature: Option<f64>) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest {
                exact_model: "logical.model".to_owned(),
                messages: vec![AiMessage::new(AiRole::User, vec![AiContent::text("hello")])],
                tools: Vec::new(),
                response_format: None,
                top_p: None,
                max_output_tokens: Some(128),
                seed: None,
                stop,
                temperature,
                output: None,
                idempotency_key: None,
                task_options: None,
            }),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_owned(),
                Value::String("MiniMax-M2.7".to_owned()),
            )]),
        }
    }

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://api.minimax.io/anthropic".to_owned(),
            credential: Some(
                ResolvedCredential::named_header("secret://minimax", "x-api-key", "secret")
                    .unwrap(),
            ),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }

    #[test]
    fn declares_a_real_derived_adapter_and_delegates_request_encoding() {
        let contract = minimax_messages_dialect_contract();
        assert_eq!(contract.base_adapter_id, CLAUDE_MESSAGES_ADAPTER_ID);
        assert!(contract.override_points.contains("base_resp_error"));
        assert!(contract.unsupported_parameters.is_empty());

        let (descriptor, registration) = minimax_messages_adapter();
        assert_eq!(
            descriptor.base_adapter_id.as_deref(),
            Some(CLAUDE_MESSAGES_ADAPTER_ID)
        );
        assert_eq!(descriptor.operations.len(), 1);
        let request = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input(Vec::new(), Some(1.5)),
                context: &context(),
            })
            .unwrap();
        assert_eq!(request.url, "https://api.minimax.io/anthropic/v1/messages");
        assert_eq!(request.headers["x-api-key"], "secret");
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON request");
        };
        assert_eq!(body["temperature"], 1.5);
    }

    #[test]
    fn rejects_out_of_range_temperature() {
        let (_, registration) = minimax_messages_adapter();
        let codec = &registration.operation_codecs[0];
        let temperature = input(Vec::new(), Some(2.1));
        assert!(codec
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &temperature,
                context: &context(),
            })
            .is_err());
    }

    #[tokio::test]
    async fn maps_minimax_base_resp_without_changing_the_base_codec() {
        let (_, registration) = minimax_messages_adapter();
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from_static(
                br#"{"base_resp":{"status_code":1004,"status_msg":"bad key"}}"#,
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let error = registration.operation_codecs[0]
            .decode(response)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Authentication);
        assert!(error.message.contains("1004"));
        assert_eq!(error.request_id.as_deref(), Some("request-1"));
    }

    #[tokio::test]
    async fn reuses_the_claude_response_contract() {
        let (_, registration) = minimax_messages_adapter();
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from(
                serde_json::to_vec(&json!({
                    "id": "msg-1",
                    "type": "message",
                    "role": "assistant",
                    "model": "MiniMax-M2.7",
                    "content": [{"type": "text", "text": "hello"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 2, "output_tokens": 1},
                    "base_resp": {"status_code": 0, "status_msg": "success"}
                }))
                .unwrap(),
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let execution = registration.operation_codecs[0]
            .decode(response)
            .await
            .unwrap();
        let ProtocolExecution::Immediate(output) = execution else {
            panic!("expected immediate response");
        };
        assert_eq!(output.usage.unwrap().total_tokens, Some(3));
    }

    #[test]
    fn operation_identity_is_the_shared_messages_operation() {
        let (descriptor, _) = minimax_messages_adapter();
        assert!(descriptor
            .operations
            .contains_key(CLAUDE_MESSAGES_OPERATION_ID));
    }
}
