use super::minimax_messages::validate_minimax_response;
use super::{
    CodecCall, CodecContext, CodecRegistration,
    CredentialKind, ExecutionMode, HttpBody, HttpRequest, HttpResponse, MaterializedResource,
    NativeTaskCodec, NativeTaskHandle, NativeTaskInput, NativeTaskOperation, NativeTaskOutput,
    NativeTaskState, OperationBinding, OperationCodec, OperationDescriptor, ProtocolError,
    ProtocolErrorKind, ProtocolExecution, ProtocolOutput, ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiUsage, AiccCall, ApiType, ResourceRef, TextToImageInvokeRequest,
};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

const T2A_OPERATION_ID: &str = "t2a.create";
const IMAGE_OPERATION_ID: &str = "image_generation.create";
const VIDEO_OPERATION_ID: &str = "video_generation.create";
const MUSIC_OPERATION_ID: &str = "music_generation.create";
const DEFAULT_MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn minimax_media_registration() -> (Vec<OperationDescriptor>, CodecRegistration) {
    let t2a = immediate_operation(T2A_OPERATION_ID, &[ApiType::AudioTextToSpeech]);
    let image = immediate_operation(
        IMAGE_OPERATION_ID,
        &[ApiType::ImageTextToImage, ApiType::ImageImageToImage],
    );
    let music = immediate_operation(MUSIC_OPERATION_ID, &[ApiType::AudioMusic]);
    let video = OperationDescriptor {
        operation_id: VIDEO_OPERATION_ID.to_string(),
        bindings: [ApiType::VideoTextToVideo, ApiType::VideoImageToVideo]
            .into_iter()
            .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::NativeTask]))
            .collect(),
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    };
    let operation_codecs = [
        (t2a.clone(), ApiType::AudioTextToSpeech),
        (image.clone(), ApiType::ImageTextToImage),
        (image.clone(), ApiType::ImageImageToImage),
        (music.clone(), ApiType::AudioMusic),
    ]
    .into_iter()
    .map(|(descriptor, api_type)| {
        Arc::new(MiniMaxImmediateCodec {
            descriptor,
            api_type,
        }) as Arc<dyn OperationCodec>
    })
    .collect();
    let native_task_codecs = [ApiType::VideoTextToVideo, ApiType::VideoImageToVideo]
        .into_iter()
        .map(|api_type| {
            Arc::new(MiniMaxVideoCodec {
                descriptor: video.clone(),
                api_type,
            }) as Arc<dyn NativeTaskCodec>
        })
        .collect();
    (
        vec![t2a, image, music, video],
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

fn immediate_operation(id: &str, api_types: &[ApiType]) -> OperationDescriptor {
    OperationDescriptor {
        operation_id: id.to_string(),
        bindings: api_types
            .iter()
            .copied()
            .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::Immediate]))
            .collect(),
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    }
}

#[derive(Clone)]
struct MiniMaxImmediateCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl OperationCodec for MiniMaxImmediateCodec {
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
        require_only_model(&call.input.resolved_parameters)?;
        let model = provider_model_id(&call.input.resolved_parameters)?;
        let (path, body) = match (&call.input.canonical_request, self.api_type) {
            (AiccCall::AudioTextToSpeech(request), ApiType::AudioTextToSpeech) => {
                let voice_id = request.voice.voice_id.as_deref().ok_or_else(|| {
                    ProtocolError::new(
                        ProtocolErrorKind::UnsupportedOperation,
                        "MiniMax T2A requires voice.voice_id",
                    )
                })?;
                if request.voice.speaker_similarity_required
                    || request.voice.gender.is_some()
                    || request.voice.language.is_some()
                    || request.voice.style.is_some()
                {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::UnsupportedOperation,
                        "MiniMax T2A cannot satisfy the requested voice contract",
                    ));
                }
                let mut voice = Map::from_iter([("voice_id".to_string(), json!(voice_id))]);
                if let Some(speed) = request.speed {
                    voice.insert("speed".to_string(), json!(speed));
                }
                let mut body = Map::from_iter([
                    ("model".to_string(), json!(model)),
                    ("text".to_string(), json!(request.text)),
                    ("stream".to_string(), json!(false)),
                    ("output_format".to_string(), json!("hex")),
                    ("voice_setting".to_string(), Value::Object(voice)),
                ]);
                if let Some(output) = &request.output {
                    let mut audio = Map::new();
                    if let Some(sample_rate) = output.sample_rate {
                        audio.insert("sample_rate".to_string(), json!(sample_rate));
                    }
                    if let Some(media_type) = &output.media_type {
                        audio.insert("format".to_string(), json!(audio_format(media_type)?));
                    }
                    if !audio.is_empty() {
                        body.insert("audio_setting".to_string(), Value::Object(audio));
                    }
                }
                ("/v1/t2a_v2", Value::Object(body))
            }
            (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => {
                ("/v1/image_generation", Value::Object(image_body(request, &model)))
            }
            (AiccCall::ImageToImage(request), ApiType::ImageImageToImage) => {
                if request.images.len() != 1 {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::UnsupportedOperation,
                        "MiniMax image-to-image requires exactly one subject image",
                    ));
                }
                let mut body = Map::from_iter([
                    ("model".to_string(), json!(model)),
                    ("prompt".to_string(), json!(request.prompt)),
                    ("response_format".to_string(), json!("url")),
                ]);
                body.insert(
                    "subject_reference".to_string(),
                    json!([{"type":"character","image_file":resource_string(&request.images[0], call.context)?}]),
                );
                ("/v1/image_generation", Value::Object(body))
            }
            (AiccCall::AudioMusic(request), ApiType::AudioMusic) => {
                let mut body = Map::from_iter([
                    ("model".to_string(), json!(model)),
                    ("prompt".to_string(), json!(request.prompt)),
                ]);
                if let Some(lyrics) = &request.lyrics {
                    body.insert("lyrics".to_string(), json!(lyrics));
                }
                if let Some(output) = &request.output {
                    let mut audio = Map::new();
                    if let Some(sample_rate) = output.sample_rate {
                        audio.insert("sample_rate".to_string(), json!(sample_rate));
                    }
                    if let Some(media_type) = &output.media_type {
                        audio.insert("format".to_string(), json!(audio_format(media_type)?));
                    }
                    if !audio.is_empty() {
                        body.insert("audio_setting".to_string(), Value::Object(audio));
                    }
                }
                ("/v1/music_generation", Value::Object(body))
            }
            _ => {
                return Err(ProtocolError::invalid_request(
                    "MiniMax media codec received the wrong canonical request",
                ))
            }
        };
        media_json_request(call.context, Method::POST, path, body)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        validate_minimax_response(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let output = match self.api_type {
            ApiType::AudioTextToSpeech => decode_hex_audio(&value, "speech")?,
            ApiType::AudioMusic => decode_hex_audio(&value, "music")?,
            ApiType::ImageTextToImage | ApiType::ImageImageToImage => decode_images(&value)?,
            _ => {
                return Err(ProtocolError::invalid_response(
                    "MiniMax media codec has an invalid API type",
                ))
            }
        };
        Ok(ProtocolExecution::Immediate(output))
    }
}

#[derive(Clone)]
struct MiniMaxVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for MiniMaxVideoCodec {
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
            NativeTaskOperation::Status => {
                let task_id = safe_id(input.remote_task_id, "task ID")?;
                media_json_request(
                    input.context,
                    Method::GET,
                    &format!("/v1/query/video_generation?task_id={task_id}"),
                    Value::Null,
                )
            }
            NativeTaskOperation::Result => {
                let file_id = safe_id(input.remote_task_id, "file ID")?;
                media_json_request(
                    input.context,
                    Method::GET,
                    &format!("/v1/files/retrieve?file_id={file_id}"),
                    Value::Null,
                )
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "MiniMax video generation does not declare cancellation",
            )),
        }
    }

    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        validate_minimax_response(&response)?;
        let retry_after = response.retry_after;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        match operation {
            NativeTaskOperation::Submit => {
                let mut handle = NativeTaskHandle::new(required_string(&value, "task_id")?)?;
                handle.poll_after = retry_after.or(Some(Duration::from_secs(2)));
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let status = required_string(&value, "status")?;
                let state = match status.to_ascii_lowercase().as_str() {
                    "preparing" | "queueing" | "queued" => NativeTaskState::Queued,
                    "processing" | "running" => NativeTaskState::Running,
                    "success" | "succeeded" => NativeTaskState::Succeeded,
                    "fail" | "failed" => NativeTaskState::Failed,
                    _ => {
                        return Err(ProtocolError::invalid_response(
                            "MiniMax video status is unknown",
                        ))
                    }
                };
                let result_ref = if state == NativeTaskState::Succeeded {
                    Some(required_string(&value, "file_id")?)
                } else {
                    None
                };
                Ok(NativeTaskOutput::Status {
                    state,
                    retry_after,
                    result_ref,
                })
            }
            NativeTaskOperation::Result => {
                let url = value
                    .pointer("/file/download_url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.trim().is_empty())
                    .ok_or_else(|| {
                        ProtocolError::invalid_response(
                            "MiniMax file response is missing file.download_url",
                        )
                    })?;
                let resource = ResourceRef::url(url.to_string(), Some("video/mp4".to_string()));
                Ok(NativeTaskOutput::Result(ProtocolOutput {
                    value: json!({"video":resource}),
                    usage: Some(AiUsage::request_units(1)),
                    artifacts: vec![AiArtifact {
                        name: "video".to_string(),
                        resource,
                        mime: Some("video/mp4".to_string()),
                        metadata: None,
                    }],
                }))
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "MiniMax video generation does not declare cancellation",
            )),
        }
    }
}

fn encode_video_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("MiniMax video submit requires canonical input")
    })?;
    require_only_model(input.resolved_parameters)?;
    let model = provider_model_id(input.resolved_parameters)?;
    let mut body = match (&codec_input.canonical_request, api_type) {
        (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => Map::from_iter([
            ("model".to_string(), json!(model)),
            ("prompt".to_string(), json!(request.prompt)),
        ]),
        (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => Map::from_iter([
            ("model".to_string(), json!(model)),
            ("prompt".to_string(), json!(request.prompt)),
            (
                "first_frame_image".to_string(),
                json!(resource_string(&request.image, input.context)?),
            ),
        ]),
        _ => {
            return Err(ProtocolError::invalid_request(
                "MiniMax video codec received the wrong canonical request",
            ))
        }
    };
    match &codec_input.canonical_request {
        AiccCall::VideoTextToVideo(request) => {
            if let Some(duration) = request.duration_seconds {
                body.insert("duration".to_string(), json!(duration));
            }
            if let Some(resolution) = &request.resolution {
                body.insert("resolution".to_string(), json!(resolution));
            }
        }
        AiccCall::VideoImageToVideo(request) => {
            if let Some(duration) = request.duration_seconds {
                body.insert("duration".to_string(), json!(duration));
            }
            if let Some(resolution) = &request.resolution {
                body.insert("resolution".to_string(), json!(resolution));
            }
        }
        _ => {}
    }
    media_json_request(
        input.context,
        Method::POST,
        "/v1/video_generation",
        Value::Object(body),
    )
}

fn image_body(request: &TextToImageInvokeRequest, model: &str) -> Map<String, Value> {
    let mut body = Map::from_iter([
        ("model".to_string(), json!(model)),
        ("prompt".to_string(), json!(request.prompt)),
        ("response_format".to_string(), json!("url")),
    ]);
    if let Some(value) = request.n {
        body.insert("n".to_string(), json!(value));
    }
    if let Some(value) = &request.aspect_ratio {
        body.insert("aspect_ratio".to_string(), json!(value));
    }
    body
}

fn decode_images(value: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let mut resources = Vec::new();
    if let Some(urls) = value.pointer("/data/image_urls").and_then(Value::as_array) {
        for url in urls {
            let url = url.as_str().filter(|url| !url.trim().is_empty()).ok_or_else(|| {
                ProtocolError::invalid_response("MiniMax image URL must be a non-empty string")
            })?;
            resources.push(ResourceRef::url(url.to_string(), Some("image/png".to_string())));
        }
    } else if let Some(images) = value.pointer("/data/image_base64").and_then(Value::as_array) {
        for image in images {
            let data = image.as_str().filter(|data| !data.trim().is_empty()).ok_or_else(|| {
                ProtocolError::invalid_response("MiniMax base64 image must be a non-empty string")
            })?;
            STANDARD.decode(data).map_err(|_| {
                ProtocolError::invalid_response("MiniMax response contains invalid base64 image")
            })?;
            resources.push(ResourceRef::base64("image/jpeg".to_string(), data.to_string()));
        }
    }
    if resources.is_empty() {
        return Err(ProtocolError::invalid_response(
            "MiniMax image response contains no images",
        ));
    }
    let artifacts = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| AiArtifact {
            name: format!("image-{}", index + 1),
            resource: resource.clone(),
            mime: Some("image/png".to_string()),
            metadata: None,
        })
        .collect();
    Ok(ProtocolOutput {
        value: json!({"images":resources,"provider_states":[]}),
        usage: Some(AiUsage::request_units(1)),
        artifacts,
    })
}

fn decode_hex_audio(value: &Value, name: &str) -> ProtocolResultValue<ProtocolOutput> {
    let encoded = value
        .pointer("/data/audio")
        .and_then(Value::as_str)
        .filter(|audio| !audio.trim().is_empty())
        .ok_or_else(|| ProtocolError::invalid_response("MiniMax response is missing data.audio"))?;
    let bytes = decode_hex(encoded)?;
    let format = value
        .pointer("/extra_info/audio_format")
        .and_then(Value::as_str)
        .unwrap_or("mp3");
    let mime = audio_mime(format).to_string();
    let resource = ResourceRef::base64(mime.clone(), STANDARD.encode(bytes));
    Ok(ProtocolOutput {
        value: json!({"audio":resource}),
        usage: Some(AiUsage::request_units(1)),
        artifacts: vec![AiArtifact {
            name: name.to_string(),
            resource,
            mime: Some(mime),
            metadata: None,
        }],
    })
}

fn decode_hex(value: &str) -> ProtocolResultValue<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(ProtocolError::invalid_response(
            "MiniMax audio hex has an odd length",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| {
                ProtocolError::invalid_response("MiniMax audio hex is not ASCII")
            })?;
            u8::from_str_radix(text, 16).map_err(|_| {
                ProtocolError::invalid_response("MiniMax audio response contains invalid hex")
            })
        })
        .collect()
}

fn media_json_request(
    context: &CodecContext,
    method: Method,
    path: &str,
    body: Value,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("MiniMax base URL is invalid"))?;
    url.set_path(path.split('?').next().unwrap_or(path));
    url.set_query(path.split_once('?').map(|(_, query)| query));
    let mut request = HttpRequest::new(method, url.to_string());
    if !body.is_null() {
        request.body = HttpBody::Json(body);
    }
    apply_media_credential(&mut request.headers, context)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request)
}

fn apply_media_credential(
    headers: &mut HeaderMap,
    context: &CodecContext,
) -> ProtocolResultValue<()> {
    let credential = context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "MiniMax media operation requires a resolved credential",
        )
    })?;
    if credential.audit().kind != CredentialKind::NamedHeader {
        return Err(ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "MiniMax media operation requires the provider API key credential",
        ));
    }
    let mut source = HeaderMap::new();
    credential.apply(&mut source)?;
    let secret = source.values().next().ok_or_else(|| {
        ProtocolError::new(ProtocolErrorKind::Authentication, "MiniMax credential is empty")
    })?;
    let value = HeaderValue::from_bytes(format!("Bearer {}", secret.to_str().map_err(|_| {
        ProtocolError::new(ProtocolErrorKind::Authentication, "MiniMax credential is invalid")
    })?).as_bytes()).map_err(|_| {
        ProtocolError::new(ProtocolErrorKind::Authentication, "MiniMax credential is invalid")
    })?;
    headers.insert(AUTHORIZATION, value);
    Ok(())
}

fn provider_model_id(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<String> {
    parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved `provider_model_id`"))
}

fn require_only_model(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<()> {
    if let Some(name) = parameters.keys().find(|name| name.as_str() != "provider_model_id") {
        return Err(ProtocolError::invalid_request(format!(
            "resolved MiniMax parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

fn resource_string(
    resource: &ResourceRef,
    context: &CodecContext,
) -> ProtocolResultValue<String> {
    match resource {
        ResourceRef::Url { url, .. } => Ok(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("MiniMax resource contains invalid base64")
            })?;
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        ResourceRef::NamedObject { .. } => {
            let MaterializedResource { bytes, mime, .. } = context.materialized_resource(resource)?;
            Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
        }
    }
}

fn safe_id(value: Option<&str>, label: &str) -> ProtocolResultValue<String> {
    let value = value.filter(|value| {
        !value.is_empty()
            && value.len() <= 512
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    });
    value
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid_request(format!("MiniMax {label} is invalid")))
}

fn required_string(value: &Value, name: &str) -> ProtocolResultValue<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("MiniMax response is missing `{name}`"))
        })
}

fn audio_format(mime: &str) -> ProtocolResultValue<&'static str> {
    match mime.to_ascii_lowercase().as_str() {
        "audio/mpeg" | "audio/mp3" | "mp3" => Ok("mp3"),
        "audio/wav" | "audio/x-wav" | "wav" => Ok("wav"),
        "audio/flac" | "flac" => Ok("flac"),
        "audio/pcm" | "pcm" => Ok("pcm"),
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "MiniMax does not support the requested audio format",
        )),
    }
}

fn audio_mime(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "pcm" => "audio/pcm",
        _ => "audio/mpeg",
    }
}
