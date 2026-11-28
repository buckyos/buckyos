use super::gemini::{
    api_endpoint, binding, decode_resource_with_default, ensure_success, finish_request,
    finite_number, gemini_http_error, is_sse, operation, provider_model_id, resource_part,
    set_default_priority, GeminiEmbeddingCodec, GeminiVideoCodec,
    GEMINI_EMBED_CONTENT_OPERATION_ID, GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
    GEMINI_PROVIDER_NAMESPACE, PREDICT_LONG_RUNNING_PRIORITY,
};
use super::openai_chat_completions::{
    normalize_vision_output, vision_caption_as_chat, vision_ocr_as_chat,
};
use super::{
    foreign_provider_state_text, provider_state_is_native, sse_frame_stream, AdapterDescriptor,
    AdapterStatus, CodecCall, CodecContext, CodecRegistration, ExecutionMode, HttpBody,
    HttpRequest, HttpResponse, NativeTaskCodec, OperationCodec, OperationDescriptor, ProtocolError,
    ProtocolErrorKind, ProtocolEvent, ProtocolExecution, ProtocolOutput, ProtocolResultValue,
    ProtocolStream, SseConfig, SseFrame, StreamingHttpResponse,
};
use async_trait::async_trait;
use buckyos_api::{
    features, AiContent, AiMessage, AiRole, AiToolResultContent, AiUsage, AiccCall,
    AiccExecutionMode, ApiType, LlmChatInvokeRequest, LlmResponseFormat, LlmResponseFormatType,
    ProviderStateCoordinate,
};
use futures_util::{stream, StreamExt};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

pub(crate) const GEMINI_GENERATE_CONTENT_ADAPTER_ID: &str = "gemini-generate-content";
pub(crate) const GEMINI_GENERATE_CONTENT_OPERATION_ID: &str = "models.generateContent";

const SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

pub(crate) fn gemini_generate_content_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let generate = operation(
        GEMINI_GENERATE_CONTENT_OPERATION_ID,
        vec![
            binding(
                ApiType::Llm,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
                [
                    features::TOOL_CALL,
                    features::JSON_SCHEMA,
                    features::VISION,
                    features::DOCUMENT,
                    features::WEB_SEARCH,
                ],
            ),
            binding(ApiType::VisionOcr, [ExecutionMode::Immediate], []),
            binding(ApiType::VisionCaption, [ExecutionMode::Immediate], []),
        ],
        false,
    );
    let embeddings = operation(
        GEMINI_EMBED_CONTENT_OPERATION_ID,
        vec![
            binding(ApiType::EmbeddingText, [ExecutionMode::Immediate], []),
            binding(ApiType::EmbeddingMultimodal, [ExecutionMode::Immediate], []),
        ],
        false,
    );
    let mut video = operation(
        GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
        vec![
            binding(ApiType::VideoGenerate, [ExecutionMode::NativeTask], []),
            binding(ApiType::VideoExtend, [ExecutionMode::NativeTask], []),
        ],
        false,
    );
    set_default_priority(&mut video, PREDICT_LONG_RUNNING_PRIORITY);
    let descriptor = AdapterDescriptor {
        protocol_family_id: "gemini".to_string(),
        protocol_adapter_id: GEMINI_GENERATE_CONTENT_ADAPTER_ID.to_string(),
        interface_generation: "generate-content-v1beta".to_string(),
        base_adapter_id: None,
        status: AdapterStatus::Preview,
        probe_priority: 0,
        probe_path: None,
        credential: super::AdapterCredentialContract::named_header("x-goog-api-key"),
        operations: BTreeMap::from([
            (generate.operation_id.clone(), generate.clone()),
            (embeddings.operation_id.clone(), embeddings.clone()),
            (video.operation_id.clone(), video.clone()),
        ]),
    };
    let mut operation_codecs: Vec<Arc<dyn OperationCodec>> =
        [ApiType::Llm, ApiType::VisionOcr, ApiType::VisionCaption]
            .into_iter()
            .map(|api_type| {
                Arc::new(GeminiGenerateContentCodec {
                    descriptor: generate.clone(),
                    api_type,
                }) as Arc<dyn OperationCodec>
            })
            .collect();
    operation_codecs.extend(
        [ApiType::EmbeddingText, ApiType::EmbeddingMultimodal]
            .into_iter()
            .map(|api_type| {
                Arc::new(GeminiEmbeddingCodec::new(embeddings.clone(), api_type))
                    as Arc<dyn OperationCodec>
            }),
    );
    let native_task_codecs = [ApiType::VideoGenerate, ApiType::VideoExtend]
        .into_iter()
        .map(|api_type| {
            Arc::new(GeminiVideoCodec::new(video.clone(), api_type)) as Arc<dyn NativeTaskCodec>
        })
        .collect();
    (
        descriptor,
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

#[derive(Clone)]
struct GeminiGenerateContentCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl OperationCodec for GeminiGenerateContentCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        crate::resource::ResourceInputForm::UrlOrBytes
    }

    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        self.descriptor
            .binding(self.api_type)
            .expect("Gemini generateContent codec binding")
            .execution_modes
            .clone()
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        call.context.validate()?;
        call.input
            .validate_for(self.descriptor.binding(call.api_type)?)?;
        match (&call.input.canonical_request, self.api_type) {
            (AiccCall::ChatCompletionsCreate(request), ApiType::Llm) => {
                encode_generate_content(request, call)
            }
            (AiccCall::VisionOcr(request), ApiType::VisionOcr) => {
                encode_generate_content(&vision_ocr_as_chat(request), call)
            }
            (AiccCall::VisionCaption(request), ApiType::VisionCaption) => {
                encode_generate_content(&vision_caption_as_chat(request), call)
            }
            _ => Err(ProtocolError::invalid_request(
                "Gemini generateContent codec received the wrong canonical request",
            )),
        }
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let request_id = response.request_id.clone();
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let mut output =
            normalize_response(&value).map_err(|error| error.with_request_id(Some(request_id)))?;
        normalize_vision_output(&mut output, self.api_type);
        Ok(ProtocolExecution::Immediate(output))
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        decode_incremental_stream(response, self.descriptor.max_response_bytes).await
    }
}

fn encode_generate_content(
    request: &LlmChatInvokeRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<HttpRequest> {
    if request.output.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Gemini generateContent does not support output media options",
        ));
    }
    let model = provider_model_id(&call.input.resolved_parameters)?;
    let (system, contents) = encode_contents(&request.messages, call.context)?;
    if contents.is_empty() {
        return Err(ProtocolError::invalid_request(
            "Gemini generateContent requires at least one conversation message",
        ));
    }
    let mut body = Map::from_iter([("contents".to_string(), Value::Array(contents))]);
    if !system.is_empty() {
        body.insert("systemInstruction".to_string(), json!({"parts": system}));
    }
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            json!([{"functionDeclarations": encode_function_declarations(request)?}]),
        );
    }
    let mut generation = Map::new();
    if let Some(tokens) = request.max_output_tokens {
        generation.insert("maxOutputTokens".to_string(), tokens.into());
    }
    if let Some(temperature) = request.temperature {
        generation.insert(
            "temperature".to_string(),
            finite_number("temperature", temperature)?,
        );
    }
    if let Some(top_p) = request.top_p {
        generation.insert("topP".to_string(), finite_number("top_p", top_p)?);
    }
    if let Some(seed) = request.seed {
        generation.insert("seed".to_string(), seed.into());
    }
    if !request.stop.is_empty() {
        generation.insert("stopSequences".to_string(), json!(request.stop));
    }
    if let Some(format) = &request.response_format {
        apply_response_format(&mut generation, format)?;
    }
    apply_resolved_parameters(&mut body, &mut generation, &call.input.resolved_parameters)?;
    if !generation.is_empty() {
        body.insert("generationConfig".to_string(), Value::Object(generation));
    }

    let stream = request.execution_mode == AiccExecutionMode::Stream;
    let method = if stream {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    let mut url = Url::parse(&api_endpoint(
        &call.context.base_url,
        &format!("models/{model}:{method}"),
    )?)
    .map_err(|_| ProtocolError::invalid_configuration("Gemini endpoint URL is invalid"))?;
    if stream {
        url.query_pairs_mut().append_pair("alt", "sse");
    }
    let mut wire = HttpRequest::new(Method::POST, url.to_string());
    wire.body = HttpBody::Json(Value::Object(body));
    finish_request(&mut wire, call.context)
}

type PendingCalls = BTreeMap<String, (String, Option<String>)>;

fn encode_contents(
    messages: &[AiMessage],
    context: &CodecContext,
) -> ProtocolResultValue<(Vec<Value>, Vec<Value>)> {
    let mut system = Vec::new();
    let mut contents = Vec::new();
    let mut pending = PendingCalls::new();
    for group in messages.chunk_by(|a, b| a.role == AiRole::Tool && b.role == AiRole::Tool) {
        for message in group {
            message.validate().map_err(|error| {
                ProtocolError::invalid_request(format!("invalid canonical message: {error}"))
            })?;
        }
        let message = &group[0];
        let (role, parts) = match message.role {
            AiRole::System | AiRole::Developer => {
                for block in &message.content {
                    let AiContent::Text { text } = block else {
                        return Err(ProtocolError::invalid_request(
                            "Gemini system instructions only accept text content",
                        ));
                    };
                    system.push(json!({"text": text}));
                }
                continue;
            }
            AiRole::User => ("user", encode_user_parts(&message.content, context)?),
            AiRole::Assistant => ("model", encode_model_parts(message, context, &mut pending)?),
            AiRole::Tool => (
                "user",
                group
                    .iter()
                    .map(|message| encode_function_response(message, context, &mut pending))
                    .collect::<ProtocolResultValue<Vec<_>>>()?,
            ),
        };
        if parts.is_empty() {
            return Err(ProtocolError::invalid_request(
                "Gemini message has no representable parts",
            ));
        }
        contents.push(json!({"role": role, "parts": parts}));
    }
    Ok((system, contents))
}

fn encode_user_parts(
    content: &[AiContent],
    context: &CodecContext,
) -> ProtocolResultValue<Vec<Value>> {
    content
        .iter()
        .map(|block| match block {
            AiContent::Text { text } => Ok(json!({"text": text})),
            AiContent::Image { source } | AiContent::Document { source, .. } => {
                resource_part(source, context)
            }
            _ => Err(ProtocolError::invalid_request(
                "Gemini user content only accepts text, images and documents",
            )),
        })
        .collect()
}

fn encode_model_parts(
    message: &AiMessage,
    context: &CodecContext,
    pending: &mut PendingCalls,
) -> ProtocolResultValue<Vec<Value>> {
    let tool_uses = message
        .content
        .iter()
        .filter_map(|block| match block {
            AiContent::ToolUse { call_id, name, .. } => Some((call_id, name)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let native_parts = message
        .content
        .iter()
        .filter_map(|block| match block {
            AiContent::ProviderState { source, value, .. }
                if provider_state_is_native(source, &context.state_coordinate) =>
            {
                value.get("parts").and_then(Value::as_array)
            }
            _ => None,
        })
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    if !native_parts.is_empty() {
        let wire_ids = native_parts
            .iter()
            .filter_map(|part| part.get("functionCall"))
            .map(|call| call.get("id").and_then(Value::as_str).map(str::to_string))
            .chain(std::iter::repeat(None));
        for ((call_id, name), wire_id) in tool_uses.into_iter().zip(wire_ids) {
            remember_call(pending, call_id, name, wire_id)?;
        }
        return Ok(native_parts);
    }

    let mut parts = Vec::new();
    let mut signed = false;
    for block in &message.content {
        match block {
            AiContent::Text { text } => parts.push(json!({"text": text})),
            AiContent::Image { source } | AiContent::Document { source, .. } => {
                parts.push(resource_part(source, context)?);
            }
            AiContent::ToolUse {
                call_id,
                name,
                args,
            } => {
                remember_call(pending, call_id, name, Some(call_id.clone()))?;
                let mut part = json!({"functionCall": {"id": call_id, "name": name, "args": args}});
                if !signed {
                    part["thoughtSignature"] = json!(SKIP_THOUGHT_SIGNATURE);
                    signed = true;
                }
                parts.push(part);
            }
            AiContent::Thinking { .. } => {}
            AiContent::ProviderState { source, .. }
                if provider_state_is_native(source, &context.state_coordinate) => {}
            AiContent::ProviderState {
                provider, value, ..
            } => {
                if let Some(text) = foreign_provider_state_text(provider, value) {
                    parts.push(json!({"text": text}));
                }
            }
            AiContent::ToolResult { .. } => {
                return Err(ProtocolError::invalid_request(
                    "Gemini tool results must use the canonical tool role",
                ));
            }
        }
    }
    Ok(parts)
}

fn remember_call(
    pending: &mut PendingCalls,
    call_id: &str,
    name: &str,
    wire_id: Option<String>,
) -> ProtocolResultValue<()> {
    if pending
        .insert(call_id.to_string(), (name.to_string(), wire_id))
        .is_some()
    {
        return Err(ProtocolError::invalid_request(
            "Gemini function call IDs must be unique",
        ));
    }
    Ok(())
}

fn encode_function_response(
    message: &AiMessage,
    context: &CodecContext,
    pending: &mut PendingCalls,
) -> ProtocolResultValue<Value> {
    let [AiContent::ToolResult {
        call_id,
        content,
        is_error,
    }] = message.content.as_slice()
    else {
        return Err(ProtocolError::invalid_request(
            "Gemini tool message must contain one tool result",
        ));
    };
    let (name, wire_id) = pending.remove(call_id).ok_or_else(|| {
        ProtocolError::invalid_request("Gemini function response has no matching function call")
    })?;
    let mut text = Vec::new();
    let mut parts = Vec::new();
    for item in content {
        match item {
            AiToolResultContent::Text { text: value } => text.push(value.as_str()),
            AiToolResultContent::Image { source }
            | AiToolResultContent::Document { source, .. } => {
                parts.push(resource_part(source, context)?)
            }
        }
    }
    let key = if *is_error { "error" } else { "output" };
    let mut response = json!({"name": name, "response": {key: text.join("\n")}});
    if let Some(id) = wire_id {
        response["id"] = json!(id);
    }
    if !parts.is_empty() {
        response["parts"] = Value::Array(parts);
    }
    Ok(json!({"functionResponse": response}))
}

fn encode_function_declarations(request: &LlmChatInvokeRequest) -> ProtocolResultValue<Vec<Value>> {
    request
        .tools
        .iter()
        .map(|tool| {
            if tool.tool_type != "function"
                || tool.name.trim().is_empty()
                || !tool.args_json_schema.is_object()
            {
                return Err(ProtocolError::invalid_request(
                    "Gemini only maps named function tools with an object schema",
                ));
            }
            let mut declaration = json!({
                "name": tool.name,
                "description": tool.description,
                "parametersJsonSchema": tool.args_json_schema
            });
            if let Some(schema) = &tool.output_schema {
                declaration["responseJsonSchema"] = schema.clone();
            }
            Ok(declaration)
        })
        .collect()
}

fn apply_response_format(
    generation: &mut Map<String, Value>,
    format: &LlmResponseFormat,
) -> ProtocolResultValue<()> {
    if format.format_type == LlmResponseFormatType::Text {
        return Ok(());
    }
    let schema = format.json_schema.as_ref().ok_or_else(|| {
        ProtocolError::invalid_request("Gemini json_schema response format requires a schema")
    })?;
    if !schema.schema.is_object() {
        return Err(ProtocolError::invalid_request(
            "Gemini response json_schema must be an object",
        ));
    }
    generation.insert("responseMimeType".to_string(), json!("application/json"));
    generation.insert("responseJsonSchema".to_string(), schema.schema.clone());
    Ok(())
}

fn apply_resolved_parameters(
    body: &mut Map<String, Value>,
    generation: &mut Map<String, Value>,
    parameters: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    for (name, value) in parameters {
        match name.as_str() {
            "provider_model_id" | "stream" => {}
            features::WEB_SEARCH => {
                let enabled = value.as_bool().ok_or_else(|| {
                    ProtocolError::invalid_request("resolved web_search must be a boolean")
                })?;
                if enabled {
                    body.entry("tools")
                        .or_insert_with(|| json!([]))
                        .as_array_mut()
                        .ok_or_else(|| ProtocolError::invalid_request("tools must be an array"))?
                        .push(json!({"googleSearch": {}}));
                }
            }
            "generationConfig" => {
                let options = value.as_object().ok_or_else(|| {
                    ProtocolError::invalid_request(
                        "resolved Gemini generationConfig must be an object",
                    )
                })?;
                for (key, option) in options {
                    generation
                        .entry(key.clone())
                        .or_insert_with(|| option.clone());
                }
            }
            "cachedContent" | "labels" | "safetySettings" | "serviceTier" | "store"
            | "toolConfig" => {
                body.insert(name.clone(), value.clone());
            }
            _ => {
                return Err(ProtocolError::invalid_request(format!(
                    "resolved Gemini parameter `{name}` is not supported"
                )));
            }
        }
    }
    Ok(())
}

fn normalize_response(value: &Value) -> ProtocolResultValue<ProtocolOutput> {
    if !value.is_object() {
        return Err(ProtocolError::invalid_response(
            "Gemini generateContent response must be an object",
        ));
    }
    let usage = decode_usage(value.get("usageMetadata"))?;
    let Some(candidate) = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
    else {
        let feedback = value
            .get("promptFeedback")
            .ok_or_else(|| ProtocolError::invalid_response("Gemini response has no candidates"))?;
        return normalized_output(
            Vec::new(),
            feedback.get("blockReason").cloned().unwrap_or(Value::Null),
            usage,
        );
    };
    let parts = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let call_prefix = value
        .get("responseId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .unwrap_or("gemini-call");
    let mut content = Vec::new();
    let mut calls = 0;
    for part in parts {
        decode_part(part, call_prefix, &mut content, &mut calls)?;
    }
    if !parts.is_empty() {
        content.push(AiContent::ProviderState {
            source: ProviderStateCoordinate::unbound(),
            provider: GEMINI_PROVIDER_NAMESPACE.to_string(),
            value: json!({"role": "model", "parts": parts}),
        });
    }
    normalized_output(
        content,
        candidate
            .get("finishReason")
            .cloned()
            .unwrap_or(Value::Null),
        usage,
    )
}

fn decode_part(
    part: &Value,
    call_prefix: &str,
    content: &mut Vec<AiContent>,
    calls: &mut usize,
) -> ProtocolResultValue<()> {
    if let Some(call) = part.get("functionCall") {
        let name = call
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                ProtocolError::invalid_response("Gemini functionCall is missing name")
            })?;
        let args = match call.get("args") {
            None | Some(Value::Null) => Default::default(),
            Some(Value::Object(args)) => args.clone().into_iter().collect(),
            Some(_) => {
                return Err(ProtocolError::invalid_response(
                    "Gemini functionCall args must be an object",
                ));
            }
        };
        let call_id = call
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{call_prefix}-{calls}"));
        *calls += 1;
        content.push(AiContent::ToolUse {
            call_id,
            name: name.to_string(),
            args,
        });
    } else if let Some(text) = part.get("text").and_then(Value::as_str) {
        if text.is_empty() {
            return Ok(());
        }
        content.push(
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                AiContent::Thinking {
                    source: ProviderStateCoordinate::unbound(),
                    summary: Some(text.to_string()),
                    text: None,
                    provider_metadata: None,
                }
            } else {
                AiContent::Text {
                    text: text.to_string(),
                }
            },
        );
    } else if let Some(data) = part.get("inlineData") {
        if data
            .get("mimeType")
            .and_then(Value::as_str)
            .is_some_and(|mime| mime.starts_with("image/"))
        {
            content.push(AiContent::Image {
                source: decode_resource_with_default(data, "application/octet-stream")?,
            });
        }
    }
    Ok(())
}

fn normalized_output(
    content: Vec<AiContent>,
    finish_reason: Value,
    usage: Option<AiUsage>,
) -> ProtocolResultValue<ProtocolOutput> {
    let message = AiMessage::new(AiRole::Assistant, content);
    message.validate().map_err(|error| {
        ProtocolError::invalid_response(format!("invalid Gemini output: {error}"))
    })?;
    let tool_calls = message.tool_calls();
    Ok(ProtocolOutput {
        value: json!({"message": message, "tool_calls": tool_calls, "finish_reason": finish_reason}),
        usage,
        artifacts: Vec::new(),
    })
}

fn decode_usage(value: Option<&Value>) -> ProtocolResultValue<Option<AiUsage>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let usage = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("Gemini usageMetadata must be an object"))?;
    let count = |field: &str| usage.get(field).and_then(Value::as_u64);
    let overflow = || ProtocolError::invalid_response("Gemini usage overflow");
    let input = count("promptTokenCount")
        .map(|prompt| prompt.checked_add(count("toolUsePromptTokenCount").unwrap_or(0)))
        .map(|input| input.ok_or_else(overflow))
        .transpose()?;
    let thoughts = count("thoughtsTokenCount");
    let output = match (count("candidatesTokenCount"), thoughts) {
        (None, None) => None,
        (candidates, thoughts) => Some(
            candidates
                .unwrap_or(0)
                .checked_add(thoughts.unwrap_or(0))
                .ok_or_else(overflow)?,
        ),
    };
    let total = count("totalTokenCount").or_else(|| {
        input
            .zip(output)
            .and_then(|(input, output)| input.checked_add(output))
    });
    let modality = |field: &str, modality: &str| {
        usage
            .get(field)?
            .as_array()?
            .iter()
            .filter(|entry| entry.get("modality").and_then(Value::as_str) == Some(modality))
            .try_fold(0u64, |total, entry| {
                total.checked_add(entry.get("tokenCount")?.as_u64()?)
            })
    };
    Ok(Some(AiUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: total,
        cache_read_input_tokens: count("cachedContentTokenCount"),
        reasoning_tokens: thoughts,
        audio_input_tokens: modality("promptTokensDetails", "AUDIO"),
        image_input_tokens: modality("promptTokensDetails", "IMAGE"),
        audio_output_tokens: modality("candidatesTokensDetails", "AUDIO"),
        image_output_tokens: modality("candidatesTokensDetails", "IMAGE"),
        request_units: None,
        ..AiUsage::default()
    }))
}

#[derive(Default)]
struct StreamState {
    parts: Vec<Value>,
    finish_reason: Option<Value>,
    prompt_feedback: Option<Value>,
    usage: Option<Value>,
    response_id: Option<Value>,
}

impl StreamState {
    fn push(&mut self, data: &str) -> ProtocolResultValue<Vec<ProtocolEvent>> {
        let value: Value = serde_json::from_str(data).map_err(|error| {
            ProtocolError::invalid_response(format!("Gemini SSE data is not valid JSON: {error}"))
        })?;
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Gemini stream failed");
            let status = error
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("stream_error");
            return Err(ProtocolError::new(
                ProtocolErrorKind::Transport,
                format!("Gemini {status}: {message}"),
            ));
        }
        if let Some(usage) = value.get("usageMetadata") {
            self.usage = Some(usage.clone());
        }
        if let Some(id) = value.get("responseId") {
            self.response_id = Some(id.clone());
        }
        if let Some(feedback) = value.get("promptFeedback") {
            self.prompt_feedback = Some(feedback.clone());
        }
        let Some(candidate) = value
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
        else {
            return Ok(Vec::new());
        };
        if let Some(reason) = candidate.get("finishReason") {
            self.finish_reason = Some(reason.clone());
        }
        let mut events = Vec::new();
        for part in candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = part
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                let kind = if part.get("thought").and_then(Value::as_bool) == Some(true) {
                    "thinking"
                } else {
                    "text"
                };
                events.push(ProtocolEvent::Delta(json!({"type": kind, "text": text})));
            } else if let Some(call) = part.get("functionCall") {
                events.push(ProtocolEvent::Delta(
                    json!({"type": "function_call", "function_call": call}),
                ));
            }
            self.append(part);
        }
        Ok(events)
    }

    fn append(&mut self, part: &Value) {
        let plain_text = |part: &Value| {
            part.as_object().is_some_and(|object| {
                object.get("text").is_some_and(Value::is_string)
                    && object.keys().all(|key| key == "text" || key == "thought")
            })
        };
        if let Some(last) = self.parts.last_mut() {
            if plain_text(last) && plain_text(part) && last.get("thought") == part.get("thought") {
                if let (Some(Value::String(text)), Some(addition)) = (
                    last.get_mut("text"),
                    part.get("text").and_then(Value::as_str),
                ) {
                    text.push_str(addition);
                    return;
                }
            }
        }
        self.parts.push(part.clone());
    }

    fn finish(&mut self) -> ProtocolResultValue<ProtocolOutput> {
        let mut response = Map::new();
        if let Some(usage) = self.usage.take() {
            response.insert("usageMetadata".to_string(), usage);
        }
        if let Some(id) = self.response_id.take() {
            response.insert("responseId".to_string(), id);
        }
        match (self.finish_reason.take(), self.prompt_feedback.take()) {
            (Some(reason), _) => {
                response.insert(
                    "candidates".to_string(),
                    json!([{
                        "content": {"role": "model", "parts": std::mem::take(&mut self.parts)},
                        "finishReason": reason
                    }]),
                );
            }
            (None, Some(feedback)) => {
                response.insert("promptFeedback".to_string(), feedback);
            }
            (None, None) => {
                return Err(ProtocolError::invalid_response(
                    "Gemini stream ended before a finishReason",
                ));
            }
        }
        normalize_response(&Value::Object(response))
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
        return Err(gemini_http_error(
            response.status,
            &response.body,
            &response.request_id,
            response.retry_after,
        ));
    }
    if !is_sse(response.headers.get(CONTENT_TYPE)) {
        return Err(ProtocolError::invalid_response(
            "Gemini streamGenerateContent must use text/event-stream",
        )
        .with_request_id(Some(response.request_id)));
    }
    let request_id = response.request_id.clone();
    let frames = sse_frame_stream(response, SseConfig::default(), max_response_bytes).await?;

    struct State {
        frames: super::SseFrameStream,
        gemini: StreamState,
        queued: VecDeque<ProtocolResultValue<ProtocolEvent>>,
        request_id: String,
        finished: bool,
    }

    let state = State {
        frames,
        gemini: StreamState::default(),
        queued: VecDeque::new(),
        request_id,
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
                Some(Ok(SseFrame::Event(event))) => state.gemini.push(&event.data),
                Some(Ok(SseFrame::StreamEnd(_) | SseFrame::Terminated { .. })) | None => {
                    state.finished = true;
                    state
                        .gemini
                        .finish()
                        .map(|output| vec![ProtocolEvent::Final(output)])
                }
                Some(Err(error)) => Err(error),
            };
            match result {
                Ok(events) => state.queued.extend(events.into_iter().map(Ok)),
                Err(error) => {
                    let request_id = error
                        .request_id
                        .clone()
                        .or_else(|| Some(state.request_id.clone()));
                    state
                        .queued
                        .push_back(Err(error.with_request_id(request_id)));
                    state.finished = true;
                }
            }
        }
    });
    Ok(ProtocolStream {
        events: Box::pin(events),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        gemini_api_key, CodecInput, CodecLimits, CodecRegistry, ProtocolContractHarness,
        GEMINI_ADAPTER_ID,
    };
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use buckyos_api::{ResourceRef, VisionOcrRequest};
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use reqwest::StatusCode;
    use std::collections::HashMap;
    use std::time::Duration;

    fn context_at(base_url: &str) -> CodecContext {
        CodecContext {
            state_namespace: None,
            base_url: base_url.to_string(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "gemini".into(),
                adapter_type: GEMINI_GENERATE_CONTENT_ADAPTER_ID.into(),
                origin_provider: "gemini".into(),
                origin_model: "gemini-test".into(),
            },
            credential: Some(gemini_api_key("secret://gemini", "top-secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }

    fn context() -> CodecContext {
        context_at("https://generativelanguage.googleapis.com/v1beta")
    }

    fn registry() -> CodecRegistry {
        let (descriptor, registration) = gemini_generate_content_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        registry
    }

    fn input(request: LlmChatInvokeRequest, extra: &[(&str, Value)]) -> CodecInput {
        let mut resolved_parameters =
            BTreeMap::from([("provider_model_id".to_string(), json!("gemini-test"))]);
        for (name, value) in extra {
            resolved_parameters.insert((*name).to_string(), value.clone());
        }
        CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(request),
            resolved_parameters,
        }
    }

    fn encode(input: &CodecInput, context: &CodecContext) -> ProtocolResultValue<HttpRequest> {
        registry().encode(
            GEMINI_GENERATE_CONTENT_ADAPTER_ID,
            GEMINI_GENERATE_CONTENT_OPERATION_ID,
            ApiType::Llm,
            input,
            context,
        )
    }

    fn body(request: &HttpRequest) -> Value {
        let HttpBody::Json(body) = &request.body else {
            panic!("expected JSON body")
        };
        body.clone()
    }

    fn codec(api_type: ApiType) -> GeminiGenerateContentCodec {
        GeminiGenerateContentCodec {
            descriptor: gemini_generate_content_adapter().0.operations
                [GEMINI_GENERATE_CONTENT_OPERATION_ID]
                .clone(),
            api_type,
        }
    }

    fn response(status: StatusCode, value: Value) -> HttpResponse {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        HttpResponse {
            status,
            headers,
            body: Bytes::from(serde_json::to_vec(&value).unwrap()),
            request_id: "request-1".to_string(),
            retry_after: None,
        }
    }

    fn sse(chunks: Vec<Value>) -> StreamingHttpResponse {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let frames = chunks
            .into_iter()
            .map(|chunk| Ok(Bytes::from(format!("data: {chunk}\r\n\r\n"))))
            .collect::<Vec<_>>();
        StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::iter(frames)),
            request_id: "stream-1".to_string(),
            retry_after: None,
        }
    }

    async fn decode(value: Value) -> ProtocolOutput {
        let ProtocolExecution::Immediate(mut output) = codec(ApiType::Llm)
            .decode(response(StatusCode::OK, value))
            .await
            .unwrap()
        else {
            panic!("expected immediate output")
        };
        crate::protocol::bind_provider_state_source(&mut output.value, &context().state_coordinate);
        output
    }

    fn tool_result(call_id: &str, text: &str) -> AiMessage {
        AiMessage::new(
            AiRole::Tool,
            vec![AiContent::ToolResult {
                call_id: call_id.to_string(),
                content: vec![AiToolResultContent::text(text)],
                is_error: false,
            }],
        )
    }

    #[test]
    fn descriptor_owns_the_models_method_family() {
        let (descriptor, _) = gemini_generate_content_adapter();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.protocol_family_id, "gemini");
        assert_eq!(descriptor.protocol_adapter_id, "gemini-generate-content");
        assert!(descriptor.base_adapter_id.is_none());
        assert!(descriptor.probe_path.is_none());
        assert_eq!(descriptor.status, AdapterStatus::Preview);
        assert_eq!(
            descriptor.credential,
            super::super::AdapterCredentialContract::named_header("x-goog-api-key")
        );
        assert_eq!(
            descriptor
                .operations
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                GEMINI_EMBED_CONTENT_OPERATION_ID,
                GEMINI_GENERATE_CONTENT_OPERATION_ID,
                GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID
            ]
        );
        let generate = &descriptor.operations[GEMINI_GENERATE_CONTENT_OPERATION_ID];
        assert_eq!(
            generate.binding(ApiType::Llm).unwrap().execution_modes,
            BTreeSet::from([ExecutionMode::Immediate, ExecutionMode::Stream])
        );
        for binding in &generate.bindings {
            assert_eq!(binding.default_priority, 1);
        }
        for binding in &descriptor.operations[GEMINI_EMBED_CONTENT_OPERATION_ID].bindings {
            assert_eq!(binding.default_priority, 1);
        }
        for binding in &descriptor.operations[GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID].bindings {
            assert_eq!(binding.default_priority, PREDICT_LONG_RUNNING_PRIORITY);
        }

        let registry = registry();
        for (operation, api_type) in [
            (GEMINI_GENERATE_CONTENT_OPERATION_ID, ApiType::VisionOcr),
            (GEMINI_GENERATE_CONTENT_OPERATION_ID, ApiType::VisionCaption),
            (
                GEMINI_EMBED_CONTENT_OPERATION_ID,
                ApiType::EmbeddingMultimodal,
            ),
        ] {
            registry
                .codec(GEMINI_GENERATE_CONTENT_ADAPTER_ID, operation, api_type)
                .unwrap();
        }
        registry
            .native_task_codec(
                GEMINI_GENERATE_CONTENT_ADAPTER_ID,
                GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
                ApiType::VideoExtend,
            )
            .unwrap();

        let (interactions, _) = crate::protocol::gemini_interactions_adapter();
        assert_eq!(interactions.protocol_adapter_id, GEMINI_ADAPTER_ID);
        assert!(!interactions
            .operations
            .contains_key(GEMINI_EMBED_CONTENT_OPERATION_ID));
        assert!(!interactions
            .operations
            .contains_key(GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID));
    }

    #[test]
    fn immediate_request_maps_messages_tools_schema_and_options() {
        let mut request = LlmChatInvokeRequest::new(
            "gemini-test@google",
            vec![
                AiMessage::text(AiRole::System, "be concise"),
                AiMessage::text(AiRole::Developer, "answer in English"),
                AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::text("what is this?"),
                        AiContent::image(ResourceRef::base64(
                            "image/png".to_string(),
                            STANDARD.encode(b"png"),
                        )),
                        AiContent::Document {
                            source: ResourceRef::url(
                                "https://example.com/a.pdf".to_string(),
                                Some("application/pdf".to_string()),
                            ),
                            title: Some("a".to_string()),
                        },
                    ],
                ),
            ],
        );
        request.tools = vec![buckyos_api::AiToolSpec {
            tool_type: "function".to_string(),
            name: "weather".to_string(),
            description: "Get the weather".to_string(),
            args_json_schema: json!({"type":"object","properties":{"city":{"type":"string"}}}),
            output_schema: Some(json!({"type":"object"})),
        }];
        request.max_output_tokens = Some(512);
        request.temperature = Some(0.4);
        request.top_p = Some(0.9);
        request.seed = Some(7);
        request.stop = vec!["END".to_string()];
        request.response_format = Some(LlmResponseFormat::json_schema(
            Some("answer".to_string()),
            json!({"type":"object","properties":{"answer":{"type":"string"}}}),
            Some(true),
        ));
        let input = input(
            request,
            &[
                (
                    "generationConfig",
                    json!({"thinkingConfig":{"thinkingLevel":"low","includeThoughts":true},"maxOutputTokens":9}),
                ),
                (
                    "safetySettings",
                    json!([{"category":"HARM_CATEGORY_HARASSMENT","threshold":"BLOCK_NONE"}]),
                ),
                (features::WEB_SEARCH, json!(true)),
                ("stream", json!(false)),
            ],
        );
        let wire = encode(&input, &context()).unwrap();
        assert_eq!(wire.method, Method::POST);
        assert_eq!(
            wire.url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-test:generateContent"
        );
        let body = body(&wire);
        assert_eq!(
            body["systemInstruction"],
            json!({"parts":[{"text":"be concise"},{"text":"answer in English"}]})
        );
        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(
            body["contents"][0]["parts"][0],
            json!({"text":"what is this?"})
        );
        assert_eq!(
            body["contents"][0]["parts"][1],
            json!({"inlineData":{"mimeType":"image/png","data":STANDARD.encode(b"png")}})
        );
        assert_eq!(
            body["contents"][0]["parts"][2],
            json!({"fileData":{"fileUri":"https://example.com/a.pdf","mimeType":"application/pdf"}})
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0],
            json!({
                "name":"weather",
                "description":"Get the weather",
                "parametersJsonSchema":{"type":"object","properties":{"city":{"type":"string"}}},
                "responseJsonSchema":{"type":"object"}
            })
        );
        assert_eq!(body["tools"][1], json!({"googleSearch":{}}));
        assert_eq!(
            body["generationConfig"],
            json!({
                "maxOutputTokens":512,
                "temperature":0.4,
                "topP":0.9,
                "seed":7,
                "stopSequences":["END"],
                "responseMimeType":"application/json",
                "responseJsonSchema":{"type":"object","properties":{"answer":{"type":"string"}}},
                "thinkingConfig":{"thinkingLevel":"low","includeThoughts":true}
            })
        );
        assert_eq!(body["safetySettings"][0]["threshold"], "BLOCK_NONE");
        assert!(body.get("stream").is_none());
        let golden = ProtocolContractHarness::default()
            .redact_header(HeaderName::from_static("x-goog-api-key"))
            .request(&wire)
            .unwrap();
        assert_eq!(golden.headers["x-goog-api-key"], "[REDACTED]");
        assert!(!format!("{wire:?}").contains("top-secret"));

        let rejected = self::input(
            LlmChatInvokeRequest::new("m@p", vec![AiMessage::text(AiRole::User, "hi")]),
            &[("generation_config", json!({"thinking_level":"low"}))],
        );
        assert_eq!(
            encode(&rejected, &context()).unwrap_err().kind,
            ProtocolErrorKind::InvalidRequest
        );
    }

    #[test]
    fn stream_uses_sse_endpoint_and_base_paths_are_kept() {
        let mut request =
            LlmChatInvokeRequest::new("m@p", vec![AiMessage::text(AiRole::User, "hi")]);
        request.execution_mode = AiccExecutionMode::Stream;
        let input = input(request, &[]);
        for (base, expected) in [
            (
                "https://generativelanguage.googleapis.com/v1beta",
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-test:streamGenerateContent?alt=sse",
            ),
            (
                "https://gateway.example.com",
                "https://gateway.example.com/v1beta/models/gemini-test:streamGenerateContent?alt=sse",
            ),
            (
                "https://gateway.example.com/google/v1/",
                "https://gateway.example.com/google/v1/models/gemini-test:streamGenerateContent?alt=sse",
            ),
        ] {
            assert_eq!(encode(&input, &context_at(base)).unwrap().url, expected);
        }
    }

    #[tokio::test]
    async fn decodes_text_thought_tool_calls_usage_and_replays_parts_verbatim() {
        let parts = json!([
            {"text":"weighing cities","thought":true},
            {"text":"Checking both."},
            {"functionCall":{"id":"fc-1","name":"weather","args":{"city":"Paris"}},"thoughtSignature":"sig-a"},
            {"functionCall":{"id":"fc-2","name":"weather","args":{"city":"Rome"}}}
        ]);
        let output = decode(json!({
            "candidates":[{"content":{"role":"model","parts":parts},"finishReason":"STOP","index":0}],
            "usageMetadata":{
                "promptTokenCount":100,"cachedContentTokenCount":20,"toolUsePromptTokenCount":5,
                "candidatesTokenCount":30,"thoughtsTokenCount":10,"totalTokenCount":145,
                "promptTokensDetails":[{"modality":"TEXT","tokenCount":60},{"modality":"IMAGE","tokenCount":40}]
            },
            "modelVersion":"gemini-test-001"
        }))
        .await;
        assert_eq!(output.value["finish_reason"], "STOP");
        let content = &output.value["message"]["content"];
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["summary"], "weighing cities");
        assert_eq!(content[1], json!({"type":"text","text":"Checking both."}));
        assert_eq!(content[2]["call_id"], "fc-1");
        assert_eq!(content[3]["args"]["city"], "Rome");
        assert_eq!(content[4]["type"], "provider_state");
        assert_eq!(content[4]["provider"], "gemini");
        assert_eq!(content[4]["value"]["parts"], parts);
        assert_eq!(output.value["tool_calls"].as_array().unwrap().len(), 2);
        let usage = output.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(105));
        assert_eq!(usage.output_tokens, Some(40));
        assert_eq!(usage.total_tokens, Some(145));
        assert_eq!(usage.cache_read_input_tokens, Some(20));
        assert_eq!(usage.reasoning_tokens, Some(10));
        assert_eq!(usage.image_input_tokens, Some(40));
        assert_eq!(usage.audio_input_tokens, Some(0));

        let assistant: AiMessage = serde_json::from_value(output.value["message"].clone()).unwrap();
        let request = LlmChatInvokeRequest::new(
            "m@p",
            vec![
                AiMessage::text(AiRole::User, "weather in Paris and Rome?"),
                assistant.clone(),
                tool_result("fc-2", "rain"),
                tool_result("fc-1", "sunny"),
            ],
        );
        let body = body(&encode(&input(request, &[]), &context()).unwrap());
        assert_eq!(body["contents"][1], json!({"role":"model","parts":parts}));
        assert_eq!(
            body["contents"][2],
            json!({"role":"user","parts":[
                {"functionResponse":{"id":"fc-2","name":"weather","response":{"output":"rain"}}},
                {"functionResponse":{"id":"fc-1","name":"weather","response":{"output":"sunny"}}}
            ]})
        );

        let mut other = context();
        other.state_coordinate.origin_model = "gemini-other".into();
        let request = LlmChatInvokeRequest::new(
            "m@p",
            vec![
                AiMessage::text(AiRole::User, "weather?"),
                assistant,
                tool_result("fc-1", "sunny"),
                tool_result("fc-2", "rain"),
            ],
        );
        let body = self::body(&encode(&input(request, &[]), &other).unwrap());
        assert_eq!(
            body["contents"][1]["parts"],
            json!([
                {"text":"Checking both."},
                {"functionCall":{"id":"fc-1","name":"weather","args":{"city":"Paris"}},"thoughtSignature":SKIP_THOUGHT_SIGNATURE},
                {"functionCall":{"id":"fc-2","name":"weather","args":{"city":"Rome"}}}
            ])
        );
    }

    #[tokio::test]
    async fn calls_without_wire_ids_get_canonical_ids_that_are_not_echoed() {
        let parts = json!([{"functionCall":{"name":"lookup","args":{}},"thoughtSignature":"sig"}]);
        let output = decode(json!({
            "candidates":[{"content":{"role":"model","parts":parts},"finishReason":"STOP"}],
            "responseId":"resp-7"
        }))
        .await;
        assert!(output.usage.is_none());
        assert_eq!(output.value["tool_calls"][0]["call_id"], "resp-7-0");
        let assistant: AiMessage = serde_json::from_value(output.value["message"].clone()).unwrap();
        let request = LlmChatInvokeRequest::new(
            "m@p",
            vec![
                AiMessage::text(AiRole::User, "look it up"),
                assistant,
                tool_result("resp-7-0", "found"),
            ],
        );
        let body = body(&encode(&input(request, &[]), &context()).unwrap());
        assert_eq!(body["contents"][1]["parts"], parts);
        assert_eq!(
            body["contents"][2]["parts"][0],
            json!({"functionResponse":{"name":"lookup","response":{"output":"found"}}})
        );

        let orphan = LlmChatInvokeRequest::new(
            "m@p",
            vec![
                AiMessage::text(AiRole::User, "hi"),
                tool_result("missing", "x"),
            ],
        );
        assert_eq!(
            encode(&input(orphan, &[]), &context()).unwrap_err().kind,
            ProtocolErrorKind::InvalidRequest
        );
    }

    #[test]
    fn foreign_history_tool_errors_and_media_results_are_mapped() {
        let request = LlmChatInvokeRequest::new(
            "m@p",
            vec![
                AiMessage::text(AiRole::User, "draw"),
                AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::ProviderState {
                            source: ProviderStateCoordinate::unbound(),
                            provider: "claude".to_string(),
                            value: json!({"type":"text","text":"earlier answer"}),
                        },
                        AiContent::tool_use("call-1", "render", HashMap::new()),
                    ],
                ),
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::ToolResult {
                        call_id: "call-1".to_string(),
                        content: vec![
                            AiToolResultContent::text("failed"),
                            AiToolResultContent::Image {
                                source: ResourceRef::base64(
                                    "image/png".to_string(),
                                    STANDARD.encode(b"img"),
                                ),
                            },
                        ],
                        is_error: true,
                    }],
                ),
            ],
        );
        let body = body(&encode(&input(request, &[]), &context()).unwrap());
        assert!(body["contents"][1]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("earlier answer"));
        assert_eq!(
            body["contents"][1]["parts"][1]["thoughtSignature"],
            SKIP_THOUGHT_SIGNATURE
        );
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"],
            json!({
                "id":"call-1","name":"render","response":{"error":"failed"},
                "parts":[{"inlineData":{"mimeType":"image/png","data":STANDARD.encode(b"img")}}]
            })
        );
    }

    #[tokio::test]
    async fn sse_is_incremental_and_keeps_signed_parts_apart() {
        let chunk =
            |parts: Value| json!({"candidates":[{"content":{"role":"model","parts":parts}}]});
        let mut last = chunk(json!([{"text":"","thoughtSignature":"sig-final"}]));
        last["candidates"][0]["finishReason"] = json!("STOP");
        last["usageMetadata"] = json!({"promptTokenCount":4,"candidatesTokenCount":2,"thoughtsTokenCount":3,"totalTokenCount":9});
        let mut partial_usage = chunk(json!([{"text":"lo"}]));
        partial_usage["usageMetadata"] = json!({"promptTokenCount":4});
        let events = codec(ApiType::Llm)
            .decode_stream(sse(vec![
                chunk(json!([{"text":"plan","thought":true}])),
                chunk(json!([{"text":"Hel"}])),
                partial_usage,
                last,
            ]))
            .await
            .unwrap()
            .events
            .collect::<Vec<_>>()
            .await;
        assert_eq!(events.len(), 4);
        let deltas = events[..3]
            .iter()
            .map(|event| match event {
                Ok(ProtocolEvent::Delta(value)) => value.clone(),
                other => panic!("expected delta, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            deltas,
            [
                json!({"type":"thinking","text":"plan"}),
                json!({"type":"text","text":"Hel"}),
                json!({"type":"text","text":"lo"})
            ]
        );
        let Ok(ProtocolEvent::Final(output)) = &events[3] else {
            panic!("expected final output")
        };
        let message: AiMessage = serde_json::from_value(output.value["message"].clone()).unwrap();
        assert_eq!(message.text_content(), "Hello");
        assert_eq!(output.value["finish_reason"], "STOP");
        assert_eq!(
            output.value["message"]["content"][2]["value"]["parts"],
            json!([
                {"text":"plan","thought":true},
                {"text":"Hello"},
                {"text":"","thoughtSignature":"sig-final"}
            ])
        );
        let usage = output.usage.as_ref().unwrap();
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(9));

        let truncated = codec(ApiType::Llm)
            .decode_stream(sse(vec![chunk(json!([{"text":"Hel"}]))]))
            .await
            .unwrap()
            .events
            .collect::<Vec<_>>()
            .await;
        let error = truncated.last().unwrap().as_ref().unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidResponse);
        assert_eq!(error.request_id.as_deref(), Some("stream-1"));

        let failed = codec(ApiType::Llm)
            .decode_stream(sse(vec![json!({
                "error":{"code":503,"status":"UNAVAILABLE","message":"overloaded"}
            })]))
            .await
            .unwrap()
            .events
            .collect::<Vec<_>>()
            .await;
        assert!(failed[0]
            .as_ref()
            .unwrap_err()
            .message
            .contains("UNAVAILABLE: overloaded"));
    }

    #[tokio::test]
    async fn errors_and_blocked_prompts_are_mapped() {
        let error = codec(ApiType::Llm)
            .decode(response(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"quota"}}),
            ))
            .await
            .unwrap_err();
        assert_eq!(error.http_status, Some(429));
        assert!(error.message.contains("RESOURCE_EXHAUSTED: quota"));

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let rejected = codec(ApiType::Llm)
            .decode_stream(StreamingHttpResponse {
                status: StatusCode::BAD_REQUEST,
                headers,
                body: Box::pin(stream::iter(vec![Ok(Bytes::from_static(
                    br#"{"error":{"status":"INVALID_ARGUMENT","message":"missing thought_signature"}}"#,
                ))])),
                request_id: "stream-2".to_string(),
                retry_after: None,
            })
            .await
            .err()
            .unwrap();
        assert_eq!(rejected.kind, ProtocolErrorKind::InvalidRequest);
        assert!(rejected.message.contains("missing thought_signature"));

        let blocked = decode(json!({
            "promptFeedback":{"blockReason":"SAFETY"},
            "usageMetadata":{"promptTokenCount":8,"totalTokenCount":8}
        }))
        .await;
        assert_eq!(blocked.value["finish_reason"], "SAFETY");
        assert_eq!(blocked.value["message"]["content"], json!([]));
        assert_eq!(blocked.usage.unwrap().input_tokens, Some(8));

        assert_eq!(
            codec(ApiType::Llm)
                .decode(response(StatusCode::OK, json!({"usageMetadata":{}})))
                .await
                .unwrap_err()
                .kind,
            ProtocolErrorKind::InvalidResponse
        );
    }

    #[tokio::test]
    async fn vision_ocr_is_lowered_to_a_chat_request() {
        let request = VisionOcrRequest::new(
            "gemini-test@google",
            ResourceRef::base64("application/pdf".to_string(), STANDARD.encode(b"pdf")),
        );
        let input = CodecInput {
            canonical_request: AiccCall::VisionOcr(request),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                json!("gemini-test"),
            )]),
        };
        let wire = registry()
            .encode(
                GEMINI_GENERATE_CONTENT_ADAPTER_ID,
                GEMINI_GENERATE_CONTENT_OPERATION_ID,
                ApiType::VisionOcr,
                &input,
                &context(),
            )
            .unwrap();
        let parts = body(&wire)["contents"][0]["parts"].clone();
        assert!(parts[0]["text"].as_str().unwrap().contains("Extract"));
        assert_eq!(parts[1]["inlineData"]["mimeType"], "application/pdf");

        let ProtocolExecution::Immediate(output) = codec(ApiType::VisionOcr)
            .decode(response(
                StatusCode::OK,
                json!({"candidates":[{"content":{"role":"model","parts":[{"text":"page one"}]},"finishReason":"STOP"}]}),
            ))
            .await
            .unwrap()
        else {
            panic!("expected immediate output")
        };
        assert_eq!(
            output.value,
            json!({"text":"page one","pages":[],"artifacts":{}})
        );
    }
}
