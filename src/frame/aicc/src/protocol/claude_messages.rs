use super::{
    sse_frame_stream, AdapterDescriptor, AdapterStatus, CodecCall, CredentialKind, ExecutionMode,
    HttpBody, HttpRequest, HttpResponse, OperationBinding, OperationCodec, OperationDescriptor,
    ProtocolError, ProtocolErrorKind, ProtocolEvent, ProtocolExecution, ProtocolOutput,
    ProtocolResultValue, ProtocolStream, SseConfig, SseFrame, StreamingHttpResponse,
};
use async_trait::async_trait;
use buckyos_api::{
    features, AiContent, AiMessage, AiRole, AiToolCall, AiToolResultContent, AiUsage, AiccCall,
    ApiType, LlmChatInvokeRequest, LlmResponseFormatType, ResourceRef,
};
use futures_util::{stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(crate) const CLAUDE_PROTOCOL_FAMILY_ID: &str = "claude";
pub(crate) const CLAUDE_MESSAGES_ADAPTER_ID: &str = "claude-messages";
pub(crate) const CLAUDE_MESSAGES_OPERATION_ID: &str = "messages.create";
pub(crate) const CLAUDE_MESSAGES_VERSION: &str = "2023-06-01";

const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const CLAUDE_PROVIDER_NAMESPACE: &str = "claude";

#[derive(Debug, Clone)]
pub(crate) struct ClaudeMessagesCodec {
    descriptor: OperationDescriptor,
}

impl ClaudeMessagesCodec {
    pub(crate) fn new() -> Self {
        Self {
            descriptor: claude_messages_operation_descriptor(),
        }
    }

    pub(crate) fn adapter_descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            protocol_family_id: CLAUDE_PROTOCOL_FAMILY_ID.to_string(),
            protocol_adapter_id: CLAUDE_MESSAGES_ADAPTER_ID.to_string(),
            interface_generation: CLAUDE_MESSAGES_VERSION.to_string(),
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
        if request.messages.len() > 100_000 {
            return Err(ProtocolError::invalid_request(
                "Claude Messages request exceeds the 100000-message limit",
            ));
        }
        for message in &request.messages {
            message.validate().map_err(|error| {
                ProtocolError::invalid_request(format!("invalid canonical message: {error}"))
            })?;
        }
        validate_canonical_options(request)?;

        let provider_model_id =
            required_string(&call.input.resolved_parameters, "provider_model_id")?;
        let max_tokens = resolved_u64(&call.input.resolved_parameters, "max_tokens")?
            .or(request.max_output_tokens)
            .ok_or_else(|| {
                ProtocolError::invalid_request(
                    "Claude Messages requires max_output_tokens or resolved max_tokens",
                )
            })?;

        let (system, messages) = encode_messages(&request.messages)?;
        if messages.is_empty() {
            return Err(ProtocolError::invalid_request(
                "Claude Messages requires at least one conversation message",
            ));
        }
        let mut body = Map::new();
        body.insert("model".to_string(), Value::String(provider_model_id));
        body.insert("messages".to_string(), Value::Array(messages));
        body.insert("max_tokens".to_string(), Value::from(max_tokens));
        if !system.is_empty() {
            body.insert("system".to_string(), Value::Array(system));
        }
        if !request.tools.is_empty() {
            body.insert("tools".to_string(), encode_tools(request)?);
        }
        if let Some(temperature) = request.temperature {
            body.insert(
                "temperature".to_string(),
                finite_number("temperature", temperature)?,
            );
        }
        if let Some(top_p) = request.top_p {
            body.insert("top_p".to_string(), finite_number("top_p", top_p)?);
        }
        if !request.stop.is_empty() {
            if request.stop.iter().any(|sequence| sequence.is_empty()) {
                return Err(ProtocolError::invalid_request(
                    "Claude stop sequences must not be empty",
                ));
            }
            body.insert(
                "stop_sequences".to_string(),
                serde_json::to_value(&request.stop).map_err(invalid_request_json)?,
            );
        }
        apply_resolved_parameters(&mut body, &call.input.resolved_parameters)?;

        let mut headers = HeaderMap::new();
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(CLAUDE_MESSAGES_VERSION),
        );
        let credential = call.context.credential.as_ref().ok_or_else(|| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Claude Messages requires a resolved x-api-key credential",
            )
        })?;
        if credential.audit().kind != CredentialKind::NamedHeader {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Claude Messages requires a named-header credential",
            ));
        }
        credential.apply(&mut headers)?;
        let mut wire_request = HttpRequest::new(
            Method::POST,
            claude_messages_endpoint(&call.context.base_url)?,
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

fn claude_messages_endpoint(base_url: &str) -> ProtocolResultValue<String> {
    let base_url = base_url.trim_end_matches('/');
    let endpoint = if base_url.ends_with("/v1") {
        format!("{base_url}/messages")
    } else {
        format!("{base_url}/v1/messages")
    };
    reqwest::Url::parse(&endpoint).map_err(|_| {
        ProtocolError::invalid_configuration("Claude Messages endpoint URL is invalid")
    })?;
    Ok(endpoint)
}

#[async_trait]
impl OperationCodec for ClaudeMessagesCodec {
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
                "Claude Messages only accepts chat.completions.create",
            )),
        }
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        if !response.status.is_success() {
            return Err(decode_error_response(response));
        }
        if is_event_stream(&response.headers) {
            return Err(ProtocolError::invalid_response(
                "Claude event-stream response must use the streaming decoder",
            )
            .with_request_id(Some(response.request_id)));
        }
        decode_immediate_response(response)
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        decode_incremental_stream(response, self.descriptor.max_response_bytes).await
    }
}

pub(crate) fn claude_messages_operation_descriptor() -> OperationDescriptor {
    let mut binding = OperationBinding::new(
        ApiType::Llm,
        [ExecutionMode::Immediate, ExecutionMode::Stream],
    );
    binding.supported_features = BTreeSet::from([
        features::TOOL_CALLING.to_string(),
        features::VISION.to_string(),
        features::PLAN.to_string(),
    ]);
    OperationDescriptor {
        operation_id: CLAUDE_MESSAGES_OPERATION_ID.to_string(),
        bindings: vec![binding],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

fn validate_canonical_options(request: &LlmChatInvokeRequest) -> ProtocolResultValue<()> {
    if let Some(format) = &request.response_format {
        if !matches!(format.format_type, LlmResponseFormatType::Text) {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Claude Messages codec does not map canonical structured output",
            ));
        }
    }
    if request.seed.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Claude Messages does not support seed",
        ));
    }
    if request.output.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Claude Messages does not support output media options",
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

fn resolved_u64(
    parameters: &BTreeMap<String, Value>,
    key: &str,
) -> ProtocolResultValue<Option<u64>> {
    parameters
        .get(key)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                ProtocolError::invalid_request(format!(
                    "resolved {key} must be an unsigned integer"
                ))
            })
        })
        .transpose()
}

fn finite_number(name: &str, value: f64) -> ProtocolResultValue<Value> {
    if !(0.0..=1.0).contains(&value) {
        return Err(ProtocolError::invalid_request(format!(
            "{name} must be between 0 and 1"
        )));
    }
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| ProtocolError::invalid_request(format!("{name} must be finite")))
}

fn invalid_request_json(error: serde_json::Error) -> ProtocolError {
    ProtocolError::invalid_request(format!("failed to encode Claude request: {error}"))
}

fn encode_messages(messages: &[AiMessage]) -> ProtocolResultValue<(Vec<Value>, Vec<Value>)> {
    let mut system = Vec::new();
    let mut wire_messages = Vec::new();
    for message in messages {
        match message.role {
            AiRole::System | AiRole::Developer => {
                for block in &message.content {
                    system.push(encode_content(block, false)?);
                }
            }
            AiRole::User | AiRole::Assistant => {
                let content = encode_message_content(&message.content)?;
                if content.is_empty() {
                    return Err(ProtocolError::invalid_request(
                        "Claude message has no representable content blocks",
                    ));
                }
                wire_messages.push(json!({
                    "role": message.role.as_str(),
                    "content": content
                }));
            }
            AiRole::Tool => wire_messages.push(json!({
                "role": "user",
                "content": message.content.iter()
                    .map(|block| encode_content(block, true))
                    .collect::<ProtocolResultValue<Vec<_>>>()?
            })),
        }
    }
    Ok((system, wire_messages))
}

fn encode_message_content(content: &[AiContent]) -> ProtocolResultValue<Vec<Value>> {
    content
        .iter()
        .filter(|block| {
            !matches!(
                block,
                AiContent::ProviderState { provider, .. }
                    if provider != CLAUDE_PROVIDER_NAMESPACE
            )
        })
        .map(|block| encode_content(block, true))
        .collect()
}

fn encode_content(content: &AiContent, allow_provider_state: bool) -> ProtocolResultValue<Value> {
    match content {
        AiContent::Text { text } => Ok(json!({"type": "text", "text": text})),
        AiContent::Image { source } => Ok(json!({
            "type": "image",
            "source": encode_resource(source, false)?
        })),
        AiContent::Document { source, title } => {
            let mut block = Map::from_iter([
                ("type".to_string(), Value::String("document".to_string())),
                ("source".to_string(), encode_resource(source, true)?),
            ]);
            if let Some(title) = title {
                block.insert("title".to_string(), Value::String(title.clone()));
            }
            Ok(Value::Object(block))
        }
        AiContent::ToolUse {
            call_id,
            name,
            args,
        } => Ok(json!({
            "type": "tool_use", "id": call_id, "name": name, "input": args
        })),
        AiContent::ToolResult {
            call_id,
            content,
            is_error,
        } => Ok(json!({
            "type": "tool_result",
            "tool_use_id": call_id,
            "content": content.iter().map(encode_tool_result_content)
                .collect::<ProtocolResultValue<Vec<_>>>()?,
            "is_error": is_error
        })),
        AiContent::Thinking {
            summary,
            text,
            provider_metadata,
        } => {
            if summary.is_some() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Claude thinking blocks do not accept canonical summaries",
                ));
            }
            let signature = provider_metadata
                .as_ref()
                .and_then(|metadata| metadata.get("signature"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ProtocolError::invalid_request(
                        "Claude thinking input must retain provider_metadata.signature",
                    )
                })?;
            Ok(json!({
                "type": "thinking",
                "thinking": text.as_deref().unwrap_or_default(),
                "signature": signature
            }))
        }
        AiContent::ProviderState { provider, value } => {
            if allow_provider_state && provider == CLAUDE_PROVIDER_NAMESPACE && value.is_object() {
                Ok(value.clone())
            } else {
                Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "content block cannot be represented by Claude Messages",
                ))
            }
        }
    }
}

fn encode_tool_result_content(content: &AiToolResultContent) -> ProtocolResultValue<Value> {
    match content {
        AiToolResultContent::Text { text } => Ok(json!({"type": "text", "text": text})),
        AiToolResultContent::Image { source } => Ok(json!({
            "type": "image", "source": encode_resource(source, false)?
        })),
        AiToolResultContent::Document { source, title } => {
            let mut block = Map::from_iter([
                ("type".to_string(), Value::String("document".to_string())),
                ("source".to_string(), encode_resource(source, true)?),
            ]);
            if let Some(title) = title {
                block.insert("title".to_string(), Value::String(title.clone()));
            }
            Ok(Value::Object(block))
        }
    }
}

fn encode_resource(resource: &ResourceRef, document: bool) -> ProtocolResultValue<Value> {
    match resource {
        ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
            "type": "base64", "media_type": mime, "data": data_base64
        })),
        ResourceRef::Url { url, .. } => Ok(json!({"type": "url", "url": url})),
        ResourceRef::NamedObject { .. } => Err(ProtocolError::invalid_request(format!(
            "Claude {} resource must be materialized before protocol encoding",
            if document { "document" } else { "image" }
        ))),
    }
}

fn encode_tools(request: &LlmChatInvokeRequest) -> ProtocolResultValue<Value> {
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            if tool.tool_type != "function" {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Claude Messages only maps function tools",
                ));
            }
            if tool.output_schema.is_some() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Claude Messages does not map tool output_schema",
                ));
            }
            if tool.name.trim().is_empty() || !tool.args_json_schema.is_object() {
                return Err(ProtocolError::invalid_request(
                    "Claude tool name must be non-empty and input schema must be an object",
                ));
            }
            Ok(json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.args_json_schema
            }))
        })
        .collect::<ProtocolResultValue<Vec<_>>>()?;
    Ok(Value::Array(tools))
}

fn apply_resolved_parameters(
    body: &mut Map<String, Value>,
    parameters: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    const ALLOWED: &[&str] = &[
        "max_tokens",
        "metadata",
        "service_tier",
        "stream",
        "thinking",
        "tool_choice",
        "top_k",
    ];
    for (name, value) in parameters {
        if name == "provider_model_id" {
            continue;
        }
        if !ALLOWED.contains(&name.as_str()) {
            return Err(ProtocolError::invalid_request(format!(
                "resolved Claude parameter `{name}` is not supported"
            )));
        }
        let valid = match name.as_str() {
            "max_tokens" | "top_k" => value.as_u64().is_some(),
            "metadata" | "thinking" | "tool_choice" => value.is_object(),
            "service_tier" => value.is_string(),
            "stream" => value.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(ProtocolError::invalid_request(format!(
                "resolved Claude parameter `{name}` has an invalid type"
            )));
        }
        body.insert(name.clone(), value.clone());
    }
    Ok(())
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
}

fn decode_immediate_response(response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
    let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
        ProtocolError::invalid_response(format!("Claude response is not valid JSON: {error}"))
            .with_request_id(Some(response.request_id.clone()))
    })?;
    let normalized = normalize_message(&value)
        .map_err(|error| error.with_request_id(Some(response.request_id.clone())))?;
    Ok(ProtocolExecution::Immediate(normalized))
}

fn normalize_message(message: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let object = message.as_object().ok_or_else(|| {
        ProtocolError::invalid_response("Claude message response must be an object")
    })?;
    if object.get("type").and_then(Value::as_str) != Some("message")
        || object.get("role").and_then(Value::as_str) != Some("assistant")
    {
        return Err(ProtocolError::invalid_response(
            "Claude response must be an assistant message",
        ));
    }
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProtocolError::invalid_response("Claude response content must be an array")
        })?;
    let blocks = content
        .iter()
        .map(decode_content)
        .collect::<ProtocolResultValue<Vec<_>>>()?;
    let ai_message = AiMessage::new(AiRole::Assistant, blocks);
    ai_message.validate().map_err(|error| {
        ProtocolError::invalid_response(format!("invalid Claude response content: {error}"))
    })?;
    let usage = decode_usage(object.get("usage"))?;
    let tool_calls = ai_message.tool_calls();
    let finish_reason = object.get("stop_reason").cloned().unwrap_or(Value::Null);
    let value = normalized_value(&ai_message, &tool_calls, finish_reason)?;
    Ok(ProtocolOutput {
        value,
        usage: Some(usage),
        artifacts: Vec::new(),
    })
}

fn decode_content(value: &Value) -> ProtocolResultValue<AiContent> {
    let block_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_response("Claude content block is missing type"))?;
    match block_type {
        "text" => Ok(AiContent::Text {
            text: required_value_string(value, "text")?,
        }),
        "tool_use" => {
            let input = value
                .get("input")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    ProtocolError::invalid_response("Claude tool_use input must be an object")
                })?;
            Ok(AiContent::ToolUse {
                call_id: required_value_string(value, "id")?,
                name: required_value_string(value, "name")?,
                args: input.clone().into_iter().collect(),
            })
        }
        "thinking" => Ok(AiContent::Thinking {
            summary: None,
            text: Some(required_value_string(value, "thinking")?),
            provider_metadata: Some(
                json!({"signature": required_value_string(value, "signature")?}),
            ),
        }),
        _ => Ok(AiContent::ProviderState {
            provider: CLAUDE_PROVIDER_NAMESPACE.to_string(),
            value: value.clone(),
        }),
    }
}

fn required_value_string(value: &Value, field: &str) -> ProtocolResultValue<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!(
                "Claude content field `{field}` must be a string"
            ))
        })
}

fn decode_usage(value: Option<&Value>) -> ProtocolResultValue<AiUsage> {
    let usage = value.and_then(Value::as_object).ok_or_else(|| {
        ProtocolError::invalid_response("successful Claude response is missing usage")
    })?;
    let input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ProtocolError::invalid_response("Claude usage.input_tokens must be an unsigned integer")
        })?;
    let output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ProtocolError::invalid_response(
                "Claude usage.output_tokens must be an unsigned integer",
            )
        })?;
    let total_tokens = input_tokens
        .checked_add(output_tokens)
        .ok_or_else(|| ProtocolError::invalid_response("Claude usage token total overflow"))?;
    Ok(AiUsage {
        input_tokens: Some(input_tokens),
        output_tokens: Some(output_tokens),
        total_tokens: Some(total_tokens),
        request_units: None,
    })
}

#[derive(Serialize)]
struct NormalizedMessage<'a> {
    message: &'a AiMessage,
    tool_calls: &'a [AiToolCall],
    finish_reason: Value,
}

fn normalized_value(
    message: &AiMessage,
    tool_calls: &[AiToolCall],
    finish_reason: Value,
) -> ProtocolResultValue<Value> {
    serde_json::to_value(NormalizedMessage {
        message,
        tool_calls,
        finish_reason,
    })
    .map_err(|error| {
        ProtocolError::invalid_response(format!("failed to normalize Claude response: {error}"))
    })
}

fn decode_error_response(response: HttpResponse) -> ProtocolError {
    let parsed: Option<Value> = serde_json::from_slice(&response.body).ok();
    let provider_type = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/type"))
        .and_then(Value::as_str);
    let provider_message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("Claude request failed");
    let message = provider_type.map_or_else(
        || provider_message.to_string(),
        |kind| format!("Claude {kind}: {provider_message}"),
    );
    let kind = match response.status {
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
            ProtocolErrorKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
        _ => ProtocolErrorKind::Transport,
    };
    ProtocolError::new(kind, message)
        .with_request_id(Some(response.request_id))
        .with_retry_after(response.retry_after)
}

#[derive(Debug)]
struct ClaudeStreamState {
    blocks: BTreeMap<usize, Value>,
    stopped_blocks: BTreeSet<usize>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    finish_reason: Value,
    started: bool,
    saw_message_stop: bool,
}

impl Default for ClaudeStreamState {
    fn default() -> Self {
        Self {
            blocks: BTreeMap::new(),
            stopped_blocks: BTreeSet::new(),
            input_tokens: None,
            output_tokens: None,
            finish_reason: Value::Null,
            started: false,
            saw_message_stop: false,
        }
    }
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
            "Claude streaming response must use text/event-stream",
        )
        .with_request_id(Some(response.request_id))
        .with_retry_after(response.retry_after));
    }
    let request_id = response.request_id.clone();
    let retry_after = response.retry_after;
    let frames = sse_frame_stream(
        response,
        SseConfig {
            termination_markers: Vec::new(),
            ..SseConfig::default()
        },
        max_response_bytes,
    )
    .await?;

    struct State {
        frames: super::SseFrameStream,
        claude: ClaudeStreamState,
        queued: VecDeque<ProtocolResultValue<ProtocolEvent>>,
        request_id: String,
        retry_after: Option<std::time::Duration>,
        finished: bool,
    }

    let state = State {
        frames,
        claude: ClaudeStreamState::default(),
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
            let result = match state.frames.next().await {
                Some(Ok(SseFrame::Event(event))) => {
                    decode_stream_event(event.event.as_deref(), &event.data, &mut state.claude)
                }
                Some(Ok(SseFrame::StreamEnd(_))) => {
                    state.finished = true;
                    if state.claude.saw_message_stop {
                        Ok(Vec::new())
                    } else {
                        Err(ProtocolError::invalid_response(
                            "Claude SSE ended before message_stop",
                        ))
                    }
                }
                Some(Ok(SseFrame::Terminated { .. })) => {
                    state.finished = true;
                    Err(ProtocolError::invalid_response(
                        "Claude SSE used an unexpected termination marker",
                    ))
                }
                Some(Err(error)) => {
                    state.finished = true;
                    Err(error)
                }
                None => {
                    state.finished = true;
                    if state.claude.saw_message_stop {
                        Ok(Vec::new())
                    } else {
                        Err(ProtocolError::invalid_response(
                            "Claude SSE ended before message_stop",
                        ))
                    }
                }
            };
            match result {
                Ok(events) => state.queued.extend(events.into_iter().map(Ok)),
                Err(error) => {
                    let request_id = error
                        .request_id
                        .clone()
                        .or_else(|| Some(state.request_id.clone()));
                    let retry_after = error.retry_after.or(state.retry_after);
                    state.queued.push_back(Err(error
                        .with_request_id(request_id)
                        .with_retry_after(retry_after)));
                    state.finished = true;
                }
            }
        }
    });
    Ok(ProtocolStream {
        events: Box::pin(events),
    })
}

fn decode_stream_event(
    event_name: Option<&str>,
    data: &str,
    state: &mut ClaudeStreamState,
) -> ProtocolResultValue<Vec<ProtocolEvent>> {
    if state.saw_message_stop {
        return Err(ProtocolError::invalid_response(
            "Claude SSE emitted an event after message_stop",
        ));
    }
    let value: Value = serde_json::from_str(data).map_err(|error| {
        ProtocolError::invalid_response(format!("Claude SSE data is not valid JSON: {error}"))
    })?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_response("Claude SSE event is missing type"))?;
    if event_name.is_some_and(|name| name != event_type) {
        return Err(ProtocolError::invalid_response(
            "Claude SSE event name does not match data type",
        ));
    }
    if !matches!(event_type, "ping" | "error" | "message_start") && !state.started {
        return Err(ProtocolError::invalid_response(
            "Claude SSE event arrived before message_start",
        ));
    }
    match event_type {
        "ping" => Ok(Vec::new()),
        "error" => Err(stream_error(&value)),
        "message_start" => {
            if state.started {
                return Err(ProtocolError::invalid_response(
                    "Claude SSE emitted message_start twice",
                ));
            }
            if value.pointer("/message/type").and_then(Value::as_str) != Some("message")
                || value.pointer("/message/role").and_then(Value::as_str) != Some("assistant")
            {
                return Err(ProtocolError::invalid_response(
                    "Claude message_start must contain an assistant message",
                ));
            }
            let usage = value
                .pointer("/message/usage")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    ProtocolError::invalid_response("Claude message_start is missing usage")
                })?;
            state.input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
            state.output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
            if state.input_tokens.is_none() || state.output_tokens.is_none() {
                return Err(ProtocolError::invalid_response(
                    "Claude message_start usage is invalid",
                ));
            }
            state.started = true;
            Ok(Vec::new())
        }
        "content_block_start" => {
            let index = event_index(&value)?;
            if index != state.blocks.len() {
                return Err(ProtocolError::invalid_response(
                    "Claude SSE content block indexes must be contiguous",
                ));
            }
            let block = value.get("content_block").cloned().ok_or_else(|| {
                ProtocolError::invalid_response(
                    "Claude content_block_start is missing content_block",
                )
            })?;
            if state.blocks.insert(index, block).is_some() {
                return Err(ProtocolError::invalid_response(
                    "Claude SSE started the same content block twice",
                ));
            }
            Ok(Vec::new())
        }
        "content_block_delta" => apply_content_delta(&value, state),
        "content_block_stop" => {
            let index = event_index(&value)?;
            if !state.stopped_blocks.insert(index) {
                return Err(ProtocolError::invalid_response(
                    "Claude SSE stopped the same content block twice",
                ));
            }
            let block = state.blocks.get_mut(&index).ok_or_else(|| {
                ProtocolError::invalid_response("Claude SSE stopped an unknown content block")
            })?;
            finalize_tool_input(block)?;
            let canonical = decode_content(block)?;
            if matches!(
                canonical,
                AiContent::Text { .. } | AiContent::Thinking { .. }
            ) {
                Ok(Vec::new())
            } else {
                Ok(vec![ProtocolEvent::Delta(
                    json!({"index": index, "content": canonical}),
                )])
            }
        }
        "message_delta" => {
            if let Some(reason) = value.pointer("/delta/stop_reason") {
                state.finish_reason = reason.clone();
            }
            if let Some(tokens) = value
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
            {
                state.output_tokens = Some(tokens);
            }
            Ok(Vec::new())
        }
        "message_stop" => {
            if state.blocks.len() != state.stopped_blocks.len() {
                return Err(ProtocolError::invalid_response(
                    "Claude SSE stopped before all content blocks completed",
                ));
            }
            state.saw_message_stop = true;
            let blocks = state
                .blocks
                .values()
                .map(decode_content)
                .collect::<ProtocolResultValue<Vec<_>>>()?;
            let message = AiMessage::new(AiRole::Assistant, blocks);
            message.validate().map_err(|error| {
                ProtocolError::invalid_response(format!("invalid Claude streamed content: {error}"))
            })?;
            let input_tokens = state.input_tokens.ok_or_else(|| {
                ProtocolError::invalid_response("Claude SSE final usage is missing input_tokens")
            })?;
            let output_tokens = state.output_tokens.ok_or_else(|| {
                ProtocolError::invalid_response("Claude SSE final usage is missing output_tokens")
            })?;
            let total_tokens = input_tokens.checked_add(output_tokens).ok_or_else(|| {
                ProtocolError::invalid_response("Claude SSE usage token total overflow")
            })?;
            let tool_calls = message.tool_calls();
            let output = ProtocolOutput {
                value: normalized_value(&message, &tool_calls, state.finish_reason.clone())?,
                usage: Some(AiUsage {
                    input_tokens: Some(input_tokens),
                    output_tokens: Some(output_tokens),
                    total_tokens: Some(total_tokens),
                    request_units: None,
                }),
                artifacts: Vec::new(),
            };
            Ok(vec![ProtocolEvent::Final(output)])
        }
        _ => Ok(vec![ProtocolEvent::Delta(json!({
            "provider_state": {"provider": CLAUDE_PROVIDER_NAMESPACE, "value": value}
        }))]),
    }
}

fn event_index(value: &Value) -> ProtocolResultValue<usize> {
    value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| ProtocolError::invalid_response("Claude SSE content block index is invalid"))
}

fn apply_content_delta(
    value: &Value,
    state: &mut ClaudeStreamState,
) -> ProtocolResultValue<Vec<ProtocolEvent>> {
    let index = event_index(value)?;
    if state.stopped_blocks.contains(&index) {
        return Err(ProtocolError::invalid_response(
            "Claude SSE emitted a delta after content_block_stop",
        ));
    }
    let delta = value
        .get("delta")
        .and_then(Value::as_object)
        .ok_or_else(|| ProtocolError::invalid_response("Claude content block delta is invalid"))?;
    let delta_type = delta.get("type").and_then(Value::as_str).ok_or_else(|| {
        ProtocolError::invalid_response("Claude content block delta is missing type")
    })?;
    let block = state
        .blocks
        .get_mut(&index)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            ProtocolError::invalid_response("Claude content delta refers to an unknown block")
        })?;
    let event = match delta_type {
        "text_delta" => {
            let text = delta_string(delta, "text")?;
            append_string(block, "text", text)?;
            Some(ProtocolEvent::Delta(json!({
                "index": index,
                "content": {"type": "text", "text": text}
            })))
        }
        "thinking_delta" => {
            let thinking = delta_string(delta, "thinking")?;
            append_string(block, "thinking", thinking)?;
            Some(ProtocolEvent::Delta(json!({
                "index": index,
                "content": {
                    "type": "thinking",
                    "summary": null,
                    "text": thinking,
                    "provider_metadata": null
                }
            })))
        }
        "signature_delta" => {
            append_string(block, "signature", delta_string(delta, "signature")?)?;
            None
        }
        "input_json_delta" => {
            append_string(
                block,
                "__partial_json",
                delta_string(delta, "partial_json")?,
            )?;
            None
        }
        _ => {
            return Ok(vec![ProtocolEvent::Delta(json!({
                "provider_state": {"provider": CLAUDE_PROVIDER_NAMESPACE, "value": value}
            }))])
        }
    };
    if delta_type == "input_json_delta" {
        if let Some(partial) = block.get("__partial_json").and_then(Value::as_str) {
            if let Ok(input) = serde_json::from_str::<Value>(partial) {
                block.insert("input".to_string(), input);
            }
        }
    }
    Ok(event.into_iter().collect())
}

fn finalize_tool_input(block: &mut Value) -> ProtocolResultValue<()> {
    let Some(block) = block.as_object_mut() else {
        return Err(ProtocolError::invalid_response(
            "Claude streamed content block must be an object",
        ));
    };
    let Some(partial) = block.remove("__partial_json") else {
        return Ok(());
    };
    let partial = partial.as_str().ok_or_else(|| {
        ProtocolError::invalid_response("Claude tool input delta buffer must be a string")
    })?;
    let input: Value = serde_json::from_str(partial).map_err(|error| {
        ProtocolError::invalid_response(format!(
            "Claude streamed tool input is not valid JSON: {error}"
        ))
    })?;
    if !input.is_object() {
        return Err(ProtocolError::invalid_response(
            "Claude streamed tool input must be an object",
        ));
    }
    block.insert("input".to_string(), input);
    Ok(())
}

fn delta_string<'a>(delta: &'a Map<String, Value>, source: &str) -> ProtocolResultValue<&'a str> {
    delta.get(source).and_then(Value::as_str).ok_or_else(|| {
        ProtocolError::invalid_response(format!("Claude {source} delta must be a string"))
    })
}

fn append_string(
    block: &mut Map<String, Value>,
    target: &str,
    addition: &str,
) -> ProtocolResultValue<()> {
    let current = block
        .entry(target.to_string())
        .or_insert_with(|| Value::String(String::new()));
    let current = current.as_str().ok_or_else(|| {
        ProtocolError::invalid_response(format!(
            "Claude content block field `{target}` must be a string"
        ))
    })?;
    let mut combined = String::with_capacity(current.len() + addition.len());
    combined.push_str(current);
    combined.push_str(addition);
    block.insert(target.to_string(), Value::String(combined));
    Ok(())
}

fn stream_error(value: &Value) -> ProtocolError {
    let kind = value
        .pointer("/error/type")
        .and_then(Value::as_str)
        .unwrap_or("stream_error");
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Claude stream failed");
    ProtocolError::new(
        ProtocolErrorKind::Transport,
        format!("Claude {kind}: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, GoldenBody, ProtocolContractHarness,
        ResolvedCredential,
    };
    use buckyos_api::{AiToolSpec, LlmChatInvokeRequest};
    use bytes::Bytes;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn codec() -> ClaudeMessagesCodec {
        ClaudeMessagesCodec::new()
    }

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://api.anthropic.com/v1".to_string(),
            credential: Some(
                ResolvedCredential::named_header("ref:claude", "x-api-key", "secret-key").unwrap(),
            ),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: MAX_REQUEST_BYTES,
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        }
    }

    fn input(request: LlmChatInvokeRequest, extra: &[(&str, Value)]) -> CodecInput {
        let mut resolved_parameters = BTreeMap::from([(
            "provider_model_id".to_string(),
            Value::String("claude-test".to_string()),
        )]);
        for (name, value) in extra {
            resolved_parameters.insert((*name).to_string(), value.clone());
        }
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(request),
            resolved_parameters,
        }
    }

    #[test]
    fn registers_a_single_reusable_messages_operation() {
        let codec = codec();
        let descriptor = codec.adapter_descriptor();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.protocol_family_id, "claude");
        assert_eq!(descriptor.protocol_adapter_id, "claude-messages");
        assert_eq!(descriptor.operations.len(), 1);
        assert!(descriptor.operations[CLAUDE_MESSAGES_OPERATION_ID]
            .binding(ApiType::Llm)
            .unwrap()
            .execution_modes
            .contains(&ExecutionMode::Stream));

        let mut registry = CodecRegistry::default();
        registry
            .register(descriptor, vec![Arc::new(codec)])
            .unwrap();
        assert!(registry
            .operation_descriptor(
                CLAUDE_MESSAGES_ADAPTER_ID,
                CLAUDE_MESSAGES_OPERATION_ID,
                ApiType::Llm,
            )
            .is_ok());
    }

    #[test]
    fn encodes_messages_tools_thinking_usage_options_and_version_header() {
        let mut request = LlmChatInvokeRequest::new(
            "ignored@instance",
            vec![
                AiMessage::text(AiRole::System, "system"),
                AiMessage::text(AiRole::Developer, "developer"),
                AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::text("hello"),
                        AiContent::Image {
                            source: ResourceRef::base64(
                                "image/png".to_string(),
                                "aW1hZ2U=".to_string(),
                            ),
                        },
                        AiContent::Document {
                            source: ResourceRef::url(
                                "https://example.invalid/doc.pdf".to_string(),
                                Some("application/pdf".to_string()),
                            ),
                            title: Some("spec".to_string()),
                        },
                    ],
                ),
                AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::Thinking {
                            summary: None,
                            text: Some("reason".to_string()),
                            provider_metadata: Some(json!({"signature": "sig"})),
                        },
                        AiContent::ToolUse {
                            call_id: "tool-1".to_string(),
                            name: "weather".to_string(),
                            args: HashMap::from([("city".to_string(), json!("Paris"))]),
                        },
                        AiContent::ProviderState {
                            provider: "openai".to_string(),
                            value: json!({"type": "foreign_state"}),
                        },
                    ],
                ),
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::ToolResult {
                        call_id: "tool-1".to_string(),
                        content: vec![AiToolResultContent::text("sunny")],
                        is_error: false,
                    }],
                ),
            ],
        );
        request.max_output_tokens = Some(512);
        request.temperature = Some(0.25);
        request.top_p = Some(0.9);
        request.stop = vec!["STOP".to_string()];
        request.tools = vec![AiToolSpec {
            tool_type: "function".to_string(),
            name: "weather".to_string(),
            description: "Weather lookup".to_string(),
            args_json_schema: json!({"type": "object"}),
            output_schema: None,
        }];
        let input = input(
            request,
            &[
                ("stream", json!(true)),
                ("thinking", json!({"type": "enabled", "budget_tokens": 256})),
                ("tool_choice", json!({"type": "auto"})),
            ],
        );
        let context = context();
        let wire = codec()
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input,
                context: &context,
            })
            .unwrap();
        let golden = ProtocolContractHarness::default()
            .redact_header(reqwest::header::HeaderName::from_static("x-api-key"))
            .request(&wire)
            .unwrap();
        assert_eq!(golden.method, "POST");
        assert_eq!(golden.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(golden.headers["anthropic-version"], CLAUDE_MESSAGES_VERSION);
        assert_eq!(golden.headers["x-api-key"], "[REDACTED]");
        let GoldenBody::Json(body) = golden.body else {
            panic!("expected JSON")
        };
        assert_eq!(body["model"], "claude-test");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["system"][0]["text"], "system");
        assert_eq!(body["system"][1]["text"], "developer");
        assert_eq!(body["messages"][2]["role"], "user");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][1]["content"][0]["signature"], "sig");
        assert_eq!(
            body["messages"][0]["content"][1]["source"]["type"],
            "base64"
        );
        assert_eq!(body["messages"][0]["content"][2]["type"], "document");
        assert_eq!(body["messages"][1]["content"].as_array().unwrap().len(), 2);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["thinking"]["budget_tokens"], 256);
        assert_eq!(body["stream"], true);
    }

    #[tokio::test]
    async fn decodes_message_blocks_tools_thinking_provider_state_and_usage() {
        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[("content-type", "application/json")],
                Bytes::from_static(
                    br#"{
                "id":"msg_1","type":"message","role":"assistant","model":"claude-test",
                "content":[
                    {"type":"thinking","thinking":"reason","signature":"sig"},
                    {"type":"text","text":"answer"},
                    {"type":"tool_use","id":"tool-1","name":"weather","input":{"city":"Paris"}},
                    {"type":"redacted_thinking","data":"opaque"}
                ],
                "stop_reason":"tool_use","stop_sequence":null,
                "usage":{"input_tokens":4,"output_tokens":3}
            }"#,
                ),
                "request-1",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = codec().decode(response).await.unwrap() else {
            panic!("expected immediate output")
        };
        assert_eq!(output.usage.unwrap().total_tokens, Some(7));
        assert_eq!(output.value["finish_reason"], "tool_use");
        assert_eq!(output.value["tool_calls"][0]["call_id"], "tool-1");
        assert_eq!(
            output.value["message"]["content"][0]["provider_metadata"]["signature"],
            "sig"
        );
        assert_eq!(output.value["message"]["content"][3]["provider"], "claude");
    }

    #[tokio::test]
    async fn decodes_sse_text_tool_thinking_and_cumulative_usage() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"type\":\"message\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"why\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool-1\",\"name\":\"weather\",\"input\":{}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Paris\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"text_delta\",\"text\":\"answer\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":9}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        let split = body.len() / 2;
        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::iter(vec![
                Ok(Bytes::copy_from_slice(&body.as_bytes()[..split])),
                Ok(Bytes::copy_from_slice(&body.as_bytes()[split..])),
            ])),
            request_id: "request-stream".to_string(),
            retry_after: None,
        };
        let codec = codec();
        let descriptor = codec.adapter_descriptor();
        let mut registry = CodecRegistry::default();
        registry
            .register(descriptor, vec![Arc::new(codec)])
            .unwrap();
        let mut output = registry
            .decode_stream(
                CLAUDE_MESSAGES_ADAPTER_ID,
                CLAUDE_MESSAGES_OPERATION_ID,
                ApiType::Llm,
                response,
            )
            .await
            .unwrap();
        let events = output.events.by_ref().collect::<Vec<_>>().await;
        assert!(events.iter().all(Result::is_ok));
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(ProtocolEvent::Delta(value))
                if value.pointer("/content/type").and_then(Value::as_str) == Some("text")
                    && value.pointer("/content/text").and_then(Value::as_str) == Some("answer")
        )));
        let ProtocolEvent::Final(final_output) = events.last().unwrap().as_ref().unwrap() else {
            panic!("expected final event")
        };
        assert_eq!(final_output.usage.as_ref().unwrap().total_tokens, Some(14));
        assert_eq!(final_output.value["tool_calls"][0]["args"]["city"], "Paris");
        assert_eq!(
            final_output.value["message"]["content"][0]["provider_metadata"]["signature"],
            "sig"
        );
    }

    #[tokio::test]
    async fn maps_provider_errors_and_rejects_missing_usage() {
        let error = ProtocolContractHarness::default().response(
            StatusCode::TOO_MANY_REQUESTS,
            &[("retry-after", "2")],
            Bytes::from_static(br#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#),
            "request-rate",
            UNIX_EPOCH,
        ).unwrap();
        let error = codec().decode(error).await.unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert_eq!(error.request_id.as_deref(), Some("request-rate"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(2)));

        let missing_usage = ProtocolContractHarness::default().response(
            StatusCode::OK,
            &[],
            Bytes::from_static(br#"{"type":"message","role":"assistant","content":[],"stop_reason":"end_turn"}"#),
            "request-invalid",
            UNIX_EPOCH,
        ).unwrap();
        assert_eq!(
            codec().decode(missing_usage).await.unwrap_err().kind,
            ProtocolErrorKind::InvalidResponse
        );
    }

    #[tokio::test]
    async fn streaming_maps_provider_error_and_disconnect_with_metadata() {
        let provider_error = StreamingHttpResponse {
            status: StatusCode::TOO_MANY_REQUESTS,
            headers: HeaderMap::new(),
            body: Box::pin(stream::iter(vec![
                Ok(Bytes::from_static(br#"{"type":"error","error":{"#)),
                Ok(Bytes::from_static(
                    br#""type":"rate_limit_error","message":"slow down"}}"#,
                )),
            ])),
            request_id: "request-rate-stream".to_string(),
            retry_after: Some(Duration::from_secs(3)),
        };
        let error = codec().decode_stream(provider_error).await.unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert!(error.message.contains("rate_limit_error"));
        assert_eq!(error.request_id.as_deref(), Some("request-rate-stream"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(3)));

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let disconnected = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::iter(vec![
                Ok(Bytes::from_static(
                    b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"type\":\"message\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
                )),
                Err(ProtocolError::new(
                    ProtocolErrorKind::Transport,
                    "connection lost",
                )),
            ])),
            request_id: "request-disconnect".to_string(),
            retry_after: Some(Duration::from_secs(1)),
        };
        let mut output = codec().decode_stream(disconnected).await.unwrap();
        let error = output.events.next().await.unwrap().unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert_eq!(error.request_id.as_deref(), Some("request-disconnect"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(1)));
        assert!(output.events.next().await.is_none());
    }

    #[tokio::test]
    async fn streaming_maps_in_band_error_event() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::once(async {
                Ok(Bytes::from_static(
                    b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"busy\"}}\n\n",
                ))
            })),
            request_id: "request-overloaded".to_string(),
            retry_after: Some(Duration::from_secs(2)),
        };
        let mut output = codec().decode_stream(response).await.unwrap();
        let error = output.events.next().await.unwrap().unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert!(error.message.contains("overloaded_error"));
        assert_eq!(error.request_id.as_deref(), Some("request-overloaded"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
        assert!(output.events.next().await.is_none());
    }

    #[test]
    fn rejects_unmapped_hard_constraints_before_http() {
        let mut request = LlmChatInvokeRequest::new(
            "ignored@instance",
            vec![AiMessage::text(AiRole::User, "hello")],
        );
        request.max_output_tokens = Some(16);
        request.seed = Some(7);
        let input = input(request, &[]);
        let context = context();
        let error = codec()
            .encode(&CodecCall {
                api_type: ApiType::Llm,
                input: &input,
                context: &context,
            })
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::UnsupportedOperation);
    }
}
