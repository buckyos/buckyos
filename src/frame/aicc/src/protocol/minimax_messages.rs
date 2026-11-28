use super::{
    AdapterDescriptor, CodecCall, CodecRegistration, ExecutionMode, HttpRequest, HttpResponse,
    OperationCodec, OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution,
    ProtocolResultValue, ProtocolStream, StreamingHttpResponse,
};
use async_trait::async_trait;
use buckyos_api::ApiType;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

pub(crate) const MINIMAX_MESSAGES_ADAPTER_ID: &str = "minimax-messages";

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MiniMaxMessagesDialectContract {
    pub base_adapter_id: &'static str,
    pub override_points: BTreeSet<&'static str>,
    pub unsupported_parameters: BTreeSet<&'static str>,
    pub unsupported_capabilities: BTreeSet<&'static str>,
}

#[cfg(test)]
pub(crate) fn minimax_messages_dialect_contract() -> MiniMaxMessagesDialectContract {
    MiniMaxMessagesDialectContract {
        base_adapter_id: super::LEGACY_CLAUDE_MESSAGES_ADAPTER_ID,
        override_points: BTreeSet::from(["base_resp_error", "inline_resources"]),
        unsupported_parameters: BTreeSet::new(),
        unsupported_capabilities: BTreeSet::new(),
    }
}

/// The ecosystem Messages client plus MiniMax's own error envelope: a
/// successful HTTP status can carry a failed `base_resp`.
pub(crate) fn minimax_messages_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (mut descriptor, registration) = super::legacy_claude_messages_adapter();
    descriptor.protocol_adapter_id = MINIMAX_MESSAGES_ADAPTER_ID.to_owned();
    descriptor.base_adapter_id = Some(super::LEGACY_CLAUDE_MESSAGES_ADAPTER_ID.to_owned());
    (
        descriptor,
        CodecRegistration {
            operation_codecs: registration
                .operation_codecs
                .into_iter()
                .map(|base| {
                    Arc::new(MiniMaxMessagesCodec {
                        descriptor: base.descriptor().clone(),
                        base,
                    }) as Arc<dyn OperationCodec>
                })
                .collect(),
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
        self.base.api_type()
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        self.base.execution_modes()
    }

    // MiniMax takes image inputs inlined, so resources keep the default
    // bytes-only form instead of the base client's URL form.
    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        self.base.encode(call)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        validate_minimax_response(&response)?;
        self.base.decode(response).await
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
        self.base.decode_stream(response).await
    }
}

pub(super) fn validate_minimax_response(response: &HttpResponse) -> ProtocolResultValue<()> {
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
        1008 => ProtocolErrorKind::ProviderRejected,
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
    let kind = super::protocol_error_kind_from_http_status(response.status);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, ResolvedCredential, CLAUDE_MESSAGES_OPERATION_ID,
    };
    use crate::protocol::{HttpBody, CLAUDE_MESSAGES_ADAPTER_ID};
    use buckyos_api::AiccCall;
    use buckyos_api::{AiContent, AiMessage, AiRole, LlmChatInvokeRequest};
    use bytes::Bytes;
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn input(stop: Vec<String>, temperature: Option<f64>) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest {
                exact_model: "logical.model".to_owned(),
                web_search: false,
                trace_id: None,
                execution_mode: Default::default(),
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
                session_id: None,
            }),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_owned(),
                Value::String("MiniMax-M2.7".to_owned()),
            )]),
        }
    }

    fn context() -> CodecContext {
        CodecContext {
            state_namespace: None,
            base_url: "https://api.minimax.io/anthropic".to_owned(),
            state_coordinate: buckyos_api::ProviderStateCoordinate {
                provider_profile_id: "minimax".into(),
                adapter_type: "minimax-messages".into(),
                origin_provider: "minimax".into(),
                origin_model: "test-model".into(),
            },
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
        assert_eq!(
            contract.base_adapter_id,
            crate::protocol::LEGACY_CLAUDE_MESSAGES_ADAPTER_ID
        );
        assert!(contract.override_points.contains("base_resp_error"));
        assert!(contract.unsupported_parameters.is_empty());

        let (descriptor, registration) = minimax_messages_adapter();
        assert_eq!(
            descriptor.base_adapter_id.as_deref(),
            Some(crate::protocol::LEGACY_CLAUDE_MESSAGES_ADAPTER_ID)
        );
        assert_eq!(descriptor.protocol_family_id, "claude");
        assert_eq!(descriptor.operations.len(), 1);
        assert!(registration.operation_codecs.iter().all(|codec| codec
            .descriptor()
            .bindings
            .iter()
            .all(|binding| binding.default_max_output_tokens.is_none())));
        let _ = CLAUDE_MESSAGES_ADAPTER_ID;
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
    fn canonical_sampling_wins_over_valid_provider_options() {
        let (_, registration) = minimax_messages_adapter();
        let mut input = input(vec!["canonical-stop".into()], Some(0.5));
        input
            .resolved_parameters
            .insert("temperature".into(), json!(1.5));
        input
            .resolved_parameters
            .insert("stop_sequences".into(), json!(["provider-stop"]));
        let encoded = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input,
                context: &context(),
            })
            .unwrap();
        let HttpBody::Json(body) = encoded.body else {
            panic!()
        };
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["stop_sequences"], json!(["canonical-stop"]));
    }

    #[test]
    fn allows_omitted_max_tokens_without_adding_a_protocol_default() {
        let (descriptor, registration) = minimax_messages_adapter();
        assert_eq!(
            descriptor.operations[CLAUDE_MESSAGES_OPERATION_ID]
                .binding(ApiType::Llm)
                .unwrap()
                .default_max_output_tokens,
            None
        );
        let mut input = input(Vec::new(), None);
        let AiccCall::ChatCompletionsCreate(request) = &mut input.canonical_request else {
            unreachable!()
        };
        request.max_output_tokens = None;
        let encoded = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input,
                context: &context(),
            })
            .unwrap();
        let HttpBody::Json(body) = encoded.body else {
            panic!()
        };
        assert!(body.get("max_tokens").is_none());
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
    async fn maps_minimax_insufficient_balance_as_non_retriable_provider_rejection() {
        let (_, registration) = minimax_messages_adapter();
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from_static(
                br#"{"base_resp":{"status_code":1008,"status_msg":"insufficient balance"}}"#,
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let error = registration.operation_codecs[0]
            .decode(response)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::ProviderRejected);
        let mapped: buckyos_api::AiccError = error.into();
        assert_eq!(mapped.code, buckyos_api::AiccErrorCode::ProviderError);
        assert!(!mapped.retriable);
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

    #[test]
    fn minimax_provider_state_round_trips_back_to_the_wire_request() {
        let (_, registration) = minimax_messages_adapter();
        let context = context();
        let state = json!({"type":"thinking","thinking":"private","signature":"opaque"});
        let input = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest {
                messages: vec![AiMessage::new(
                    AiRole::Assistant,
                    vec![AiContent::ProviderState {
                        source: context.state_coordinate.clone(),
                        provider: "minimax".to_owned(),
                        value: state.clone(),
                    }],
                )],
                ..match input(Vec::new(), None).canonical_request {
                    AiccCall::ChatCompletionsCreate(request) => request,
                    _ => unreachable!(),
                }
            }),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_owned(),
                Value::String("MiniMax-M2.7".to_owned()),
            )]),
        };
        let request = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input,
                context: &context,
            })
            .unwrap();
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON request");
        };
        assert_eq!(body["messages"][0]["content"][0], state);
    }
}
