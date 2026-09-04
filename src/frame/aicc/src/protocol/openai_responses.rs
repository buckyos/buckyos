use super::{
    sse_frame_stream, AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration,
    CredentialKind, ExecutionMode, HttpBody, HttpRequest, HttpResponse, MaterializedResource,
    MultipartBody, MultipartPart, NativeTaskCodec, NativeTaskHandle, NativeTaskInput,
    NativeTaskOperation, NativeTaskOutput, NativeTaskState, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolEvent, ProtocolExecution,
    ProtocolOutput, ProtocolResultValue, ProtocolStream, SseConfig, SseFrame, SseFramer,
    SseStreamEnd, StreamingHttpResponse,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiContent, AiMessage, AiRole, AiToolResultContent, AiUsage, AiccCall, ApiType,
    AudioSpeechRecognitionRequest, AudioTextToSpeechRequest, EmbeddingTextItem,
    ImageInpaintRequest, ImageToImageRequest, LlmChatInvokeRequest, LlmResponseFormatType,
    ResourceRef as PublicResourceRef, TextToImageInvokeRequest,
};
use futures_util::{stream, StreamExt};
use reqwest::header::{HeaderValue, CONTENT_TYPE};
use reqwest::{Method, StatusCode, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const OPENAI_RESPONSES_ADAPTER_ID: &str = "openai-responses";
pub(crate) const OPENAI_RESPONSES_OPERATION_ID: &str = "responses.create";
pub(crate) const OPENAI_EMBEDDINGS_OPERATION_ID: &str = "embeddings.create";
pub(crate) const OPENAI_IMAGES_GENERATE_OPERATION_ID: &str = "images.generate";
pub(crate) const OPENAI_IMAGES_EDIT_OPERATION_ID: &str = "images.edit";
pub(crate) const OPENAI_AUDIO_SPEECH_OPERATION_ID: &str = "audio.speech";
pub(crate) const OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID: &str = "audio.transcriptions";
pub(crate) const OPENAI_VIDEOS_OPERATION_ID: &str = "videos.create";

const OPENAI_PROVIDER_NAMESPACE: &str = "openai";
const DEFAULT_MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn openai_responses_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let responses = operation(
        OPENAI_RESPONSES_OPERATION_ID,
        vec![
            binding(
                ApiType::Llm,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
                [
                    buckyos_api::features::TOOL_CALL,
                    buckyos_api::features::JSON_SCHEMA,
                    "reasoning",
                    buckyos_api::features::VISION,
                ],
            ),
            binding(
                ApiType::ImageTextToImage,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
                ["image_generation"],
            ),
            binding(
                ApiType::ImageImageToImage,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
                ["image_generation"],
            ),
        ],
        false,
    );
    let embeddings = operation(
        OPENAI_EMBEDDINGS_OPERATION_ID,
        vec![binding(
            ApiType::EmbeddingText,
            [ExecutionMode::Immediate],
            std::iter::empty::<&str>(),
        )],
        false,
    );
    let images_generate = operation(
        OPENAI_IMAGES_GENERATE_OPERATION_ID,
        vec![binding(
            ApiType::ImageTextToImage,
            [ExecutionMode::Immediate],
            std::iter::empty::<&str>(),
        )],
        false,
    );
    let images_edit = operation(
        OPENAI_IMAGES_EDIT_OPERATION_ID,
        vec![
            binding(
                ApiType::ImageImageToImage,
                [ExecutionMode::Immediate],
                std::iter::empty::<&str>(),
            ),
            binding(
                ApiType::ImageInpaint,
                [ExecutionMode::Immediate],
                std::iter::empty::<&str>(),
            ),
        ],
        false,
    );
    let audio_speech = operation(
        OPENAI_AUDIO_SPEECH_OPERATION_ID,
        vec![binding(
            ApiType::AudioTextToSpeech,
            [ExecutionMode::Immediate],
            std::iter::empty::<&str>(),
        )],
        false,
    );
    let audio_transcriptions = operation(
        OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID,
        vec![binding(
            ApiType::AudioSpeechRecognition,
            [ExecutionMode::Immediate],
            std::iter::empty::<&str>(),
        )],
        false,
    );
    let videos = operation(
        OPENAI_VIDEOS_OPERATION_ID,
        vec![
            binding(
                ApiType::VideoTextToVideo,
                [ExecutionMode::NativeTask],
                std::iter::empty::<&str>(),
            ),
            binding(
                ApiType::VideoImageToVideo,
                [ExecutionMode::NativeTask],
                std::iter::empty::<&str>(),
            ),
        ],
        true,
    );

    let operations = BTreeMap::from([
        (responses.operation_id.clone(), responses.clone()),
        (embeddings.operation_id.clone(), embeddings.clone()),
        (
            images_generate.operation_id.clone(),
            images_generate.clone(),
        ),
        (images_edit.operation_id.clone(), images_edit.clone()),
        (audio_speech.operation_id.clone(), audio_speech.clone()),
        (
            audio_transcriptions.operation_id.clone(),
            audio_transcriptions.clone(),
        ),
        (videos.operation_id.clone(), videos.clone()),
    ]);
    let descriptor = AdapterDescriptor {
        protocol_family_id: "openai".to_string(),
        protocol_adapter_id: OPENAI_RESPONSES_ADAPTER_ID.to_string(),
        interface_generation: "responses-v1".to_string(),
        base_adapter_id: None,
        status: AdapterStatus::Stable,
        operations,
    };
    let operation_codecs: Vec<Arc<dyn OperationCodec>> = vec![
        Arc::new(OpenAiResponsesCodec::new(responses.clone(), ApiType::Llm)),
        Arc::new(OpenAiResponsesCodec::new(
            responses.clone(),
            ApiType::ImageTextToImage,
        )),
        Arc::new(OpenAiResponsesCodec::new(
            responses,
            ApiType::ImageImageToImage,
        )),
        Arc::new(OpenAiEmbeddingCodec::new(embeddings)),
        Arc::new(OpenAiImageCodec::new(
            images_generate,
            ApiType::ImageTextToImage,
        )),
        Arc::new(OpenAiImageCodec::new(
            images_edit.clone(),
            ApiType::ImageImageToImage,
        )),
        Arc::new(OpenAiImageCodec::new(images_edit, ApiType::ImageInpaint)),
        Arc::new(OpenAiAudioCodec::new(
            audio_speech,
            ApiType::AudioTextToSpeech,
        )),
        Arc::new(OpenAiAudioCodec::new(
            audio_transcriptions,
            ApiType::AudioSpeechRecognition,
        )),
    ];
    let native_task_codecs: Vec<Arc<dyn NativeTaskCodec>> = vec![
        Arc::new(OpenAiVideoCodec::new(
            videos.clone(),
            ApiType::VideoTextToVideo,
        )),
        Arc::new(OpenAiVideoCodec::new(videos, ApiType::VideoImageToVideo)),
    ];
    (
        descriptor,
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

fn operation(
    id: &str,
    bindings: Vec<OperationBinding>,
    supports_cancel: bool,
) -> OperationDescriptor {
    OperationDescriptor {
        operation_id: id.to_string(),
        bindings,
        supports_cancel,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    }
}

fn binding(
    api_type: ApiType,
    modes: impl IntoIterator<Item = ExecutionMode>,
    features: impl IntoIterator<Item = &'static str>,
) -> OperationBinding {
    let mut binding = OperationBinding::new(api_type, modes);
    binding.supported_features = features.into_iter().map(str::to_string).collect();
    binding
}

#[derive(Clone)]
struct OpenAiResponsesCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl OpenAiResponsesCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl OperationCodec for OpenAiResponsesCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate, ExecutionMode::Stream])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let body = match (&call.input.canonical_request, self.api_type) {
            (AiccCall::ChatCompletionsCreate(request), ApiType::Llm) => {
                encode_responses_llm(request, call)?
            }
            (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => {
                encode_responses_image_generate(request, call)?
            }
            (AiccCall::ImageToImage(request), ApiType::ImageImageToImage) => {
                encode_responses_image_edit(request, call)?
            }
            _ => {
                return Err(ProtocolError::invalid_request(
                    "OpenAI Responses codec received the wrong canonical request",
                ))
            }
        };
        json_request(call, Method::POST, "responses", body)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        if is_sse(response.headers.get(CONTENT_TYPE)) {
            return decode_buffered_responses_stream(response);
        }
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        Ok(ProtocolExecution::Immediate(decode_response_object(
            &value,
        )?))
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        if !response.status.is_success() {
            let response = response
                .into_bounded_error_response(self.descriptor.max_response_bytes)
                .await?;
            return Err(openai_http_error(
                response.status,
                &response.body,
                &response.request_id,
                response.retry_after,
            ));
        }
        if !is_sse(response.headers.get(CONTENT_TYPE)) {
            return Err(ProtocolError::invalid_response(
                "OpenAI Responses stream must use text/event-stream",
            )
            .with_request_id(Some(response.request_id)));
        }
        responses_protocol_stream(response, self.descriptor.max_response_bytes).await
    }
}

fn encode_responses_llm(
    request: &LlmChatInvokeRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Value> {
    for message in &request.messages {
        message.validate().map_err(|error| {
            ProtocolError::invalid_request(format!("invalid canonical message: {error}"))
        })?;
    }
    if request.seed.is_some() || !request.stop.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI Responses does not support canonical seed or stop parameters",
        ));
    }
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(provider_model_id(call)?));
    body.insert(
        "input".to_string(),
        Value::Array(encode_response_input(&request.messages, call)?),
    );
    if !request.tools.is_empty() {
        if request
            .tools
            .iter()
            .any(|tool| tool.tool_type != "function")
        {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenAI Responses canonical tools must use the function type",
            ));
        }
        body.insert(
            "tools".to_string(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": tool.tool_type,
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.args_json_schema
                        })
                    })
                    .collect(),
            ),
        );
    }
    if let Some(format) = &request.response_format {
        let format = match format.format_type {
            LlmResponseFormatType::Text => json!({"type": "text"}),
            LlmResponseFormatType::Json | LlmResponseFormatType::JsonObject => {
                json!({"type": "json_object"})
            }
            LlmResponseFormatType::JsonSchema => {
                let schema = format.json_schema.as_ref().ok_or_else(|| {
                    ProtocolError::invalid_request(
                        "json_schema response format is missing its schema",
                    )
                })?;
                json!({
                    "type": "json_schema",
                    "name": schema.name.clone().unwrap_or_else(|| "response".to_string()),
                    "schema": schema.schema,
                    "strict": schema.strict.unwrap_or(true)
                })
            }
        };
        body.insert("text".to_string(), json!({"format": format}));
    }
    insert_optional(
        &mut body,
        "temperature",
        request.temperature.map(Value::from),
    );
    insert_optional(&mut body, "top_p", request.top_p.map(Value::from));
    insert_optional(
        &mut body,
        "max_output_tokens",
        request.max_output_tokens.map(Value::from),
    );
    apply_responses_parameters(&mut body, &call.input.resolved_parameters)?;
    Ok(Value::Object(body))
}

fn encode_response_input(
    messages: &[AiMessage],
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Vec<Value>> {
    let mut items = Vec::new();
    for message in messages {
        if message.role == AiRole::Tool {
            let AiContent::ToolResult {
                call_id,
                content,
                is_error,
            } = &message.content[0]
            else {
                return Err(ProtocolError::invalid_request(
                    "tool message must contain one tool result",
                ));
            };
            items.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": encode_tool_result(content, *is_error, call)?
            }));
            continue;
        }
        let replays_output_message = message.role == AiRole::Assistant
            && message.content.iter().any(|block| {
                matches!(
                    block,
                    AiContent::ProviderState { provider, value }
                        if provider == OPENAI_PROVIDER_NAMESPACE
                            && value.get("type").and_then(Value::as_str) == Some("refusal")
                )
            });
        let mut content = Vec::new();
        let flush = |content: &mut Vec<Value>, items: &mut Vec<Value>| {
            if !content.is_empty() {
                items.push(json!({
                    "type": "message",
                    "role": message.role.as_str(),
                    "content": std::mem::take(content)
                }));
            }
        };
        for block in &message.content {
            match block {
                AiContent::Text { text } => content.push(json!({
                    "type": if replays_output_message { "output_text" } else { "input_text" },
                    "text": text
                })),
                AiContent::Image { source } => content.push(encode_input_image(source, call)?),
                AiContent::Document { source, title } => {
                    content.push(encode_input_file(source, title.as_deref(), call)?)
                }
                AiContent::ToolUse {
                    call_id,
                    name,
                    args,
                } => {
                    flush(&mut content, &mut items);
                    items.push(json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": serde_json::to_string(args).map_err(|_| {
                            ProtocolError::invalid_request("tool arguments are not serializable")
                        })?
                    }));
                }
                AiContent::Thinking { .. } => {}
                AiContent::ProviderState { provider, value }
                    if provider == OPENAI_PROVIDER_NAMESPACE =>
                {
                    validate_provider_state(value)?;
                    if replays_output_message
                        && value.get("type").and_then(Value::as_str) == Some("refusal")
                    {
                        content.push(value.clone());
                    } else {
                        flush(&mut content, &mut items);
                        items.push(value.clone());
                    }
                }
                AiContent::ProviderState { .. } => {}
                AiContent::ToolResult { .. } => {
                    return Err(ProtocolError::invalid_request(
                        "tool result block must use the canonical tool role",
                    ))
                }
            }
        }
        flush(&mut content, &mut items);
    }
    Ok(items)
}

fn encode_tool_result(
    content: &[AiToolResultContent],
    is_error: bool,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Value> {
    let mut output = Vec::new();
    for block in content {
        match block {
            AiToolResultContent::Text { text } => {
                output.push(json!({"type": "input_text", "text": text}))
            }
            AiToolResultContent::Image { source } => output.push(encode_input_image(source, call)?),
            AiToolResultContent::Document { source, title } => {
                output.push(encode_input_file(source, title.as_deref(), call)?)
            }
        }
    }
    if is_error {
        output.insert(
            0,
            json!({"type": "input_text", "text": "The tool returned an error."}),
        );
        Ok(Value::Array(output))
    } else {
        Ok(Value::Array(output))
    }
}

fn validate_provider_state(value: &Value) -> ProtocolResultValue<()> {
    if value.get("type").and_then(Value::as_str).is_none() {
        return Err(ProtocolError::invalid_request(
            "OpenAI ProviderState must contain a string type",
        ));
    }
    Ok(())
}

fn apply_responses_parameters(
    body: &mut Map<String, Value>,
    parameters: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    for (name, value) in parameters {
        if name == "provider_model_id" {
            continue;
        }
        let valid = match name.as_str() {
            "background" | "parallel_tool_calls" | "store" | "stream" => value.is_boolean(),
            "include" => value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            "metadata" | "reasoning" => value.is_object(),
            "service_tier" | "truncation" => value.is_string(),
            "tool_choice" => value.is_string() || value.is_object(),
            _ => false,
        };
        if !valid {
            return Err(ProtocolError::invalid_request(format!(
                "resolved OpenAI Responses parameter `{name}` is not supported or has an invalid type"
            )));
        }
        body.insert(name.clone(), value.clone());
    }
    Ok(())
}

fn require_parameter_subset(
    parameters: &BTreeMap<String, Value>,
    allowed: &[&str],
    operation: &str,
) -> ProtocolResultValue<()> {
    if let Some(name) = parameters
        .keys()
        .find(|name| name.as_str() != "provider_model_id" && !allowed.contains(&name.as_str()))
    {
        return Err(ProtocolError::invalid_request(format!(
            "resolved OpenAI {operation} parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

fn encode_responses_image_generate(
    request: &TextToImageInvokeRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Value> {
    if request.negative_prompt.is_some()
        || request.seed.is_some()
        || request.style.is_some()
        || request.aspect_ratio.is_some()
        || request.n.is_some_and(|count| count != 1)
    {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI Responses image generation received an unsupported hard parameter",
        ));
    }
    let tool = image_generation_tool(
        request.size.as_deref(),
        request.quality.as_deref(),
        request
            .output
            .as_ref()
            .and_then(|output| output.media_type.as_deref()),
        "generate",
    )?;
    let mut body = json!({
        "model": provider_model_id(call)?,
        "input": request.prompt,
        "tools": [tool],
        "tool_choice": {"type": "image_generation"}
    });
    if let Value::Object(body) = &mut body {
        apply_responses_parameters(body, &call.input.resolved_parameters)?;
    }
    Ok(body)
}

fn encode_responses_image_edit(
    request: &ImageToImageRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Value> {
    if request.images.is_empty() || request.strength.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI Responses image edit requires images and does not support strength",
        ));
    }
    let mut content = vec![json!({"type": "input_text", "text": request.prompt})];
    for image in &request.images {
        content.push(encode_input_image(image, call)?);
    }
    let tool = image_generation_tool(
        request
            .output
            .as_ref()
            .and_then(|output| output.size.as_deref()),
        None,
        request
            .output
            .as_ref()
            .and_then(|output| output.media_type.as_deref()),
        "edit",
    )?;
    let mut body = json!({
        "model": provider_model_id(call)?,
        "input": [{"role": "user", "content": content}],
        "tools": [tool],
        "tool_choice": {"type": "image_generation"}
    });
    if let Value::Object(body) = &mut body {
        apply_responses_parameters(body, &call.input.resolved_parameters)?;
    }
    Ok(body)
}

fn image_generation_tool(
    size: Option<&str>,
    quality: Option<&str>,
    media_type: Option<&str>,
    action: &str,
) -> ProtocolResultValue<Value> {
    let mut tool = Map::from_iter([
        ("type".to_string(), json!("image_generation")),
        ("action".to_string(), json!(action)),
    ]);
    insert_optional(&mut tool, "size", size.map(|value| json!(value)));
    insert_optional(&mut tool, "quality", quality.map(|value| json!(value)));
    if let Some(media_type) = media_type {
        tool.insert(
            "output_format".to_string(),
            json!(image_format(media_type)?),
        );
    }
    Ok(Value::Object(tool))
}

fn decode_response_object(response: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    if status == "failed" {
        return Err(response_failure(response));
    }
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::invalid_response("OpenAI response output is missing"))?;
    let mut blocks = Vec::new();
    let mut artifacts = Vec::new();
    let mut images = Vec::new();
    for item in output {
        decode_response_item(item, &mut blocks, &mut artifacts, &mut images)?;
    }
    let message = AiMessage::new(AiRole::Assistant, blocks);
    message.validate().map_err(|error| {
        ProtocolError::invalid_response(format!("invalid OpenAI response content: {error}"))
    })?;
    let tool_calls = message.tool_calls();
    let finish_reason = match status {
        "completed" => Some("stop".to_string()),
        "incomplete" => response
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some("incomplete".to_string())),
        other => Some(other.to_string()),
    };
    let provider_states = message
        .content
        .iter()
        .filter(|block| matches!(block, AiContent::ProviderState { .. }))
        .cloned()
        .collect::<Vec<_>>();
    let value = json!({
        "message": message,
        "tool_calls": tool_calls,
        "finish_reason": finish_reason,
        "provider_task_ref": response.get("id").cloned().unwrap_or(Value::Null),
        "images": images,
        "provider_states": provider_states
    });
    Ok(ProtocolOutput {
        value,
        usage: decode_usage(response.get("usage"))?,
        artifacts,
    })
}

fn decode_response_item(
    item: &Value,
    blocks: &mut Vec<AiContent>,
    artifacts: &mut Vec<AiArtifact>,
    images: &mut Vec<PublicResourceRef>,
) -> ProtocolResultValue<()> {
    let item_type = item.get("type").and_then(Value::as_str).ok_or_else(|| {
        ProtocolError::invalid_response("OpenAI response output item is missing type")
    })?;
    match item_type {
        "message" => {
            let content = item
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ProtocolError::invalid_response("OpenAI message content is invalid")
                })?;
            for part in content {
                match part.get("type").and_then(Value::as_str) {
                    Some("output_text") => blocks.push(AiContent::Text {
                        text: required_string(part, "text", "OpenAI output text")?,
                    }),
                    Some("refusal") => blocks.push(provider_state(part.clone())),
                    _ => blocks.push(provider_state(part.clone())),
                }
            }
        }
        "function_call" => {
            let arguments = required_string(item, "arguments", "OpenAI function arguments")?;
            let args: Value = serde_json::from_str(&arguments).map_err(|error| {
                ProtocolError::invalid_response(format!(
                    "OpenAI function arguments are invalid JSON: {error}"
                ))
            })?;
            let args = args.as_object().ok_or_else(|| {
                ProtocolError::invalid_response("OpenAI function arguments must be an object")
            })?;
            blocks.push(AiContent::ToolUse {
                call_id: required_string(item, "call_id", "OpenAI function call")?,
                name: required_string(item, "name", "OpenAI function call")?,
                args: args.clone().into_iter().collect::<HashMap<_, _>>(),
            });
        }
        "reasoning" => {
            let summary = item
                .get("summary")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|summary| !summary.is_empty());
            let text = item
                .get("content")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|text| !text.is_empty());
            blocks.push(AiContent::Thinking {
                summary,
                text,
                provider_metadata: Some(json!({
                    "id": item.get("id").cloned().unwrap_or(Value::Null),
                    "encrypted_content": item.get("encrypted_content").cloned().unwrap_or(Value::Null)
                })),
            });
            blocks.push(provider_state(item.clone()));
        }
        "image_generation_call" => {
            if matches!(
                item.get("status").and_then(Value::as_str),
                Some("failed" | "rejected")
            ) {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidResponse,
                    "OpenAI image generation failed",
                ));
            }
            let result = item
                .get("result")
                .and_then(Value::as_str)
                .filter(|result| !result.is_empty())
                .ok_or_else(|| {
                    ProtocolError::new(
                        ProtocolErrorKind::InvalidResponse,
                        "OpenAI image generation result is missing",
                    )
                })?
                .to_string();
            let bytes = STANDARD.decode(&result).map_err(|_| {
                ProtocolError::new(
                    ProtocolErrorKind::InvalidResponse,
                    "OpenAI image generation result is not valid base64",
                )
            })?;
            if bytes.is_empty() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidResponse,
                    "OpenAI image generation result is empty",
                ));
            }
            let mime = item
                .get("output_format")
                .and_then(Value::as_str)
                .map(image_mime)
                .transpose()?
                .unwrap_or("image/png");
            let resource = PublicResourceRef::base64(mime.to_string(), result);
            let index = images.len();
            images.push(resource.clone());
            artifacts.push(AiArtifact {
                name: format!("image-{index}"),
                resource,
                mime: Some(mime.to_string()),
                metadata: item.get("id").cloned().map(|id| json!({"provider_id": id})),
            });
            blocks.push(provider_state(item.clone()));
        }
        _ => blocks.push(provider_state(item.clone())),
    }
    Ok(())
}

fn provider_state(value: Value) -> AiContent {
    AiContent::ProviderState {
        provider: OPENAI_PROVIDER_NAMESPACE.to_string(),
        value,
    }
}

fn decode_usage(value: Option<&Value>) -> ProtocolResultValue<Option<AiUsage>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let input_tokens = value.get("input_tokens").and_then(Value::as_u64);
    let output_tokens = value.get("output_tokens").and_then(Value::as_u64);
    let total_tokens = value
        .get("total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => input.checked_add(output),
            _ => None,
        });
    Ok(Some(AiUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        request_units: None,
    }))
}

fn decode_buffered_responses_stream(
    response: HttpResponse,
) -> ProtocolResultValue<ProtocolExecution> {
    let mut framer = SseFramer::new(SseConfig {
        termination_markers: Vec::new(),
        ..SseConfig::default()
    })?;
    let mut events = Vec::new();
    for frame in framer.push(&response.body)? {
        events.extend(decode_response_frame(frame)?);
    }
    for frame in framer.finish(SseStreamEnd::EndOfStream)? {
        events.extend(decode_response_frame(frame)?);
    }
    require_final_event(&events)?;
    Ok(ProtocolExecution::Stream(ProtocolStream {
        events: Box::pin(stream::iter(events.into_iter().map(Ok))),
    }))
}

async fn responses_protocol_stream(
    response: StreamingHttpResponse,
    max_response_bytes: usize,
) -> ProtocolResultValue<ProtocolStream> {
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
        pending: VecDeque<ProtocolResultValue<ProtocolEvent>>,
        finished: bool,
        final_seen: bool,
        request_id: String,
        retry_after: Option<Duration>,
    }
    let state = State {
        frames,
        pending: VecDeque::new(),
        finished: false,
        final_seen: false,
        request_id,
        retry_after,
    };
    let events = stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                if state.final_seen {
                    state.finished = true;
                    return Some((
                        Err(ProtocolError::invalid_response(
                            "OpenAI Responses stream emitted data after its final event",
                        )
                        .with_request_id(Some(state.request_id.clone()))
                        .with_retry_after(state.retry_after)),
                        state,
                    ));
                }
                if matches!(event, Ok(ProtocolEvent::Final(_))) {
                    state.final_seen = true;
                }
                return Some((event, state));
            }
            if state.finished {
                return None;
            }
            match state.frames.next().await {
                Some(Ok(SseFrame::Event(event))) => {
                    match decode_response_frame(SseFrame::Event(event)) {
                        Ok(events) => state.pending.extend(events.into_iter().map(Ok)),
                        Err(error) => {
                            state.finished = true;
                            state.pending.push_back(Err(error
                                .with_request_id(Some(state.request_id.clone()))
                                .with_retry_after(state.retry_after)));
                        }
                    }
                }
                Some(Ok(SseFrame::StreamEnd(_))) | None => {
                    state.finished = true;
                    if !state.final_seen {
                        state.pending.push_back(Err(ProtocolError::invalid_response(
                            "OpenAI Responses stream ended before a final event",
                        )
                        .with_request_id(Some(state.request_id.clone()))
                        .with_retry_after(state.retry_after)));
                    }
                }
                Some(Ok(SseFrame::Terminated { .. })) => {
                    state.finished = true;
                    state.pending.push_back(Err(ProtocolError::invalid_response(
                        "OpenAI Responses stream used an unexpected termination marker",
                    )
                    .with_request_id(Some(state.request_id.clone()))
                    .with_retry_after(state.retry_after)));
                }
                Some(Err(error)) => {
                    state.finished = true;
                    state.pending.push_back(Err(error));
                }
            }
        }
    });
    Ok(ProtocolStream {
        events: Box::pin(events),
    })
}

fn decode_response_frame(frame: SseFrame) -> ProtocolResultValue<Vec<ProtocolEvent>> {
    let SseFrame::Event(event) = frame else {
        return match frame {
            SseFrame::StreamEnd(_) => Ok(Vec::new()),
            SseFrame::Terminated { .. } => Err(ProtocolError::invalid_response(
                "OpenAI Responses stream used an unexpected termination marker",
            )),
            SseFrame::Event(_) => unreachable!(),
        };
    };
    let value: Value = serde_json::from_str(&event.data).map_err(|error| {
        ProtocolError::invalid_response(format!("OpenAI SSE event is invalid JSON: {error}"))
    })?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_response("OpenAI SSE event is missing its type"))?;
    if event
        .event
        .as_deref()
        .is_some_and(|wire| wire != event_type)
    {
        return Err(ProtocolError::invalid_response(
            "OpenAI SSE event name and data type disagree",
        ));
    }
    match event_type {
        "response.output_text.delta" => Ok(vec![ProtocolEvent::Delta(json!({
            "type": "text",
            "text": required_string(&value, "delta", "OpenAI text delta")?,
            "output_index": value.get("output_index"),
            "content_index": value.get("content_index")
        }))]),
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            Ok(vec![ProtocolEvent::Delta(json!({
                "type": "thinking",
                "text": required_string(&value, "delta", "OpenAI reasoning delta")?,
                "provider_state": {"provider": OPENAI_PROVIDER_NAMESPACE, "value": value}
            }))])
        }
        "response.function_call_arguments.delta" => Ok(vec![ProtocolEvent::Delta(json!({
            "type": "tool_arguments",
            "call_id": value.get("call_id"),
            "item_id": value.get("item_id"),
            "delta": required_string(&value, "delta", "OpenAI tool arguments delta")?
        }))]),
        "response.image_generation_call.partial_image" => {
            Ok(vec![ProtocolEvent::Progress(json!({
                "type": "image_partial",
                "item_id": value.get("item_id"),
                "partial_image_index": value.get("partial_image_index"),
                "provider_state": {"provider": OPENAI_PROVIDER_NAMESPACE, "value": value}
            }))])
        }
        "response.completed" => {
            let response = value.get("response").ok_or_else(|| {
                ProtocolError::invalid_response("OpenAI completed event is missing response")
            })?;
            Ok(vec![ProtocolEvent::Final(decode_response_object(
                response,
            )?)])
        }
        "response.incomplete" => {
            let response = value.get("response").ok_or_else(|| {
                ProtocolError::invalid_response("OpenAI incomplete event is missing response")
            })?;
            Ok(vec![ProtocolEvent::Final(decode_response_object(
                response,
            )?)])
        }
        "response.failed" | "error" => {
            Err(response_failure(value.get("response").unwrap_or(&value)))
        }
        _ => Ok(vec![ProtocolEvent::Delta(json!({
            "provider_state": {"provider": OPENAI_PROVIDER_NAMESPACE, "value": value}
        }))]),
    }
}

fn require_final_event(events: &[ProtocolEvent]) -> ProtocolResultValue<()> {
    if events
        .iter()
        .filter(|event| matches!(event, ProtocolEvent::Final(_)))
        .count()
        != 1
    {
        return Err(ProtocolError::invalid_response(
            "OpenAI Responses stream must contain exactly one final event",
        ));
    }
    if !matches!(events.last(), Some(ProtocolEvent::Final(_))) {
        return Err(ProtocolError::invalid_response(
            "OpenAI Responses stream emitted data after its final event",
        ));
    }
    Ok(())
}

fn response_failure(value: &Value) -> ProtocolError {
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
        })
        .unwrap_or("OpenAI response failed");
    ProtocolError::new(ProtocolErrorKind::Transport, message)
}

fn provider_model_id(call: &CodecCall<'_>) -> ProtocolResultValue<String> {
    required_parameter(&call.input.resolved_parameters, "provider_model_id")
}

fn required_parameter(
    parameters: &BTreeMap<String, Value>,
    name: &str,
) -> ProtocolResultValue<String> {
    parameters
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid_request(format!("missing resolved `{name}`")))
}

fn required_string(value: &Value, name: &str, label: &str) -> ProtocolResultValue<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid_response(format!("{label} is missing `{name}`")))
}

fn insert_optional(body: &mut Map<String, Value>, name: &str, value: Option<Value>) {
    if let Some(value) = value {
        body.insert(name.to_string(), value);
    }
}

fn endpoint(base_url: &str, path: &str) -> ProtocolResultValue<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProtocolError::invalid_configuration("OpenAI base URL is invalid"))?;
    let base_path = url.path().trim_end_matches('/');
    let prefix = if base_path.is_empty() {
        "/v1"
    } else {
        base_path
    };
    url.set_path(&format!("{prefix}/{}", path.trim_start_matches('/')));
    Ok(url.to_string())
}

fn json_request(
    call: &CodecCall<'_>,
    method: Method,
    path: &str,
    body: Value,
) -> ProtocolResultValue<HttpRequest> {
    call.context.validate()?;
    let mut request = HttpRequest::new(method, endpoint(&call.context.base_url, path)?);
    request.body = HttpBody::Json(body);
    finish_request(&mut request, call.context)
}

fn finish_request(
    request: &mut HttpRequest,
    context: &super::CodecContext,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let credential = context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "OpenAI operation requires a resolved Bearer credential",
        )
    })?;
    if credential.audit().kind != CredentialKind::Bearer {
        return Err(ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "OpenAI operation requires a Bearer credential",
        ));
    }
    credential.apply(&mut request.headers)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request.clone())
}

fn ensure_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    Err(openai_http_error(
        response.status,
        &response.body,
        &response.request_id,
        response.retry_after,
    ))
}

fn openai_http_error(
    status: StatusCode,
    body: &[u8],
    request_id: &str,
    retry_after: Option<Duration>,
) -> ProtocolError {
    let value: Option<Value> = serde_json::from_slice(body).ok();
    let provider_type = value
        .as_ref()
        .and_then(|value| value.pointer("/error/type"))
        .and_then(Value::as_str);
    let provider_code = value
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .and_then(Value::as_str);
    let message = value
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("OpenAI request failed");
    let label = provider_code.or(provider_type).unwrap_or("http_error");
    ProtocolError::new(
        http_error_kind(status),
        format!("OpenAI {label}: {message}"),
    )
    .with_request_id(Some(request_id.to_string()))
    .with_retry_after(retry_after)
}

fn http_error_kind(status: StatusCode) -> ProtocolErrorKind {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
        StatusCode::REQUEST_TIMEOUT => ProtocolErrorKind::Timeout,
        StatusCode::TOO_MANY_REQUESTS => ProtocolErrorKind::Transport,
        status if status.is_server_error() => ProtocolErrorKind::Transport,
        _ => ProtocolErrorKind::InvalidRequest,
    }
}

fn is_sse(value: Option<&HeaderValue>) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"))
}

fn encode_input_image(
    source: &PublicResourceRef,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Value> {
    Ok(json!({
        "type": "input_image",
        "image_url": resource_data_or_url(source, call)?
    }))
}

fn encode_input_file(
    source: &PublicResourceRef,
    title: Option<&str>,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Value> {
    match source {
        PublicResourceRef::Url { url, .. } => Ok(json!({
            "type": "input_file",
            "file_url": url,
            "filename": title
        })),
        _ => Ok(json!({
            "type": "input_file",
            "file_data": resource_data_or_url(source, call)?,
            "filename": title.unwrap_or("document")
        })),
    }
}

fn resource_data_or_url(
    source: &PublicResourceRef,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<String> {
    match source {
        PublicResourceRef::Url { url, .. } => Ok(url.clone()),
        PublicResourceRef::Base64 { mime, data_base64 } => {
            STANDARD
                .decode(data_base64)
                .map_err(|_| ProtocolError::invalid_request("resource contains invalid base64"))?;
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        PublicResourceRef::NamedObject { .. } => {
            let resource = call.context.materialized_resource(source)?;
            Ok(format!(
                "data:{};base64,{}",
                resource.mime,
                STANDARD.encode(&resource.bytes)
            ))
        }
    }
}

fn multipart_resource(
    source: &PublicResourceRef,
    context: &super::CodecContext,
    default_name: &str,
) -> ProtocolResultValue<MaterializedResource> {
    let mut resource = context.materialized_resource(source)?.clone();
    if resource.file_name.is_none() {
        resource.file_name = Some(default_name.to_string());
    }
    Ok(resource)
}

fn image_format(media_type: &str) -> ProtocolResultValue<&'static str> {
    match media_type {
        "image/png" => Ok("png"),
        "image/jpeg" => Ok("jpeg"),
        "image/webp" => Ok("webp"),
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI image operation does not support the requested media type",
        )),
    }
}

fn image_mime(format: &str) -> ProtocolResultValue<&'static str> {
    match format {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        _ => Err(ProtocolError::invalid_response(
            "OpenAI image result contains an unknown output format",
        )),
    }
}

#[derive(Clone)]
struct OpenAiEmbeddingCodec {
    descriptor: OperationDescriptor,
}

impl OpenAiEmbeddingCodec {
    fn new(descriptor: OperationDescriptor) -> Self {
        Self { descriptor }
    }
}

#[async_trait]
impl OperationCodec for OpenAiEmbeddingCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::EmbeddingText
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let AiccCall::EmbeddingText(request) = &call.input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "OpenAI embeddings codec received the wrong canonical request",
            ));
        };
        require_parameter_subset(&call.input.resolved_parameters, &[], "embeddings")?;
        if request.items.is_empty() {
            return Err(ProtocolError::invalid_request(
                "OpenAI embeddings input must not be empty",
            ));
        }
        if request.chunking.is_some()
            || request.embedding_space_id.is_some()
            || request.normalize == Some(false)
            || request
                .prefer_artifact
                .as_ref()
                .is_some_and(|value| value == &json!(true))
        {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenAI embeddings received an unsupported canonical transform",
            ));
        }
        let input = request
            .items
            .iter()
            .map(|item| embedding_input(item, call))
            .collect::<ProtocolResultValue<Vec<_>>>()?;
        let mut body = Map::from_iter([
            ("model".to_string(), json!(provider_model_id(call)?)),
            ("input".to_string(), Value::Array(input)),
            ("encoding_format".to_string(), json!("float")),
        ]);
        insert_optional(&mut body, "dimensions", request.dimensions.map(Value::from));
        json_request(call, Method::POST, "embeddings", Value::Object(body))
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
            ProtocolError::invalid_response("OpenAI embeddings response is missing data")
        })?;
        let model = required_string(&value, "model", "OpenAI embeddings response")?;
        let mut normalized = Vec::with_capacity(data.len());
        for item in data {
            let index = item.get("index").and_then(Value::as_u64).ok_or_else(|| {
                ProtocolError::invalid_response("OpenAI embedding index is invalid")
            })?;
            let embedding = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ProtocolError::invalid_response("OpenAI embedding vector is invalid")
                })?;
            if embedding.is_empty() || embedding.iter().any(|value| value.as_f64().is_none()) {
                return Err(ProtocolError::invalid_response(
                    "OpenAI embedding vector must contain finite numbers",
                ));
            }
            normalized.push(json!({
                "index": index,
                "id": null,
                "embedding": embedding,
                "embedding_space_id": format!(
                    "{model}:{}:cosine:normalized:v1",
                    embedding.len()
                )
            }));
        }
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"data": normalized}),
            usage: decode_embedding_usage(value.get("usage"))?,
            artifacts: Vec::new(),
        }))
    }
}

fn embedding_input(item: &EmbeddingTextItem, call: &CodecCall<'_>) -> ProtocolResultValue<Value> {
    match item {
        EmbeddingTextItem::Text { text, .. } => Ok(json!(text)),
        EmbeddingTextItem::Resource { resource, .. } => {
            let materialized = multipart_resource(resource, call.context, "embedding-input.txt")?;
            if !materialized.mime.starts_with("text/") && materialized.mime != "application/json" {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "OpenAI text embeddings require a textual resource",
                ));
            }
            let text = std::str::from_utf8(&materialized.bytes).map_err(|_| {
                ProtocolError::invalid_request("embedding resource is not valid UTF-8")
            })?;
            Ok(json!(text))
        }
    }
}

fn decode_embedding_usage(value: Option<&Value>) -> ProtocolResultValue<Option<AiUsage>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let input_tokens = value
        .get("prompt_tokens")
        .or_else(|| value.get("input_tokens"))
        .and_then(Value::as_u64);
    let total_tokens = value.get("total_tokens").and_then(Value::as_u64);
    Ok(Some(AiUsage {
        input_tokens,
        output_tokens: None,
        total_tokens,
        request_units: None,
    }))
}

#[derive(Clone)]
struct OpenAiImageCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl OpenAiImageCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl OperationCodec for OpenAiImageCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        match (&call.input.canonical_request, self.api_type) {
            (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => {
                encode_image_generation(request, call)
            }
            (AiccCall::ImageToImage(request), ApiType::ImageImageToImage) => {
                encode_image_edit(request, None, call)
            }
            (AiccCall::ImageInpaint(request), ApiType::ImageInpaint) => {
                encode_image_inpaint(request, call)
            }
            _ => Err(ProtocolError::invalid_request(
                "OpenAI Images codec received the wrong canonical request",
            )),
        }
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let images = decode_images(&value)?;
        let artifacts = images
            .iter()
            .enumerate()
            .map(|(index, image)| AiArtifact {
                name: format!("image-{index}"),
                resource: image.clone(),
                mime: resource_mime(image).map(str::to_string),
                metadata: None,
            })
            .collect();
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"images": images, "provider_states": []}),
            usage: decode_usage(value.get("usage"))?,
            artifacts,
        }))
    }
}

fn encode_image_generation(
    request: &TextToImageInvokeRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<HttpRequest> {
    require_parameter_subset(&call.input.resolved_parameters, &[], "Images generation")?;
    if request.negative_prompt.is_some() || request.aspect_ratio.is_some() || request.seed.is_some()
    {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI Images generation received an unsupported hard parameter",
        ));
    }
    let mut body = Map::from_iter([
        ("model".to_string(), json!(provider_model_id(call)?)),
        ("prompt".to_string(), json!(request.prompt)),
    ]);
    insert_optional(&mut body, "n", request.n.map(Value::from));
    insert_optional(&mut body, "size", request.size.as_ref().map(|v| json!(v)));
    insert_optional(
        &mut body,
        "quality",
        request.quality.as_ref().map(|v| json!(v)),
    );
    insert_optional(&mut body, "style", request.style.as_ref().map(|v| json!(v)));
    if let Some(media_type) = request
        .output
        .as_ref()
        .and_then(|output| output.media_type.as_deref())
    {
        body.insert(
            "output_format".to_string(),
            json!(image_format(media_type)?),
        );
    }
    json_request(
        call,
        Method::POST,
        "images/generations",
        Value::Object(body),
    )
}

fn encode_image_edit(
    request: &ImageToImageRequest,
    mask: Option<&PublicResourceRef>,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<HttpRequest> {
    require_parameter_subset(&call.input.resolved_parameters, &[], "Images edit")?;
    if request.images.is_empty() || request.strength.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI Images edit requires images and does not support strength",
        ));
    }
    let mut body = MultipartBody::new(32, call.context.limits.max_request_bytes)?;
    body.push(MultipartPart::bytes("model", provider_model_id(call)?))?;
    body.push(MultipartPart::bytes("prompt", request.prompt.clone()))?;
    for (index, source) in request.images.iter().enumerate() {
        push_multipart_resource(
            &mut body,
            if request.images.len() == 1 {
                "image"
            } else {
                "image[]"
            },
            source,
            call.context,
            &format!("image-{index}.bin"),
        )?;
    }
    if let Some(mask) = mask {
        push_multipart_resource(&mut body, "mask", mask, call.context, "mask.png")?;
    }
    if let Some(output) = &request.output {
        if let Some(size) = &output.size {
            body.push(MultipartPart::bytes("size", size.clone()))?;
        }
        if let Some(media_type) = &output.media_type {
            body.push(MultipartPart::bytes(
                "output_format",
                image_format(media_type)?.as_bytes().to_vec(),
            ))?;
        }
    }
    multipart_request(call, Method::POST, "images/edits", body)
}

fn encode_image_inpaint(
    request: &ImageInpaintRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<HttpRequest> {
    use buckyos_api::MaskSemantics;
    if !matches!(
        request.mask_semantics,
        None | Some(MaskSemantics::AlphaZeroIsEditArea)
    ) {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI Images mask requires alpha-zero edit semantics",
        ));
    }
    let synthetic = ImageToImageRequest {
        exact_model: request.exact_model.clone(),
        images: vec![request.image.clone()],
        prompt: request.prompt.clone(),
        strength: None,
        output: request.output.clone(),
        idempotency_key: request.idempotency_key.clone(),
        task_options: request.task_options.clone(),
    };
    encode_image_edit(&synthetic, Some(&request.mask), call)
}

fn push_multipart_resource(
    body: &mut MultipartBody,
    name: &str,
    source: &PublicResourceRef,
    context: &super::CodecContext,
    default_name: &str,
) -> ProtocolResultValue<()> {
    let resource = multipart_resource(source, context, default_name)?;
    body.push(MultipartPart::file(
        name,
        resource.bytes,
        resource
            .file_name
            .unwrap_or_else(|| default_name.to_string()),
        resource.mime,
    ))
}

fn multipart_request(
    call: &CodecCall<'_>,
    method: Method,
    path: &str,
    body: MultipartBody,
) -> ProtocolResultValue<HttpRequest> {
    call.context.validate()?;
    let mut request = HttpRequest::new(method, endpoint(&call.context.base_url, path)?);
    request.body = HttpBody::Multipart(body);
    finish_request(&mut request, call.context)
}

fn multipart_request_context(
    context: &super::CodecContext,
    method: Method,
    path: &str,
    body: MultipartBody,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut request = HttpRequest::new(method, endpoint(&context.base_url, path)?);
    request.body = HttpBody::Multipart(body);
    finish_request(&mut request, context)
}

fn decode_images(value: &Value) -> ProtocolResultValue<Vec<PublicResourceRef>> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::invalid_response("OpenAI Images response is missing data"))?;
    if data.is_empty() {
        return Err(ProtocolError::invalid_response(
            "OpenAI Images response contains no images",
        ));
    }
    data.iter()
        .map(|item| {
            if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
                STANDARD.decode(encoded).map_err(|_| {
                    ProtocolError::invalid_response("OpenAI image contains invalid base64")
                })?;
                let mime = item
                    .get("output_format")
                    .and_then(Value::as_str)
                    .map(image_mime)
                    .transpose()?
                    .unwrap_or("image/png");
                Ok(PublicResourceRef::base64(
                    mime.to_string(),
                    encoded.to_string(),
                ))
            } else if let Some(url) = item.get("url").and_then(Value::as_str) {
                Ok(PublicResourceRef::url(url.to_string(), None))
            } else {
                Err(ProtocolError::invalid_response(
                    "OpenAI image result has neither b64_json nor url",
                ))
            }
        })
        .collect()
}

fn resource_mime(resource: &PublicResourceRef) -> Option<&str> {
    match resource {
        PublicResourceRef::Base64 { mime, .. } => Some(mime),
        PublicResourceRef::Url { mime_hint, .. } => mime_hint.as_deref(),
        PublicResourceRef::NamedObject { .. } => None,
    }
}

#[derive(Clone)]
struct OpenAiAudioCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl OpenAiAudioCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl OperationCodec for OpenAiAudioCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        match (&call.input.canonical_request, self.api_type) {
            (AiccCall::AudioTextToSpeech(request), ApiType::AudioTextToSpeech) => {
                encode_audio_speech(request, call)
            }
            (AiccCall::AudioSpeechRecognition(request), ApiType::AudioSpeechRecognition) => {
                encode_audio_transcription(request, call)
            }
            _ => Err(ProtocolError::invalid_request(
                "OpenAI Audio codec received the wrong canonical request",
            )),
        }
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        match self.api_type {
            ApiType::AudioTextToSpeech => decode_audio_speech(response),
            ApiType::AudioSpeechRecognition => decode_audio_transcription(response),
            _ => Err(ProtocolError::invalid_response(
                "OpenAI Audio codec has an invalid API type",
            )),
        }
    }
}

fn encode_audio_speech(
    request: &AudioTextToSpeechRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<HttpRequest> {
    require_parameter_subset(&call.input.resolved_parameters, &["instructions"], "speech")?;
    let voice = request.voice.voice_id.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI speech requires a resolved voice_id",
        )
    })?;
    if request.voice.speaker_similarity_required
        || request.voice.gender.is_some()
        || request.voice.language.is_some()
    {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI speech cannot satisfy the requested voice contract",
        ));
    }
    let mut body = Map::from_iter([
        ("model".to_string(), json!(provider_model_id(call)?)),
        ("input".to_string(), json!(request.text)),
        ("voice".to_string(), json!(voice)),
    ]);
    insert_optional(&mut body, "speed", request.speed.map(Value::from));
    if let Some(instructions) = call.input.resolved_parameters.get("instructions") {
        if !instructions.is_string() {
            return Err(ProtocolError::invalid_request(
                "resolved OpenAI speech instructions must be a string",
            ));
        }
        body.insert("instructions".to_string(), instructions.clone());
    } else if let Some(style) = &request.voice.style {
        body.insert("instructions".to_string(), json!(style));
    }
    if let Some(output) = &request.output {
        if output.sample_rate.is_some() {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenAI speech does not accept an arbitrary sample rate",
            ));
        }
        if let Some(media_type) = &output.media_type {
            body.insert(
                "response_format".to_string(),
                json!(audio_format(media_type)?),
            );
        }
    }
    json_request(call, Method::POST, "audio/speech", Value::Object(body))
}

fn decode_audio_speech(response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
    if response.body.is_empty() {
        return Err(ProtocolError::invalid_response(
            "OpenAI speech response is empty",
        ));
    }
    let mime = response
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("audio/mpeg")
        .to_string();
    let resource = PublicResourceRef::base64(mime.clone(), STANDARD.encode(&response.body));
    Ok(ProtocolExecution::Immediate(ProtocolOutput {
        value: json!({"audio": resource}),
        usage: None,
        artifacts: vec![AiArtifact {
            name: "speech".to_string(),
            resource,
            mime: Some(mime),
            metadata: None,
        }],
    }))
}

fn encode_audio_transcription(
    request: &AudioSpeechRecognitionRequest,
    call: &CodecCall<'_>,
) -> ProtocolResultValue<HttpRequest> {
    require_parameter_subset(&call.input.resolved_parameters, &[], "transcription")?;
    if request.diarization == Some(true) && request.timestamps.is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI diarized transcription does not support timestamp granularities",
        ));
    }
    if request
        .output_formats
        .as_ref()
        .is_some_and(|formats| formats.iter().any(|format| format != "json"))
    {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI transcription codec only exposes its canonical JSON result",
        ));
    }
    let resource = multipart_resource(&request.audio, call.context, "audio-input.bin")?;
    let mut body = MultipartBody::new(16, call.context.limits.max_request_bytes)?;
    body.push(MultipartPart::file(
        "file",
        resource.bytes,
        resource
            .file_name
            .unwrap_or_else(|| "audio-input.bin".to_string()),
        resource.mime,
    ))?;
    body.push(MultipartPart::bytes("model", provider_model_id(call)?))?;
    if let Some(language) = &request.language {
        body.push(MultipartPart::bytes("language", language.clone()))?;
    }
    let response_format = if request.diarization == Some(true) {
        "diarized_json"
    } else if request.timestamps.is_some() {
        "verbose_json"
    } else {
        "json"
    };
    body.push(MultipartPart::bytes("response_format", response_format))?;
    if request.diarization == Some(true) {
        body.push(MultipartPart::bytes("chunking_strategy", "auto"))?;
    }
    if let Some(timestamps) = &request.timestamps {
        let granularities = match timestamps.as_str() {
            "segment" => vec!["segment"],
            "word" => vec!["word"],
            "both" => vec!["segment", "word"],
            _ => {
                return Err(ProtocolError::invalid_request(
                    "OpenAI transcription timestamps must be segment, word, or both",
                ))
            }
        };
        for granularity in granularities {
            body.push(MultipartPart::bytes(
                "timestamp_granularities[]",
                granularity,
            ))?;
        }
    }
    multipart_request(call, Method::POST, "audio/transcriptions", body)
}

fn decode_audio_transcription(response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_response("OpenAI transcription is missing text"))?;
    let segments = value
        .get("segments")
        .and_then(Value::as_array)
        .map(|segments| {
            segments
                .iter()
                .enumerate()
                .map(|(index, segment)| {
                    Ok(json!({
                        "id": segment.get("id").map(value_string).unwrap_or_else(|| index.to_string()),
                        "start_seconds": segment.get("start").and_then(Value::as_f64).unwrap_or(0.0),
                        "end_seconds": segment.get("end").and_then(Value::as_f64).unwrap_or(0.0),
                        "text": segment.get("text").and_then(Value::as_str).unwrap_or(""),
                        "speaker": segment.get("speaker").cloned().unwrap_or(Value::Null),
                        "confidence": segment.get("confidence").cloned().unwrap_or(Value::Null)
                    }))
                })
                .collect::<ProtocolResultValue<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let usage = match value
        .get("usage")
        .and_then(|usage| usage.get("type"))
        .and_then(Value::as_str)
    {
        Some("tokens") => decode_usage(value.get("usage"))?,
        Some("duration") => Some(AiUsage::request_units(1)),
        _ => None,
    };
    Ok(ProtocolExecution::Immediate(ProtocolOutput {
        value: json!({
            "text": text,
            "segments": segments,
            "artifacts": {},
            "diagnostic": {
                "language": value.get("language"),
                "duration": value.get("duration")
            }
        }),
        usage,
        artifacts: Vec::new(),
    }))
}

fn value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn audio_format(media_type: &str) -> ProtocolResultValue<&'static str> {
    match media_type {
        "audio/mpeg" => Ok("mp3"),
        "audio/opus" => Ok("opus"),
        "audio/aac" => Ok("aac"),
        "audio/flac" => Ok("flac"),
        "audio/wav" | "audio/x-wav" => Ok("wav"),
        "audio/pcm" | "audio/L16" => Ok("pcm"),
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "OpenAI speech does not support the requested media type",
        )),
    }
}

#[derive(Clone)]
struct OpenAiVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl OpenAiVideoCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl NativeTaskCodec for OpenAiVideoCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn operations(&self) -> BTreeSet<NativeTaskOperation> {
        BTreeSet::from([
            NativeTaskOperation::Submit,
            NativeTaskOperation::Status,
            NativeTaskOperation::Result,
            NativeTaskOperation::Cancel,
        ])
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        match input.operation {
            NativeTaskOperation::Submit => encode_video_submit(input, self.api_type),
            NativeTaskOperation::Status => native_request(input, Method::GET, ""),
            NativeTaskOperation::Result => native_request(input, Method::GET, "/content"),
            NativeTaskOperation::Cancel => native_request(input, Method::DELETE, ""),
        }
    }

    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        ensure_success(&response)?;
        match operation {
            NativeTaskOperation::Submit => decode_video_submit(response),
            NativeTaskOperation::Status => decode_video_status(response),
            NativeTaskOperation::Result => decode_video_result(response),
            NativeTaskOperation::Cancel => decode_video_cancel(response),
        }
    }
}

fn encode_video_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    require_parameter_subset(input.resolved_parameters, &["seconds", "size"], "videos")?;
    if input
        .resolved_parameters
        .get("seconds")
        .is_some_and(|value| !value.is_string() && !value.is_number())
        || input
            .resolved_parameters
            .get("size")
            .is_some_and(|value| !value.is_string())
    {
        return Err(ProtocolError::invalid_request(
            "resolved OpenAI videos seconds or size has an invalid type",
        ));
    }
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("OpenAI video submit requires a canonical request")
    })?;
    let (prompt, duration_seconds, resolution, image) =
        match (&codec_input.canonical_request, api_type) {
            (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => {
                if request.aspect_ratio.is_some()
                    || request.generate_audio == Some(true)
                    || request.seed.is_some()
                    || request
                        .output
                        .as_ref()
                        .is_some_and(|output| output.fps.is_some())
                {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::UnsupportedOperation,
                        "OpenAI video generation received an unsupported hard parameter",
                    ));
                }
                (
                    request.prompt.clone(),
                    request.duration_seconds,
                    request.resolution.clone(),
                    None,
                )
            }
            (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => {
                if request.aspect_ratio.is_some() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::UnsupportedOperation,
                        "OpenAI image-to-video requires a resolved size instead of aspect_ratio",
                    ));
                }
                (
                    request.prompt.clone(),
                    request.duration_seconds,
                    request.resolution.clone(),
                    Some(&request.image),
                )
            }
            _ => {
                return Err(ProtocolError::invalid_request(
                    "OpenAI video codec received the wrong canonical request",
                ))
            }
        };
    let model = required_parameter(input.resolved_parameters, "provider_model_id")?;
    let mut body = MultipartBody::new(16, input.context.limits.max_request_bytes)?;
    body.push(MultipartPart::bytes("model", model))?;
    body.push(MultipartPart::bytes("prompt", prompt))?;
    if let Some(seconds) = input
        .resolved_parameters
        .get("seconds")
        .map(value_string)
        .or_else(|| duration_seconds.map(format_duration_seconds))
    {
        body.push(MultipartPart::bytes("seconds", seconds))?;
    }
    if let Some(size) = input
        .resolved_parameters
        .get("size")
        .map(value_string)
        .or(resolution)
    {
        body.push(MultipartPart::bytes("size", size))?;
    }
    if let Some(image) = image {
        let resource = multipart_resource(image, input.context, "input-reference.bin")?;
        body.push(MultipartPart::file(
            "input_reference",
            resource.bytes,
            resource
                .file_name
                .unwrap_or_else(|| "input-reference.bin".to_string()),
            resource.mime,
        ))?;
    }
    multipart_request_context(input.context, Method::POST, "videos", body)
}

fn format_duration_seconds(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn native_request(
    input: &NativeTaskInput<'_>,
    method: Method,
    suffix: &str,
) -> ProtocolResultValue<HttpRequest> {
    let remote_task_id = input.remote_task_id.ok_or_else(|| {
        ProtocolError::invalid_request("OpenAI video lifecycle requires a task ID")
    })?;
    validate_path_id(remote_task_id)?;
    let mut request = HttpRequest::new(
        method,
        endpoint(
            &input.context.base_url,
            &format!("videos/{remote_task_id}{suffix}"),
        )?,
    );
    finish_request(&mut request, input.context)
}

fn validate_path_id(value: &str) -> ProtocolResultValue<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ProtocolError::invalid_request(
            "OpenAI video task ID contains invalid path characters",
        ));
    }
    Ok(())
}

fn decode_video_submit(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let id = required_string(&value, "id", "OpenAI video job")?;
    let mut handle = NativeTaskHandle::new(id)?;
    handle.state = decode_video_state(&value)?;
    handle.poll_after = response.retry_after.or(Some(Duration::from_secs(1)));
    handle.cancel_supported = true;
    Ok(NativeTaskOutput::Submitted(handle))
}

fn decode_video_status(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    Ok(NativeTaskOutput::Status {
        state: decode_video_state(&value)?,
        retry_after: response.retry_after,
    })
}

fn decode_video_state(value: &Value) -> ProtocolResultValue<NativeTaskState> {
    match value.get("status").and_then(Value::as_str) {
        Some("queued") => Ok(NativeTaskState::Queued),
        Some("in_progress") | Some("processing") | Some("running") => Ok(NativeTaskState::Running),
        Some("completed") | Some("succeeded") => Ok(NativeTaskState::Succeeded),
        Some("failed") => Ok(NativeTaskState::Failed),
        Some("cancelled") | Some("canceled") | Some("deleted") => Ok(NativeTaskState::Cancelled),
        _ => Err(ProtocolError::invalid_response(
            "OpenAI video job contains an unknown status",
        )),
    }
}

fn decode_video_result(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    if response.body.is_empty() {
        return Err(ProtocolError::invalid_response(
            "OpenAI video content is empty",
        ));
    }
    let mime = response
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("video/mp4")
        .to_string();
    let resource = PublicResourceRef::base64(mime.clone(), STANDARD.encode(response.body));
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: json!({"video": resource}),
        usage: None,
        artifacts: vec![AiArtifact {
            name: "video".to_string(),
            resource,
            mime: Some(mime),
            metadata: None,
        }],
    }))
}

fn decode_video_cancel(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let accepted = value
        .get("deleted")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            value
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "cancelled" | "canceled" | "deleted"))
        });
    Ok(NativeTaskOutput::Cancelled { accepted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, GoldenBody, ProtocolContractHarness,
        ResolvedCredential,
    };
    use buckyos_api::{
        AiOutputOptions, AiToolSpec, EmbeddingTextRequest, MaskSemantics, VideoImageToVideoRequest,
        VideoTextToVideoRequest, VoiceSpec,
    };
    use bytes::Bytes;
    use futures_util::{stream, StreamExt};
    use reqwest::header::{HeaderMap, HeaderValue};
    use std::time::{Duration, UNIX_EPOCH};

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://api.openai.com/v1".to_string(),
            credential: Some(
                ResolvedCredential::bearer("secret://openai/key", "top-secret").unwrap(),
            ),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
                max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            },
        }
    }

    fn context_with_resource(
        source: &PublicResourceRef,
        bytes: &'static [u8],
        mime: &str,
        file_name: Option<&str>,
    ) -> CodecContext {
        let mut context = context();
        context.resources.insert(
            crate::resource::ResourceKey::from_ref(source).into_string(),
            MaterializedResource::new(
                Bytes::from_static(bytes),
                mime,
                file_name.map(str::to_string),
            )
            .unwrap(),
        );
        context
    }

    fn input(call: AiccCall) -> CodecInput {
        CodecInput {
            canonical_request: call,
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                json!("openai-test-model"),
            )]),
        }
    }

    fn registry() -> CodecRegistry {
        let (descriptor, codecs) = openai_responses_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, codecs).unwrap();
        registry
    }

    #[test]
    fn registers_seven_operations_without_derived_provider_branches() {
        let registry = registry();
        let descriptor = registry.adapter(OPENAI_RESPONSES_ADAPTER_ID).unwrap();
        assert_eq!(descriptor.protocol_family_id, "openai");
        assert_eq!(descriptor.base_adapter_id, None);
        assert_eq!(descriptor.operations.len(), 7);
        assert!(registry
            .operation_descriptor(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
            )
            .is_ok());
        assert!(registry
            .native_task_codec(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_VIDEOS_OPERATION_ID,
                ApiType::VideoTextToVideo,
            )
            .is_ok());
    }

    #[test]
    fn encodes_responses_messages_tools_structured_output_and_reasoning() {
        let mut request = LlmChatInvokeRequest::new(
            "ignored@instance",
            vec![
                AiMessage::text(AiRole::Developer, "be precise"),
                AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::Text {
                            text: "weather?".to_string(),
                        },
                        AiContent::Image {
                            source: PublicResourceRef::url(
                                "https://example.test/image.png".to_string(),
                                Some("image/png".to_string()),
                            ),
                        },
                    ],
                ),
                AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::Thinking {
                            summary: Some("ignored neutral summary".to_string()),
                            text: None,
                            provider_metadata: None,
                        },
                        AiContent::ProviderState {
                            provider: "openai".to_string(),
                            value: json!({"type":"reasoning","id":"rs_1","encrypted_content":"opaque"}),
                        },
                        AiContent::ToolUse {
                            call_id: "call_1".to_string(),
                            name: "weather".to_string(),
                            args: HashMap::from([("city".to_string(), json!("Paris"))]),
                        },
                    ],
                ),
                AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::Text {
                            text: "cannot answer fully".to_string(),
                        },
                        AiContent::ProviderState {
                            provider: "openai".to_string(),
                            value: json!({"type":"refusal","refusal":"restricted"}),
                        },
                    ],
                ),
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::ToolResult {
                        call_id: "call_1".to_string(),
                        content: vec![AiToolResultContent::text("sunny")],
                        is_error: false,
                    }],
                ),
            ],
        );
        request.tools.push(AiToolSpec {
            tool_type: "function".to_string(),
            name: "weather".to_string(),
            description: "Weather lookup".to_string(),
            args_json_schema: json!({"type":"object","properties":{"city":{"type":"string"}}}),
            output_schema: None,
        });
        request.response_format = Some(buckyos_api::LlmResponseFormat::json_schema(
            Some("answer".to_string()),
            json!({"type":"object"}),
            Some(true),
        ));
        request.max_output_tokens = Some(256);
        let mut input = input(AiccCall::ChatCompletionsCreate(request));
        input.resolved_parameters.insert(
            "reasoning".to_string(),
            json!({"effort":"high","summary":"auto"}),
        );
        input
            .resolved_parameters
            .insert("stream".to_string(), json!(true));
        let request = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                &input,
                &context(),
            )
            .unwrap();
        let golden = ProtocolContractHarness::default()
            .request(&request)
            .unwrap();
        assert_eq!(golden.url, "https://api.openai.com/v1/responses");
        assert_eq!(golden.headers["authorization"], "[REDACTED]");
        let GoldenBody::Json(ref body) = golden.body else {
            panic!("expected JSON request")
        };
        assert_eq!(body["model"], "openai-test-model");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["stream"], true);
        let inputs = body["input"].as_array().unwrap();
        assert!(inputs.iter().any(|item| item["type"] == "reasoning"));
        assert!(inputs.iter().any(|item| item["type"] == "function_call"));
        assert!(inputs.iter().any(|item| {
            item["type"] == "message"
                && item["content"]
                    .as_array()
                    .is_some_and(|content| content.iter().any(|part| part["type"] == "refusal"))
        }));
        assert!(inputs
            .iter()
            .any(|item| item["type"] == "function_call_output"));
        ProtocolContractHarness::default()
            .assert_no_secrets(&format!("{request:?} {golden:?}"), &["top-secret"])
            .unwrap();
    }

    #[tokio::test]
    async fn decodes_ordered_response_tools_reasoning_images_usage_and_provider_state() {
        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[("content-type", "application/json")],
                Bytes::from_static(
                    br#"{
                      "id":"resp_1","status":"completed",
                      "output":[
                        {"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"why"}],"encrypted_content":"opaque"},
                        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer","annotations":[]}]},
                        {"type":"function_call","call_id":"call_1","name":"weather","arguments":"{\"city\":\"Paris\"}"},
                        {"type":"image_generation_call","id":"ig_1","status":"completed","result":"aW1hZ2U=","output_format":"png"},
                        {"type":"web_search_call","id":"ws_1","status":"completed"}
                      ],
                      "usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12}
                    }"#,
                ),
                "request-1",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry()
            .decode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected immediate result")
        };
        assert_eq!(output.usage.unwrap().total_tokens, Some(12));
        assert_eq!(output.value["message"]["content"][0]["type"], "thinking");
        assert_eq!(
            output.value["message"]["content"][1]["type"],
            "provider_state"
        );
        assert_eq!(output.value["message"]["content"][2]["text"], "answer");
        assert_eq!(output.value["tool_calls"][0]["args"]["city"], "Paris");
        assert_eq!(output.value["images"][0]["kind"], "base64");
        assert_eq!(output.artifacts[0].mime.as_deref(), Some("image/png"));
    }

    #[tokio::test]
    async fn rejects_failed_missing_and_invalid_responses_image_results() {
        for output_item in [
            json!({"type":"image_generation_call","status":"failed"}),
            json!({"type":"image_generation_call","status":"completed"}),
            json!({"type":"image_generation_call","status":"completed","result":"%%%"}),
        ] {
            let response = ProtocolContractHarness::default()
                .response(
                    StatusCode::OK,
                    &[],
                    Bytes::from(
                        serde_json::to_vec(&json!({
                            "id": "resp_image_error",
                            "status": "completed",
                            "output": [output_item]
                        }))
                        .unwrap(),
                    ),
                    "request-image-error",
                    UNIX_EPOCH,
                )
                .unwrap();
            let error = registry()
                .decode(
                    OPENAI_RESPONSES_ADAPTER_ID,
                    OPENAI_RESPONSES_OPERATION_ID,
                    ApiType::ImageTextToImage,
                    response,
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind, ProtocolErrorKind::InvalidResponse);
            let public_error: buckyos_api::AiccError = error.into();
            assert_eq!(public_error.code, buckyos_api::AiccErrorCode::ProviderError);
        }
    }

    #[tokio::test]
    async fn decodes_fragmented_responses_sse_and_rejects_missing_final() {
        let completed = concat!(
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hel\",\"output_index\":0,\"content_index\":0}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{}\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]}],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
        );
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let split = completed.len() / 2;
        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers: headers.clone(),
            body: Box::pin(stream::iter(vec![
                Ok(Bytes::copy_from_slice(&completed.as_bytes()[..split])),
                Ok(Bytes::copy_from_slice(&completed.as_bytes()[split..])),
            ])),
            request_id: "request-stream".to_string(),
            retry_after: None,
        };
        let mut stream = registry()
            .decode_stream(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                response,
            )
            .await
            .unwrap();
        let events = stream.events.by_ref().collect::<Vec<_>>().await;
        assert!(events.iter().all(Result::is_ok));
        assert!(matches!(
            events.last().unwrap().as_ref().unwrap(),
            ProtocolEvent::Final(_)
        ));

        let trailing = format!(
            "{completed}event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"late\"}}\n\n"
        );
        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers: headers.clone(),
            body: Box::pin(stream::iter(vec![Ok(Bytes::from(trailing))])),
            request_id: "request-trailing".to_string(),
            retry_after: None,
        };
        let stream = registry()
            .decode_stream(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                response,
            )
            .await
            .unwrap();
        let events = stream.events.collect::<Vec<_>>().await;
        assert!(matches!(events.first(), Some(Ok(ProtocolEvent::Delta(_)))));
        assert!(events.last().unwrap().is_err());

        let response = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::iter(vec![Ok(Bytes::from_static(
                b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
            ))])),
            request_id: "request-truncated".to_string(),
            retry_after: None,
        };
        let stream = registry()
            .decode_stream(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                response,
            )
            .await
            .unwrap();
        let events = stream.events.collect::<Vec<_>>().await;
        assert!(events.last().unwrap().is_err());
    }

    #[tokio::test]
    async fn maps_openai_http_errors_with_request_id_and_retry_after() {
        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::TOO_MANY_REQUESTS,
                &[("retry-after", "2")],
                Bytes::from_static(
                    br#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"slow down"}}"#,
                ),
                "request-rate",
                UNIX_EPOCH,
            )
            .unwrap();
        let error = registry()
            .decode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                response,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert_eq!(error.request_id.as_deref(), Some("request-rate"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
        assert!(error.message.contains("rate_limit_exceeded"));

        let stream_response = StreamingHttpResponse {
            status: StatusCode::BAD_REQUEST,
            headers: HeaderMap::new(),
            body: Box::pin(stream::iter(vec![Ok(Bytes::from_static(
                br#"{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"context is too long"}}"#,
            ))])),
            request_id: "request-context".to_string(),
            retry_after: None,
        };
        let error = registry()
            .decode_stream(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::Llm,
                stream_response,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidRequest);
        assert_eq!(error.request_id.as_deref(), Some("request-context"));
        assert!(error.message.contains("context_length_exceeded"));
    }

    #[tokio::test]
    async fn embeddings_round_trip_and_reject_non_text_resources() {
        let request = EmbeddingTextRequest::new(
            "ignored@instance",
            vec![
                EmbeddingTextItem::Text {
                    text: "one".to_string(),
                    id: Some("a".to_string()),
                },
                EmbeddingTextItem::Text {
                    text: "two".to_string(),
                    id: Some("b".to_string()),
                },
            ],
        );
        let wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_EMBEDDINGS_OPERATION_ID,
                ApiType::EmbeddingText,
                &input(AiccCall::EmbeddingText(request)),
                &context(),
            )
            .unwrap();
        let GoldenBody::Json(body) = ProtocolContractHarness::default()
            .request(&wire)
            .unwrap()
            .body
        else {
            panic!("expected JSON")
        };
        assert_eq!(body["input"], json!(["one", "two"]));

        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[],
                Bytes::from_static(
                    br#"{"object":"list","model":"text-embedding-test","data":[{"object":"embedding","index":0,"embedding":[0.25,0.75]}],"usage":{"prompt_tokens":3,"total_tokens":3}}"#,
                ),
                "request-embedding",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry()
            .decode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_EMBEDDINGS_OPERATION_ID,
                ApiType::EmbeddingText,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected immediate")
        };
        assert_eq!(output.value["data"][0]["embedding"][1], 0.75);
        assert_eq!(output.usage.unwrap().input_tokens, Some(3));
    }

    #[tokio::test]
    async fn images_generate_and_edit_use_independent_wire_operations() {
        let mut generate = TextToImageInvokeRequest::new("ignored@instance", "a lighthouse");
        generate.size = Some("1024x1024".to_string());
        generate.output = Some(AiOutputOptions {
            media_type: Some("image/png".to_string()),
            ..AiOutputOptions::default()
        });
        let responses_wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::ImageTextToImage,
                &input(AiccCall::ImagesGenerate(generate.clone())),
                &context(),
            )
            .unwrap();
        let GoldenBody::Json(responses_body) = ProtocolContractHarness::default()
            .request(&responses_wire)
            .unwrap()
            .body
        else {
            panic!("expected Responses JSON")
        };
        assert_eq!(responses_body["tools"][0]["type"], "image_generation");
        assert_eq!(responses_body["tools"][0]["action"], "generate");

        let wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_IMAGES_GENERATE_OPERATION_ID,
                ApiType::ImageTextToImage,
                &input(AiccCall::ImagesGenerate(generate)),
                &context(),
            )
            .unwrap();
        assert_eq!(wire.url, "https://api.openai.com/v1/images/generations");

        let image = PublicResourceRef::base64("image/png".to_string(), "aW1hZ2U=".to_string());
        let materialized_context =
            context_with_resource(&image, b"image", "image/png", Some("input.png"));
        let edit = ImageToImageRequest::new(
            "ignored@instance",
            vec![image.clone()],
            "add clouds".to_string(),
        );
        let responses_wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_RESPONSES_OPERATION_ID,
                ApiType::ImageImageToImage,
                &input(AiccCall::ImageToImage(edit.clone())),
                &materialized_context,
            )
            .unwrap();
        let GoldenBody::Json(responses_body) = ProtocolContractHarness::default()
            .request(&responses_wire)
            .unwrap()
            .body
        else {
            panic!("expected Responses JSON")
        };
        assert_eq!(responses_body["tools"][0]["action"], "edit");
        assert_eq!(
            responses_body["input"][0]["content"][1]["type"],
            "input_image"
        );

        let wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_IMAGES_EDIT_OPERATION_ID,
                ApiType::ImageImageToImage,
                &input(AiccCall::ImageToImage(edit)),
                &materialized_context,
            )
            .unwrap();
        let GoldenBody::Multipart(parts) = ProtocolContractHarness::default()
            .request(&wire)
            .unwrap()
            .body
        else {
            panic!("expected multipart")
        };
        assert!(parts.iter().any(|part| part.name == "image"));

        let inpaint = ImageInpaintRequest {
            exact_model: "ignored@instance".to_string(),
            image: image.clone(),
            mask: image,
            prompt: "replace sky".to_string(),
            mask_semantics: Some(MaskSemantics::AlphaZeroIsEditArea),
            output: None,
            idempotency_key: None,
            task_options: None,
        };
        assert!(registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_IMAGES_EDIT_OPERATION_ID,
                ApiType::ImageInpaint,
                &input(AiccCall::ImageInpaint(inpaint)),
                &materialized_context,
            )
            .is_ok());

        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[],
                Bytes::from_static(br#"{"data":[{"b64_json":"aW1hZ2U=","output_format":"png"}]}"#),
                "request-image",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry()
            .decode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_IMAGES_GENERATE_OPERATION_ID,
                ApiType::ImageTextToImage,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected immediate")
        };
        assert_eq!(output.artifacts.len(), 1);
    }

    #[tokio::test]
    async fn audio_speech_and_materialized_url_transcription_map_multipart_and_usage() {
        let speech = AudioTextToSpeechRequest::new(
            "ignored@instance",
            "hello".to_string(),
            VoiceSpec {
                voice_id: Some("alloy".to_string()),
                ..VoiceSpec::default()
            },
        );
        let wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_AUDIO_SPEECH_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                &input(AiccCall::AudioTextToSpeech(speech)),
                &context(),
            )
            .unwrap();
        assert_eq!(wire.url, "https://api.openai.com/v1/audio/speech");
        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[("content-type", "audio/mpeg")],
                Bytes::from_static(b"audio"),
                "request-speech",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry()
            .decode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_AUDIO_SPEECH_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected speech")
        };
        assert_eq!(output.artifacts[0].mime.as_deref(), Some("audio/mpeg"));

        let audio = PublicResourceRef::url(
            "https://download.invalid/audio.wav?credential=must-not-leak".to_string(),
            Some("audio/wav".to_string()),
        );
        let materialized_context =
            context_with_resource(&audio, b"audio", "audio/wav", Some("audio.wav"));
        let mut transcription =
            AudioSpeechRecognitionRequest::new("ignored@instance", audio.clone());
        transcription.timestamps = Some("segment".to_string());
        let error = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &input(AiccCall::AudioSpeechRecognition(transcription.clone())),
                &context(),
            )
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidRequest);
        let rendered_error = format!("{error:?}");
        assert!(!rendered_error.contains("must-not-leak"));
        assert!(!rendered_error.contains("audio"));
        assert!(!rendered_error.contains("top-secret"));
        let wire = registry()
            .encode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &input(AiccCall::AudioSpeechRecognition(transcription)),
                &materialized_context,
            )
            .unwrap();
        let GoldenBody::Multipart(parts) = ProtocolContractHarness::default()
            .request(&wire)
            .unwrap()
            .body
        else {
            panic!("expected multipart")
        };
        assert!(parts.iter().any(|part| part.name == "file"));
        let file = parts.iter().find(|part| part.name == "file").unwrap();
        assert_eq!(file.bytes, b"audio");
        assert_eq!(file.mime.as_deref(), Some("audio/wav"));
        assert_eq!(file.file_name.as_deref(), Some("audio.wav"));
        assert!(parts
            .iter()
            .any(|part| part.name == "response_format" && part.bytes == b"verbose_json"));
        let rendered_request = format!("{wire:?}");
        assert!(!rendered_request.contains("must-not-leak"));
        assert!(!rendered_request.contains("top-secret"));

        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[],
                Bytes::from_static(
                    br#"{"text":"hello","segments":[{"id":0,"start":0.0,"end":1.0,"text":"hello"}],"usage":{"type":"tokens","input_tokens":2,"output_tokens":1,"total_tokens":3}}"#,
                ),
                "request-asr",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry()
            .decode(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected transcription")
        };
        assert_eq!(output.value["segments"][0]["text"], "hello");
        assert_eq!(output.usage.unwrap().total_tokens, Some(3));
    }

    #[tokio::test]
    async fn videos_cover_submit_status_content_cancel_and_validate_task_ids() {
        let registry = registry();
        let mut context = context();
        let parameters = BTreeMap::from([
            ("provider_model_id".to_string(), json!("sora-test")),
            ("seconds".to_string(), json!("8")),
            ("size".to_string(), json!("1280x720")),
        ]);
        let codec_input = input(AiccCall::VideoTextToVideo(VideoTextToVideoRequest::new(
            "ignored@instance",
            "a cat playing piano".to_string(),
        )));
        let submit = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&codec_input),
            resolved_parameters: &parameters,
            context: &context,
        };
        let request = registry
            .encode_native(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_VIDEOS_OPERATION_ID,
                ApiType::VideoTextToVideo,
                &submit,
            )
            .unwrap();
        assert_eq!(request.url, "https://api.openai.com/v1/videos");

        let image = PublicResourceRef::base64("image/png".to_string(), "aW1hZ2U=".to_string());
        context.resources.insert(
            crate::resource::ResourceKey::from_ref(&image).into_string(),
            MaterializedResource::new(
                Bytes::from_static(b"image"),
                "image/png",
                Some("input.png".to_string()),
            )
            .unwrap(),
        );
        let image_codec_input = input(AiccCall::VideoImageToVideo(VideoImageToVideoRequest::new(
            "ignored@instance",
            image,
            "animate the cat".to_string(),
        )));
        let image_submit = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&image_codec_input),
            resolved_parameters: &parameters,
            context: &context,
        };
        let request = registry
            .encode_native(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_VIDEOS_OPERATION_ID,
                ApiType::VideoImageToVideo,
                &image_submit,
            )
            .unwrap();
        let GoldenBody::Multipart(parts) = ProtocolContractHarness::default()
            .request(&request)
            .unwrap()
            .body
        else {
            panic!("expected video multipart")
        };
        assert!(parts
            .iter()
            .any(|part| part.name == "input_reference" && part.bytes == b"image"));

        let response = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[("retry-after", "1")],
                Bytes::from_static(br#"{"id":"video_1","status":"queued"}"#),
                "request-video",
                UNIX_EPOCH,
            )
            .unwrap();
        let NativeTaskOutput::Submitted(handle) = registry
            .decode_native(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_VIDEOS_OPERATION_ID,
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Submit,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected submitted")
        };
        assert_eq!(handle.remote_task_id, "video_1");
        assert_eq!(handle.state, NativeTaskState::Queued);
        assert!(handle.cancel_supported);

        for (operation, method, suffix) in [
            (NativeTaskOperation::Status, Method::GET, ""),
            (NativeTaskOperation::Result, Method::GET, "/content"),
            (NativeTaskOperation::Cancel, Method::DELETE, ""),
        ] {
            let input = NativeTaskInput {
                operation,
                remote_task_id: Some("video_1"),
                codec_input: None,
                resolved_parameters: &parameters,
                context: &context,
            };
            let request = registry
                .encode_native(
                    OPENAI_RESPONSES_ADAPTER_ID,
                    OPENAI_VIDEOS_OPERATION_ID,
                    ApiType::VideoTextToVideo,
                    &input,
                )
                .unwrap();
            assert_eq!(request.method, method);
            assert_eq!(
                request.url,
                format!("https://api.openai.com/v1/videos/video_1{suffix}")
            );
        }

        let invalid = NativeTaskInput {
            operation: NativeTaskOperation::Status,
            remote_task_id: Some("../secret"),
            codec_input: None,
            resolved_parameters: &parameters,
            context: &context,
        };
        assert!(registry
            .encode_native(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_VIDEOS_OPERATION_ID,
                ApiType::VideoTextToVideo,
                &invalid,
            )
            .is_err());

        let content = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[("content-type", "video/mp4")],
                Bytes::from_static(b"video"),
                "request-content",
                UNIX_EPOCH,
            )
            .unwrap();
        let NativeTaskOutput::Result(output) = registry
            .decode_native(
                OPENAI_RESPONSES_ADAPTER_ID,
                OPENAI_VIDEOS_OPERATION_ID,
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Result,
                content,
            )
            .await
            .unwrap()
        else {
            panic!("expected result")
        };
        assert_eq!(output.artifacts[0].mime.as_deref(), Some("video/mp4"));

        let cancelled = ProtocolContractHarness::default()
            .response(
                StatusCode::OK,
                &[],
                Bytes::from_static(br#"{"id":"video_1","deleted":true}"#),
                "request-cancel",
                UNIX_EPOCH,
            )
            .unwrap();
        assert!(matches!(
            registry
                .decode_native(
                    OPENAI_RESPONSES_ADAPTER_ID,
                    OPENAI_VIDEOS_OPERATION_ID,
                    ApiType::VideoTextToVideo,
                    NativeTaskOperation::Cancel,
                    cancelled,
                )
                .await
                .unwrap(),
            NativeTaskOutput::Cancelled { accepted: true }
        ));
    }

    #[test]
    fn rejects_foreign_credentials_and_unmapped_hard_parameters() {
        let mut bad_context = context();
        bad_context.credential =
            Some(ResolvedCredential::named_header("secret://key", "x-api-key", "secret").unwrap());
        let request = LlmChatInvokeRequest::new(
            "ignored@instance",
            vec![AiMessage::text(AiRole::User, "hello")],
        );
        assert_eq!(
            registry()
                .encode(
                    OPENAI_RESPONSES_ADAPTER_ID,
                    OPENAI_RESPONSES_OPERATION_ID,
                    ApiType::Llm,
                    &input(AiccCall::ChatCompletionsCreate(request.clone())),
                    &bad_context,
                )
                .unwrap_err()
                .kind,
            ProtocolErrorKind::Authentication
        );
        let mut request = request;
        request.seed = Some(7);
        assert_eq!(
            registry()
                .encode(
                    OPENAI_RESPONSES_ADAPTER_ID,
                    OPENAI_RESPONSES_OPERATION_ID,
                    ApiType::Llm,
                    &input(AiccCall::ChatCompletionsCreate(request)),
                    &context(),
                )
                .unwrap_err()
                .kind,
            ProtocolErrorKind::UnsupportedOperation
        );
    }
}
