use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecInput, CodecRegistration, ExecutionMode,
    HttpRequest, HttpResponse, OperationCodec, OperationDescriptor, ProtocolError, ProtocolEvent,
    ProtocolExecution, ProtocolOutput, ProtocolResultValue, ProtocolStream, StreamingHttpResponse,
    OPENAI_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::ApiType;
use futures_util::StreamExt;
use reqwest::{
    header::{HeaderName, HeaderValue},
    Url,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const DEEPSEEK_RESPONSES_ADAPTER_ID: &str = "deepseek-responses";
pub(crate) const DOUBAO_RESPONSES_ADAPTER_ID: &str = "doubao-responses";
pub(crate) const OPENROUTER_RESPONSES_ADAPTER_ID: &str = "openrouter-responses";
pub(crate) const QWEN_RESPONSES_ADAPTER_ID: &str = "qwen-responses";

const QWEN_SESSION_CACHE_PARAMETER: &str = "session_cache";
const QWEN_SESSION_CACHE_HEADER: &str = "x-dashscope-session-cache";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponsesDialectKind {
    DeepSeek,
    Doubao,
    OpenRouter,
    Qwen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResponsesDialectContract {
    pub protocol_adapter_id: &'static str,
    pub base_adapter_id: &'static str,
    pub override_points: BTreeSet<ResponsesOverridePoint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum ResponsesOverridePoint {
    EndpointPath,
    ProviderStateNamespace,
    SessionCacheHeader,
}

impl ResponsesDialectKind {
    pub(crate) fn contract(self) -> ResponsesDialectContract {
        match self {
            Self::DeepSeek => ResponsesDialectContract {
                protocol_adapter_id: DEEPSEEK_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from([
                    ResponsesOverridePoint::EndpointPath,
                    ResponsesOverridePoint::ProviderStateNamespace,
                ]),
            },
            Self::Doubao => ResponsesDialectContract {
                protocol_adapter_id: DOUBAO_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from([ResponsesOverridePoint::ProviderStateNamespace]),
            },
            Self::OpenRouter => ResponsesDialectContract {
                protocol_adapter_id: OPENROUTER_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from([ResponsesOverridePoint::ProviderStateNamespace]),
            },
            Self::Qwen => ResponsesDialectContract {
                protocol_adapter_id: QWEN_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from([
                    ResponsesOverridePoint::SessionCacheHeader,
                    ResponsesOverridePoint::ProviderStateNamespace,
                ]),
            },
        }
    }

    #[cfg(test)]
    fn provider_namespace(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::Doubao => "doubao",
            Self::OpenRouter => "openrouter",
            Self::Qwen => "qwen",
        }
    }
}

pub(crate) fn openai_responses_compatible_adapters(
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

pub(crate) fn responses_dialect_adapter(
    dialect: ResponsesDialectKind,
) -> ProtocolResultValue<(AdapterDescriptor, CodecRegistration)> {
    let reported_cost_currency = (dialect == ResponsesDialectKind::OpenRouter).then_some("USD");
    let (base_descriptor, base_registration) =
        super::openai_responses::openai_responses_adapter_with_reported_cost_currency(
            reported_cost_currency,
        );
    let operation = base_descriptor
        .operations
        .get(OPENAI_RESPONSES_OPERATION_ID)
        .cloned()
        .ok_or_else(|| ProtocolError::invalid_configuration("Responses operation is missing"))?;
    let base_codecs = base_registration
        .operation_codecs
        .into_iter()
        .filter(|codec| codec.descriptor().operation_id == OPENAI_RESPONSES_OPERATION_ID)
        .collect::<Vec<_>>();
    if !base_codecs
        .iter()
        .any(|codec| codec.api_type() == ApiType::Llm)
    {
        return Err(ProtocolError::invalid_configuration(
            "Responses LLM codec is missing",
        ));
    }
    let contract = dialect.contract();
    let mut descriptor = AdapterDescriptor {
        protocol_family_id: base_descriptor.protocol_family_id,
        protocol_adapter_id: contract.protocol_adapter_id.to_string(),
        interface_generation: "responses-v1".to_string(),
        base_adapter_id: Some(contract.base_adapter_id.to_string()),
        component_adapter_ids: Vec::new(),
        status: AdapterStatus::Stable,
        probe_priority: 200,
        probe_path: None,
        credential: super::AdapterCredentialContract::bearer(),
        operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
    };
    let strategy = dialect_strategy(dialect);
    let mut registration = CodecRegistration {
        operation_codecs: base_codecs
            .into_iter()
            .map(|base| {
                Arc::new(ResponsesDialectCodec {
                    dialect: strategy.clone(),
                    descriptor: operation.clone(),
                    api_type: base.api_type(),
                    base,
                }) as Arc<dyn OperationCodec>
            })
            .collect(),
        native_task_codecs: Vec::new(),
    };
    let media = match dialect {
        ResponsesDialectKind::Doubao => Some((
            "doubao",
            super::DOUBAO_MEDIA_ADAPTER_ID,
            super::doubao_media::doubao_media_registration(),
        )),
        ResponsesDialectKind::Qwen => Some((
            "qwen",
            super::QWEN_MEDIA_ADAPTER_ID,
            super::qwen_media::qwen_media_registration(),
        )),
        _ => None,
    };
    if let Some((family, component_id, (operations, media_registration))) = media {
        descriptor.protocol_family_id = family.to_owned();
        descriptor.component_adapter_ids = vec![
            OPENAI_RESPONSES_ADAPTER_ID.to_owned(),
            component_id.to_owned(),
        ];
        for operation in operations {
            descriptor
                .operations
                .insert(operation.operation_id.clone(), operation);
        }
        registration
            .operation_codecs
            .extend(media_registration.operation_codecs);
        registration
            .native_task_codecs
            .extend(media_registration.native_task_codecs);
    }
    if dialect == ResponsesDialectKind::Doubao {
        let (operation, speech_registration) = super::doubao_speech::doubao_speech_registration();
        descriptor
            .component_adapter_ids
            .push(super::doubao_speech::DOUBAO_SPEECH_ADAPTER_ID.to_owned());
        descriptor
            .operations
            .insert(operation.operation_id.clone(), operation);
        registration
            .operation_codecs
            .extend(speech_registration.operation_codecs);
    }
    Ok((descriptor, registration))
}

struct ResponsesDialectCodec {
    dialect: Arc<dyn ResponsesDialectStrategy>,
    descriptor: OperationDescriptor,
    api_type: ApiType,
    base: Arc<dyn OperationCodec>,
}

#[derive(Default)]
struct PreparedDialectRequest {
    body_extensions: BTreeMap<String, Value>,
    session_cache: Option<Value>,
}

trait ResponsesDialectStrategy: Send + Sync {
    fn provider_namespace(&self) -> &'static str;

    fn prepare_parameters(
        &self,
        _parameters: &mut BTreeMap<String, Value>,
    ) -> ProtocolResultValue<PreparedDialectRequest> {
        Ok(PreparedDialectRequest::default())
    }

    fn transform_request(
        &self,
        _call: &CodecCall<'_>,
        request: HttpRequest,
        _prepared: PreparedDialectRequest,
    ) -> ProtocolResultValue<HttpRequest> {
        Ok(request)
    }
}

struct StandardDialect(&'static str);

impl ResponsesDialectStrategy for StandardDialect {
    fn provider_namespace(&self) -> &'static str {
        self.0
    }
}

struct DeepSeekDialect;

impl ResponsesDialectStrategy for DeepSeekDialect {
    fn provider_namespace(&self) -> &'static str {
        "deepseek"
    }

    fn transform_request(
        &self,
        call: &CodecCall<'_>,
        mut request: HttpRequest,
        _prepared: PreparedDialectRequest,
    ) -> ProtocolResultValue<HttpRequest> {
        let base = Url::parse(&call.context.base_url)
            .map_err(|_| ProtocolError::invalid_configuration("DeepSeek base URL is invalid"))?;
        if base.path().trim_matches('/').is_empty() {
            let mut endpoint = base;
            endpoint.set_path("/responses");
            request.url = endpoint.to_string();
        }
        Ok(request)
    }
}

struct OpenRouterDialect;

impl ResponsesDialectStrategy for OpenRouterDialect {
    fn provider_namespace(&self) -> &'static str {
        "openrouter"
    }

    fn prepare_parameters(
        &self,
        parameters: &mut BTreeMap<String, Value>,
    ) -> ProtocolResultValue<PreparedDialectRequest> {
        Ok(PreparedDialectRequest {
            body_extensions: take_openrouter_parameters(parameters)?,
            session_cache: None,
        })
    }

    fn transform_request(
        &self,
        _call: &CodecCall<'_>,
        mut request: HttpRequest,
        prepared: PreparedDialectRequest,
    ) -> ProtocolResultValue<HttpRequest> {
        if prepared.body_extensions.is_empty() {
            return Ok(request);
        }
        let super::HttpBody::Json(body) = &mut request.body else {
            return Err(ProtocolError::invalid_configuration(
                "OpenRouter Responses request body is not JSON",
            ));
        };
        let body = body.as_object_mut().ok_or_else(|| {
            ProtocolError::invalid_configuration(
                "OpenRouter Responses request body is not an object",
            )
        })?;
        body.extend(prepared.body_extensions);
        Ok(request)
    }
}

struct QwenDialect;

impl ResponsesDialectStrategy for QwenDialect {
    fn provider_namespace(&self) -> &'static str {
        "qwen"
    }

    fn prepare_parameters(
        &self,
        parameters: &mut BTreeMap<String, Value>,
    ) -> ProtocolResultValue<PreparedDialectRequest> {
        Ok(PreparedDialectRequest {
            body_extensions: BTreeMap::new(),
            session_cache: parameters.remove(QWEN_SESSION_CACHE_PARAMETER),
        })
    }

    fn transform_request(
        &self,
        _call: &CodecCall<'_>,
        mut request: HttpRequest,
        prepared: PreparedDialectRequest,
    ) -> ProtocolResultValue<HttpRequest> {
        if let Some(enabled) = prepared.session_cache {
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
}

fn dialect_strategy(dialect: ResponsesDialectKind) -> Arc<dyn ResponsesDialectStrategy> {
    match dialect {
        ResponsesDialectKind::DeepSeek => Arc::new(DeepSeekDialect),
        ResponsesDialectKind::Doubao => Arc::new(StandardDialect("doubao")),
        ResponsesDialectKind::OpenRouter => Arc::new(OpenRouterDialect),
        ResponsesDialectKind::Qwen => Arc::new(QwenDialect),
    }
}

#[async_trait]
impl OperationCodec for ResponsesDialectCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        self.base.execution_modes()
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let mut parameters = call.input.resolved_parameters.clone();
        let prepared = self.dialect.prepare_parameters(&mut parameters)?;
        let input = CodecInput {
            canonical_request: call.input.canonical_request.clone(),
            resolved_parameters: parameters,
        };
        let delegated = CodecCall {
            api_type: call.api_type,
            input: &input,
            context: call.context,
        };
        let request = self.base.encode(&delegated)?;
        self.dialect.transform_request(call, request, prepared)
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

fn take_openrouter_parameters(
    parameters: &mut BTreeMap<String, Value>,
) -> ProtocolResultValue<BTreeMap<String, Value>> {
    let mut extensions = BTreeMap::new();
    for name in [
        "cache_control",
        "debug",
        "image_config",
        "max_tool_calls",
        "modalities",
        "models",
        "plugins",
        "prompt_cache_key",
        "provider",
        "route",
        "safety_identifier",
        "session_id",
        "stop_server_tools_when",
    ] {
        let Some(value) = parameters.remove(name) else {
            continue;
        };
        let valid = match name {
            "cache_control" | "debug" | "image_config" | "provider" => value.is_object(),
            "max_tool_calls" => value.is_u64(),
            "modalities" | "models" => value
                .as_array()
                .is_some_and(|items| !items.is_empty() && items.iter().all(Value::is_string)),
            "plugins" | "stop_server_tools_when" => value.is_array(),
            "prompt_cache_key" | "route" | "safety_identifier" | "session_id" => {
                value.as_str().is_some_and(|value| !value.trim().is_empty())
            }
            _ => false,
        };
        if !valid {
            return Err(ProtocolError::invalid_request(format!(
                "OpenRouter Responses parameter `{name}` has an invalid value"
            )));
        }
        extensions.insert(name.to_owned(), value);
    }
    Ok(extensions)
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
    use crate::protocol::{
        openai_responses_adapter, CodecContext, CodecLimits, CodecRegistry, HttpBody,
        ResolvedCredential,
    };
    use buckyos_api::{AiContent, AiMessage, AiRole, AiccCall, LlmChatInvokeRequest};
    use reqwest::header::AUTHORIZATION;
    use serde_json::json;
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
            state_coordinate: buckyos_api::ProviderStateCoordinate {
                provider_profile_id: "test".into(),
                adapter_type: OPENAI_RESPONSES_ADAPTER_ID.into(),
                origin_provider: "test".into(),
                origin_model: "test-model".into(),
            },
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
    fn derived_adapters_declare_base_and_provider_media_components() {
        let adapters = openai_responses_compatible_adapters().unwrap();
        assert_eq!(adapters.len(), 3);
        for (descriptor, registration) in adapters {
            assert_eq!(
                descriptor.base_adapter_id.as_deref(),
                Some(OPENAI_RESPONSES_ADAPTER_ID)
            );
            assert!(descriptor
                .operations
                .contains_key(OPENAI_RESPONSES_OPERATION_ID));
            if descriptor.protocol_adapter_id == DEEPSEEK_RESPONSES_ADAPTER_ID {
                assert_eq!(descriptor.operations.len(), 1);
                assert!(registration
                    .operation_codecs
                    .iter()
                    .any(|codec| codec.api_type() == ApiType::Llm));
                assert!(registration
                    .operation_codecs
                    .iter()
                    .any(|codec| codec.api_type() == ApiType::VisionOcr));
                assert!(registration
                    .operation_codecs
                    .iter()
                    .any(|codec| codec.api_type() == ApiType::VisionCaption));
                assert!(registration.native_task_codecs.is_empty());
            } else {
                let expected_operations = match descriptor.protocol_adapter_id.as_str() {
                    DOUBAO_RESPONSES_ADAPTER_ID => 5,
                    QWEN_RESPONSES_ADAPTER_ID => 4,
                    _ => 3,
                };
                assert_eq!(descriptor.operations.len(), expected_operations);
                assert!(registration.operation_codecs.len() >= 1);
                assert!(!registration.native_task_codecs.is_empty());
                let expected_components =
                    if descriptor.protocol_adapter_id == DOUBAO_RESPONSES_ADAPTER_ID {
                        3
                    } else {
                        2
                    };
                assert_eq!(descriptor.component_adapter_ids.len(), expected_components);
            }
        }
        assert_eq!(
            ResponsesDialectKind::DeepSeek.contract().override_points,
            BTreeSet::from([
                ResponsesOverridePoint::EndpointPath,
                ResponsesOverridePoint::ProviderStateNamespace,
            ])
        );
        assert!(ResponsesDialectKind::Qwen
            .contract()
            .override_points
            .contains(&ResponsesOverridePoint::SessionCacheHeader));
        assert_eq!(
            ResponsesDialectKind::OpenRouter.contract().override_points,
            BTreeSet::from([ResponsesOverridePoint::ProviderStateNamespace])
        );
    }

    #[test]
    fn derived_registration_is_one_way_and_base_remains_independently_usable() {
        let (base_descriptor, base_registration) = openai_responses_adapter();
        let mut registry = CodecRegistry::default();
        registry
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        for (descriptor, registration) in [
            super::super::doubao_media_adapter(),
            super::super::doubao_speech_adapter(),
            super::super::qwen_media_adapter(),
        ] {
            registry.register_codecs(descriptor, registration).unwrap();
        }
        for (descriptor, registration) in openai_responses_compatible_adapters().unwrap() {
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
            ResponsesDialectKind::OpenRouter,
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

    #[tokio::test]
    async fn openrouter_assigns_usd_to_reported_numeric_cost() {
        let (_, registration) =
            responses_dialect_adapter(ResponsesDialectKind::OpenRouter).unwrap();
        let response = HttpResponse {
            status: reqwest::StatusCode::OK,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from_static(
                br#"{"id":"resp_1","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2,"cost":0.001}}"#,
            ),
            request_id: "request-1".into(),
            retry_after: None,
        };
        let ProtocolExecution::Immediate(output) = registration.operation_codecs[0]
            .decode(response)
            .await
            .unwrap()
        else {
            panic!("expected immediate response")
        };
        let cost = output.usage.unwrap().cost.unwrap();
        assert_eq!(cost.amount, 0.001);
        assert_eq!(cost.currency, "USD");
    }

    #[test]
    fn derived_provider_state_round_trips_back_to_the_wire_request() {
        for dialect in [
            ResponsesDialectKind::DeepSeek,
            ResponsesDialectKind::Doubao,
            ResponsesDialectKind::Qwen,
        ] {
            let (_, registration) = responses_dialect_adapter(dialect).unwrap();
            let mut context = context("https://provider.example/v1");
            context.state_coordinate.adapter_type =
                dialect.contract().protocol_adapter_id.to_owned();
            context.state_coordinate.origin_provider = dialect.provider_namespace().to_owned();
            let state = json!({"type":"reasoning","id":"reasoning-1","encrypted_content":"opaque"});
            let input = CodecInput {
                canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                    "logical.model",
                    vec![AiMessage::new(
                        AiRole::Assistant,
                        vec![AiContent::ProviderState {
                            source: context.state_coordinate.clone(),
                            provider: dialect.provider_namespace().to_owned(),
                            value: state.clone(),
                        }],
                    )],
                )),
                resolved_parameters: BTreeMap::from([(
                    "provider_model_id".to_owned(),
                    Value::String("provider-model".to_owned()),
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
            assert_eq!(body["input"][0], state);
        }
    }

    #[test]
    fn deepseek_empty_base_path_uses_official_responses_endpoint() {
        let (_, registration) = responses_dialect_adapter(ResponsesDialectKind::DeepSeek).unwrap();
        let request = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input(BTreeMap::new()),
                context: &context("https://api.deepseek.com"),
            })
            .unwrap();
        assert_eq!(request.url, "https://api.deepseek.com/responses");
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
