//! The ecosystem-style Messages client (`legacy-claude-messages`): what it
//! changes from the official client, exercised through the real registration.

#[cfg(test)]
mod tests {
    use crate::protocol::{
        CodecCall, CodecContext, CodecInput, CodecLimits, HttpBody, HttpResponse,
        ProtocolExecution, ResolvedCredential, CLAUDE_MESSAGES_OPERATION_ID,
    };
    use buckyos_api::AiccCall;
    use buckyos_api::ApiType;
    use buckyos_api::{AiContent, AiMessage, AiRole, LlmChatInvokeRequest};
    use bytes::Bytes;
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;
    use serde_json::json;
    use serde_json::Value;
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
                adapter_type: "legacy-claude-messages".into(),
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
    fn canonical_sampling_wins_over_valid_provider_options() {
        let (_, registration) = crate::protocol::legacy_claude_messages_adapter();
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
        let (descriptor, registration) = crate::protocol::legacy_claude_messages_adapter();
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
        let (_, registration) = crate::protocol::legacy_claude_messages_adapter();
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
    async fn reuses_the_claude_response_contract() {
        let (_, registration) = crate::protocol::legacy_claude_messages_adapter();
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
        let (descriptor, _) = crate::protocol::legacy_claude_messages_adapter();
        assert!(descriptor
            .operations
            .contains_key(CLAUDE_MESSAGES_OPERATION_ID));
    }

    #[test]
    fn provider_state_round_trips_back_to_the_wire_request() {
        let (_, registration) = crate::protocol::legacy_claude_messages_adapter();
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
