use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecInput, CodecRegistration, ExecutionMode,
    HttpRequest, HttpResponse, OperationCodec, OperationDescriptor, ProtocolError,
    ProtocolExecution, ProtocolResultValue, ProtocolStream, StreamingHttpResponse,
    OPENAI_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::{AiContent, AiccCall, ApiType};
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const OPENROUTER_RESPONSES_ADAPTER_ID: &str = "openrouter-responses";
pub(crate) const BAILIAN_RESPONSES_ADAPTER_ID: &str = "bailian-responses";

const BAILIAN_SESSION_CACHE_PARAMETER: &str = "session_cache";
const BAILIAN_SESSION_CACHE_HEADER: &str = "x-dashscope-session-cache";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponsesDialectKind {
    OpenRouter,
    Bailian,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResponsesDialectContract {
    pub protocol_adapter_id: &'static str,
    pub base_adapter_id: &'static str,
    pub override_points: BTreeSet<ResponsesOverridePoint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum ResponsesOverridePoint {
    SessionCacheHeader,
}

impl ResponsesDialectKind {
    pub(crate) fn contract(self) -> ResponsesDialectContract {
        match self {
            Self::OpenRouter => ResponsesDialectContract {
                protocol_adapter_id: OPENROUTER_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::new(),
            },
            Self::Bailian => ResponsesDialectContract {
                protocol_adapter_id: BAILIAN_RESPONSES_ADAPTER_ID,
                base_adapter_id: OPENAI_RESPONSES_ADAPTER_ID,
                override_points: BTreeSet::from([ResponsesOverridePoint::SessionCacheHeader]),
            },
        }
    }

    #[cfg(test)]
    fn provider_namespace(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::Bailian => "qwen",
        }
    }
}

pub(crate) fn openai_responses_compatible_adapters(
) -> ProtocolResultValue<Vec<(AdapterDescriptor, CodecRegistration)>> {
    Ok(vec![responses_dialect_adapter(
        ResponsesDialectKind::Bailian,
    )?])
}

pub(crate) fn responses_dialect_adapter(
    dialect: ResponsesDialectKind,
) -> ProtocolResultValue<(AdapterDescriptor, CodecRegistration)> {
    let reported_cost_currency = (dialect == ResponsesDialectKind::OpenRouter).then_some("USD");
    let (base_descriptor, base_registration) = super::openai_responses::openai_platform_client(
        OPENAI_RESPONSES_ADAPTER_ID,
        None,
        &[OPENAI_RESPONSES_OPERATION_ID],
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
    let descriptor = AdapterDescriptor {
        protocol_family_id: base_descriptor.protocol_family_id,
        protocol_adapter_id: contract.protocol_adapter_id.to_string(),
        interface_generation: "responses-v1".to_string(),
        base_adapter_id: Some(contract.base_adapter_id.to_string()),
        status: AdapterStatus::Stable,
        probe_priority: 200,
        probe_path: None,
        credential: super::AdapterCredentialContract::bearer(),
        operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
    };
    let strategy = dialect_strategy(dialect);
    let registration = CodecRegistration {
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

struct OpenRouterDialect;

impl ResponsesDialectStrategy for OpenRouterDialect {
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
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            for tool in tools {
                if tool.get("type").and_then(Value::as_str) == Some("web_search") {
                    tool["type"] = Value::String("openrouter:web_search".into());
                }
            }
        }
        Ok(request)
    }
}

struct BailianDialect;

impl ResponsesDialectStrategy for BailianDialect {
    fn prepare_parameters(
        &self,
        parameters: &mut BTreeMap<String, Value>,
    ) -> ProtocolResultValue<PreparedDialectRequest> {
        Ok(PreparedDialectRequest {
            body_extensions: BTreeMap::new(),
            session_cache: parameters.remove(BAILIAN_SESSION_CACHE_PARAMETER),
        })
    }

    fn transform_request(
        &self,
        call: &CodecCall<'_>,
        mut request: HttpRequest,
        prepared: PreparedDialectRequest,
    ) -> ProtocolResultValue<HttpRequest> {
        let super::HttpBody::Json(body) = &mut request.body else {
            return Err(ProtocolError::invalid_configuration(
                "Bailian request body must be JSON",
            ));
        };
        rewrite_bailian_video_inputs(body, call)?;
        if let Some(enabled) = prepared.session_cache {
            let enabled = enabled.as_bool().ok_or_else(|| {
                ProtocolError::invalid_request("Bailian session_cache must be a boolean")
            })?;
            request.headers.insert(
                HeaderName::from_static(BAILIAN_SESSION_CACHE_HEADER),
                HeaderValue::from_static(if enabled { "enable" } else { "disable" }),
            );
        }
        Ok(request)
    }
}

fn bailian_document_is_video(
    source: &buckyos_api::ResourceRef,
    context: &super::CodecContext,
) -> ProtocolResultValue<bool> {
    let mime = match source {
        buckyos_api::ResourceRef::Url { url, mime_hint } => {
            if let Some(mime) = mime_hint.as_deref() {
                return Ok(mime.to_ascii_lowercase().starts_with("video/"));
            }
            return Ok(reqwest::Url::parse(url)
                .ok()
                .is_some_and(|url| url.path().to_ascii_lowercase().ends_with(".mp4")));
        }
        buckyos_api::ResourceRef::Base64 { mime, .. } => Some(mime.as_str()),
        buckyos_api::ResourceRef::NamedObject { .. } => {
            Some(context.materialized_resource(source)?.mime.as_str())
        }
    };
    Ok(mime.is_some_and(|mime| mime.to_ascii_lowercase().starts_with("video/")))
}

fn rewrite_bailian_video_inputs(
    value: &mut Value,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<()> {
    let mut video_documents = Vec::new();
    if let AiccCall::ChatCompletionsCreate(request) = &call.input.canonical_request {
        for message in &request.messages {
            for content in &message.content {
                match content {
                    AiContent::Document { source, .. } => {
                        video_documents.push(bailian_document_is_video(source, call.context)?);
                    }
                    AiContent::ToolResult { content, .. } => {
                        for result in content {
                            if let buckyos_api::AiToolResultContent::Document { source, .. } =
                                result
                            {
                                video_documents
                                    .push(bailian_document_is_video(source, call.context)?);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut document_index = 0;
    rewrite_qwen_video_values(value, &video_documents, &mut document_index);
    Ok(())
}

fn rewrite_qwen_video_values(value: &mut Value, video_documents: &[bool], index: &mut usize) {
    match value {
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| rewrite_qwen_video_values(value, video_documents, index)),
        Value::Object(object) => {
            let is_input_file = object.get("type").and_then(Value::as_str) == Some("input_file");
            let is_video = is_input_file
                && video_documents.get(*index).copied().unwrap_or_else(|| {
                    object
                        .get("file_url")
                        .and_then(Value::as_str)
                        .is_some_and(|url| {
                            reqwest::Url::parse(url).ok().is_some_and(|url| {
                                url.path().to_ascii_lowercase().ends_with(".mp4")
                            })
                        })
                });
            if is_input_file {
                *index += 1;
            }
            if is_video {
                object.insert("type".to_owned(), Value::String("input_video".to_owned()));
                if let Some(url) = object.remove("file_url") {
                    object.insert("video_url".to_owned(), url);
                }
                if let Some(data) = object.remove("file_data") {
                    object.insert("data".to_owned(), data);
                }
                object.remove("filename");
            }
            object
                .values_mut()
                .for_each(|value| rewrite_qwen_video_values(value, video_documents, index));
        }
        _ => {}
    }
}

fn dialect_strategy(dialect: ResponsesDialectKind) -> Arc<dyn ResponsesDialectStrategy> {
    match dialect {
        ResponsesDialectKind::OpenRouter => Arc::new(OpenRouterDialect),
        ResponsesDialectKind::Bailian => Arc::new(BailianDialect),
    }
}

#[async_trait]
impl OperationCodec for ResponsesDialectCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        self.base.resource_input_form()
    }

    fn resource_input_form_for(
        &self,
        resource_pointers: &[&str],
    ) -> crate::resource::ResourceInputForm {
        self.base.resource_input_form_for(resource_pointers)
    }

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
        self.base.decode(response).await
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        self.base.decode_stream(response).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        openai_responses_adapter, CodecContext, CodecLimits, CodecRegistry, HttpBody,
        ResolvedCredential,
    };
    use buckyos_api::{AiContent, AiMessage, AiRole, AiccCall, LlmChatInvokeRequest, ResourceRef};
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
            state_namespace: None,
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
        assert_eq!(adapters.len(), 1);
        for (descriptor, registration) in adapters {
            assert_eq!(
                descriptor.base_adapter_id.as_deref(),
                Some(OPENAI_RESPONSES_ADAPTER_ID)
            );
            assert!(descriptor
                .operations
                .contains_key(OPENAI_RESPONSES_OPERATION_ID));
            assert_eq!(descriptor.operations.len(), 1);
            for api_type in [ApiType::Llm, ApiType::VisionOcr, ApiType::VisionCaption] {
                assert!(registration
                    .operation_codecs
                    .iter()
                    .any(|codec| codec.api_type() == api_type));
            }
            assert!(registration.native_task_codecs.is_empty());
        }
        assert!(ResponsesDialectKind::Bailian
            .contract()
            .override_points
            .contains(&ResponsesOverridePoint::SessionCacheHeader));
        assert!(ResponsesDialectKind::OpenRouter
            .contract()
            .override_points
            .is_empty());
    }

    #[test]
    fn derived_registration_is_one_way_and_base_remains_independently_usable() {
        let (base_descriptor, base_registration) = openai_responses_adapter();
        let mut registry = CodecRegistry::default();
        registry
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        for (descriptor, registration) in [super::super::bailian_media_adapter()] {
            registry.register_codecs(descriptor, registration).unwrap();
        }
        for (descriptor, registration) in openai_responses_compatible_adapters().unwrap() {
            registry.register_derived(descriptor, registration).unwrap();
        }
        for adapter_id in [OPENAI_RESPONSES_ADAPTER_ID, BAILIAN_RESPONSES_ADAPTER_ID] {
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
            ResponsesDialectKind::OpenRouter,
            ResponsesDialectKind::Bailian,
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

    #[tokio::test]
    async fn qwen_normalizes_inline_image_state_and_replays_provider_wire() {
        let (_, registration) = responses_dialect_adapter(ResponsesDialectKind::Bailian).unwrap();
        let response = HttpResponse {
            status: reqwest::StatusCode::OK,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from_static(
                br#"{"id":"resp_qwen_1","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done","annotations":[]}]},{"type":"image_generation_call","id":"ig_qwen_1","status":"completed","result":"aW1hZ2U=","output_format":"png"}],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
            ),
            request_id: "request-qwen-1".into(),
            retry_after: None,
        };
        let ProtocolExecution::Immediate(mut output) = registration.operation_codecs[0]
            .decode(response)
            .await
            .unwrap()
        else {
            panic!("expected immediate response")
        };
        let mut qwen_context =
            context("https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1");
        qwen_context.state_coordinate.adapter_type = BAILIAN_RESPONSES_ADAPTER_ID.to_owned();
        qwen_context.state_coordinate.origin_provider = "qwen".to_owned();
        crate::protocol::finish_provider_state(
            &mut output.value,
            Some(&crate::protocol::StateNamespaceRename {
                from: "openai".to_owned(),
                to: "qwen".to_owned(),
            }),
            &qwen_context.state_coordinate,
        );

        let image_state = output.value["provider_states"]
            .as_array()
            .unwrap()
            .iter()
            .find(|state| state["value"]["type"] == "image_generation_call")
            .unwrap();
        assert_eq!(image_state["provider"], "qwen");
        assert_eq!(image_state["value"]["result"]["kind"], "base64");
        assert!(!image_state["value"]["result"].is_string());

        let assistant: AiMessage = serde_json::from_value(output.value["message"].clone()).unwrap();
        let replay_input = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "logical.model",
                vec![AiMessage::text(AiRole::User, "continue"), assistant],
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_owned(),
                Value::String("qwen3.8-max".to_owned()),
            )]),
        };
        let replay = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &replay_input,
                context: &qwen_context,
            })
            .unwrap();
        let HttpBody::Json(body) = replay.body else {
            panic!("expected JSON request")
        };
        assert!(body["input"].as_array().unwrap().iter().any(|item| {
            item["type"] == "image_generation_call" && item["result"] == "aW1hZ2U="
        }));
    }

    #[test]
    fn derived_provider_state_round_trips_back_to_the_wire_request() {
        for dialect in [
            ResponsesDialectKind::OpenRouter,
            ResponsesDialectKind::Bailian,
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
    fn qwen_maps_session_cache_to_header_without_leaking_it_to_base_body() {
        let (_, registration) = responses_dialect_adapter(ResponsesDialectKind::Bailian).unwrap();
        let request = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input(BTreeMap::from([(
                    BAILIAN_SESSION_CACHE_PARAMETER.to_string(),
                    Value::Bool(true),
                )])),
                context: &context(
                    "https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                ),
            })
            .unwrap();
        assert_eq!(
            request.headers[BAILIAN_SESSION_CACHE_HEADER],
            HeaderValue::from_static("enable")
        );
        let crate::protocol::HttpBody::Json(body) = request.body else {
            panic!("expected JSON request")
        };
        assert!(body.get(BAILIAN_SESSION_CACHE_PARAMETER).is_none());
    }

    #[test]
    fn qwen_maps_video_mime_to_responses_video_input_without_filename_conventions() {
        let (_, registration) = responses_dialect_adapter(ResponsesDialectKind::Bailian).unwrap();
        let input = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "logical.model",
                vec![AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::Text {
                            text: "describe the video".into(),
                        },
                        AiContent::Document {
                            source: ResourceRef::url(
                                "https://example.test/media?id=4827".into(),
                                Some("video/mp4".into()),
                            ),
                            title: None,
                        },
                        AiContent::Document {
                            source: ResourceRef::url(
                                "https://example.test/document?id=4827".into(),
                                Some("application/pdf".into()),
                            ),
                            title: Some("video.mp4".into()),
                        },
                    ],
                )],
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                Value::String("qwen3.8-omni-flash".to_string()),
            )]),
        };
        let request = registration.operation_codecs[0]
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input,
                context: &context(
                    "https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                ),
            })
            .unwrap();
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON request")
        };
        assert_eq!(
            body["input"][0]["content"][1],
            json!({
                "type": "input_video",
                "video_url": "https://example.test/media?id=4827"
            })
        );
        assert_eq!(body["input"][0]["content"][2]["type"], "input_file");
        assert_eq!(
            body["input"][0]["content"][2]["file_url"],
            "https://example.test/document?id=4827"
        );
    }

    #[test]
    fn provider_parameter_policy_is_not_hardcoded_in_dialects() {
        for (dialect, parameter) in [
            (ResponsesDialectKind::OpenRouter, "store"),
            (ResponsesDialectKind::Bailian, "background"),
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
}
