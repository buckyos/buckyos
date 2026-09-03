use super::{
    openai_responses_adapter, AdapterDescriptor, AdapterStatus, CodecCall, CodecInput,
    CodecRegistration, ExecutionMode, HttpRequest, HttpResponse, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolEvent, ProtocolExecution, ProtocolOutput,
    ProtocolResultValue, ProtocolStream, StreamingHttpResponse, OPENAI_RESPONSES_ADAPTER_ID,
    OPENAI_RESPONSES_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::ApiType;
use futures_util::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const DEEPSEEK_RESPONSES_ADAPTER_ID: &str = "deepseek-responses";
pub(crate) const DOUBAO_RESPONSES_ADAPTER_ID: &str = "doubao-responses";
pub(crate) const QWEN_RESPONSES_ADAPTER_ID: &str = "qwen-responses";

const QWEN_SESSION_CACHE_PARAMETER: &str = "session_cache";
const QWEN_SESSION_CACHE_HEADER: &str = "x-dashscope-session-cache";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponsesDialectKind {
    DeepSeek,
    Doubao,
    Qwen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResponsesDialectContract {
    pub protocol_adapter_id: &'static str,
    pub base_adapter_id: &'static str,
    pub override_points: BTreeSet<&'static str>,
}

impl ResponsesDialectKind {
    pub(crate) fn contract(self) -> ResponsesDialectContract {
        match self {
            Self::DeepSeek => ResponsesDialectContract {
                protocol_adapter_id: DEEPSEEK_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from(["provider_state_namespace"]),
            },
            Self::Doubao => ResponsesDialectContract {
                protocol_adapter_id: DOUBAO_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from(["provider_state_namespace"]),
            },
            Self::Qwen => ResponsesDialectContract {
                protocol_adapter_id: QWEN_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from([
                    "session_cache_header",
                    "provider_state_namespace",
                ]),
            },
        }
    }

    fn provider_namespace(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::Doubao => "doubao",
            Self::Qwen => "qwen",
        }
    }
}

pub(crate) fn wp08e_responses_adapters(
) -> ProtocolResultValue<Vec<(AdapterDescriptor, CodecRegistration)>> {
    [
        ResponsesDialectKind::DeepSeek,
        ResponsesDialectKind::Doubao,
        ResponsesDialectKind::Qwen,
    ]
    .into_iter()
    .map(responses_dialect_adapter)
    .collect()
}

fn responses_dialect_adapter(
    dialect: ResponsesDialectKind,
) -> ProtocolResultValue<(AdapterDescriptor, CodecRegistration)> {
    let (base_descriptor, mut base_registration) = openai_responses_adapter();
    let operation = base_descriptor
        .operations
        .get(OPENAI_RESPONSES_OPERATION_ID)
        .cloned()
        .ok_or_else(|| ProtocolError::invalid_configuration("Responses operation is missing"))?;
    let codec_index = base_registration
        .operation_codecs
        .iter()
        .position(|codec| {
            codec.descriptor().operation_id == OPENAI_RESPONSES_OPERATION_ID
                && codec.api_type() == ApiType::Llm
        })
        .ok_or_else(|| ProtocolError::invalid_configuration("Responses LLM codec is missing"))?;
    let base_codec = base_registration.operation_codecs.swap_remove(codec_index);
    let contract = dialect.contract();
    let descriptor = AdapterDescriptor {
        protocol_family_id: base_descriptor.protocol_family_id,
        protocol_adapter_id: contract.protocol_adapter_id.to_string(),
        interface_generation: "responses-v1".to_string(),
        base_adapter_id: Some(contract.base_adapter_id.to_string()),
        status: AdapterStatus::Stable,
        operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
    };
    let codec: Arc<dyn OperationCodec> = Arc::new(ResponsesDialectCodec {
        dialect,
        descriptor: operation,
        base: base_codec,
    });
    Ok((
        descriptor,
        CodecRegistration {
            operation_codecs: vec![codec],
            native_task_codecs: Vec::new(),
        },
    ))
}

struct ResponsesDialectCodec {
    dialect: ResponsesDialectKind,
    descriptor: OperationDescriptor,
    base: Arc<dyn OperationCodec>,
}

#[async_trait]
impl OperationCodec for ResponsesDialectCodec {
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
        let mut parameters = call.input.resolved_parameters.clone();
        let session_cache = if self.dialect == ResponsesDialectKind::Qwen {
            parameters.remove(QWEN_SESSION_CACHE_PARAMETER)
        } else {
            None
        };
        let input = CodecInput {
            canonical_request: call.input.canonical_request.clone(),
            resolved_parameters: parameters,
        };
        let delegated = CodecCall {
            api_type: call.api_type,
            input: &input,
            context: call.context,
        };
        let mut request = self.base.encode(&delegated)?;
        if let Some(enabled) = session_cache {
            let enabled = enabled.as_bool().ok_or_else(|| {
                ProtocolError::invalid_request("Qwen session_cache must be a boolean")
            })?;
            request.headers.insert(
                HeaderName::from_static(QWEN_SESSION_CACHE_HEADER),
                HeaderValue::from_static(if enabled { "enable" } else { "disable" }),
            );
        }
        Ok(request)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        let execution = self.base.decode(response).await?;
        Ok(rewrite_execution_namespace(
            execution,
            self.dialect.provider_namespace(),
        ))
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        let stream = self.base.decode_stream(response).await?;
        let namespace = self.dialect.provider_namespace();
        Ok(ProtocolStream {
            events: Box::pin(
                stream
                    .events
                    .map(move |event| event.map(|event| rewrite_event_namespace(event, namespace))),
            ),
        })
    }
}

fn rewrite_execution_namespace(execution: ProtocolExecution, namespace: &str) -> ProtocolExecution {
    match execution {
        ProtocolExecution::Immediate(mut output) => {
            rewrite_value_namespace(&mut output.value, namespace);
            ProtocolExecution::Immediate(output)
        }
        ProtocolExecution::Stream(stream) => ProtocolExecution::Stream(stream),
        ProtocolExecution::NativeTask(task) => ProtocolExecution::NativeTask(task),
    }
}

fn rewrite_event_namespace(event: ProtocolEvent, namespace: &str) -> ProtocolEvent {
    match event {
        ProtocolEvent::Delta(mut value) => {
            rewrite_value_namespace(&mut value, namespace);
            ProtocolEvent::Delta(value)
        }
        ProtocolEvent::Progress(mut value) => {
            rewrite_value_namespace(&mut value, namespace);
            ProtocolEvent::Progress(value)
        }
        ProtocolEvent::Final(mut output) => {
            rewrite_output_namespace(&mut output, namespace);
            ProtocolEvent::Final(output)
        }
    }
}

fn rewrite_output_namespace(output: &mut ProtocolOutput, namespace: &str) {
    rewrite_value_namespace(&mut output.value, namespace);
}

fn rewrite_value_namespace(value: &mut Value, namespace: &str) {
    match value {
        Value::Array(values) => {
            for value in values {
                rewrite_value_namespace(value, namespace);
            }
        }
        Value::Object(values) => {
            if values.get("provider").and_then(Value::as_str) == Some("openai") {
                values.insert("provider".to_string(), Value::String(namespace.to_string()));
            }
            for value in values.values_mut() {
                rewrite_value_namespace(value, namespace);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CodecContext, CodecLimits, CodecRegistry, ResolvedCredential};
    use buckyos_api::{AiContent, AiMessage, AiRole, AiccCall, LlmChatInvokeRequest};
    use reqwest::header::AUTHORIZATION;
    use std::time::Duration;

    fn input(parameters: BTreeMap<String, Value>) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "logical.model",
                vec![AiMessage::new(
                    AiRole::User,
                    vec![AiContent::Text {
                        text: "hello".to_string(),
                    }],
                )],
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                Value::String("provider-model".to_string()),
            )])
            .into_iter()
            .chain(parameters)
            .collect(),
        }
    }

    fn context(base_url: &str) -> CodecContext {
        CodecContext {
            base_url: base_url.to_string(),
            credential: Some(ResolvedCredential::bearer("secret://provider", "secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(10),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }

    #[test]
    fn derived_adapters_declare_base_overrides_and_only_llm_operation() {
        let adapters = wp08e_responses_adapters().unwrap();
        assert_eq!(adapters.len(), 3);
        for (descriptor, registration) in adapters {
            assert_eq!(
                descriptor.base_adapter_id.as_deref(),
                Some(OPENAI_RESPONSES_ADAPTER_ID)
            );
            assert_eq!(descriptor.operations.len(), 1);
            assert!(descriptor
                .operations
                .contains_key(OPENAI_RESPONSES_OPERATION_ID));
            assert_eq!(registration.operation_codecs.len(), 1);
            assert!(registration.native_task_codecs.is_empty());
        }
        assert_eq!(
            ResponsesDialectKind::DeepSeek.contract().override_points,
            BTreeSet::from(["provider_state_namespace"])
        );
        assert!(ResponsesDialectKind::Qwen
            .contract()
            .override_points
            .contains("session_cache_header"));
    }

    #[test]
    fn derived_registration_is_one_way_and_base_remains_independently_usable() {
        let (base_descriptor, base_registration) = openai_responses_adapter();
        let mut registry = CodecRegistry::default();
        registry
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        for (descriptor, registration) in wp08e_responses_adapters().unwrap() {
            registry.register_derived(descriptor, registration).unwrap();
        }
        for adapter_id in [
            OPENAI_RESPONSES_ADAPTER_ID,
            DEEPSEEK_RESPONSES_ADAPTER_ID,
            DOUBAO_RESPONSES_ADAPTER_ID,
            QWEN_RESPONSES_ADAPTER_ID,
        ] {
            assert!(registry
                .operation_descriptor(adapter_id, OPENAI_RESPONSES_OPERATION_ID, ApiType::Llm,)
                .is_ok());
        }

        let (base_descriptor, base_registration) = openai_responses_adapter();
        let mut base_only = CodecRegistry::default();
        base_only
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        assert!(base_only.adapter(OPENAI_RESPONSES_ADAPTER_ID).is_some());
    }

    #[test]
    fn all_dialects_delegate_the_base_responses_contract() {
        for dialect in [
            ResponsesDialectKind::DeepSeek,
            ResponsesDialectKind::Doubao,
            ResponsesDialectKind::Qwen,
        ] {
            let (_, registration) = responses_dialect_adapter(dialect).unwrap();
            let request = registration.operation_codecs[0]
                .encode(&CodecCall {
                    api_type: ApiType::Llm,
                    input: &input(BTreeMap::new()),
                    context: &context("https://provider.example/v1"),
                })
                .unwrap();
            assert_eq!(request.url, "https://provider.example/v1/responses");
            assert!(request.headers.contains_key(AUTHORIZATION));
        }
    }

    #[test]
    fn qwen_maps_session_cache_to_header_without_leaking_it_to_base_body() {
        let (_, registration) = responses_dialect_adapter(ResponsesDialectKind::Qwen).unwrap();
        let request = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input(BTreeMap::from([(
                    QWEN_SESSION_CACHE_PARAMETER.to_string(),
                    Value::Bool(true),
                )])),
                context: &context(
                    "https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                ),
            })
            .unwrap();
        assert_eq!(
            request.headers[QWEN_SESSION_CACHE_HEADER],
            HeaderValue::from_static("enable")
        );
        let crate::protocol::HttpBody::Json(body) = request.body else {
            panic!("expected JSON request")
        };
        assert!(body.get(QWEN_SESSION_CACHE_PARAMETER).is_none());
    }

    #[test]
    fn provider_parameter_policy_is_not_hardcoded_in_dialects() {
        for (dialect, parameter) in [
            (ResponsesDialectKind::DeepSeek, "store"),
            (ResponsesDialectKind::Qwen, "background"),
        ] {
            let (_, registration) = responses_dialect_adapter(dialect).unwrap();
            let request = registration.operation_codecs[0]
                .encode(&CodecCall {
                    api_type: ApiType::Llm,
                    input: &input(BTreeMap::from([(parameter.to_string(), Value::Bool(true))])),
                    context: &context("https://provider.example/v1"),
                })
                .unwrap();
            let crate::protocol::HttpBody::Json(body) = request.body else {
                panic!("expected JSON request")
            };
            assert_eq!(body[parameter], Value::Bool(true));
        }
    }

    #[test]
    fn provider_state_is_namespaced_to_the_derived_provider() {
        let mut value = serde_json::json!({
            "message": {"content": [{"provider": "openai", "value": {"type": "vendor_tool"}}]},
            "provider_states": [{"provider": "openai", "value": {"type": "vendor_tool"}}]
        });
        rewrite_value_namespace(&mut value, "doubao");
        assert_eq!(
            value.pointer("/provider_states/0/provider"),
            Some(&Value::String("doubao".to_string()))
        );
        assert_eq!(
            value.pointer("/message/content/0/provider"),
            Some(&Value::String("doubao".to_string()))
        );
    }
}
