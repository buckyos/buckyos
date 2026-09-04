use super::{
    sse_frame_stream, AdapterDescriptor, AdapterStatus, CodecCall, CodecContext, CodecRegistration,
    CredentialKind, ExecutionMode, HttpBody, HttpRequest, HttpResponse, NativeTaskCodec,
    NativeTaskHandle, NativeTaskInput, NativeTaskOperation, NativeTaskOutput, NativeTaskState,
    OperationBinding, OperationCodec, OperationDescriptor, ProtocolError, ProtocolErrorKind,
    ProtocolEvent, ProtocolExecution, ProtocolOutput, ProtocolResultValue, ProtocolStream,
    ResolvedCredential, SseConfig, SseFrame, StreamingHttpResponse,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    features, AiArtifact, AiContent, AiMessage, AiRole, AiToolResultContent, AiUsage, AiccCall,
    ApiType, AudioMusicRequest, AudioSpeechRecognitionRequest, AudioTextToSpeechRequest,
    EmbeddingMultimodalRequest, EmbeddingTextItem, EmbeddingTextRequest, ImageToImageRequest,
    LlmChatInvokeRequest, ResourceRef, TextToImageInvokeRequest, VideoExtendRequest,
    VideoImageToVideoRequest, VideoTextToVideoRequest, VideoToVideoRequest, VisionCaptionRequest,
    VisionDetectRequest, VisionOcrRequest, VisionSegmentRequest,
};
use bytes::Bytes;
use futures_util::{stream, StreamExt};
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const GEMINI_ADAPTER_ID: &str = "gemini-interactions";
pub(crate) const GEMINI_INTERACTIONS_OPERATION_ID: &str = "interactions.create";
pub(crate) const GEMINI_EMBED_CONTENT_OPERATION_ID: &str = "models.embedContent";
pub(crate) const GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID: &str = "models.predictLongRunning";

const GEMINI_PROVIDER_NAMESPACE: &str = "gemini";
const DEFAULT_MAX_REQUEST_BYTES: usize = 100 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn gemini_api_key(
    reference: &str,
    secret: impl Into<String>,
) -> ProtocolResultValue<ResolvedCredential> {
    ResolvedCredential::named_header(reference, "x-goog-api-key", secret)
}

pub(crate) fn gemini_interactions_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let interactions = operation(
        GEMINI_INTERACTIONS_OPERATION_ID,
        vec![
            binding(
                ApiType::Llm,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
                [features::TOOL_CALL, features::JSON_SCHEMA, features::VISION],
            ),
            binding(ApiType::VisionOcr, [ExecutionMode::Immediate], []),
            binding(ApiType::VisionCaption, [ExecutionMode::Immediate], []),
            binding(ApiType::VisionDetect, [ExecutionMode::Immediate], []),
            binding(ApiType::VisionSegment, [ExecutionMode::Immediate], []),
            binding(
                ApiType::AudioSpeechRecognition,
                [ExecutionMode::Immediate],
                [],
            ),
            binding(
                ApiType::ImageTextToImage,
                [ExecutionMode::Immediate],
                [features::IMAGE_GENERATION],
            ),
            binding(
                ApiType::ImageImageToImage,
                [ExecutionMode::Immediate],
                [features::IMAGE_GENERATION],
            ),
            binding(
                ApiType::AudioTextToSpeech,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
                [],
            ),
            binding(ApiType::AudioMusic, [ExecutionMode::Immediate], []),
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
    let video = operation(
        GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
        vec![
            binding(ApiType::VideoTextToVideo, [ExecutionMode::NativeTask], []),
            binding(ApiType::VideoImageToVideo, [ExecutionMode::NativeTask], []),
            binding(ApiType::VideoToVideo, [ExecutionMode::NativeTask], []),
            binding(ApiType::VideoExtend, [ExecutionMode::NativeTask], []),
        ],
        false,
    );
    let descriptor = AdapterDescriptor {
        protocol_family_id: "gemini".to_string(),
        protocol_adapter_id: GEMINI_ADAPTER_ID.to_string(),
        interface_generation: "interactions-v1beta".to_string(),
        base_adapter_id: None,
        status: AdapterStatus::Preview,
        operations: BTreeMap::from([
            (interactions.operation_id.clone(), interactions.clone()),
            (embeddings.operation_id.clone(), embeddings.clone()),
            (video.operation_id.clone(), video.clone()),
        ]),
    };
    let direct_types = [
        ApiType::Llm,
        ApiType::VisionOcr,
        ApiType::VisionCaption,
        ApiType::VisionDetect,
        ApiType::VisionSegment,
        ApiType::AudioSpeechRecognition,
        ApiType::ImageTextToImage,
        ApiType::ImageImageToImage,
        ApiType::AudioTextToSpeech,
        ApiType::AudioMusic,
    ];
    let mut operation_codecs: Vec<Arc<dyn OperationCodec>> = direct_types
        .into_iter()
        .map(|api_type| {
            Arc::new(GeminiInteractionCodec::new(interactions.clone(), api_type))
                as Arc<dyn OperationCodec>
        })
        .collect();
    operation_codecs.extend([
        Arc::new(GeminiEmbeddingCodec::new(
            embeddings.clone(),
            ApiType::EmbeddingText,
        )) as Arc<dyn OperationCodec>,
        Arc::new(GeminiEmbeddingCodec::new(
            embeddings,
            ApiType::EmbeddingMultimodal,
        )) as Arc<dyn OperationCodec>,
    ]);
    let native_task_codecs = [
        ApiType::VideoTextToVideo,
        ApiType::VideoImageToVideo,
        ApiType::VideoToVideo,
        ApiType::VideoExtend,
    ]
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

fn operation(
    operation_id: &str,
    bindings: Vec<OperationBinding>,
    supports_cancel: bool,
) -> OperationDescriptor {
    OperationDescriptor {
        operation_id: operation_id.to_string(),
        bindings,
        supports_cancel,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    }
}

fn binding<const N: usize>(
    api_type: ApiType,
    modes: impl IntoIterator<Item = ExecutionMode>,
    features: [&str; N],
) -> OperationBinding {
    let mut binding = OperationBinding::new(api_type, modes);
    binding.supported_features = features.into_iter().map(str::to_string).collect();
    binding
}

#[derive(Clone)]
struct GeminiInteractionCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl GeminiInteractionCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl OperationCodec for GeminiInteractionCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        self.descriptor
            .binding(self.api_type)
            .expect("Gemini codec binding")
            .execution_modes
            .clone()
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let body = encode_interaction(call, self.api_type)?;
        json_request(call.context, Method::POST, "interactions", body)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        if is_sse(response.headers.get(CONTENT_TYPE)) {
            return decode_buffered_interaction_stream(response, self.api_type);
        }
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        Ok(ProtocolExecution::Immediate(normalize_interaction(
            &value,
            self.api_type,
        )?))
    }

    async fn decode_stream(
        &self,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        ensure_stream_success(&response)?;
        if !is_sse(response.headers.get(CONTENT_TYPE)) {
            return Err(ProtocolError::invalid_response(
                "Gemini Interactions stream must use text/event-stream",
            )
            .with_request_id(Some(response.request_id)));
        }
        interaction_protocol_stream(response, self.api_type).await
    }
}

fn encode_interaction(call: &CodecCall<'_>, api_type: ApiType) -> ProtocolResultValue<Value> {
    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        Value::String(provider_model_id(&call.input.resolved_parameters)?),
    );
    match (&call.input.canonical_request, api_type) {
        (AiccCall::ChatCompletionsCreate(request), ApiType::Llm) => {
            encode_llm(request, call, &mut body)?
        }
        (AiccCall::VisionOcr(request), ApiType::VisionOcr) => {
            encode_vision_ocr(request, call, &mut body)?
        }
        (AiccCall::VisionCaption(request), ApiType::VisionCaption) => {
            encode_vision_caption(request, call, &mut body)?
        }
        (AiccCall::VisionDetect(request), ApiType::VisionDetect) => {
            encode_vision_detect(request, call, &mut body)?
        }
        (AiccCall::VisionSegment(request), ApiType::VisionSegment) => {
            encode_vision_segment(request, call, &mut body)?
        }
        (AiccCall::AudioSpeechRecognition(request), ApiType::AudioSpeechRecognition) => {
            encode_asr(request, call, &mut body)?
        }
        (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => {
            encode_text_to_image(request, &mut body)?
        }
        (AiccCall::ImageToImage(request), ApiType::ImageImageToImage) => {
            encode_image_to_image(request, call, &mut body)?
        }
        (AiccCall::AudioTextToSpeech(request), ApiType::AudioTextToSpeech) => {
            encode_tts(request, &mut body)?
        }
        (AiccCall::AudioMusic(request), ApiType::AudioMusic) => encode_music(request, &mut body)?,
        _ => {
            return Err(ProtocolError::invalid_request(
                "Gemini Interactions codec received the wrong canonical request",
            ))
        }
    }
    apply_interaction_parameters(&mut body, &call.input.resolved_parameters)?;
    Ok(Value::Object(body))
}

fn encode_llm(
    request: &LlmChatInvokeRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    let mut input = Vec::new();
    let mut system = Vec::new();
    for message in &request.messages {
        message.validate().map_err(|error| {
            ProtocolError::invalid_request(format!("invalid canonical message: {error}"))
        })?;
        match message.role {
            AiRole::System | AiRole::Developer => {
                system.extend(message.content.iter().filter_map(|content| match content {
                    AiContent::Text { text } => Some(text.clone()),
                    _ => None,
                }));
            }
            AiRole::User | AiRole::Assistant => input.push(json!({
                "type": if message.role == AiRole::User { "user_input" } else { "model_output" },
                "content": encode_content_blocks(&message.content, call)?
            })),
            AiRole::Tool => {
                let AiContent::ToolResult {
                    call_id,
                    content,
                    is_error,
                } = &message.content[0]
                else {
                    return Err(ProtocolError::invalid_request(
                        "Gemini tool message is invalid",
                    ));
                };
                input.push(json!({
                    "type": "function_result",
                    "id": call_id,
                    "result": encode_tool_result(content, call)?,
                    "is_error": is_error
                }));
            }
        }
    }
    body.insert("input".to_string(), Value::Array(input));
    if !system.is_empty() {
        body.insert(
            "system_instruction".to_string(),
            Value::String(system.join("\n")),
        );
    }
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_string(),
            Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        if tool.tool_type != "function" || tool.name.trim().is_empty() {
                            return Err(ProtocolError::invalid_request(
                                "Gemini only maps named function tools",
                            ));
                        }
                        Ok(json!({
                            "type": "function",
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.args_json_schema
                        }))
                    })
                    .collect::<ProtocolResultValue<Vec<_>>>()?,
            ),
        );
    }
    let mut generation = Map::new();
    insert_number(&mut generation, "temperature", request.temperature)?;
    insert_number(&mut generation, "top_p", request.top_p)?;
    if let Some(tokens) = request.max_output_tokens {
        generation.insert("max_output_tokens".to_string(), tokens.into());
    }
    if let Some(seed) = request.seed {
        generation.insert("seed".to_string(), seed.into());
    }
    if !request.stop.is_empty() {
        generation.insert("stop_sequences".to_string(), json!(request.stop));
    }
    if !generation.is_empty() {
        body.insert("generation_config".to_string(), Value::Object(generation));
    }
    if let Some(format) = &request.response_format {
        body.insert(
            "response_format".to_string(),
            serde_json::to_value(format).map_err(|error| {
                ProtocolError::invalid_request(format!("failed to encode response format: {error}"))
            })?,
        );
    }
    Ok(())
}

fn encode_content_blocks(
    content: &[AiContent],
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Vec<Value>> {
    content
        .iter()
        .map(|block| match block {
            AiContent::Text { text } => Ok(json!({"type":"text", "text":text})),
            AiContent::Image { source } => encode_resource(source, "image", call.context),
            AiContent::Document { source, title } => {
                let mut value = encode_resource(source, "document", call.context)?;
                if let (Some(title), Some(object)) = (title, value.as_object_mut()) {
                    object.insert("display_name".to_string(), Value::String(title.clone()));
                }
                Ok(value)
            }
            AiContent::ToolUse {
                call_id,
                name,
                args,
            } => Ok(json!({
                "type":"function_call", "id":call_id, "name":name, "arguments":args
            })),
            AiContent::Thinking {
                summary,
                text,
                provider_metadata,
            } => Ok(json!({
                "type":"thought", "summary":summary, "text":text, "metadata":provider_metadata
            })),
            AiContent::ProviderState { provider, value }
                if provider == GEMINI_PROVIDER_NAMESPACE =>
            {
                Ok(value.clone())
            }
            AiContent::ProviderState { .. } => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Gemini cannot restore another provider's state",
            )),
            AiContent::ToolResult { .. } => Err(ProtocolError::invalid_request(
                "Gemini tool results must use the canonical tool role",
            )),
        })
        .collect()
}

fn encode_tool_result(
    content: &[AiToolResultContent],
    call: &CodecCall<'_>,
) -> ProtocolResultValue<Vec<Value>> {
    content
        .iter()
        .map(|part| match part {
            AiToolResultContent::Text { text } => Ok(json!({"type":"text", "text":text})),
            AiToolResultContent::Image { source } => encode_resource(source, "image", call.context),
            AiToolResultContent::Document { source, title } => {
                let mut value = encode_resource(source, "document", call.context)?;
                if let (Some(title), Some(object)) = (title, value.as_object_mut()) {
                    object.insert("display_name".to_string(), Value::String(title.clone()));
                }
                Ok(value)
            }
        })
        .collect()
}

fn encode_vision_ocr(
    request: &VisionOcrRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::Array(vec![
        json!({"type":"text", "text":"Extract all text from this document and return structured OCR JSON."}),
        encode_resource(&request.document, resource_kind(&request.document, "document"), call.context)?,
    ]));
    body.insert("response_format".to_string(), json!({"type":"json_object"}));
    Ok(())
}

fn encode_vision_caption(
    request: &VisionCaptionRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::Array(vec![
        json!({"type":"text", "text":format!("Caption this image{}{}.", request.style.as_ref().map(|v| format!(" in {v} style")).unwrap_or_default(), request.language.as_ref().map(|v| format!(" using {v}")).unwrap_or_default())}),
        encode_resource(&request.image, "image", call.context)?,
    ]));
    Ok(())
}

fn encode_vision_detect(
    request: &VisionDetectRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::Array(vec![
        json!({"type":"text", "text":format!("Detect objects and return JSON. Classes: {:?}", request.classes)}),
        encode_resource(&request.image, "image", call.context)?,
    ]));
    body.insert("response_format".to_string(), json!({"type":"json_object"}));
    Ok(())
}

fn encode_vision_segment(
    request: &VisionSegmentRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::Array(vec![
        json!({"type":"text", "text":format!("Segment the requested subject and return mask JSON. Prompt: {:?}", request.prompt)}),
        encode_resource(&request.image, "image", call.context)?,
    ]));
    body.insert("response_format".to_string(), json!({"type":"json_object"}));
    Ok(())
}

fn encode_asr(
    request: &AudioSpeechRecognitionRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert(
        "input".to_string(),
        Value::Array(vec![encode_resource(
            &request.audio,
            "audio",
            call.context,
        )?]),
    );
    body.insert(
        "generation_config".to_string(),
        json!({"transcription_config": {
            "language": request.language, "timestamps": request.timestamps,
            "diarization": request.diarization, "output_formats": request.output_formats
        }}),
    );
    Ok(())
}

fn encode_text_to_image(
    request: &TextToImageInvokeRequest,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::String(request.prompt.clone()));
    body.insert("response_format".to_string(), json!({"type":"image", "mime_type": request.output.as_ref().and_then(|v| v.media_type.clone()).unwrap_or_else(|| "image/png".to_string())}));
    let mut config = Map::new();
    if let Some(ratio) = &request.aspect_ratio {
        config.insert("aspect_ratio".to_string(), ratio.clone().into());
    }
    if let Some(seed) = request.seed {
        config.insert("seed".to_string(), seed.into());
    }
    if let Some(n) = request.n {
        config.insert("candidate_count".to_string(), n.into());
    }
    if !config.is_empty() {
        body.insert("generation_config".to_string(), Value::Object(config));
    }
    Ok(())
}

fn encode_image_to_image(
    request: &ImageToImageRequest,
    call: &CodecCall<'_>,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    if request.images.is_empty() {
        return Err(ProtocolError::invalid_request(
            "Gemini image edit requires at least one image",
        ));
    }
    let mut input = vec![json!({"type":"text", "text":request.prompt})];
    for image in &request.images {
        input.push(encode_resource(image, "image", call.context)?);
    }
    body.insert("input".to_string(), Value::Array(input));
    body.insert("response_format".to_string(), json!({"type":"image", "mime_type": request.output.as_ref().and_then(|v| v.media_type.clone()).unwrap_or_else(|| "image/png".to_string())}));
    Ok(())
}

fn encode_tts(
    request: &AudioTextToSpeechRequest,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::String(request.text.clone()));
    body.insert("response_format".to_string(), json!({"type":"audio", "mime_type": request.output.as_ref().and_then(|v| v.media_type.clone()).unwrap_or_else(|| "audio/pcm".to_string())}));
    body.insert(
        "generation_config".to_string(),
        json!({"speech_config": {
            "voice_name": request.voice.voice_id, "language_code": request.voice.language,
            "style": request.voice.style, "speed": request.speed
        }}),
    );
    Ok(())
}

fn encode_music(
    request: &AudioMusicRequest,
    body: &mut Map<String, Value>,
) -> ProtocolResultValue<()> {
    body.insert("input".to_string(), Value::String(request.prompt.clone()));
    body.insert("response_format".to_string(), json!({"type":"audio", "mime_type": request.output.as_ref().and_then(|v| v.media_type.clone()).unwrap_or_else(|| "audio/mpeg".to_string())}));
    let mut config = Map::new();
    if let Some(duration) = request.duration_seconds {
        config.insert(
            "duration_seconds".to_string(),
            finite_number("duration_seconds", duration)?,
        );
    }
    if let Some(instrumental) = request.instrumental {
        config.insert("instrumental".to_string(), instrumental.into());
    }
    if let Some(lyrics) = &request.lyrics {
        config.insert("lyrics".to_string(), lyrics.clone().into());
    }
    if let Some(seed) = request.seed {
        config.insert("seed".to_string(), seed.into());
    }
    if !config.is_empty() {
        body.insert("generation_config".to_string(), Value::Object(config));
    }
    Ok(())
}

fn apply_interaction_parameters(
    body: &mut Map<String, Value>,
    parameters: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    const ALLOWED: &[&str] = &[
        "background",
        "generation_config",
        "previous_interaction_id",
        "response_format",
        "store",
        "stream",
        "system_instruction",
        "tools",
        "safety_settings",
    ];
    for (name, value) in parameters {
        if name == "provider_model_id" {
            continue;
        }
        if !ALLOWED.contains(&name.as_str()) {
            return Err(ProtocolError::invalid_request(format!(
                "resolved Gemini parameter `{name}` is not supported"
            )));
        }
        body.insert(name.clone(), value.clone());
    }
    Ok(())
}

fn encode_resource(
    source: &ResourceRef,
    kind: &str,
    context: &CodecContext,
) -> ProtocolResultValue<Value> {
    match source {
        ResourceRef::Url { url, mime_hint } => {
            Ok(json!({"type":kind, "uri":url, "mime_type":mime_hint}))
        }
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD
                .decode(data_base64)
                .map_err(|_| ProtocolError::invalid_request("resource contains invalid base64"))?;
            Ok(json!({"type":kind, "data":data_base64, "mime_type":mime}))
        }
        ResourceRef::NamedObject { .. } => {
            let resource = context.materialized_resource(source)?;
            Ok(
                json!({"type":kind, "data":STANDARD.encode(&resource.bytes), "mime_type":resource.mime}),
            )
        }
    }
}

fn resource_kind<'a>(source: &ResourceRef, fallback: &'a str) -> &'a str {
    let mime = match source {
        ResourceRef::Url { mime_hint, .. } => mime_hint.as_deref(),
        ResourceRef::Base64 { mime, .. } => Some(mime.as_str()),
        ResourceRef::NamedObject { .. } => None,
    };
    match mime.and_then(|mime| mime.split('/').next()) {
        Some("image") => "image",
        Some("audio") => "audio",
        Some("video") => "video",
        _ => fallback,
    }
}

fn insert_number(
    body: &mut Map<String, Value>,
    name: &str,
    value: Option<f64>,
) -> ProtocolResultValue<()> {
    if let Some(value) = value {
        body.insert(name.to_string(), finite_number(name, value)?);
    }
    Ok(())
}

fn finite_number(name: &str, value: f64) -> ProtocolResultValue<Value> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| ProtocolError::invalid_request(format!("{name} must be finite")))
}

#[derive(Clone)]
struct GeminiEmbeddingCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl GeminiEmbeddingCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl OperationCodec for GeminiEmbeddingCodec {
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
        let model = provider_model_id(&call.input.resolved_parameters)?;
        let content = match (&call.input.canonical_request, self.api_type) {
            (AiccCall::EmbeddingText(request), ApiType::EmbeddingText) => {
                encode_text_embedding(request, call.context)?
            }
            (AiccCall::EmbeddingMultimodal(request), ApiType::EmbeddingMultimodal) => {
                encode_multimodal_embedding(request, call.context)?
            }
            _ => {
                return Err(ProtocolError::invalid_request(
                    "Gemini embedding codec received the wrong canonical request",
                ))
            }
        };
        let dimensions = match &call.input.canonical_request {
            AiccCall::EmbeddingText(request) => request.dimensions,
            AiccCall::EmbeddingMultimodal(request) => request.dimensions,
            _ => None,
        };
        let mut body = Map::from_iter([("content".to_string(), content)]);
        if let Some(dimensions) = dimensions {
            body.insert("outputDimensionality".to_string(), dimensions.into());
        }
        apply_embedding_parameters(&mut body, &call.input.resolved_parameters)?;
        json_request(
            call.context,
            Method::POST,
            &format!("models/{model}:embedContent"),
            Value::Object(body),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let embedding = value
            .pointer("/embedding/values")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProtocolError::invalid_response(
                    "Gemini embedding response is missing embedding.values",
                )
            })?;
        let values = embedding
            .iter()
            .map(|value| {
                value.as_f64().map(|number| number as f32).ok_or_else(|| {
                    ProtocolError::invalid_response("Gemini embedding value must be numeric")
                })
            })
            .collect::<ProtocolResultValue<Vec<_>>>()?;
        let usage = decode_embedding_usage(&value)?;
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"data":[{"index":0,"id":null,"embedding":values,"embedding_space_id":"gemini"}],"data_resource":null}),
            usage,
            artifacts: Vec::new(),
        }))
    }
}

fn encode_text_embedding(
    request: &EmbeddingTextRequest,
    context: &CodecContext,
) -> ProtocolResultValue<Value> {
    if request.items.len() != 1 {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Gemini models.embedContent requires exactly one canonical embedding item",
        ));
    }
    let part = match &request.items[0] {
        EmbeddingTextItem::Text { text, .. } => json!({"text":text}),
        EmbeddingTextItem::Resource { resource, .. } => embedding_resource_part(resource, context)?,
    };
    Ok(json!({"parts":[part]}))
}

fn encode_multimodal_embedding(
    request: &EmbeddingMultimodalRequest,
    context: &CodecContext,
) -> ProtocolResultValue<Value> {
    if request.items.len() != 1 {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Gemini models.embedContent requires exactly one canonical multimodal item",
        ));
    }
    let item = &request.items[0];
    if item.text.is_none() && item.image.is_none() {
        return Err(ProtocolError::invalid_request(
            "Gemini multimodal embedding item must contain text or image",
        ));
    }
    let mut parts = Vec::new();
    if let Some(text) = &item.text {
        parts.push(json!({"text":text}));
    }
    if let Some(image) = &item.image {
        parts.push(embedding_resource_part(image, context)?);
    }
    Ok(json!({"parts":parts}))
}

fn embedding_resource_part(
    resource: &ResourceRef,
    context: &CodecContext,
) -> ProtocolResultValue<Value> {
    match resource {
        ResourceRef::Url { url, mime_hint } => {
            Ok(json!({"fileData":{"fileUri":url,"mimeType":mime_hint}}))
        }
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("embedding resource contains invalid base64")
            })?;
            Ok(json!({"inlineData":{"mimeType":mime,"data":data_base64}}))
        }
        ResourceRef::NamedObject { .. } => {
            let resource = context.materialized_resource(resource)?;
            Ok(
                json!({"inlineData":{"mimeType":resource.mime,"data":STANDARD.encode(&resource.bytes)}}),
            )
        }
    }
}

fn apply_embedding_parameters(
    body: &mut Map<String, Value>,
    parameters: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    for (name, value) in parameters {
        if name == "provider_model_id" || name == "outputDimensionality" {
            continue;
        }
        if !matches!(name.as_str(), "taskType" | "title" | "embedContentConfig") {
            return Err(ProtocolError::invalid_request(format!(
                "resolved Gemini embedding parameter `{name}` is not supported"
            )));
        }
        body.insert(name.clone(), value.clone());
    }
    Ok(())
}

fn decode_embedding_usage(value: &Value) -> ProtocolResultValue<Option<AiUsage>> {
    let Some(usage) = value.get("usageMetadata") else {
        return Ok(None);
    };
    let input = usage
        .get("promptTokenCount")
        .or_else(|| usage.get("inputTokenCount"))
        .and_then(Value::as_u64);
    let total = usage
        .get("totalTokenCount")
        .and_then(Value::as_u64)
        .or(input);
    Ok(Some(AiUsage {
        input_tokens: input,
        output_tokens: Some(0),
        total_tokens: total,
        request_units: None,
    }))
}

fn normalize_interaction(value: &Value, api_type: ApiType) -> ProtocolResultValue<ProtocolOutput> {
    let object = value.as_object().ok_or_else(|| {
        ProtocolError::invalid_response("Gemini interaction response must be an object")
    })?;
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ProtocolError::invalid_response("Gemini interaction response is missing status")
        })?;
    if matches!(status, "failed" | "cancelled" | "canceled" | "incomplete") {
        return Err(interaction_failure(value));
    }
    if !matches!(status, "completed" | "requires_action") {
        return Err(ProtocolError::invalid_response(
            "Gemini immediate interaction has not reached a terminal status",
        ));
    }
    let outputs = interaction_outputs(value)?;
    let usage = decode_usage(value.get("usage"))?;
    match api_type {
        ApiType::Llm => normalize_llm(value, &outputs, usage),
        ApiType::ImageTextToImage | ApiType::ImageImageToImage => {
            normalize_media(&outputs, usage, "image", "images")
        }
        ApiType::AudioTextToSpeech | ApiType::AudioMusic => {
            normalize_media(&outputs, usage, "audio", "audio")
        }
        ApiType::AudioSpeechRecognition => Ok(ProtocolOutput {
            value: json!({"text":output_text(&outputs),"segments":[],"artifacts":{},"diagnostic":null}),
            usage,
            artifacts: Vec::new(),
        }),
        ApiType::VisionCaption => {
            let text = output_text(&outputs);
            let parsed = parse_json_text(&text);
            let captions = parsed
                .get("captions")
                .cloned()
                .unwrap_or_else(|| json!([{"text":text,"confidence":null}]));
            Ok(ProtocolOutput {
                value: json!({"captions":captions}),
                usage,
                artifacts: Vec::new(),
            })
        }
        ApiType::VisionOcr => normalize_structured(
            &outputs,
            usage,
            "text",
            json!({"text":output_text(&outputs),"pages":[],"artifacts":{}}),
        ),
        ApiType::VisionDetect => {
            normalize_structured(&outputs, usage, "detections", json!({"detections":[]}))
        }
        ApiType::VisionSegment => {
            normalize_structured(&outputs, usage, "masks", json!({"masks":[]}))
        }
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Gemini interaction response API type is unsupported",
        )),
    }
}

fn interaction_outputs(value: &Value) -> ProtocolResultValue<Vec<Value>> {
    if let Some(outputs) = value.get("outputs").and_then(Value::as_array) {
        return Ok(outputs.clone());
    }
    let mut outputs = Vec::new();
    if let Some(steps) = value.get("steps").and_then(Value::as_array) {
        for step in steps {
            if step.get("type").and_then(Value::as_str) == Some("model_output") {
                if let Some(content) = step.get("content").and_then(Value::as_array) {
                    outputs.extend(content.iter().cloned());
                }
            } else if step.get("type").and_then(Value::as_str) == Some("function_call") {
                outputs.push(step.clone());
            }
        }
    }
    if outputs.is_empty() {
        return Err(ProtocolError::invalid_response(
            "Gemini interaction response has no outputs",
        ));
    }
    Ok(outputs)
}

fn normalize_llm(
    interaction: &Value,
    outputs: &[Value],
    usage: Option<AiUsage>,
) -> ProtocolResultValue<ProtocolOutput> {
    let mut content = Vec::new();
    for output in outputs {
        match output.get("type").and_then(Value::as_str) {
            Some("text") => content.push(AiContent::Text {
                text: required_output_string(output, "text")?,
            }),
            Some("function_call") => {
                let arguments = output
                    .get("arguments")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        ProtocolError::invalid_response(
                            "Gemini function_call arguments must be an object",
                        )
                    })?;
                content.push(AiContent::ToolUse {
                    call_id: required_output_string(output, "id")?,
                    name: required_output_string(output, "name")?,
                    args: arguments.clone().into_iter().collect(),
                });
            }
            Some("thought") => content.push(AiContent::Thinking {
                summary: output
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                text: output
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                provider_metadata: output.get("metadata").cloned(),
            }),
            Some("image") => content.push(AiContent::Image {
                source: decode_resource(output)?,
            }),
            Some("document") => content.push(AiContent::Document {
                source: decode_resource(output)?,
                title: output
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
            _ => content.push(AiContent::ProviderState {
                provider: GEMINI_PROVIDER_NAMESPACE.to_string(),
                value: output.clone(),
            }),
        }
    }
    if let Some(safety) = interaction
        .get("safety_ratings")
        .or_else(|| interaction.get("safety"))
    {
        content.push(AiContent::ProviderState {
            provider: GEMINI_PROVIDER_NAMESPACE.to_string(),
            value: json!({"safety":safety}),
        });
    }
    let message = AiMessage::new(AiRole::Assistant, content);
    message.validate().map_err(|error| {
        ProtocolError::invalid_response(format!("invalid Gemini output: {error}"))
    })?;
    let tool_calls = message.tool_calls();
    Ok(ProtocolOutput {
        value: json!({"message":message,"tool_calls":tool_calls,"finish_reason":interaction.get("status").cloned().unwrap_or(Value::Null)}),
        usage,
        artifacts: Vec::new(),
    })
}

fn normalize_media(
    outputs: &[Value],
    usage: Option<AiUsage>,
    kind: &str,
    field: &str,
) -> ProtocolResultValue<ProtocolOutput> {
    let mut resources = Vec::new();
    let mut artifacts = Vec::new();
    for (index, output) in outputs
        .iter()
        .filter(|output| output.get("type").and_then(Value::as_str) == Some(kind))
        .enumerate()
    {
        let resource = decode_resource(output)?;
        let mime = output
            .get("mime_type")
            .and_then(Value::as_str)
            .map(str::to_string);
        artifacts.push(AiArtifact {
            name: format!("{kind}-{index}"),
            resource: resource.clone(),
            mime,
            metadata: None,
        });
        resources.push(resource);
    }
    if resources.is_empty() {
        return Err(ProtocolError::invalid_response(format!(
            "Gemini response has no {kind} output"
        )));
    }
    let value = if field == "images" {
        json!({"images":resources,"provider_states":[]})
    } else {
        Value::Object(Map::from_iter([(
            field.to_string(),
            json!(resources.into_iter().next()),
        )]))
    };
    Ok(ProtocolOutput {
        value,
        usage,
        artifacts,
    })
}

fn normalize_structured(
    outputs: &[Value],
    usage: Option<AiUsage>,
    required_field: &str,
    fallback: Value,
) -> ProtocolResultValue<ProtocolOutput> {
    let parsed = parse_json_text(&output_text(outputs));
    let value = if parsed.get(required_field).is_some() {
        parsed
    } else {
        fallback
    };
    Ok(ProtocolOutput {
        value,
        usage,
        artifacts: Vec::new(),
    })
}

fn output_text(outputs: &[Value]) -> String {
    outputs
        .iter()
        .filter(|output| output.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|output| output.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_json_text(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn decode_resource(value: &Value) -> ProtocolResultValue<ResourceRef> {
    let mime = value
        .get("mime_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();
    if let Some(data) = value.get("data").and_then(Value::as_str) {
        STANDARD.decode(data).map_err(|_| {
            ProtocolError::invalid_response("Gemini media output contains invalid base64")
        })?;
        return Ok(ResourceRef::base64(mime, data.to_string()));
    }
    value
        .get("uri")
        .and_then(Value::as_str)
        .map(|uri| ResourceRef::url(uri.to_string(), Some(mime)))
        .ok_or_else(|| {
            ProtocolError::invalid_response("Gemini media output is missing data or uri")
        })
}

fn required_output_string(value: &Value, field: &str) -> ProtocolResultValue<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("Gemini output is missing `{field}`"))
        })
}

fn decode_usage(value: Option<&Value>) -> ProtocolResultValue<Option<AiUsage>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| ProtocolError::invalid_response("Gemini usage must be an object"))?;
    let input = object.get("total_input_tokens").and_then(Value::as_u64);
    let output = object.get("total_output_tokens").and_then(Value::as_u64);
    let total = object
        .get("total_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            input
                .zip(output)
                .and_then(|(input, output)| input.checked_add(output))
        });
    Ok(Some(AiUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: total,
        request_units: None,
    }))
}

fn interaction_failure(value: &Value) -> ProtocolError {
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| value.get("status").and_then(Value::as_str))
        .unwrap_or("Gemini interaction failed");
    ProtocolError::new(ProtocolErrorKind::Transport, message)
}

fn decode_buffered_interaction_stream(
    response: HttpResponse,
    api_type: ApiType,
) -> ProtocolResultValue<ProtocolExecution> {
    let mut framer = super::SseFramer::new(SseConfig::default())?;
    let mut frames = framer.push(&response.body)?;
    frames.extend(framer.finish(super::SseStreamEnd::EndOfStream)?);
    let mut events = Vec::new();
    let mut saw_final = false;
    for frame in frames {
        for event in decode_interaction_frame(frame, api_type)? {
            saw_final |= matches!(event, ProtocolEvent::Final(_));
            events.push(Ok(event));
        }
    }
    if !saw_final {
        return Err(ProtocolError::invalid_response(
            "Gemini stream ended before interaction.completed",
        ));
    }
    Ok(ProtocolExecution::Stream(ProtocolStream {
        events: Box::pin(stream::iter(events)),
    }))
}

async fn interaction_protocol_stream(
    response: StreamingHttpResponse,
    api_type: ApiType,
) -> ProtocolResultValue<ProtocolStream> {
    let request_id = response.request_id.clone();
    let frames =
        sse_frame_stream(response, SseConfig::default(), DEFAULT_MAX_RESPONSE_BYTES).await?;
    let events = frames
        .scan(false, move |final_seen, frame| {
            let request_id = request_id.clone();
            let is_end = matches!(frame, Ok(SseFrame::StreamEnd(_)));
            let result = frame.and_then(|frame| decode_interaction_frame(frame, api_type));
            let items = match result {
                Ok(decoded) => {
                    if decoded
                        .iter()
                        .any(|event| matches!(event, ProtocolEvent::Final(_)))
                    {
                        *final_seen = true;
                    }
                    if is_end && !*final_seen {
                        vec![Err(ProtocolError::invalid_response(
                            "Gemini stream ended before interaction.completed",
                        )
                        .with_request_id(Some(request_id)))]
                    } else {
                        decoded.into_iter().map(Ok).collect()
                    }
                }
                Err(error) => vec![Err(error.with_request_id(Some(request_id)))],
            };
            futures_util::future::ready(Some(items))
        })
        .flat_map(stream::iter);
    Ok(ProtocolStream {
        events: Box::pin(events),
    })
}

fn decode_interaction_frame(
    frame: SseFrame,
    api_type: ApiType,
) -> ProtocolResultValue<Vec<ProtocolEvent>> {
    let event = match frame {
        SseFrame::Event(event) => event,
        SseFrame::Terminated { marker } if marker == "[DONE]" => return Ok(Vec::new()),
        SseFrame::Terminated { .. } => {
            return Err(ProtocolError::invalid_response(
                "Gemini stream used an unknown termination marker",
            ))
        }
        SseFrame::StreamEnd(_) => return Ok(Vec::new()),
    };
    let value: Value = serde_json::from_str(&event.data).map_err(|error| {
        ProtocolError::invalid_response(format!("Gemini SSE data is not valid JSON: {error}"))
    })?;
    let event_type = value
        .get("event_type")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_response("Gemini SSE event is missing event_type"))?;
    if event
        .event
        .as_deref()
        .is_some_and(|name| name != event_type)
    {
        return Err(ProtocolError::invalid_response(
            "Gemini SSE event name does not match event_type",
        ));
    }
    match event_type {
        "step.delta" | "content.delta" => Ok(vec![ProtocolEvent::Delta(
            value.get("delta").cloned().ok_or_else(|| {
                ProtocolError::invalid_response("Gemini delta event is missing delta")
            })?,
        )]),
        "interaction.status_update" | "step.start" | "step.stop" | "interaction.created" => {
            Ok(vec![ProtocolEvent::Progress(value)])
        }
        "interaction.completed" => {
            let interaction = value.get("interaction").ok_or_else(|| {
                ProtocolError::invalid_response("Gemini completed event is missing interaction")
            })?;
            Ok(vec![ProtocolEvent::Final(normalize_interaction(
                interaction,
                api_type,
            )?)])
        }
        "interaction.failed" | "error" => Err(interaction_failure(&value)),
        _ => Ok(vec![ProtocolEvent::Delta(
            json!({"provider_state":{"provider":GEMINI_PROVIDER_NAMESPACE,"value":value}}),
        )]),
    }
}

#[derive(Clone)]
struct GeminiVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

impl GeminiVideoCodec {
    fn new(descriptor: OperationDescriptor, api_type: ApiType) -> Self {
        Self {
            descriptor,
            api_type,
        }
    }
}

#[async_trait]
impl NativeTaskCodec for GeminiVideoCodec {
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
        ])
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        match input.operation {
            NativeTaskOperation::Submit => encode_video_submit(input, self.api_type),
            NativeTaskOperation::Status | NativeTaskOperation::Result => {
                encode_operation_get(input)
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Gemini video cancellation is not declared",
            )),
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
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Gemini video cancellation is not declared",
            )),
        }
    }
}

fn encode_video_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("Gemini video submit requires canonical input")
    })?;
    let model = provider_model_id(&codec_input.resolved_parameters)?;
    let (instance, mut parameters) = match (&codec_input.canonical_request, api_type) {
        (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => {
            video_text_instance(request)
        }
        (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => {
            video_image_instance(request, input.context)?
        }
        (AiccCall::VideoToVideo(request), ApiType::VideoToVideo) => {
            video_video_instance(request, input.context)?
        }
        (AiccCall::VideoExtend(request), ApiType::VideoExtend) => {
            video_extend_instance(request, input.context)?
        }
        _ => {
            return Err(ProtocolError::invalid_request(
                "Gemini video codec received the wrong canonical request",
            ))
        }
    };
    apply_video_parameters(&mut parameters, input.resolved_parameters)?;
    json_request(
        input.context,
        Method::POST,
        &format!("models/{model}:predictLongRunning"),
        json!({"instances":[instance],"parameters":parameters}),
    )
}

fn video_text_instance(request: &VideoTextToVideoRequest) -> (Value, Map<String, Value>) {
    let mut parameters = Map::new();
    if let Some(value) = request.duration_seconds {
        parameters.insert("durationSeconds".to_string(), json!(value));
    }
    if let Some(value) = &request.aspect_ratio {
        parameters.insert("aspectRatio".to_string(), json!(value));
    }
    if let Some(value) = &request.resolution {
        parameters.insert("resolution".to_string(), json!(value));
    }
    if let Some(value) = request.generate_audio {
        parameters.insert("generateAudio".to_string(), json!(value));
    }
    if let Some(value) = request.seed {
        parameters.insert("seed".to_string(), json!(value));
    }
    (json!({"prompt":request.prompt}), parameters)
}

fn video_image_instance(
    request: &VideoImageToVideoRequest,
    context: &CodecContext,
) -> ProtocolResultValue<(Value, Map<String, Value>)> {
    let mut instance = Map::from_iter([("prompt".to_string(), json!(request.prompt))]);
    instance.insert(
        "image".to_string(),
        video_resource(&request.image, context)?,
    );
    let mut parameters = Map::new();
    if let Some(value) = request.duration_seconds {
        parameters.insert("durationSeconds".to_string(), json!(value));
    }
    if let Some(value) = &request.aspect_ratio {
        parameters.insert("aspectRatio".to_string(), json!(value));
    }
    if let Some(value) = &request.resolution {
        parameters.insert("resolution".to_string(), json!(value));
    }
    Ok((Value::Object(instance), parameters))
}

fn video_video_instance(
    request: &VideoToVideoRequest,
    context: &CodecContext,
) -> ProtocolResultValue<(Value, Map<String, Value>)> {
    let mut parameters = Map::new();
    if let Some(value) = request.preserve_motion {
        parameters.insert("preserveMotion".to_string(), json!(value));
    }
    if let Some(value) = &request.time_range {
        parameters.insert(
            "timeRange".to_string(),
            json!({"startSeconds":value.start_seconds,"endSeconds":value.end_seconds}),
        );
    }
    Ok((
        json!({"prompt":request.prompt,"video":video_resource(&request.video, context)?}),
        parameters,
    ))
}

fn video_extend_instance(
    request: &VideoExtendRequest,
    context: &CodecContext,
) -> ProtocolResultValue<(Value, Map<String, Value>)> {
    let mut instance = Map::from_iter([
        ("prompt".to_string(), json!(request.prompt)),
        (
            "video".to_string(),
            video_resource(&request.video, context)?,
        ),
    ]);
    if let Some(value) = &request.continuation_handle {
        instance.insert("continuationHandle".to_string(), json!(value));
    }
    let mut parameters = Map::new();
    if let Some(value) = request.duration_seconds {
        parameters.insert("durationSeconds".to_string(), json!(value));
    }
    if let Some(value) = &request.resolution {
        parameters.insert("resolution".to_string(), json!(value));
    }
    Ok((Value::Object(instance), parameters))
}

fn video_resource(resource: &ResourceRef, context: &CodecContext) -> ProtocolResultValue<Value> {
    match resource {
        ResourceRef::Url { url, mime_hint } => Ok(json!({"uri":url,"mimeType":mime_hint})),
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("video resource contains invalid base64")
            })?;
            Ok(json!({"bytesBase64Encoded":data_base64,"mimeType":mime}))
        }
        ResourceRef::NamedObject { .. } => {
            let resource = context.materialized_resource(resource)?;
            Ok(
                json!({"bytesBase64Encoded":STANDARD.encode(&resource.bytes),"mimeType":resource.mime}),
            )
        }
    }
}

fn apply_video_parameters(
    parameters: &mut Map<String, Value>,
    resolved: &BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    for (name, value) in resolved {
        if name == "provider_model_id" {
            continue;
        }
        if !matches!(
            name.as_str(),
            "aspectRatio"
                | "durationSeconds"
                | "generateAudio"
                | "negativePrompt"
                | "numberOfVideos"
                | "personGeneration"
                | "resolution"
                | "seed"
        ) {
            return Err(ProtocolError::invalid_request(format!(
                "resolved Gemini video parameter `{name}` is not supported"
            )));
        }
        parameters.insert(name.clone(), value.clone());
    }
    Ok(())
}

fn encode_operation_get(input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
    let name = input.remote_task_id.ok_or_else(|| {
        ProtocolError::invalid_request("Gemini operation lifecycle requires an operation name")
    })?;
    let normalized = validate_operation_name(name)?;
    json_request(input.context, Method::GET, &normalized, Value::Null)
}

fn validate_operation_name(value: &str) -> ProtocolResultValue<String> {
    let value = value.trim_start_matches('/');
    if !value.starts_with("operations/")
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.'))
    {
        return Err(ProtocolError::invalid_request(
            "Gemini operation name is invalid",
        ));
    }
    Ok(value.to_string())
}

fn decode_video_submit(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let retry_after = response.retry_after;
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let name = required_output_string(&value, "name")?;
    let mut handle = NativeTaskHandle::new(name)?;
    handle.state = if value.get("done").and_then(Value::as_bool) == Some(true) {
        NativeTaskState::Succeeded
    } else {
        NativeTaskState::Submitted
    };
    handle.poll_after = retry_after.or(Some(Duration::from_secs(2)));
    Ok(NativeTaskOutput::Submitted(handle))
}

fn decode_video_status(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let retry_after = response.retry_after;
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let state = if value.get("done").and_then(Value::as_bool) != Some(true) {
        NativeTaskState::Running
    } else if value.get("error").is_some() {
        NativeTaskState::Failed
    } else {
        NativeTaskState::Succeeded
    };
    Ok(NativeTaskOutput::Status { state, retry_after })
}

fn decode_video_result(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    if value.get("done").and_then(Value::as_bool) != Some(true) {
        return Err(ProtocolError::invalid_response(
            "Gemini operation result is not ready",
        ));
    }
    if value.get("error").is_some() {
        return Err(interaction_failure(&value));
    }
    let media =
        find_media_value(value.get("response").unwrap_or(&value), "video").ok_or_else(|| {
            ProtocolError::invalid_response("Gemini video operation has no video result")
        })?;
    let resource = decode_resource(media)?;
    let mime = media
        .get("mime_type")
        .or_else(|| media.get("mimeType"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: json!({"video":resource}),
        usage: decode_usage(value.pointer("/response/usage"))?,
        artifacts: vec![AiArtifact {
            name: "video".to_string(),
            resource,
            mime,
            metadata: None,
        }],
    }))
}

fn find_media_value<'a>(value: &'a Value, kind: &str) -> Option<&'a Value> {
    if value.get("type").and_then(Value::as_str) == Some(kind) {
        return Some(value);
    }
    match value {
        Value::Array(values) => values
            .iter()
            .find_map(|value| find_media_value(value, kind)),
        Value::Object(values) => values
            .values()
            .find_map(|value| find_media_value(value, kind)),
        _ => None,
    }
}

fn provider_model_id(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<String> {
    parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved `provider_model_id`"))
}

fn api_endpoint(base_url: &str, path: &str) -> ProtocolResultValue<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProtocolError::invalid_configuration("Gemini base URL is invalid"))?;
    let base_path = url.path().trim_end_matches('/');
    let prefix = if base_path.is_empty() {
        "/v1beta"
    } else {
        base_path
    };
    url.set_path(&format!("{prefix}/{}", path.trim_start_matches('/')));
    Ok(url.to_string())
}

fn upload_endpoint(base_url: &str) -> ProtocolResultValue<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProtocolError::invalid_configuration("Gemini base URL is invalid"))?;
    url.set_path("/upload/v1beta/files");
    Ok(url.to_string())
}

fn json_request(
    context: &CodecContext,
    method: Method,
    path: &str,
    body: Value,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut request = HttpRequest::new(method.clone(), api_endpoint(&context.base_url, path)?);
    if method != Method::GET && method != Method::DELETE {
        request.body = HttpBody::Json(body);
    }
    finish_request(&mut request, context)
}

fn finish_request(
    request: &mut HttpRequest,
    context: &CodecContext,
) -> ProtocolResultValue<HttpRequest> {
    let credential = context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "Gemini operation requires a resolved x-goog-api-key credential",
        )
    })?;
    if credential.audit().kind != CredentialKind::NamedHeader {
        return Err(ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "Gemini operation requires a named-header credential",
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
    Err(gemini_http_error(
        response.status,
        &response.body,
        &response.request_id,
        response.retry_after,
    ))
}

fn ensure_stream_success(response: &StreamingHttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    Err(ProtocolError::new(
        http_error_kind(response.status),
        format!("Gemini HTTP {}", response.status.as_u16()),
    )
    .with_request_id(Some(response.request_id.clone()))
    .with_retry_after(response.retry_after))
}

fn gemini_http_error(
    status: StatusCode,
    body: &[u8],
    request_id: &str,
    retry_after: Option<Duration>,
) -> ProtocolError {
    let value: Option<Value> = serde_json::from_slice(body).ok();
    let code = value
        .as_ref()
        .and_then(|value| value.pointer("/error/status"))
        .and_then(Value::as_str)
        .unwrap_or("http_error");
    let message = value
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("Gemini request failed");
    ProtocolError::new(http_error_kind(status), format!("Gemini {code}: {message}"))
        .with_request_id(Some(request_id.to_string()))
        .with_retry_after(retry_after)
}

fn http_error_kind(status: StatusCode) -> ProtocolErrorKind {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ProtocolErrorKind::Timeout,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiFile {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
    #[serde(default)]
    pub download_uri: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeminiUploadSession {
    pub upload_url: String,
    pub content_length: usize,
    pub mime_type: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GeminiFilesCodec;

impl GeminiFilesCodec {
    pub(crate) fn start_upload(
        &self,
        context: &CodecContext,
        display_name: &str,
        mime_type: &str,
        content_length: usize,
    ) -> ProtocolResultValue<HttpRequest> {
        if display_name.is_empty()
            || display_name.len() > 512
            || mime_type.trim().is_empty()
            || content_length == 0
            || content_length > context.limits.max_request_bytes
        {
            return Err(ProtocolError::invalid_request(
                "Gemini file upload metadata is invalid",
            ));
        }
        let mut request = HttpRequest::new(Method::POST, upload_endpoint(&context.base_url)?);
        request.body = HttpBody::Json(json!({"file":{"display_name":display_name}}));
        request.headers.insert(
            HeaderName::from_static("x-goog-upload-protocol"),
            HeaderValue::from_static("resumable"),
        );
        request.headers.insert(
            HeaderName::from_static("x-goog-upload-command"),
            HeaderValue::from_static("start"),
        );
        request.headers.insert(
            HeaderName::from_static("x-goog-upload-header-content-length"),
            HeaderValue::from_str(&content_length.to_string()).map_err(|_| {
                ProtocolError::invalid_request("Gemini upload content length is invalid")
            })?,
        );
        request.headers.insert(
            HeaderName::from_static("x-goog-upload-header-content-type"),
            HeaderValue::from_str(mime_type).map_err(|_| {
                ProtocolError::invalid_request("Gemini upload MIME type is invalid")
            })?,
        );
        finish_request(&mut request, context)
    }

    pub(crate) fn decode_start_upload(
        &self,
        response: HttpResponse,
        content_length: usize,
        mime_type: &str,
    ) -> ProtocolResultValue<GeminiUploadSession> {
        ensure_success(&response)?;
        let upload_url = response
            .headers
            .get("x-goog-upload-url")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProtocolError::invalid_response(
                    "Gemini upload start response is missing x-goog-upload-url",
                )
            })?
            .to_string();
        let parsed = Url::parse(&upload_url)
            .map_err(|_| ProtocolError::invalid_response("Gemini upload URL is invalid"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ProtocolError::invalid_response(
                "Gemini upload URL must use HTTP or HTTPS",
            ));
        }
        Ok(GeminiUploadSession {
            upload_url,
            content_length,
            mime_type: mime_type.to_string(),
        })
    }

    pub(crate) fn upload(
        &self,
        context: &CodecContext,
        session: &GeminiUploadSession,
        bytes: Bytes,
    ) -> ProtocolResultValue<HttpRequest> {
        if bytes.len() != session.content_length {
            return Err(ProtocolError::invalid_request(
                "Gemini upload bytes do not match declared content length",
            ));
        }
        let mut request = HttpRequest::new(Method::POST, session.upload_url.clone());
        request.body = HttpBody::Bytes {
            bytes,
            content_type: Some(HeaderValue::from_str(&session.mime_type).map_err(|_| {
                ProtocolError::invalid_request("Gemini upload MIME type is invalid")
            })?),
        };
        request.headers.insert(
            HeaderName::from_static("x-goog-upload-offset"),
            HeaderValue::from_static("0"),
        );
        request.headers.insert(
            HeaderName::from_static("x-goog-upload-command"),
            HeaderValue::from_static("upload, finalize"),
        );
        finish_request(&mut request, context)
    }

    pub(crate) fn decode_file(&self, response: HttpResponse) -> ProtocolResultValue<GeminiFile> {
        ensure_success(&response)?;
        let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
        let file = value.get("file").unwrap_or(&value);
        let file: GeminiFile = serde_json::from_value(file.clone()).map_err(|error| {
            ProtocolError::invalid_response(format!("Gemini file response is invalid: {error}"))
        })?;
        validate_file_name(&file.name)?;
        Ok(file)
    }

    pub(crate) fn get(
        &self,
        context: &CodecContext,
        name: &str,
    ) -> ProtocolResultValue<HttpRequest> {
        json_request(
            context,
            Method::GET,
            &validate_file_name(name)?,
            Value::Null,
        )
    }

    pub(crate) fn delete(
        &self,
        context: &CodecContext,
        name: &str,
    ) -> ProtocolResultValue<HttpRequest> {
        json_request(
            context,
            Method::DELETE,
            &validate_file_name(name)?,
            Value::Null,
        )
    }
}

fn validate_file_name(value: &str) -> ProtocolResultValue<String> {
    if !value.starts_with("files/")
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'/' | b'-')
        })
    {
        return Err(ProtocolError::invalid_request(
            "Gemini file name is invalid",
        ));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CodecInput, CodecLimits, ProtocolContractHarness};
    use buckyos_api::{AiMessage, EmbeddingMultimodalItem, VideoTextToVideoRequest};
    use futures_util::{stream, StreamExt};
    use reqwest::header::HeaderMap;

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            credential: Some(gemini_api_key("secret://gemini", "top-secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
                max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            },
        }
    }

    fn response(status: StatusCode, content_type: &str, value: Value) -> HttpResponse {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
        HttpResponse {
            status,
            headers,
            body: Bytes::from(serde_json::to_vec(&value).unwrap()),
            request_id: "request-1".to_string(),
            retry_after: None,
        }
    }

    fn completed_text() -> Value {
        json!({
            "id":"interaction-1",
            "object":"interaction",
            "status":"completed",
            "model":"gemini-test",
            "outputs":[{"type":"text","text":"hello"}],
            "usage":{"total_input_tokens":4,"total_output_tokens":3,"total_tokens":7}
        })
    }

    #[test]
    fn descriptor_registers_frozen_operations_without_generate_content() {
        let (descriptor, registration) = gemini_interactions_adapter();
        assert_eq!(descriptor.protocol_adapter_id, GEMINI_ADAPTER_ID);
        assert_eq!(descriptor.operations.len(), 3);
        assert!(descriptor
            .operations
            .contains_key(GEMINI_INTERACTIONS_OPERATION_ID));
        assert!(descriptor
            .operations
            .contains_key(GEMINI_EMBED_CONTENT_OPERATION_ID));
        assert!(descriptor
            .operations
            .contains_key(GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID));
        assert!(!format!("{descriptor:?}").contains("generateContent"));
        let mut registry = super::super::CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        registry
            .operation_descriptor(
                GEMINI_ADAPTER_ID,
                GEMINI_INTERACTIONS_OPERATION_ID,
                ApiType::AudioMusic,
            )
            .unwrap();
        registry
            .operation_descriptor(
                GEMINI_ADAPTER_ID,
                GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
                ApiType::VideoExtend,
            )
            .unwrap();
    }

    #[test]
    fn interaction_request_maps_messages_tools_and_redacts_auth() {
        let (descriptor, registration) = gemini_interactions_adapter();
        let mut registry = super::super::CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        let mut request = LlmChatInvokeRequest::new(
            "gemini-test@google",
            vec![
                AiMessage::text(AiRole::System, "be concise"),
                AiMessage::text(AiRole::User, "hello"),
            ],
        );
        request.max_output_tokens = Some(64);
        let input = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(request),
            resolved_parameters: BTreeMap::from([
                ("provider_model_id".to_string(), json!("gemini-test")),
                ("stream".to_string(), json!(true)),
            ]),
        };
        let wire = registry
            .encode(
                GEMINI_ADAPTER_ID,
                GEMINI_INTERACTIONS_OPERATION_ID,
                ApiType::Llm,
                &input,
                &context(),
            )
            .unwrap();
        assert_eq!(
            wire.url,
            "https://generativelanguage.googleapis.com/v1beta/interactions"
        );
        let HttpBody::Json(body) = &wire.body else {
            panic!("expected JSON body")
        };
        assert_eq!(body["model"], "gemini-test");
        assert_eq!(body["system_instruction"], "be concise");
        assert_eq!(body["generation_config"]["max_output_tokens"], 64);
        assert_eq!(body["stream"], true);
        let golden = ProtocolContractHarness::default()
            .redact_header(HeaderName::from_static("x-goog-api-key"))
            .request(&wire)
            .unwrap();
        assert_eq!(golden.headers["x-goog-api-key"], "[REDACTED]");
        assert!(!format!("{wire:?}").contains("top-secret"));
    }

    #[tokio::test]
    async fn immediate_interaction_normalizes_text_tool_and_usage() {
        let descriptor =
            gemini_interactions_adapter().0.operations[GEMINI_INTERACTIONS_OPERATION_ID].clone();
        let codec = GeminiInteractionCodec::new(descriptor, ApiType::Llm);
        let value = json!({
            "id":"interaction-1","status":"requires_action",
            "steps":[
                {"type":"model_output","content":[{"type":"text","text":"checking"}]},
                {"type":"function_call","id":"call-1","name":"weather","arguments":{"city":"Paris"}}
            ],
            "usage":{"total_input_tokens":4,"total_output_tokens":3,"total_tokens":7}
        });
        let ProtocolExecution::Immediate(output) = codec
            .decode(response(StatusCode::OK, "application/json", value))
            .await
            .unwrap()
        else {
            panic!("expected immediate")
        };
        assert_eq!(output.usage.unwrap().total_tokens, Some(7));
        assert_eq!(output.value["message"]["content"][0]["text"], "checking");
        assert_eq!(output.value["tool_calls"][0]["call_id"], "call-1");
    }

    #[tokio::test]
    async fn interaction_sse_is_incremental_and_requires_completed_event() {
        let descriptor =
            gemini_interactions_adapter().0.operations[GEMINI_INTERACTIONS_OPERATION_ID].clone();
        let codec = GeminiInteractionCodec::new(descriptor, ApiType::Llm);
        let completed =
            json!({"event_type":"interaction.completed","interaction":completed_text()});
        let chunks = vec![
            Ok(Bytes::from_static(b"event: step.delta\ndata: {\"event_type\":\"step.delta\",\"delta\":{\"type\":\"text\",\"text\":\"hel\"}}\n\n")),
            Ok(Bytes::from(format!("event: interaction.completed\ndata: {completed}\n\ndata: [DONE]\n\n"))),
        ];
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
        let wire = StreamingHttpResponse {
            status: StatusCode::OK,
            headers,
            body: Box::pin(stream::iter(chunks)),
            request_id: "stream-1".to_string(),
            retry_after: None,
        };
        let mut events = codec.decode_stream(wire).await.unwrap().events;
        assert!(matches!(
            events.next().await.unwrap().unwrap(),
            ProtocolEvent::Delta(_)
        ));
        assert!(matches!(
            events.next().await.unwrap().unwrap(),
            ProtocolEvent::Final(_)
        ));
        assert!(events.next().await.is_none());
    }

    #[test]
    fn embeddings_encode_text_and_multimodal_content() {
        let descriptor =
            gemini_interactions_adapter().0.operations[GEMINI_EMBED_CONTENT_OPERATION_ID].clone();
        let text_codec = GeminiEmbeddingCodec::new(descriptor.clone(), ApiType::EmbeddingText);
        let text = CodecInput {
            canonical_request: AiccCall::EmbeddingText(EmbeddingTextRequest::new(
                "embedding@google",
                vec![EmbeddingTextItem::Text {
                    text: "hello".to_string(),
                    id: Some("a".to_string()),
                }],
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                json!("gemini-embedding-001"),
            )]),
        };
        let wire = text_codec
            .encode(&CodecCall {
                api_type: ApiType::EmbeddingText,
                input: &text,
                context: &context(),
            })
            .unwrap();
        assert!(wire
            .url
            .ends_with("/v1beta/models/gemini-embedding-001:embedContent"));
        let HttpBody::Json(body) = wire.body else {
            panic!("expected JSON")
        };
        assert_eq!(body["content"]["parts"][0]["text"], "hello");

        let multimodal_codec = GeminiEmbeddingCodec::new(descriptor, ApiType::EmbeddingMultimodal);
        let multimodal = CodecInput {
            canonical_request: AiccCall::EmbeddingMultimodal(EmbeddingMultimodalRequest::new(
                "embedding@google",
                vec![EmbeddingMultimodalItem {
                    id: "m".to_string(),
                    text: Some("caption".to_string()),
                    image: Some(ResourceRef::base64(
                        "image/png".to_string(),
                        STANDARD.encode(b"png"),
                    )),
                }],
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                json!("gemini-embedding-2"),
            )]),
        };
        let wire = multimodal_codec
            .encode(&CodecCall {
                api_type: ApiType::EmbeddingMultimodal,
                input: &multimodal,
                context: &context(),
            })
            .unwrap();
        let HttpBody::Json(body) = wire.body else {
            panic!("expected JSON")
        };
        assert_eq!(body["content"]["parts"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn embedding_response_and_http_error_are_mapped() {
        let descriptor =
            gemini_interactions_adapter().0.operations[GEMINI_EMBED_CONTENT_OPERATION_ID].clone();
        let codec = GeminiEmbeddingCodec::new(descriptor, ApiType::EmbeddingText);
        let ProtocolExecution::Immediate(output) = codec.decode(response(StatusCode::OK, "application/json", json!({"embedding":{"values":[0.1,0.2]},"usageMetadata":{"promptTokenCount":2,"totalTokenCount":2}}))).await.unwrap() else { panic!("expected immediate") };
        assert_eq!(
            output.value["data"][0]["embedding"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let error = codec
            .decode(response(
                StatusCode::UNAUTHORIZED,
                "application/json",
                json!({"error":{"status":"UNAUTHENTICATED","message":"bad key"}}),
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Authentication);
        assert!(!format!("{error:?}").contains("top-secret"));
    }

    #[tokio::test]
    async fn video_native_task_maps_submit_status_and_result() {
        let descriptor = gemini_interactions_adapter().0.operations
            [GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID]
            .clone();
        let codec = GeminiVideoCodec::new(descriptor, ApiType::VideoTextToVideo);
        let request = CodecInput {
            canonical_request: AiccCall::VideoTextToVideo(VideoTextToVideoRequest::new(
                "veo@google",
                "ocean".to_string(),
            )),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                json!("veo-test"),
            )]),
        };
        let empty = BTreeMap::new();
        let ctx = context();
        let submit = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&request),
            resolved_parameters: &empty,
            context: &ctx,
        };
        let wire = codec.encode_native(&submit).unwrap();
        assert!(wire
            .url
            .ends_with("/v1beta/models/veo-test:predictLongRunning"));
        let NativeTaskOutput::Submitted(handle) = codec
            .decode_native(
                NativeTaskOperation::Submit,
                response(
                    StatusCode::OK,
                    "application/json",
                    json!({"name":"operations/video-1","done":false}),
                ),
            )
            .await
            .unwrap()
        else {
            panic!("expected handle")
        };
        assert_eq!(handle.remote_task_id, "operations/video-1");
        let lifecycle = NativeTaskInput {
            operation: NativeTaskOperation::Status,
            remote_task_id: Some("operations/video-1"),
            codec_input: None,
            resolved_parameters: &empty,
            context: &ctx,
        };
        assert!(codec
            .encode_native(&lifecycle)
            .unwrap()
            .url
            .ends_with("/v1beta/operations/video-1"));
        let NativeTaskOutput::Result(output) = codec.decode_native(NativeTaskOperation::Result, response(StatusCode::OK, "application/json", json!({"done":true,"response":{"outputs":[{"type":"video","mime_type":"video/mp4","data":STANDARD.encode(b"mp4")}]}}))).await.unwrap() else { panic!("expected result") };
        assert_eq!(output.artifacts.len(), 1);
    }

    #[test]
    fn files_support_resumable_upload_get_and_delete() {
        let files = GeminiFilesCodec;
        let ctx = context();
        let start = files
            .start_upload(&ctx, "clip.mp3", "audio/mpeg", 3)
            .unwrap();
        assert_eq!(
            start.url,
            "https://generativelanguage.googleapis.com/upload/v1beta/files"
        );
        assert_eq!(start.headers["x-goog-upload-command"], "start");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-upload-url",
            HeaderValue::from_static("https://upload.example/session-1"),
        );
        let session = files
            .decode_start_upload(
                HttpResponse {
                    status: StatusCode::OK,
                    headers,
                    body: Bytes::new(),
                    request_id: "request-1".to_string(),
                    retry_after: None,
                },
                3,
                "audio/mpeg",
            )
            .unwrap();
        let upload = files
            .upload(&ctx, &session, Bytes::from_static(b"mp3"))
            .unwrap();
        assert_eq!(upload.headers["x-goog-upload-command"], "upload, finalize");
        assert!(files
            .get(&ctx, "files/abc-123")
            .unwrap()
            .url
            .ends_with("/v1beta/files/abc-123"));
        assert_eq!(
            files.delete(&ctx, "files/abc-123").unwrap().method,
            Method::DELETE
        );
    }
}
