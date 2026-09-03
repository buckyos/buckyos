use super::{
    sse_frame_stream, AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration,
    ExecutionMode, HttpBody, HttpRequest, HttpResponse, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolEvent, ProtocolExecution,
    ProtocolOutput, ProtocolResultValue, ProtocolStream, SseConfig, SseFrame,
    StreamingHttpResponse,
};
use async_trait::async_trait;
use buckyos_api::{
    features, AiContent, AiMessage, AiRole, AiToolResultContent, AiUsage, AiccCall, ApiType,
    LlmChatInvokeRequest, LlmResponseFormat, LlmResponseFormatType, ResourceRef,
};
use futures_util::{stream, StreamExt};
use reqwest::header::{HeaderMap, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

pub(crate) const OPENAI_PROTOCOL_FAMILY_ID: &str = "openai";
pub(crate) const OPENAI_CHAT_COMPLETIONS_ADAPTER_ID: &str = "openai-chat-completions";
pub(crate) const OPENAI_CHAT_COMPLETIONS_OPERATION_ID: &str = "chat.completions.create";

const OPENAI_CHAT_COMPLETIONS_GENERATION: &str = "v1";
const OPENAI_PROVIDER_NAMESPACE: &str = "openai";
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn openai_chat_completions_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let codec = OpenAiChatCompletionsCodec::new();
    (
        codec.adapter_descriptor(),
        CodecRegistration {
            operation_codecs: vec![Arc::new(codec)],
            native_task_codecs: Vec::new(),
        },
    )
}

#[derive(Debug, Clone)]
pub(crate) struct OpenAiChatCompletionsCodec {
    descriptor: OperationDescriptor,
}

impl OpenAiChatCompletionsCodec {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: openai_chat_completions_operation_descriptor(),
        }
    }

    pub(crate) fn adapter_descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            protocol_family_id: OPENAI_PROTOCOL_FAMILY_ID.to_string(),
            protocol_adapter_id: OPENAI_CHAT_COMPLETIONS_ADAPTER_ID.to_string(),
            interface_generation: OPENAI_CHAT_COMPLETIONS_GENERATION.to_string(),
            base_adapter_id: None,
            status: AdapterStatus::Stable,
            operations: BTreeMap::from([(
                self.descriptor.operation_id.clone(),
                self.descriptor.clone(),
            )]),
        }
    }

    fn encode_chat(
        &self,
        request: &LlmChatInvokeRequest,
        call: &CodecCall<'_>,
    ) -> ProtocolResultValue<HttpRequest> {
        validate_request(request)?;
        let provider_model_id =
            required_string(&call.input.resolved_parameters, "provider_model_id")?;
        let mut body = Map::from_iter([
            ("model".to_string(), Value::String(provider_model_id)),
            (
                "messages".to_string(),
                Value::Array(encode_messages(&request.messages)?),
            ),
        ]);
        if !request.tools.is_empty() {
            body.insert("tools".to_string(), encode_tools(request)?);
        }
        if let Some(response_format) = &request.response_format {
            body.insert(
                "response_format".to_string(),
                encode_response_format(response_format)?,
            );
        }
        if let Some(temperature) = request.temperature {
            body.insert(
                "temperature".to_string(),
                bounded_number("temperature", temperature, 0.0, 2.0)?,
            );
        }
        if let Some(top_p) = request.top_p {
            body.insert(
                "top_p".to_string(),
                bounded_number("top_p", top_p, 0.0, 1.0)?,
            );
        }
        if let Some(max_output_tokens) = request.max_output_tokens {
            if max_output_tokens == 0 {
                return Err(ProtocolError::invalid_request(
                    "max_output_tokens must be greater than zero",
                ));
            }
            body.insert(
                "max_completion_tokens".to_string(),
                Value::from(max_output_tokens),
            );
        }
        if let Some(seed) = request.seed {
            body.insert("seed".to_string(), Value::from(seed));
        }
        if !request.stop.is_empty() {
            body.insert(
                "stop".to_string(),
                serde_json::to_value(&request.stop).map_err(invalid_request_json)?,
            );
        }
        apply_resolved_parameters(&mut body, &call.input.resolved_parameters)?;

        let mut headers = HeaderMap::new();
        let credential = call.context.credential.as_ref().ok_or_else(|| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "OpenAI Chat Completions requires a resolved credential",
            )
        })?;
        credential.apply(&mut headers)?;

        let mut wire_request = HttpRequest::new(
            Method::POST,
            chat_completions_endpoint(&call.context.base_url)?,
        );
        wire_request.headers = headers;
        wire_request.body = HttpBody::Json(Value::Object(body));
        wire_request.timeout = Some(call.context.limits.request_timeout);
        wire_request.max_request_bytes = Some(
            call.context
                .limits
                .max_request_bytes
                .min(self.descriptor.max_request_bytes),
        );
        wire_request.max_response_bytes = Some(
            call.context
                .limits
                .max_response_bytes
                .min(self.descriptor.max_response_bytes),
        );
        Ok(wire_request)
    }
}

impl Default for OpenAiChatCompletionsCodec {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OperationCodec for OpenAiChatCompletionsCodec {
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
        call.context.validate()?;
        call.input
            .validate_for(self.descriptor.binding(call.api_type)?)?;
        match &call.input.canonical_request {
            AiccCall::ChatCompletionsCreate(request) => self.encode_chat(request, call),
            _ => Err(ProtocolError::invalid_request(
                "OpenAI Chat Completions only accepts chat.completions.create",
            )),
        }
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        if !response.status.is_success() {
            return Err(decode_error_response(response));
        }
        let request_id = response.request_id.clone();
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let output = normalize_completion(&value)
            .map_err(|error| error.with_request_id(Some(request_id)))?;
        Ok(ProtocolExecution::Immediate(output))
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        decode_incremental_stream(response, self.descriptor.max_response_bytes).await
    }
}

pub(crate) fn openai_chat_completions_operation_descriptor() -> OperationDescriptor {
    let mut binding = OperationBinding::new(
        ApiType::Llm,
        [ExecutionMode::Immediate, ExecutionMode::Stream],
    );
    binding.supported_features = BTreeSet::from([
        features::TOOL_CALLING.to_string(),
        features::JSON_OUTPUT.to_string(),
        features::VISION.to_string(),
    ]);
    OperationDescriptor {
        operation_id: OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_string(),
        bindings: vec![binding],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

fn chat_completions_endpoint(base_url: &str) -> ProtocolResultValue<String> {
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    reqwest::Url::parse(&endpoint).map_err(|_| {
        ProtocolError::invalid_configuration("Chat Completions endpoint URL is invalid")
    })?;
    Ok(endpoint)
}

fn validate_request(request: &LlmChatInvokeRequest) -> ProtocolResultValue<()> {
    for message in &request.messages {
        message.validate().map_err(|error| {
            ProtocolError::invalid_request(format!("invalid canonical message: {error}"))
        })?;
    }
    if request.messages.is_empty() {
        return Err(ProtocolError::invalid_request(
            "Chat Completions requires at least one message",
        ));
    }
    if request.stop.len() > 4 || request.stop.iter().any(|stop| stop.is_empty()) {
        return Err(ProtocolError::invalid_request(
            "stop must contain between zero and four non-empty strings",
        ));
    }
    if request.output.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Chat Completions does not map canonical output media options",
        ));
    }
    Ok(())
}

fn encode_messages(messages: &[AiMessage]) -> ProtocolResultValue<Vec<Value>> {
    messages.iter().map(encode_message).collect()
}

fn encode_message(message: &AiMessage) -> ProtocolResultValue<Value> {
    match message.role {
        AiRole::System | AiRole::Developer => Ok(json!({
            "role": message.role.as_str(),
            "content": text_only_content(&message.content)?
        })),
        AiRole::User => Ok(json!({
            "role": "user",
            "content": encode_user_content(&message.content)?
        })),
        AiRole::Assistant => encode_assistant_message(&message.content),
        AiRole::Tool => encode_tool_message(&message.content),
    }
}

fn text_only_content(content: &[AiContent]) -> ProtocolResultValue<String> {
    let mut text = String::new();
    for block in content {
        let AiContent::Text { text: part } = block else {
            return Err(ProtocolError::invalid_request(
                "Chat Completions message role only accepts text content",
            ));
        };
        text.push_str(part);
    }
    Ok(text)
}

fn encode_user_content(content: &[AiContent]) -> ProtocolResultValue<Value> {
    if content
        .iter()
        .all(|block| matches!(block, AiContent::Text { .. }))
    {
        return Ok(Value::String(text_only_content(content)?));
    }
    let parts = content
        .iter()
        .map(|block| match block {
            AiContent::Text { text } => Ok(json!({"type": "text", "text": text})),
            AiContent::Image { source } => Ok(json!({
                "type": "image_url",
                "image_url": {"url": encode_image_url(source)?}
            })),
            AiContent::Document { .. } => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "base Chat Completions does not map document content",
            )),
            _ => Err(ProtocolError::invalid_request(
                "Chat Completions user message contains an invalid content block",
            )),
        })
        .collect::<ProtocolResultValue<Vec<_>>>()?;
    Ok(Value::Array(parts))
}

fn encode_image_url(source: &ResourceRef) -> ProtocolResultValue<String> {
    match source {
        ResourceRef::Url { url, .. } => Ok(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        ResourceRef::NamedObject { .. } => Err(ProtocolError::invalid_request(
            "Chat Completions image resource must be materialized before protocol encoding",
        )),
    }
}

fn encode_assistant_message(content: &[AiContent]) -> ProtocolResultValue<Value> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut refusal = None;
    for block in content {
        match block {
            AiContent::Text { text: part } => text.push_str(part),
            AiContent::ToolUse {
                call_id,
                name,
                args,
            } => tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(args).map_err(invalid_request_json)?
                }
            })),
            AiContent::Thinking { .. } => {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "base Chat Completions does not map thinking content",
                ));
            }
            AiContent::ProviderState { provider, value }
                if provider == OPENAI_PROVIDER_NAMESPACE
                    && value.get("type").and_then(Value::as_str)
                        == Some("chat_completion_refusal") =>
            {
                refusal = Some(required_value_string(value, "refusal")?);
            }
            AiContent::ProviderState { .. } => {}
            _ => {
                return Err(ProtocolError::invalid_request(
                    "Chat Completions assistant history contains an unsupported content block",
                ));
            }
        }
    }
    let mut message = Map::from_iter([
        ("role".to_string(), Value::String("assistant".to_string())),
        (
            "content".to_string(),
            if text.is_empty() {
                Value::Null
            } else {
                Value::String(text)
            },
        ),
    ]);
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    if let Some(refusal) = refusal {
        message.insert("refusal".to_string(), Value::String(refusal));
    }
    Ok(Value::Object(message))
}

fn encode_tool_message(content: &[AiContent]) -> ProtocolResultValue<Value> {
    let [AiContent::ToolResult {
        call_id,
        content,
        is_error: _,
    }] = content
    else {
        return Err(ProtocolError::invalid_request(
            "Chat Completions tool message must contain one tool_result",
        ));
    };
    let mut text = String::new();
    for block in content {
        match block {
            AiToolResultContent::Text { text: part } => text.push_str(part),
            AiToolResultContent::Image { .. } | AiToolResultContent::Document { .. } => {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "base Chat Completions only maps text tool results",
                ));
            }
        }
    }
    Ok(json!({
        "role": "tool",
        "tool_call_id": call_id,
        "content": text
    }))
}

fn encode_tools(request: &LlmChatInvokeRequest) -> ProtocolResultValue<Value> {
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            if tool.tool_type != "function" {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "base Chat Completions only maps function tools",
                ));
            }
            if tool.output_schema.is_some() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Chat Completions function tools do not map output_schema",
                ));
            }
            if tool.name.trim().is_empty() || !tool.args_json_schema.is_object() {
                return Err(ProtocolError::invalid_request(
                    "function tool name must be non-empty and parameters must be an object",
                ));
            }
            Ok(json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.args_json_schema
                }
            }))
        })
        .collect::<ProtocolResultValue<Vec<_>>>()?;
    Ok(Value::Array(tools))
}

fn encode_response_format(format: &LlmResponseFormat) -> ProtocolResultValue<Value> {
    match format.format_type {
        LlmResponseFormatType::Text => Ok(json!({"type": "text"})),
        LlmResponseFormatType::Json | LlmResponseFormatType::JsonObject => {
            if format.json_schema.is_some() {
                return Err(ProtocolError::invalid_request(
                    "json_object response format must not include json_schema",
                ));
            }
            Ok(json!({"type": "json_object"}))
        }
        LlmResponseFormatType::JsonSchema => {
            let schema = format.json_schema.as_ref().ok_or_else(|| {
                ProtocolError::invalid_request("json_schema response format requires a schema")
            })?;
            if !schema.schema.is_object() {
                return Err(ProtocolError::invalid_request(
                    "response json_schema must be an object",
                ));
            }
            let name = schema
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_request(
                        "Chat Completions json_schema requires a non-empty name",
                    )
                })?;
            let mut wire = Map::from_iter([
                ("name".to_string(), Value::String(name.to_string())),
                ("schema".to_string(), schema.schema.clone()),
            ]);
            if let Some(strict) = schema.strict {
                wire.insert("strict".to_string(), Value::Bool(strict));
            }
            Ok(json!({"type": "json_schema", "json_schema": wire}))
        }
    }
}

fn apply_resolved_parameters(
    body: &mut Map<String, Value>,
    parameters: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    for (name, value) in parameters {
        if name == "provider_model_id" {
            continue;
        }
        match name.as_str() {
            "stream" | "parallel_tool_calls" | "logprobs" if value.is_boolean() => {}
            "max_completion_tokens" if value.as_u64().is_some_and(|value| value > 0) => {}
            "top_logprobs" if value.as_u64().is_some_and(|value| value <= 20) => {}
            "frequency_penalty" | "presence_penalty"
                if value
                    .as_f64()
                    .is_some_and(|value| (-2.0..=2.0).contains(&value)) => {}
            "service_tier" | "reasoning_effort" if value.is_string() => {}
            "tool_choice" if value.is_string() || value.is_object() => {}
            "stream_options" => validate_stream_options(value)?,
            _ => {
                return Err(ProtocolError::invalid_request(format!(
                    "resolved Chat Completions parameter `{name}` is not supported or has an invalid value"
                )));
            }
        }
        body.insert(name.clone(), value.clone());
    }
    if body.get("stream").and_then(Value::as_bool) == Some(true)
        && !body.contains_key("stream_options")
    {
        body.insert("stream_options".to_string(), json!({"include_usage": true}));
    }
    if body.contains_key("stream_options")
        && body.get("stream").and_then(Value::as_bool) != Some(true)
    {
        return Err(ProtocolError::invalid_request(
            "stream_options requires stream=true",
        ));
    }
    Ok(())
}

fn validate_stream_options(value: &Value) -> ProtocolResultValue<()> {
    let options = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_request("stream_options must be an object"))?;
    if options.iter().any(|(name, value)| {
        !matches!(name.as_str(), "include_usage" | "include_obfuscation") || !value.is_boolean()
    }) {
        return Err(ProtocolError::invalid_request(
            "stream_options only accepts boolean include_usage/include_obfuscation",
        ));
    }
    Ok(())
}

fn required_string(parameters: &BTreeMap<String, Value>, key: &str) -> ProtocolResultValue<String> {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProtocolError::invalid_request(format!("resolved {key} must be a non-empty string"))
        })
}

fn bounded_number(
    name: &str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> ProtocolResultValue<Value> {
    if !value.is_finite() || !(minimum..=maximum).contains(&value) {
        return Err(ProtocolError::invalid_request(format!(
            "{name} must be between {minimum} and {maximum}"
        )));
    }
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| ProtocolError::invalid_request(format!("{name} must be finite")))
}

fn invalid_request_json(error: serde_json::Error) -> ProtocolError {
    ProtocolError::invalid_request(format!(
        "failed to encode Chat Completions request: {error}"
    ))
}

fn normalize_completion(value: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::invalid_response("Chat Completions response must be an object")
    })?;
    if object.get("object").and_then(Value::as_str) != Some("chat.completion") {
        return Err(ProtocolError::invalid_response(
            "Chat Completions response has an invalid object type",
        ));
    }
    let choices = object
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::invalid_response("response choices must be an array"))?;
    if choices.len() != 1 {
        return Err(ProtocolError::invalid_response(
            "Chat Completions response must contain exactly one choice",
        ));
    }
    let choice = choices[0]
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("response choice must be an object"))?;
    if choice.get("index").and_then(Value::as_u64) != Some(0) {
        return Err(ProtocolError::invalid_response(
            "Chat Completions response choice index must be zero",
        ));
    }
    let message = decode_assistant_message(choice.get("message"))?;
    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProtocolError::invalid_response("response finish_reason is missing"))?;
    let usage = decode_usage(object.get("usage"))?;
    normalized_output(message, finish_reason, usage)
}

fn decode_assistant_message(value: Option<&Value>) -> ProtocolResultValue<AiMessage> {
    let message = value.and_then(Value::as_object).ok_or_else(|| {
        ProtocolError::invalid_response("response assistant message must be an object")
    })?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Err(ProtocolError::invalid_response(
            "response message role must be assistant",
        ));
    }
    let mut content = Vec::new();
    match message.get("content") {
        Some(Value::String(text)) if !text.is_empty() => {
            content.push(AiContent::Text { text: text.clone() });
        }
        Some(Value::String(_) | Value::Null) | None => {}
        _ => {
            return Err(ProtocolError::invalid_response(
                "response message content must be a string or null",
            ));
        }
    }
    if let Some(refusal) = message.get("refusal") {
        match refusal {
            Value::String(refusal) if !refusal.is_empty() => {
                content.push(AiContent::ProviderState {
                    provider: OPENAI_PROVIDER_NAMESPACE.to_string(),
                    value: json!({
                        "type": "chat_completion_refusal",
                        "refusal": refusal
                    }),
                });
            }
            Value::Null => {}
            _ => {
                return Err(ProtocolError::invalid_response(
                    "response refusal must be a string or null",
                ));
            }
        }
    }
    if let Some(tool_calls) = message.get("tool_calls") {
        let tool_calls = tool_calls.as_array().ok_or_else(|| {
            ProtocolError::invalid_response("response tool_calls must be an array")
        })?;
        content.extend(
            tool_calls
                .iter()
                .map(decode_tool_call)
                .collect::<ProtocolResultValue<Vec<_>>>()?,
        );
    }
    let message = AiMessage::new(AiRole::Assistant, content);
    message.validate().map_err(|error| {
        ProtocolError::invalid_response(format!("invalid normalized assistant message: {error}"))
    })?;
    Ok(message)
}

fn decode_tool_call(value: &Value) -> ProtocolResultValue<AiContent> {
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("tool call must be an object"))?;
    if object.get("type").and_then(Value::as_str) != Some("function") {
        return Err(ProtocolError::invalid_response(
            "base Chat Completions only accepts function tool calls",
        ));
    }
    let function = object
        .get("function")
        .and_then(Value::as_object)
        .ok_or_else(|| ProtocolError::invalid_response("tool call function is missing"))?;
    let arguments = required_value_string_from_map(function, "arguments")?;
    let arguments: Value = serde_json::from_str(&arguments)
        .map_err(|_| ProtocolError::invalid_response("tool call arguments are not valid JSON"))?;
    let arguments = arguments.as_object().ok_or_else(|| {
        ProtocolError::invalid_response("tool call arguments must decode to an object")
    })?;
    Ok(AiContent::ToolUse {
        call_id: required_value_string_from_map(object, "id")?,
        name: required_value_string_from_map(function, "name")?,
        args: arguments.clone().into_iter().collect(),
    })
}

fn decode_usage(value: Option<&Value>) -> ProtocolResultValue<Option<AiUsage>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let usage = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("usage must be an object"))?;
    Ok(Some(AiUsage {
        input_tokens: Some(required_u64(usage, "prompt_tokens")?),
        output_tokens: Some(required_u64(usage, "completion_tokens")?),
        total_tokens: Some(required_u64(usage, "total_tokens")?),
        request_units: None,
    }))
}

fn normalized_output(
    message: AiMessage,
    finish_reason: &str,
    usage: Option<AiUsage>,
) -> ProtocolResultValue<ProtocolOutput> {
    let tool_calls = message.tool_calls();
    let value = json!({
        "message": message,
        "tool_calls": tool_calls,
        "finish_reason": finish_reason
    });
    Ok(ProtocolOutput {
        value,
        usage,
        artifacts: Vec::new(),
    })
}

fn decode_error_response(response: HttpResponse) -> ProtocolError {
    let parsed: Option<Value> = serde_json::from_slice(&response.body).ok();
    let provider_type = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/type"))
        .and_then(Value::as_str);
    let provider_code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .or_else(|| Some(value.to_string()))
        });
    let provider_message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("Chat Completions request failed");
    let label = provider_code.as_deref().or(provider_type);
    let message = label.map_or_else(
        || provider_message.to_string(),
        |label| format!("OpenAI {label}: {provider_message}"),
    );
    let kind = match response.status {
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
            ProtocolErrorKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ProtocolErrorKind::Timeout,
        _ => ProtocolErrorKind::Transport,
    };
    ProtocolError::new(kind, message)
        .with_request_id(Some(response.request_id))
        .with_retry_after(response.retry_after)
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct ChatCompletionStreamState {
    text: String,
    refusal: String,
    tool_calls: BTreeMap<usize, PartialToolCall>,
    finish_reason: Option<String>,
    usage: Option<AiUsage>,
    saw_chunk: bool,
}

async fn decode_incremental_stream(
    response: StreamingHttpResponse,
    max_response_bytes: usize,
) -> ProtocolResultValue<ProtocolStream> {
    if !response.status.is_success() {
        let response = response
            .into_bounded_error_response(max_response_bytes)
            .await?;
        return Err(decode_error_response(response));
    }
    if !is_event_stream(&response.headers) {
        return Err(ProtocolError::invalid_response(
            "Chat Completions streaming response must use text/event-stream",
        )
        .with_request_id(Some(response.request_id))
        .with_retry_after(response.retry_after));
    }
    let request_id = response.request_id.clone();
    let retry_after = response.retry_after;
    let frames = sse_frame_stream(response, SseConfig::default(), max_response_bytes).await?;

    struct State {
        frames: super::SseFrameStream,
        chat: ChatCompletionStreamState,
        queued: VecDeque<ProtocolResultValue<ProtocolEvent>>,
        request_id: String,
        retry_after: Option<std::time::Duration>,
        finished: bool,
    }

    let state = State {
        frames,
        chat: ChatCompletionStreamState::default(),
        queued: VecDeque::new(),
        request_id,
        retry_after,
        finished: false,
    };
    let events = stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.queued.pop_front() {
                return Some((event, state));
            }
            if state.finished {
                return None;
            }
            let next = state.frames.next().await;
            let result = match next {
                Some(Ok(SseFrame::Event(event))) => {
                    apply_stream_chunk(&event.data, &mut state.chat)
                }
                Some(Ok(SseFrame::Terminated { marker })) if marker == "[DONE]" => {
                    state.finished = true;
                    finalize_stream(&mut state.chat)
                        .map(|output| vec![ProtocolEvent::Final(output)])
                }
                Some(Ok(SseFrame::Terminated { .. })) => {
                    state.finished = true;
                    Err(ProtocolError::invalid_response(
                        "Chat Completions stream used an unknown termination marker",
                    ))
                }
                Some(Ok(SseFrame::StreamEnd(_))) | None => {
                    state.finished = true;
                    Err(ProtocolError::invalid_response(
                        "Chat Completions stream ended before [DONE]",
                    ))
                }
                Some(Err(error)) => {
                    state.finished = true;
                    Err(error)
                }
            };
            match result {
                Ok(events) => state.queued.extend(events.into_iter().map(Ok)),
                Err(error) => {
                    state.finished = true;
                    let error = error
                        .with_request_id(Some(state.request_id.clone()))
                        .with_retry_after(state.retry_after);
                    state.queued.push_back(Err(error));
                }
            }
        }
    });
    Ok(ProtocolStream {
        events: Box::pin(events),
    })
}

fn apply_stream_chunk(
    data: &str,
    state: &mut ChatCompletionStreamState,
) -> ProtocolResultValue<Vec<ProtocolEvent>> {
    let value: Value = serde_json::from_str(data).map_err(|_| {
        ProtocolError::invalid_response("Chat Completions SSE data is not valid JSON")
    })?;
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::invalid_response("Chat Completions SSE chunk must be an object")
    })?;
    if let Some(error) = object.get("error").and_then(Value::as_object) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Chat Completions stream failed");
        let label = error
            .get("code")
            .or_else(|| error.get("type"))
            .and_then(Value::as_str);
        return Err(ProtocolError::new(
            ProtocolErrorKind::Transport,
            label.map_or_else(
                || message.to_string(),
                |label| format!("OpenAI {label}: {message}"),
            ),
        ));
    }
    if object.get("object").and_then(Value::as_str) != Some("chat.completion.chunk") {
        return Err(ProtocolError::invalid_response(
            "Chat Completions SSE chunk has an invalid object type",
        ));
    }
    state.saw_chunk = true;
    if let Some(usage) = decode_usage(object.get("usage"))? {
        state.usage = Some(usage);
    }
    let choices = object
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::invalid_response("SSE choices must be an array"))?;
    if choices.len() > 1 {
        return Err(ProtocolError::invalid_response(
            "Chat Completions SSE must not contain multiple choices",
        ));
    }
    let Some(choice) = choices.first() else {
        return Ok(Vec::new());
    };
    let choice = choice
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("SSE choice must be an object"))?;
    if choice.get("index").and_then(Value::as_u64) != Some(0) {
        return Err(ProtocolError::invalid_response(
            "Chat Completions SSE choice index must be zero",
        ));
    }
    if state.finish_reason.is_some()
        && choice
            .get("delta")
            .and_then(Value::as_object)
            .is_some_and(|delta| !delta.is_empty())
    {
        return Err(ProtocolError::invalid_response(
            "Chat Completions SSE emitted a delta after finish_reason",
        ));
    }
    let mut events = Vec::new();
    if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
        if let Some(role) = delta.get("role") {
            if role.as_str() != Some("assistant") {
                return Err(ProtocolError::invalid_response(
                    "Chat Completions role delta must be assistant",
                ));
            }
        }
        if let Some(content) = delta.get("content") {
            match content {
                Value::String(content) => {
                    state.text.push_str(content);
                    events.push(ProtocolEvent::Delta(json!({
                        "type": "text_delta",
                        "text": content
                    })));
                }
                Value::Null => {}
                _ => {
                    return Err(ProtocolError::invalid_response(
                        "Chat Completions content delta must be a string or null",
                    ));
                }
            }
        }
        if let Some(refusal) = delta.get("refusal") {
            match refusal {
                Value::String(refusal) => {
                    state.refusal.push_str(refusal);
                    events.push(ProtocolEvent::Delta(json!({
                        "type": "refusal_delta",
                        "refusal": refusal
                    })));
                }
                Value::Null => {}
                _ => {
                    return Err(ProtocolError::invalid_response(
                        "Chat Completions refusal delta must be a string or null",
                    ));
                }
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls") {
            let tool_calls = tool_calls.as_array().ok_or_else(|| {
                ProtocolError::invalid_response("SSE tool_calls delta must be an array")
            })?;
            for tool_call in tool_calls {
                events.push(apply_tool_call_delta(tool_call, state)?);
            }
        }
    }
    if let Some(finish_reason) = choice.get("finish_reason") {
        match finish_reason {
            Value::String(reason) if !reason.is_empty() => {
                state.finish_reason = Some(reason.clone());
            }
            Value::Null => {}
            _ => {
                return Err(ProtocolError::invalid_response(
                    "SSE finish_reason must be a string or null",
                ));
            }
        }
    }
    Ok(events)
}

fn apply_tool_call_delta(
    value: &Value,
    state: &mut ChatCompletionStreamState,
) -> ProtocolResultValue<ProtocolEvent> {
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("SSE tool call delta must be an object"))?;
    let index = object
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| ProtocolError::invalid_response("SSE tool call index is invalid"))?;
    if let Some(tool_type) = object.get("type").and_then(Value::as_str) {
        if tool_type != "function" {
            return Err(ProtocolError::invalid_response(
                "base Chat Completions only accepts function tool deltas",
            ));
        }
    }
    let partial = state.tool_calls.entry(index).or_default();
    if let Some(id) = object.get("id").and_then(Value::as_str) {
        if id.is_empty() || partial.id.as_ref().is_some_and(|current| current != id) {
            return Err(ProtocolError::invalid_response(
                "SSE tool call ID is empty or changed during streaming",
            ));
        }
        partial.id = Some(id.to_string());
    }
    let mut name_delta = "";
    let mut arguments_delta = "";
    if let Some(function) = object.get("function") {
        let function = function.as_object().ok_or_else(|| {
            ProtocolError::invalid_response("SSE tool call function must be an object")
        })?;
        if let Some(name) = function.get("name") {
            name_delta = name.as_str().ok_or_else(|| {
                ProtocolError::invalid_response("SSE tool call name delta must be a string")
            })?;
            partial.name.push_str(name_delta);
        }
        if let Some(arguments) = function.get("arguments") {
            arguments_delta = arguments.as_str().ok_or_else(|| {
                ProtocolError::invalid_response("SSE tool arguments delta must be a string")
            })?;
            partial.arguments.push_str(arguments_delta);
        }
    }
    Ok(ProtocolEvent::Delta(json!({
        "type": "tool_call_delta",
        "index": index,
        "call_id": object.get("id").cloned().unwrap_or(Value::Null),
        "name": name_delta,
        "arguments": arguments_delta
    })))
}

fn finalize_stream(state: &mut ChatCompletionStreamState) -> ProtocolResultValue<ProtocolOutput> {
    if !state.saw_chunk {
        return Err(ProtocolError::invalid_response(
            "Chat Completions stream terminated without chunks",
        ));
    }
    let finish_reason = state.finish_reason.take().ok_or_else(|| {
        ProtocolError::invalid_response("Chat Completions stream is missing finish_reason")
    })?;
    let mut content = Vec::new();
    if !state.text.is_empty() {
        content.push(AiContent::Text {
            text: std::mem::take(&mut state.text),
        });
    }
    if !state.refusal.is_empty() {
        content.push(AiContent::ProviderState {
            provider: OPENAI_PROVIDER_NAMESPACE.to_string(),
            value: json!({
                "type": "chat_completion_refusal",
                "refusal": std::mem::take(&mut state.refusal)
            }),
        });
    }
    let tool_calls = std::mem::take(&mut state.tool_calls);
    for (expected, (index, partial)) in tool_calls.into_iter().enumerate() {
        if index != expected {
            return Err(ProtocolError::invalid_response(
                "Chat Completions streamed tool call indexes are not contiguous",
            ));
        }
        let call_id = partial.id.ok_or_else(|| {
            ProtocolError::invalid_response("streamed tool call is missing an ID")
        })?;
        if partial.name.is_empty() {
            return Err(ProtocolError::invalid_response(
                "streamed tool call is missing a function name",
            ));
        }
        let arguments: Value = serde_json::from_str(&partial.arguments).map_err(|_| {
            ProtocolError::invalid_response("streamed tool arguments are not valid JSON")
        })?;
        let arguments = arguments.as_object().ok_or_else(|| {
            ProtocolError::invalid_response("streamed tool arguments must decode to an object")
        })?;
        content.push(AiContent::ToolUse {
            call_id,
            name: partial.name,
            args: arguments.clone().into_iter().collect(),
        });
    }
    let message = AiMessage::new(AiRole::Assistant, content);
    message.validate().map_err(|error| {
        ProtocolError::invalid_response(format!("invalid streamed assistant message: {error}"))
    })?;
    normalized_output(message, &finish_reason, state.usage.take())
}

fn required_value_string(value: &Value, key: &str) -> ProtocolResultValue<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProtocolError::invalid_request(format!("provider state `{key}` must be a string"))
        })
}

fn required_value_string_from_map(
    value: &Map<String, Value>,
    key: &str,
) -> ProtocolResultValue<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("response field `{key}` is missing"))
        })
}

fn required_u64(value: &Map<String, Value>, key: &str) -> ProtocolResultValue<u64> {
    value.get(key).and_then(Value::as_u64).ok_or_else(|| {
        ProtocolError::invalid_response(format!("usage field `{key}` is missing or invalid"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, GoldenBody, ProtocolContractHarness,
        ResolvedCredential,
    };
    use buckyos_api::{AiToolSpec, LlmChatInvokeRequest, LlmResponseFormat};
    use bytes::Bytes;
    use futures_util::{stream, StreamExt};
    use reqwest::header::{HeaderValue, AUTHORIZATION};
    use std::collections::HashMap;
    use std::time::{Duration, UNIX_EPOCH};

    fn codec() -> OpenAiChatCompletionsCodec {
        OpenAiChatCompletionsCodec::new()
    }

    fn context(base_url: &str) -> CodecContext {
        CodecContext {
            base_url: base_url.to_string(),
            credential: Some(ResolvedCredential::bearer("secret://chat", "test-secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }

    fn input(request: LlmChatInvokeRequest, extra: &[(&str, Value)]) -> CodecInput {
        let mut resolved_parameters = BTreeMap::from([(
            "provider_model_id".to_string(),
            Value::String("provider-model".to_string()),
        )]);
        for (name, value) in extra {
            resolved_parameters.insert((*name).to_string(), value.clone());
        }
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(request),
            resolved_parameters,
        }
    }

    fn encode(
        codec: &OpenAiChatCompletionsCodec,
        input: &CodecInput,
        context: &CodecContext,
    ) -> ProtocolResultValue<HttpRequest> {
        codec.encode(&CodecCall {
            api_type: ApiType::Llm,
            input,
            context,
        })
    }

    fn success_response(body: Value) -> HttpResponse {
        HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from(serde_json::to_vec(&body).unwrap()),
            request_id: "request-1".to_string(),
            retry_after: None,
        }
    }

    #[test]
    fn registers_one_protocol_family_operation_without_responses_fallback() {
        let (descriptor, codecs) = openai_chat_completions_adapter();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.protocol_family_id, "openai");
        assert_eq!(
            descriptor.protocol_adapter_id,
            OPENAI_CHAT_COMPLETIONS_ADAPTER_ID
        );
        assert_eq!(descriptor.base_adapter_id, None);
        assert_eq!(descriptor.operations.len(), 1);
        let operation = &descriptor.operations[OPENAI_CHAT_COMPLETIONS_OPERATION_ID];
        assert_eq!(operation.bindings.len(), 1);
        assert_eq!(operation.bindings[0].api_type, ApiType::Llm);
        assert_eq!(
            operation.bindings[0].execution_modes,
            BTreeSet::from([ExecutionMode::Immediate, ExecutionMode::Stream])
        );

        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, codecs).unwrap();
        assert!(registry
            .operation_descriptor(
                OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
                ApiType::Llm,
            )
            .is_ok());
        assert!(registry.adapter("openai-responses").is_none());
    }

    #[test]
    fn encodes_canonical_messages_tools_structured_output_and_stream_options() {
        let mut request = LlmChatInvokeRequest::new(
            "must-not-be-used@provider",
            vec![
                AiMessage::text(AiRole::System, "system"),
                AiMessage::text(AiRole::Developer, "developer"),
                AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::Text {
                            text: "inspect".to_string(),
                        },
                        AiContent::Image {
                            source: ResourceRef::Base64 {
                                mime: "image/png".to_string(),
                                data_base64: "aW1hZ2U=".to_string(),
                            },
                        },
                    ],
                ),
                AiMessage::new(
                    AiRole::Assistant,
                    vec![AiContent::ToolUse {
                        call_id: "call-1".to_string(),
                        name: "weather".to_string(),
                        args: HashMap::from([("city".to_string(), json!("Paris"))]),
                    }],
                ),
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::ToolResult {
                        call_id: "call-1".to_string(),
                        content: vec![AiToolResultContent::Text {
                            text: "sunny".to_string(),
                        }],
                        is_error: false,
                    }],
                ),
            ],
        );
        request.tools = vec![AiToolSpec {
            tool_type: "function".to_string(),
            name: "weather".to_string(),
            description: "Weather lookup".to_string(),
            args_json_schema: json!({"type": "object"}),
            output_schema: None,
        }];
        request.response_format = Some(LlmResponseFormat::json_schema(
            Some("weather_result".to_string()),
            json!({"type": "object"}),
            Some(true),
        ));
        request.temperature = Some(0.5);
        request.top_p = Some(0.9);
        request.max_output_tokens = Some(256);
        request.seed = Some(7);
        request.stop = vec!["STOP".to_string()];
        let input = input(
            request,
            &[
                ("stream", json!(true)),
                ("tool_choice", json!("auto")),
                ("parallel_tool_calls", json!(true)),
            ],
        );
        let wire = encode(&codec(), &input, &context("https://gateway.example/api/v1")).unwrap();
        let golden = ProtocolContractHarness::default().request(&wire).unwrap();
        assert_eq!(golden.method, "POST");
        assert_eq!(
            golden.url,
            "https://gateway.example/api/v1/chat/completions"
        );
        assert_eq!(golden.headers["authorization"], "[REDACTED]");
        let GoldenBody::Json(body) = golden.body else {
            panic!("expected JSON request")
        };
        assert_eq!(body["model"], "provider-model");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "developer");
        assert_eq!(
            body["messages"][2]["content"][1]["image_url"]["url"],
            "data:image/png;base64,aW1hZ2U="
        );
        assert_eq!(
            body["messages"][3]["tool_calls"][0]["function"]["arguments"],
            r#"{"city":"Paris"}"#
        );
        assert_eq!(body["messages"][4]["tool_call_id"], "call-1");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["max_completion_tokens"], 256);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn rejects_unlowered_resources_unknown_parameters_and_missing_credentials() {
        let request = LlmChatInvokeRequest::new(
            "model@provider",
            vec![AiMessage::new(
                AiRole::User,
                vec![AiContent::Image {
                    source: ResourceRef::named_object(ndn_lib::ObjId::new("chunk:123456").unwrap()),
                }],
            )],
        );
        let resource_input = input(request, &[]);
        assert_eq!(
            encode(
                &codec(),
                &resource_input,
                &context("https://example.test/v1"),
            )
            .unwrap_err()
            .kind,
            ProtocolErrorKind::InvalidRequest
        );

        let request = LlmChatInvokeRequest::new(
            "model@provider",
            vec![AiMessage::text(AiRole::User, "hello")],
        );
        let unknown_parameter_input = input(request.clone(), &[("extra_body", json!({"x": 1}))]);
        assert_eq!(
            encode(
                &codec(),
                &unknown_parameter_input,
                &context("https://example.test/v1"),
            )
            .unwrap_err()
            .kind,
            ProtocolErrorKind::InvalidRequest
        );

        let mut missing_credential_context = context("https://example.test/v1");
        missing_credential_context.credential = None;
        assert_eq!(
            encode(&codec(), &input(request, &[]), &missing_credential_context)
                .unwrap_err()
                .kind,
            ProtocolErrorKind::Authentication
        );
    }

    #[test]
    fn rejects_base_unsupported_thinking_and_non_stream_options() {
        let thinking_request = LlmChatInvokeRequest::new(
            "model@provider",
            vec![AiMessage::new(
                AiRole::Assistant,
                vec![AiContent::Thinking {
                    summary: Some("private reasoning".to_string()),
                    text: None,
                    provider_metadata: None,
                }],
            )],
        );
        assert_eq!(
            encode(
                &codec(),
                &input(thinking_request, &[]),
                &context("https://example.test/v1"),
            )
            .unwrap_err()
            .kind,
            ProtocolErrorKind::UnsupportedOperation
        );

        let request = LlmChatInvokeRequest::new(
            "model@provider",
            vec![AiMessage::text(AiRole::User, "hello")],
        );
        assert_eq!(
            encode(
                &codec(),
                &input(
                    request,
                    &[("stream_options", json!({"include_usage": true}))]
                ),
                &context("https://example.test/v1"),
            )
            .unwrap_err()
            .kind,
            ProtocolErrorKind::InvalidRequest
        );
    }

    #[tokio::test]
    async fn decodes_text_refusal_tool_calls_finish_reason_and_usage() {
        let response = success_response(json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "provider-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "answer",
                    "refusal": "restricted",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "weather",
                            "arguments": "{\"city\":\"Paris\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 4,
                "total_tokens": 9
            }
        }));
        let ProtocolExecution::Immediate(output) = codec().decode(response).await.unwrap() else {
            panic!("expected immediate output")
        };
        assert_eq!(output.value["finish_reason"], "tool_calls");
        assert_eq!(output.value["message"]["content"][0]["text"], "answer");
        assert_eq!(
            output.value["message"]["content"][1]["provider"],
            OPENAI_PROVIDER_NAMESPACE
        );
        assert_eq!(output.value["tool_calls"][0]["call_id"], "call-1");
        assert_eq!(output.value["tool_calls"][0]["args"]["city"], "Paris");
        assert_eq!(output.usage.unwrap().total_tokens, Some(9));
    }

    #[tokio::test]
    async fn maps_official_error_shape_status_request_id_and_retry_after() {
        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::TOO_MANY_REQUESTS,
                &[("retry-after", "3")],
                Bytes::from_static(
                    br#"{"error":{"message":"slow down","type":"rate_limit_error","code":"rate_limit"}}"#,
                ),
                "request-rate",
                UNIX_EPOCH,
            )
            .unwrap();
        let error = codec().decode(response).await.unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert!(error.message.contains("rate_limit"));
        assert_eq!(error.request_id.as_deref(), Some("request-rate"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(3)));
    }

    #[tokio::test]
    async fn incrementally_decodes_fragmented_text_tool_arguments_usage_and_done() {
        let chunks = vec![
            Ok(Bytes::from_static(
                b"data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\",\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"weather\",\"arguments\":\"{\\\"city\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Paris\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            )),
            Ok(Bytes::from_static(
                b"data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":4,\"total_tokens\":9}}\n\n",
            )),
            Ok(Bytes::from_static(b"data: [DO")),
            Ok(Bytes::from_static(b"NE]\n\n")),
        ];
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::iter(chunks)),
            request_id: "request-stream".to_string(),
            retry_after: None,
        };
        let mut stream = codec().decode_stream(response).await.unwrap();
        let mut deltas = Vec::new();
        let mut final_output = None;
        while let Some(event) = stream.events.next().await {
            match event.unwrap() {
                ProtocolEvent::Delta(delta) => deltas.push(delta),
                ProtocolEvent::Final(output) => final_output = Some(output),
                ProtocolEvent::Progress(_) => {}
            }
        }
        assert_eq!(deltas[0], json!({"type": "text_delta", "text": "Hel"}));
        assert!(deltas
            .iter()
            .any(|delta| delta["type"] == "tool_call_delta"));
        let final_output = final_output.expect("final output");
        assert_eq!(final_output.value["message"]["content"][0]["text"], "Hello");
        assert_eq!(final_output.value["tool_calls"][0]["call_id"], "call-1");
        assert_eq!(final_output.value["tool_calls"][0]["args"]["city"], "Paris");
        assert_eq!(final_output.value["finish_reason"], "tool_calls");
        assert_eq!(final_output.usage.unwrap().total_tokens, Some(9));
    }

    #[tokio::test]
    async fn rejects_stream_without_done_and_invalid_tool_arguments() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::once(async {
                Ok(Bytes::from_static(
                    b"data: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n",
                ))
            })),
            request_id: "request-disconnect".to_string(),
            retry_after: None,
        };
        let mut stream = codec().decode_stream(response).await.unwrap();
        assert!(stream.events.next().await.unwrap().is_ok());
        let error = stream.events.next().await.unwrap().unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidResponse);
        assert_eq!(error.request_id.as_deref(), Some("request-disconnect"));

        let response = success_response(json!({
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {"name": "tool", "arguments": "not-json"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }));
        assert_eq!(
            codec().decode(response).await.unwrap_err().kind,
            ProtocolErrorKind::InvalidResponse
        );
    }

    #[tokio::test]
    async fn openrouter_kimi_and_glm_share_the_identical_base_contract() {
        let consumers = [
            ("openrouter-openai", "https://openrouter.example/api/v1"),
            ("kimi-chat", "https://kimi.example/v1"),
            ("glm-chat", "https://glm.example/api/paas/v4"),
        ];
        for (consumer, base_url) in consumers {
            let request = LlmChatInvokeRequest::new(
                "model@provider",
                vec![AiMessage::text(AiRole::User, "hello")],
            );
            let wire = encode(&codec(), &input(request, &[]), &context(base_url))
                .unwrap_or_else(|error| panic!("{consumer} base encode failed: {error}"));
            assert_eq!(wire.url, format!("{base_url}/chat/completions"));
            let ProtocolExecution::Immediate(output) = codec()
                .decode(success_response(json!({
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "ok"},
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 1,
                        "completion_tokens": 1,
                        "total_tokens": 2
                    }
                })))
                .await
                .unwrap_or_else(|error| panic!("{consumer} base decode failed: {error}"))
            else {
                panic!("{consumer} did not use immediate base contract")
            };
            assert_eq!(output.value["message"]["content"][0]["text"], "ok");
        }
    }

    #[test]
    fn debug_and_golden_contract_do_not_expose_credentials() {
        let request = LlmChatInvokeRequest::new(
            "model@provider",
            vec![AiMessage::text(AiRole::User, "hello")],
        );
        let input = input(request, &[]);
        let context = context("https://example.test/v1");
        let wire = encode(&codec(), &input, &context).unwrap();
        let rendered = format!("{context:?} {wire:?}");
        assert!(!rendered.contains("test-secret"));
        let golden = ProtocolContractHarness::default().request(&wire).unwrap();
        assert_eq!(golden.headers[AUTHORIZATION.as_str()], "[REDACTED]");
    }
}
