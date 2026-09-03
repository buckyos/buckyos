use super::{
    openai_chat_completions_operation_descriptor, AdapterDescriptor, AdapterStatus,
    ChatCompletionsImmediateExtensions, ChatCompletionsStreamExtensions,
    ChatCompletionsTokenLimitParameter, CodecRegistration, OpenAiChatCompletionsCodec,
    OpenAiChatCompletionsDialect, ProtocolError, ProtocolResultValue,
    OPENAI_CHAT_COMPLETIONS_ADAPTER_ID, OPENAI_PROTOCOL_FAMILY_ID,
};
use buckyos_api::{AiContent, AiRole, LlmChatInvokeRequest};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const OPENROUTER_CHAT_ADAPTER_ID: &str = "openrouter-openai";
pub(crate) const KIMI_CHAT_ADAPTER_ID: &str = "kimi-chat";
pub(crate) const GLM_CHAT_ADAPTER_ID: &str = "glm-chat";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChatCompletionsDialectContract {
    pub adapter_id: &'static str,
    pub base_adapter_id: &'static str,
    pub request_extensions: BTreeSet<&'static str>,
    pub response_extensions: BTreeSet<&'static str>,
    pub unsupported_features: BTreeSet<&'static str>,
}

pub(crate) fn openrouter_chat_contract() -> ChatCompletionsDialectContract {
    ChatCompletionsDialectContract {
        adapter_id: OPENROUTER_CHAT_ADAPTER_ID,
        base_adapter_id: OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
        request_extensions: BTreeSet::from([
            "models",
            "plugins",
            "provider",
            "reasoning",
            "route",
            "transforms",
        ]),
        response_extensions: BTreeSet::from([
            "openrouter_metadata",
            "provider",
            "reasoning",
            "reasoning_details",
        ]),
        unsupported_features: BTreeSet::new(),
    }
}

pub(crate) fn kimi_chat_contract() -> ChatCompletionsDialectContract {
    ChatCompletionsDialectContract {
        adapter_id: KIMI_CHAT_ADAPTER_ID,
        base_adapter_id: OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
        request_extensions: BTreeSet::from([
            "partial",
            "prompt_cache_key",
            "reasoning_content",
            "thinking",
            "video_url",
        ]),
        response_extensions: BTreeSet::from(["cached_tokens", "reasoning_content"]),
        unsupported_features: BTreeSet::new(),
    }
}

pub(crate) fn glm_chat_contract() -> ChatCompletionsDialectContract {
    ChatCompletionsDialectContract {
        adapter_id: GLM_CHAT_ADAPTER_ID,
        base_adapter_id: OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
        request_extensions: BTreeSet::from(["reasoning_content", "thinking", "tool_stream"]),
        response_extensions: BTreeSet::from(["reasoning_content", "tool_stream"]),
        unsupported_features: BTreeSet::new(),
    }
}

pub(crate) fn openrouter_chat_adapter() -> (AdapterDescriptor, CodecRegistration) {
    derived_adapter(OPENROUTER_CHAT_ADAPTER_ID, Arc::new(OpenRouterDialect))
}

pub(crate) fn kimi_chat_adapter() -> (AdapterDescriptor, CodecRegistration) {
    derived_adapter(KIMI_CHAT_ADAPTER_ID, Arc::new(KimiDialect))
}

pub(crate) fn glm_chat_adapter() -> (AdapterDescriptor, CodecRegistration) {
    derived_adapter(GLM_CHAT_ADAPTER_ID, Arc::new(GlmDialect))
}

fn derived_adapter(
    adapter_id: &str,
    dialect: Arc<dyn OpenAiChatCompletionsDialect>,
) -> (AdapterDescriptor, CodecRegistration) {
    let operation = openai_chat_completions_operation_descriptor();
    let codec = OpenAiChatCompletionsCodec::with_dialect(dialect);
    (
        AdapterDescriptor {
            protocol_family_id: OPENAI_PROTOCOL_FAMILY_ID.to_owned(),
            protocol_adapter_id: adapter_id.to_owned(),
            interface_generation: "v1".to_owned(),
            base_adapter_id: Some(OPENAI_CHAT_COMPLETIONS_ADAPTER_ID.to_owned()),
            status: AdapterStatus::Stable,
            operations: BTreeMap::from([(operation.operation_id.clone(), operation)]),
        },
        CodecRegistration {
            operation_codecs: vec![Arc::new(codec)],
            native_task_codecs: Vec::new(),
        },
    )
}

#[derive(Debug)]
struct OpenRouterDialect;

impl OpenAiChatCompletionsDialect for OpenRouterDialect {
    fn transform_resolved_parameter(
        &self,
        name: &str,
        value: &Value,
    ) -> ProtocolResultValue<Option<(String, Value)>> {
        let valid = match name {
            "provider" | "reasoning" => value.is_object(),
            "models" => string_array(value),
            "plugins" | "transforms" => value.is_array(),
            "route" => value.is_string(),
            _ => return Ok(None),
        };
        if !valid {
            return Err(ProtocolError::invalid_request(format!(
                "OpenRouter parameter `{name}` has an invalid value"
            )));
        }
        Ok(Some((name.to_owned(), value.clone())))
    }

    fn transform_request(
        &self,
        _request: &LlmChatInvokeRequest,
        _body: &mut Map<String, Value>,
        headers: &mut HeaderMap,
    ) -> ProtocolResultValue<()> {
        headers.insert("x-openrouter-metadata", HeaderValue::from_static("enabled"));
        headers.insert(
            "x-openrouter-title",
            HeaderValue::from_static("BuckyOS AICC"),
        );
        Ok(())
    }

    fn transform_immediate_response(
        &self,
        response: &mut Map<String, Value>,
    ) -> ProtocolResultValue<ChatCompletionsImmediateExtensions> {
        let mut extensions = reasoning_from_response(response, "openrouter")?;
        let mut metadata = Map::new();
        for field in ["openrouter_metadata", "provider"] {
            if let Some(value) = response.remove(field) {
                metadata.insert(field.to_owned(), value);
            }
        }
        if !metadata.is_empty() {
            extensions.content.push(AiContent::ProviderState {
                provider: "openrouter".to_owned(),
                value: Value::Object(metadata),
            });
        }
        Ok(extensions)
    }

    fn transform_stream_chunk(
        &self,
        chunk: &mut Map<String, Value>,
    ) -> ProtocolResultValue<ChatCompletionsStreamExtensions> {
        let mut extensions = reasoning_from_stream(chunk)?;
        let mut metadata = Map::new();
        for field in ["openrouter_metadata", "provider"] {
            if let Some(value) = chunk.remove(field) {
                metadata.insert(field.to_owned(), value);
            }
        }
        if !metadata.is_empty() {
            extensions.content.push(AiContent::ProviderState {
                provider: "openrouter".to_owned(),
                value: Value::Object(metadata),
            });
        }
        Ok(extensions)
    }
}

#[derive(Debug)]
struct KimiDialect;

impl OpenAiChatCompletionsDialect for KimiDialect {
    fn token_limit_parameter(&self) -> ChatCompletionsTokenLimitParameter {
        ChatCompletionsTokenLimitParameter::MaxTokens
    }

    fn allows_unmapped_message_content(&self, role: AiRole, content: &AiContent) -> bool {
        role == AiRole::Assistant
            && (matches!(content, AiContent::Thinking { .. })
                || matches!(
                    content,
                    AiContent::ProviderState { provider, .. } if provider == "kimi"
                ))
    }

    fn transform_resolved_parameter(
        &self,
        name: &str,
        value: &Value,
    ) -> ProtocolResultValue<Option<(String, Value)>> {
        let valid = match name {
            "thinking" => valid_thinking(value),
            "prompt_cache_key" => nonempty_string(value),
            _ => return Ok(None),
        };
        if !valid {
            return Err(ProtocolError::invalid_request(format!(
                "Kimi parameter `{name}` has an invalid value"
            )));
        }
        Ok(Some((name.to_owned(), value.clone())))
    }

    fn transform_request(
        &self,
        request: &LlmChatInvokeRequest,
        body: &mut Map<String, Value>,
        _headers: &mut HeaderMap,
    ) -> ProtocolResultValue<()> {
        restore_assistant_state(request, body, "kimi", true)
    }

    fn transform_immediate_response(
        &self,
        response: &mut Map<String, Value>,
    ) -> ProtocolResultValue<ChatCompletionsImmediateExtensions> {
        reasoning_from_response(response, "kimi")
    }

    fn transform_stream_chunk(
        &self,
        chunk: &mut Map<String, Value>,
    ) -> ProtocolResultValue<ChatCompletionsStreamExtensions> {
        reasoning_from_stream(chunk)
    }
}

#[derive(Debug)]
struct GlmDialect;

impl OpenAiChatCompletionsDialect for GlmDialect {
    fn token_limit_parameter(&self) -> ChatCompletionsTokenLimitParameter {
        ChatCompletionsTokenLimitParameter::MaxTokens
    }

    fn allows_unmapped_message_content(&self, role: AiRole, content: &AiContent) -> bool {
        role == AiRole::Assistant && matches!(content, AiContent::Thinking { .. })
    }

    fn transform_resolved_parameter(
        &self,
        name: &str,
        value: &Value,
    ) -> ProtocolResultValue<Option<(String, Value)>> {
        let valid = match name {
            "thinking" => valid_thinking(value),
            "tool_stream" => value.is_boolean(),
            _ => return Ok(None),
        };
        if !valid {
            return Err(ProtocolError::invalid_request(format!(
                "GLM parameter `{name}` has an invalid value"
            )));
        }
        Ok(Some((name.to_owned(), value.clone())))
    }

    fn transform_request(
        &self,
        request: &LlmChatInvokeRequest,
        body: &mut Map<String, Value>,
        _headers: &mut HeaderMap,
    ) -> ProtocolResultValue<()> {
        restore_assistant_state(request, body, "glm", false)
    }

    fn transform_immediate_response(
        &self,
        response: &mut Map<String, Value>,
    ) -> ProtocolResultValue<ChatCompletionsImmediateExtensions> {
        reasoning_from_response(response, "glm")
    }

    fn transform_stream_chunk(
        &self,
        chunk: &mut Map<String, Value>,
    ) -> ProtocolResultValue<ChatCompletionsStreamExtensions> {
        normalize_glm_tool_stream(chunk)?;
        reasoning_from_stream(chunk)
    }
}

fn restore_assistant_state(
    request: &LlmChatInvokeRequest,
    body: &mut Map<String, Value>,
    provider: &str,
    allow_partial: bool,
) -> ProtocolResultValue<()> {
    let wire_messages = body
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ProtocolError::invalid_configuration("encoded messages are missing"))?;
    if wire_messages.len() != request.messages.len() {
        return Err(ProtocolError::invalid_configuration(
            "encoded message count changed before dialect transform",
        ));
    }
    for (canonical, wire) in request.messages.iter().zip(wire_messages) {
        if canonical.role != AiRole::Assistant {
            continue;
        }
        let wire = wire.as_object_mut().ok_or_else(|| {
            ProtocolError::invalid_configuration("encoded assistant message is not an object")
        })?;
        for content in &canonical.content {
            match content {
                AiContent::Thinking {
                    text: Some(text), ..
                } if !text.is_empty() => {
                    wire.insert("reasoning_content".to_owned(), Value::String(text.clone()));
                }
                AiContent::Thinking { .. } => {}
                AiContent::ProviderState {
                    provider: owner,
                    value,
                } if owner == provider => {
                    if !allow_partial {
                        continue;
                    }
                    let partial =
                        value
                            .get("partial")
                            .and_then(Value::as_bool)
                            .ok_or_else(|| {
                                ProtocolError::invalid_request(
                                    "Kimi ProviderState must contain boolean `partial`",
                                )
                            })?;
                    wire.insert("partial".to_owned(), Value::Bool(partial));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn reasoning_from_response(
    response: &mut Map<String, Value>,
    provider: &str,
) -> ProtocolResultValue<ChatCompletionsImmediateExtensions> {
    let Some(message) = first_choice_part_mut(response, "message")? else {
        return Ok(ChatCompletionsImmediateExtensions::default());
    };
    let text = take_optional_string(message, &["reasoning_content", "reasoning"])?;
    let provider_metadata = message.remove("reasoning_details");
    let mut content = Vec::new();
    if text.is_some() || provider_metadata.is_some() {
        content.push(AiContent::Thinking {
            summary: None,
            text,
            provider_metadata,
        });
    }
    if let Some(Value::Object(details)) = response.get_mut("usage") {
        if let Some(cached_tokens) = details.remove("cached_tokens") {
            content.push(AiContent::ProviderState {
                provider: provider.to_owned(),
                value: json!({"type": "cached_tokens", "value": cached_tokens}),
            });
        }
    }
    Ok(ChatCompletionsImmediateExtensions {
        content,
        usage: None,
    })
}

fn reasoning_from_stream(
    chunk: &mut Map<String, Value>,
) -> ProtocolResultValue<ChatCompletionsStreamExtensions> {
    let Some(delta) = first_choice_part_mut(chunk, "delta")? else {
        return Ok(ChatCompletionsStreamExtensions::default());
    };
    let thinking_delta = take_optional_string(delta, &["reasoning_content", "reasoning"])?;
    Ok(ChatCompletionsStreamExtensions {
        thinking_delta,
        content: Vec::new(),
        usage: None,
    })
}

fn normalize_glm_tool_stream(chunk: &mut Map<String, Value>) -> ProtocolResultValue<()> {
    let Some(delta) = first_choice_part_mut(chunk, "delta")? else {
        return Ok(());
    };
    if delta.contains_key("tool_calls") {
        delta.remove("tool_stream");
        return Ok(());
    }
    if let Some(tool_stream) = delta.remove("tool_stream") {
        if !tool_stream.is_array() {
            return Err(ProtocolError::invalid_response(
                "GLM tool_stream must be an array",
            ));
        }
        delta.insert("tool_calls".to_owned(), tool_stream);
    }
    Ok(())
}

fn first_choice_part_mut<'a>(
    root: &'a mut Map<String, Value>,
    part: &str,
) -> ProtocolResultValue<Option<&'a mut Map<String, Value>>> {
    let Some(choices) = root.get_mut("choices") else {
        return Ok(None);
    };
    let choices = choices
        .as_array_mut()
        .ok_or_else(|| ProtocolError::invalid_response("choices must be an array"))?;
    let Some(choice) = choices.first_mut() else {
        return Ok(None);
    };
    let choice = choice
        .as_object_mut()
        .ok_or_else(|| ProtocolError::invalid_response("choice must be an object"))?;
    match choice.get_mut(part) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_object_mut().map(Some).ok_or_else(|| {
            ProtocolError::invalid_response(format!("choice {part} must be an object"))
        }),
    }
}

fn take_optional_string(
    object: &mut Map<String, Value>,
    names: &[&str],
) -> ProtocolResultValue<Option<String>> {
    let mut result = None;
    for name in names {
        let Some(value) = object.remove(*name) else {
            continue;
        };
        match value {
            Value::String(value) if !value.is_empty() && result.is_none() => result = Some(value),
            Value::String(_) | Value::Null => {}
            _ => {
                return Err(ProtocolError::invalid_response(format!(
                    "{name} must be a string or null"
                )));
            }
        }
    }
    Ok(result)
}

fn valid_thinking(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 1
        && matches!(
            object.get("type").and_then(Value::as_str),
            Some("enabled" | "disabled")
        )
}

fn nonempty_string(value: &Value) -> bool {
    value.as_str().is_some_and(|value| !value.trim().is_empty())
}

fn string_array(value: &Value) -> bool {
    value.as_array().is_some_and(|values| {
        !values.is_empty()
            && values
                .iter()
                .all(|value| value.as_str().is_some_and(|value| !value.trim().is_empty()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        openai_chat_completions_adapter, CodecContext, CodecInput, CodecLimits, CodecRegistry,
        HttpBody, ResolvedCredential, OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
    };
    use buckyos_api::{AiMessage, AiccCall, ApiType, LlmChatInvokeRequest};
    use std::time::Duration;

    fn registry_with(derived: (AdapterDescriptor, CodecRegistration)) -> CodecRegistry {
        let mut registry = CodecRegistry::default();
        let (base, codecs) = openai_chat_completions_adapter();
        registry.register_codecs(base, codecs).unwrap();
        registry.register_derived(derived.0, derived.1).unwrap();
        registry
    }

    fn input(parameters: BTreeMap<String, Value>) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "ignored@provider",
                vec![AiMessage::text(AiRole::User, "hello")],
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_owned(),
                Value::String("model".to_owned()),
            )])
            .into_iter()
            .chain(parameters)
            .collect(),
        }
    }

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://example.test/v1".to_owned(),
            credential: Some(ResolvedCredential::bearer("secret://test", "secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }

    #[test]
    fn all_dialects_declare_and_register_one_way_base_reuse() {
        for (contract, adapter) in [
            (openrouter_chat_contract(), openrouter_chat_adapter()),
            (kimi_chat_contract(), kimi_chat_adapter()),
            (glm_chat_contract(), glm_chat_adapter()),
        ] {
            assert_eq!(contract.base_adapter_id, OPENAI_CHAT_COMPLETIONS_ADAPTER_ID);
            assert_eq!(adapter.0.protocol_adapter_id, contract.adapter_id);
            assert_eq!(
                adapter.0.base_adapter_id.as_deref(),
                Some(contract.base_adapter_id)
            );
            let registry = registry_with(adapter);
            assert!(registry.adapter(contract.adapter_id).is_some());
        }
    }

    #[test]
    fn openrouter_accepts_only_typed_routing_options_and_adds_attribution() {
        let registry = registry_with(openrouter_chat_adapter());
        let request = registry
            .encode(
                OPENROUTER_CHAT_ADAPTER_ID,
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
                ApiType::Llm,
                &input(BTreeMap::from([(
                    "provider".to_owned(),
                    json!({"order": ["Anthropic", "OpenAI"]}),
                )])),
                &context(),
            )
            .unwrap();
        assert_eq!(request.headers["x-openrouter-metadata"], "enabled");
        assert_eq!(request.headers["x-openrouter-title"], "BuckyOS AICC");
        let HttpBody::Json(body) = request.body else {
            panic!()
        };
        assert_eq!(body["provider"]["order"][0], "Anthropic");

        assert!(registry
            .encode(
                OPENROUTER_CHAT_ADAPTER_ID,
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
                ApiType::Llm,
                &input(BTreeMap::from([("provider".to_owned(), json!("any"))])),
                &context(),
            )
            .is_err());
    }

    #[test]
    fn kimi_and_glm_map_token_thinking_cache_and_tool_extensions() {
        for (adapter_id, adapter, parameter) in [
            (
                KIMI_CHAT_ADAPTER_ID,
                kimi_chat_adapter(),
                ("prompt_cache_key", json!("session-1")),
            ),
            (
                GLM_CHAT_ADAPTER_ID,
                glm_chat_adapter(),
                ("tool_stream", json!(true)),
            ),
        ] {
            let registry = registry_with(adapter);
            let mut codec_input = input(BTreeMap::from([
                ("thinking".to_owned(), json!({"type": "enabled"})),
                (parameter.0.to_owned(), parameter.1),
            ]));
            if let AiccCall::ChatCompletionsCreate(request) = &mut codec_input.canonical_request {
                request.max_output_tokens = Some(64);
            }
            let request = registry
                .encode(
                    adapter_id,
                    OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
                    ApiType::Llm,
                    &codec_input,
                    &context(),
                )
                .unwrap();
            let HttpBody::Json(body) = request.body else {
                panic!()
            };
            assert_eq!(body["max_tokens"], 64);
            assert_eq!(body["thinking"]["type"], "enabled");
            assert!(body.get("max_completion_tokens").is_none());
        }

        for (adapter_id, adapter, parameter) in [
            (
                KIMI_CHAT_ADAPTER_ID,
                kimi_chat_adapter(),
                ("prompt_cache_key", json!("  ")),
            ),
            (
                GLM_CHAT_ADAPTER_ID,
                glm_chat_adapter(),
                ("tool_stream", json!("true")),
            ),
        ] {
            assert!(registry_with(adapter)
                .encode(
                    adapter_id,
                    OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
                    ApiType::Llm,
                    &input(BTreeMap::from([(parameter.0.to_owned(), parameter.1)])),
                    &context(),
                )
                .is_err());
        }
    }

    #[test]
    fn kimi_round_trips_partial_thinking_and_cached_usage() {
        let registry = registry_with(kimi_chat_adapter());
        let mut codec_input = input(BTreeMap::new());
        if let AiccCall::ChatCompletionsCreate(request) = &mut codec_input.canonical_request {
            request.messages.push(AiMessage::new(
                AiRole::Assistant,
                vec![
                    AiContent::Text {
                        text: "draft".to_owned(),
                    },
                    AiContent::Thinking {
                        summary: None,
                        text: Some("prior reasoning".to_owned()),
                        provider_metadata: None,
                    },
                    AiContent::ProviderState {
                        provider: "kimi".to_owned(),
                        value: json!({"partial": true}),
                    },
                ],
            ));
        }
        let request = registry
            .encode(
                KIMI_CHAT_ADAPTER_ID,
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
                ApiType::Llm,
                &codec_input,
                &context(),
            )
            .unwrap();
        let HttpBody::Json(body) = request.body else {
            panic!()
        };
        assert_eq!(body["messages"][1]["partial"], true);
        assert_eq!(body["messages"][1]["reasoning_content"], "prior reasoning");

        let mut response = json!({
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "answer", "reasoning_content": "thought"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 4, "completion_tokens": 3, "total_tokens": 7, "cached_tokens": 2}
        });
        let extensions = KimiDialect
            .transform_immediate_response(response.as_object_mut().unwrap())
            .unwrap();
        assert!(matches!(
            &extensions.content[0],
            AiContent::Thinking { text: Some(text), .. } if text == "thought"
        ));
        assert!(matches!(
            &extensions.content[1],
            AiContent::ProviderState { provider, .. } if provider == "kimi"
        ));
    }

    #[test]
    fn glm_stream_maps_reasoning_and_tool_stream_without_copying_base_parser() {
        let mut chunk = json!({
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": {
                    "reasoning_content": "plan",
                    "tool_stream": [{"index": 0, "id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]
                },
                "finish_reason": null
            }]
        });
        let extensions = GlmDialect
            .transform_stream_chunk(chunk.as_object_mut().unwrap())
            .unwrap();
        assert_eq!(extensions.thinking_delta.as_deref(), Some("plan"));
        let delta = &chunk["choices"][0]["delta"];
        assert!(delta.get("tool_stream").is_none());
        assert_eq!(delta["tool_calls"][0]["function"]["name"], "lookup");
    }

    #[test]
    fn openrouter_preserves_channel_metadata_as_provider_state() {
        let mut response = json!({
            "object": "chat.completion",
            "provider": "Anthropic",
            "openrouter_metadata": {"provider_name": "Anthropic", "model": "claude"},
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "answer", "reasoning": "thought"},
                "finish_reason": "stop"
            }]
        });
        let extensions = OpenRouterDialect
            .transform_immediate_response(response.as_object_mut().unwrap())
            .unwrap();
        assert!(matches!(
            &extensions.content[0],
            AiContent::Thinking { text: Some(text), .. } if text == "thought"
        ));
        assert!(matches!(
            &extensions.content[1],
            AiContent::ProviderState { provider, value }
                if provider == "openrouter" && value["provider"] == "Anthropic"
        ));
    }
}
