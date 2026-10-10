use super::minimax_messages::validate_minimax_response;
use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecContext, CodecRegistration, CredentialKind,
    ExecutionMode, HttpBody, HttpRequest, HttpResponse, MaterializedResource, MultipartBody,
    MultipartPart, NativeTaskCodec, NativeTaskHandle, NativeTaskInput, NativeTaskOperation,
    NativeTaskOutput, NativeTaskState, OperationBinding, OperationCodec, OperationDescriptor,
    ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput, ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{AiArtifact, AiUsage, AiccCall, ApiType, ResourceRef, TextToImageInvokeRequest};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

const T2A_OPERATION_ID: &str = "t2a.create";
const SPEECH_TO_TEXT_OPERATION_ID: &str = "speech_to_text.create";
pub(crate) const MINIMAX_MEDIA_ADAPTER_ID: &str = "minimax-media";
const IMAGE_OPERATION_ID: &str = "image_generation.create";
const VIDEO_OPERATION_ID: &str = "video_generation.create";
const VIDEO_V2_OPERATION_ID: &str = "video_generation.v2.create";
const MUSIC_OPERATION_ID: &str = "music_generation.create";
const DEFAULT_MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn minimax_media_registration() -> (Vec<OperationDescriptor>, CodecRegistration) {
    let t2a = immediate_operation(T2A_OPERATION_ID, &[ApiType::AudioTextToSpeech]);
    let speech_to_text = immediate_operation(
        SPEECH_TO_TEXT_OPERATION_ID,
        &[ApiType::AudioSpeechRecognition],
    );
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
    let video_v2 = OperationDescriptor {
        operation_id: VIDEO_V2_OPERATION_ID.to_string(),
        bindings: [ApiType::VideoTextToVideo, ApiType::VideoImageToVideo]
            .into_iter()
            .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::NativeTask]))
            .collect(),
        supports_cancel: true,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    };
    let operation_codecs = [
        (t2a.clone(), ApiType::AudioTextToSpeech),
        (speech_to_text.clone(), ApiType::AudioSpeechRecognition),
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
    let native_task_codecs = [(video.clone(), false), (video_v2.clone(), true)]
        .into_iter()
        .flat_map(|(descriptor, v2)| {
            [ApiType::VideoTextToVideo, ApiType::VideoImageToVideo].map(|api_type| {
                Arc::new(MiniMaxVideoCodec {
                    descriptor: descriptor.clone(),
                    api_type,
                    v2,
                }) as Arc<dyn NativeTaskCodec>
            })
        })
        .collect();
    (
        vec![t2a, speech_to_text, image, music, video, video_v2],
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

pub(crate) fn minimax_media_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (operations, registration) = minimax_media_registration();
    (
        AdapterDescriptor {
            protocol_family_id: "minimax".to_owned(),
            protocol_adapter_id: MINIMAX_MEDIA_ADAPTER_ID.to_owned(),
            interface_generation: "v1".to_owned(),
            base_adapter_id: None,
            component_adapter_ids: Vec::new(),
            status: AdapterStatus::Stable,
            probe_priority: 200,
            probe_path: None,
            credential: super::AdapterCredentialContract::named_header("x-api-key"),
            operations: operations
                .into_iter()
                .map(|operation| (operation.operation_id.clone(), operation))
                .collect(),
        },
        registration,
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
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // One struct serves several operations with different wire grammars, so
        // the form has to follow the api type rather than the codec. Deciding it
        // per codec would force the whole struct to the strictest member — the
        // multipart speech-to-text upload — and an image-edit caller would keep
        // downloading a payload the protocol could have taken as a URL.
        match self.api_type {
            // Speech-to-text posts the audio as a multipart file.
            ApiType::AudioSpeechRecognition => crate::resource::ResourceInputForm::BytesOnly,
            // Image-to-image addresses its subject image with `resource_string`,
            // which writes the URL verbatim.
            ApiType::ImageImageToImage => crate::resource::ResourceInputForm::UrlOrBytes,
            // The remaining operations carry no media input; stay on the
            // conservative default so a future addition fails safe.
            _ => crate::resource::ResourceInputForm::BytesOnly,
        }
    }

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
        if let (AiccCall::AudioSpeechRecognition(request), ApiType::AudioSpeechRecognition) =
            (&call.input.canonical_request, self.api_type)
        {
            require_only_model(&call.input.resolved_parameters)?;
            let model = provider_model_id(&call.input.resolved_parameters)?;
            let mut resource = call.context.materialized_resource(&request.audio)?.clone();
            let file_name = resource
                .file_name
                .take()
                .unwrap_or_else(|| default_audio_file_name(&resource.mime).to_owned());
            if request.output_formats.as_ref().is_some_and(|formats| {
                formats
                    .iter()
                    .any(|format| !matches!(format.as_str(), "json" | "verbose_json"))
            }) {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "MiniMax speech-to-text canonical output supports JSON only",
                ));
            }
            let response_format =
                if request.diarization == Some(true) || request.timestamps.is_some() {
                    "verbose_json"
                } else {
                    "json"
                };
            let mut body = MultipartBody::new(8, call.context.limits.max_request_bytes)?;
            body.push(MultipartPart::bytes("model", model))?;
            body.push(MultipartPart::file(
                "file",
                resource.bytes,
                file_name,
                resource.mime,
            ))?;
            body.push(MultipartPart::bytes("response_format", response_format))?;
            body.push(MultipartPart::bytes("stream", "false"))?;
            if let Some(level) = &request.timestamps {
                let level = match level.as_str() {
                    "segment" => "sentence",
                    "word" | "both" => "word",
                    _ => {
                        return Err(ProtocolError::invalid_request(
                            "MiniMax transcription timestamps must be segment, word, or both",
                        ));
                    }
                };
                body.push(MultipartPart::bytes("timestamp_level", level))?;
            }
            let mut encoded = media_multipart_request(call.context, "/v1/speech_to_text", body)?;
            if let Some(language) = request
                .language
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                encoded.headers.insert(
                    reqwest::header::HeaderName::from_static("language"),
                    HeaderValue::from_str(language).map_err(|_| {
                        ProtocolError::invalid_request("MiniMax transcription language is invalid")
                    })?,
                );
            }
            return Ok(encoded);
        }
        if self.api_type == ApiType::AudioTextToSpeech {
            require_parameters(&call.input.resolved_parameters, &["voice_setting"])?;
        } else {
            require_only_model(&call.input.resolved_parameters)?;
        }
        let model = provider_model_id(&call.input.resolved_parameters)?;
        let (path, body) = match (&call.input.canonical_request, self.api_type) {
            (AiccCall::AudioTextToSpeech(request), ApiType::AudioTextToSpeech) => {
                let mut voice = call
                    .input
                    .resolved_parameters
                    .get("voice_setting")
                    .and_then(Value::as_object)
                    .cloned()
                    .ok_or_else(|| {
                        ProtocolError::new(
                            ProtocolErrorKind::UnsupportedOperation,
                            "MiniMax T2A requires resolved voice_setting",
                        )
                    })?;
                if !voice.get("voice_id").is_some_and(Value::is_string) {
                    return Err(ProtocolError::invalid_request(
                        "resolved MiniMax voice_setting.voice_id must be a string",
                    ));
                }
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
            (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => (
                "/v1/image_generation",
                Value::Object(image_body(request, &model)),
            ),
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
                match (&request.lyrics, request.instrumental) {
                    (Some(lyrics), _) => {
                        body.insert("lyrics".to_string(), json!(lyrics));
                    }
                    (None, Some(true)) => {
                        body.insert("is_instrumental".to_string(), json!(true));
                    }
                    // Vocal tracks need lyrics; let MiniMax write them from the prompt.
                    (None, _) => {
                        body.insert("lyrics_optimizer".to_string(), json!(true));
                    }
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
                ));
            }
        };
        media_json_request(call.context, Method::POST, path, body)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        validate_minimax_response(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let output = match self.api_type {
            ApiType::AudioTextToSpeech => decode_hex_audio(&value, "speech")?,
            ApiType::AudioSpeechRecognition => decode_speech_to_text(&value)?,
            ApiType::AudioMusic => decode_hex_audio(&value, "music")?,
            ApiType::ImageTextToImage | ApiType::ImageImageToImage => decode_images(&value)?,
            _ => {
                return Err(ProtocolError::invalid_response(
                    "MiniMax media codec has an invalid API type",
                ));
            }
        };
        Ok(ProtocolExecution::Immediate(output))
    }
}

#[derive(Clone)]
struct MiniMaxVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
    v2: bool,
}

#[async_trait]
impl NativeTaskCodec for MiniMaxVideoCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // `resource_string` hands a URL straight through; only caller-supplied
        // bytes are inlined.
        crate::resource::ResourceInputForm::UrlOrBytes
    }

    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        self.api_type
    }

    fn operations(&self) -> BTreeSet<NativeTaskOperation> {
        let mut operations = BTreeSet::from([
            NativeTaskOperation::Submit,
            NativeTaskOperation::Status,
            NativeTaskOperation::Result,
        ]);
        if self.v2 {
            operations.insert(NativeTaskOperation::Cancel);
        }
        operations
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        match input.operation {
            NativeTaskOperation::Submit => encode_video_submit(input, self.api_type, self.v2),
            NativeTaskOperation::Status => {
                let task_id = safe_id(input.remote_task_id, "task ID")?;
                let path = if self.v2 {
                    format!("/v2/query/video_generation/{task_id}")
                } else {
                    format!("/v1/query/video_generation?task_id={task_id}")
                };
                media_json_request(input.context, Method::GET, &path, Value::Null)
            }
            NativeTaskOperation::Result => {
                let result_id = safe_id(input.remote_task_id, "result ID")?;
                let path = if self.v2 {
                    format!("/v2/query/video_generation/{result_id}")
                } else {
                    format!("/v1/files/retrieve?file_id={result_id}")
                };
                media_json_request(input.context, Method::GET, &path, Value::Null)
            }
            NativeTaskOperation::Cancel if self.v2 => {
                let task_id = safe_id(input.remote_task_id, "task ID")?;
                media_json_request(
                    input.context,
                    Method::DELETE,
                    &format!("/v2/video_generation/{task_id}"),
                    Value::Null,
                )
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "MiniMax legacy video generation does not declare cancellation",
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
                handle.cancel_supported = self.v2;
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let status = if self.v2 {
                    value
                        .pointer("/task/status")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            ProtocolError::invalid_response(
                                "MiniMax V2 video response is missing task.status",
                            )
                        })?
                        .to_owned()
                } else {
                    required_string(&value, "status")?
                };
                let state = match status.to_ascii_lowercase().as_str() {
                    "preparing" | "queueing" | "queued" => NativeTaskState::Queued,
                    "processing" | "running" => NativeTaskState::Running,
                    "success" | "succeeded" => NativeTaskState::Succeeded,
                    "fail" | "failed" => {
                        let error = value
                            .pointer("/task/error")
                            .or_else(|| value.get("error"))
                            .unwrap_or(&value);
                        return Err(ProtocolError::new(
                            ProtocolErrorKind::ProviderRejected,
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("MiniMax video task failed"),
                        )
                        .with_provider_code(error.get("code").filter(|code| !code.is_null()).map(
                            |code| {
                                code.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| code.to_string())
                            },
                        ))
                        .with_http_status(response.status.as_u16())
                        .with_request_id(Some(response.request_id.clone())));
                    }
                    "cancelled" | "canceled" => NativeTaskState::Cancelled,
                    _ => {
                        return Err(ProtocolError::invalid_response(
                            "MiniMax video status is unknown",
                        ));
                    }
                };
                let result_ref = if state == NativeTaskState::Succeeded {
                    Some(if self.v2 {
                        value
                            .pointer("/task/id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .ok_or_else(|| {
                                ProtocolError::invalid_response(
                                    "MiniMax V2 video response is missing task.id",
                                )
                            })?
                            .to_owned()
                    } else {
                        required_string(&value, "file_id")?
                    })
                } else {
                    None
                };
                Ok(NativeTaskOutput::Status {
                    result_usage: None,
                    state,
                    retry_after: retry_after.or(Some(Duration::from_secs(2))),
                    result_ref,
                    result_artifacts: BTreeMap::new(),
                })
            }
            NativeTaskOperation::Result => {
                let pointer = if self.v2 {
                    "/task/content/url"
                } else {
                    "/file/download_url"
                };
                let url = value
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .filter(|url| !url.trim().is_empty())
                    .ok_or_else(|| {
                        ProtocolError::invalid_response(
                            "MiniMax video result is missing its download URL",
                        )
                    })?;
                let resource = ResourceRef::url(url.to_string(), Some("video/mp4".to_string()));
                Ok(NativeTaskOutput::Result(ProtocolOutput {
                    value: json!({"video":resource}),
                    usage: Some(AiUsage {
                        video_seconds: value
                            .pointer("/task/usage/output_seconds")
                            .or_else(|| value.pointer("/task/duration"))
                            .and_then(Value::as_f64),
                        ..AiUsage::request_units(1)
                    }),
                    artifacts: vec![AiArtifact {
                        name: "video".to_string(),
                        resource,
                        mime: Some("video/mp4".to_string()),
                        metadata: None,
                    }],
                }))
            }
            NativeTaskOperation::Cancel if self.v2 => {
                let accepted = value
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| matches!(status, "cancelled" | "deleted"));
                Ok(NativeTaskOutput::Cancelled { accepted })
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "MiniMax legacy video generation does not declare cancellation",
            )),
        }
    }
}

fn encode_video_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
    v2: bool,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("MiniMax video submit requires canonical input")
    })?;
    require_only_model(input.resolved_parameters)?;
    let model = provider_model_id(input.resolved_parameters)?;
    let mut body = match (&codec_input.canonical_request, api_type) {
        (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => Map::from_iter([
            ("model".to_string(), json!(model)),
            (
                if v2 { "content" } else { "prompt" }.to_string(),
                if v2 {
                    json!([{"type":"text","text":request.prompt}])
                } else {
                    json!(request.prompt)
                },
            ),
        ]),
        (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => {
            let image = resource_string(&request.image, input.context)?;
            if v2 {
                Map::from_iter([
                    ("model".to_string(), json!(model)),
                    (
                        "content".to_string(),
                        json!([
                            {"type":"text","text":request.prompt},
                            {"type":"image_url","image_url":{"url":image},"role":"first_frame"}
                        ]),
                    ),
                ])
            } else {
                Map::from_iter([
                    ("model".to_string(), json!(model)),
                    ("prompt".to_string(), json!(request.prompt)),
                    ("first_frame_image".to_string(), json!(image)),
                ])
            }
        }
        _ => {
            return Err(ProtocolError::invalid_request(
                "MiniMax video codec received the wrong canonical request",
            ));
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
    if v2 {
        let duration = body
            .get("duration")
            .and_then(Value::as_f64)
            .unwrap_or(5.0)
            .round() as u64;
        body.insert("duration".to_string(), json!(duration));
        let resolution = body
            .get("resolution")
            .and_then(Value::as_str)
            .unwrap_or("768P");
        let resolution = match resolution.to_ascii_lowercase().as_str() {
            "480p" => "480P",
            "720p" | "768p" => "768P",
            "2k" | "1440p" => "2K",
            _ => {
                return Err(ProtocolError::invalid_request(
                    "MiniMax V2 resolution must be 480P, 768P, or 2K",
                ))
            }
        };
        body.insert("resolution".to_string(), json!(resolution));
        let ratio = match &codec_input.canonical_request {
            AiccCall::VideoTextToVideo(request) => {
                request.aspect_ratio.as_deref().unwrap_or("16:9")
            }
            AiccCall::VideoImageToVideo(_) => "adaptive",
            _ => unreachable!(),
        };
        body.insert("ratio".to_string(), json!(ratio));
    }
    media_json_request(
        input.context,
        Method::POST,
        if v2 {
            "/v2/video_generation"
        } else {
            "/v1/video_generation"
        },
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
            let url = url
                .as_str()
                .filter(|url| !url.trim().is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_response("MiniMax image URL must be a non-empty string")
                })?;
            resources.push(ResourceRef::url(
                url.to_string(),
                Some("image/png".to_string()),
            ));
        }
    } else if let Some(images) = value
        .pointer("/data/image_base64")
        .and_then(Value::as_array)
    {
        for image in images {
            let data = image
                .as_str()
                .filter(|data| !data.trim().is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_response(
                        "MiniMax base64 image must be a non-empty string",
                    )
                })?;
            STANDARD.decode(data).map_err(|_| {
                ProtocolError::invalid_response("MiniMax response contains invalid base64 image")
            })?;
            resources.push(ResourceRef::base64(
                "image/jpeg".to_string(),
                data.to_string(),
            ));
        }
    }
    if resources.is_empty() {
        return Err(ProtocolError::invalid_response(
            "MiniMax image response contains no images",
        ));
    }
    let image_units = resources.len() as u64;
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
        usage: Some(AiUsage {
            image_units: Some(image_units),
            ..AiUsage::request_units(1)
        }),
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
    let duration_ms = value
        .pointer(if name == "music" {
            "/extra_info/music_duration"
        } else {
            "/extra_info/audio_length"
        })
        .and_then(Value::as_f64)
        .filter(|duration| duration.is_finite() && *duration >= 0.0);
    Ok(ProtocolOutput {
        value: json!({"audio":resource}),
        usage: Some(AiUsage {
            audio_seconds: duration_ms.map(|duration| duration / 1_000.0),
            ..AiUsage::request_units(1)
        }),
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
            let text = std::str::from_utf8(pair)
                .map_err(|_| ProtocolError::invalid_response("MiniMax audio hex is not ASCII"))?;
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

fn media_multipart_request(
    context: &CodecContext,
    path: &str,
    body: MultipartBody,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("MiniMax base URL is invalid"))?;
    url.set_path(path);
    url.set_query(None);
    let mut request = HttpRequest::new(Method::POST, url.to_string());
    request.body = HttpBody::Multipart(body);
    apply_media_credential(&mut request.headers, context)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request)
}

fn default_audio_file_name(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or(mime).trim() {
        "audio/mpeg" | "audio/mp3" => "audio-input.mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "audio-input.wav",
        "audio/mp4" | "audio/x-m4a" => "audio-input.m4a",
        "audio/ogg" => "audio-input.ogg",
        "audio/flac" | "audio/x-flac" => "audio-input.flac",
        "audio/aiff" => "audio-input.aiff",
        "audio/aac" => "audio-input.aac",
        "audio/opus" => "audio-input.opus",
        _ => "audio-input.bin",
    }
}

fn decode_speech_to_text(value: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_response("MiniMax ASR response is missing text"))?;
    let segments = value
        .get("segments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, segment)| {
                    json!({
                        "id": segment.get("id").map(value_string).unwrap_or_else(|| index.to_string()),
                        "start_seconds": segment.get("start").or_else(|| segment.get("start_time")).and_then(Value::as_f64).unwrap_or(0.0),
                        "end_seconds": segment.get("end").or_else(|| segment.get("end_time")).and_then(Value::as_f64).unwrap_or(0.0),
                        "text": segment.get("text").and_then(Value::as_str).unwrap_or(""),
                        "speaker": segment.get("speaker").cloned().unwrap_or(Value::Null),
                        "confidence": segment.get("confidence").cloned().unwrap_or(Value::Null)
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let duration = value.get("duration").and_then(Value::as_f64);
    Ok(ProtocolOutput {
        value: json!({
            "text": text,
            "segments": segments,
            "artifacts": {},
            "diagnostic": {
                "duration": duration,
                "n_speakers": value.get("n_speakers"),
                "trace_id": value.get("trace_id")
            }
        }),
        usage: duration.map(|audio_seconds| AiUsage {
            audio_seconds: Some(audio_seconds),
            ..AiUsage::request_units(1)
        }),
        artifacts: Vec::new(),
    })
}

fn value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
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
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "MiniMax credential is empty",
        )
    })?;
    let value = HeaderValue::from_bytes(
        format!(
            "Bearer {}",
            secret.to_str().map_err(|_| {
                ProtocolError::new(
                    ProtocolErrorKind::Authentication,
                    "MiniMax credential is invalid",
                )
            })?
        )
        .as_bytes(),
    )
    .map_err(|_| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "MiniMax credential is invalid",
        )
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
    require_parameters(parameters, &[])
}

fn require_parameters(
    parameters: &BTreeMap<String, Value>,
    allowed: &[&str],
) -> ProtocolResultValue<()> {
    if let Some(name) = parameters
        .keys()
        .find(|name| name.as_str() != "provider_model_id" && !allowed.contains(&name.as_str()))
    {
        return Err(ProtocolError::invalid_request(format!(
            "resolved MiniMax parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

fn resource_string(resource: &ResourceRef, context: &CodecContext) -> ProtocolResultValue<String> {
    // Materialization may have handed this resource over as a URL because the
    // protocol takes one (`ResourceInputForm`), in which case there are no
    // bytes to inline and the URL is the only usable form.
    if let Some(url) = context.materialized_url(resource) {
        return Ok(url.to_string());
    }
    match resource {
        ResourceRef::Url { url, .. } => Ok(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("MiniMax resource contains invalid base64")
            })?;
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        ResourceRef::NamedObject { .. } => {
            let MaterializedResource { bytes, mime, .. } =
                context.materialized_resource(resource)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CodecInput, CodecLimits, ResolvedCredential};
    use buckyos_api::{AudioMusicRequest, ProviderStateCoordinate, VideoTextToVideoRequest};
    use bytes::Bytes;
    use reqwest::StatusCode;
    use serde_json::json;

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://api.minimax.io".to_owned(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "minimax".to_owned(),
                adapter_type: MINIMAX_MEDIA_ADAPTER_ID.to_owned(),
                origin_provider: "minimax".to_owned(),
                origin_model: "MiniMax-H3".to_owned(),
            },
            credential: Some(
                ResolvedCredential::named_header("secret://minimax", "x-api-key", "secret")
                    .unwrap(),
            ),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(10),
                max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
                max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            },
        }
    }

    fn response(value: Value) -> HttpResponse {
        HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from(serde_json::to_vec(&value).unwrap()),
            request_id: "request-1".to_owned(),
            retry_after: None,
        }
    }

    #[test]
    fn speech_to_text_normalizes_numeric_segment_ids() {
        let output = decode_speech_to_text(&json!({
            "text": "hello",
            "duration": 1.0,
            "segments": [{"id": 0, "start": 0.0, "end": 1.0, "text": "hello"}]
        }))
        .unwrap();
        assert_eq!(output.value["segments"][0]["id"], "0");
    }

    #[test]
    fn music_without_lyrics_is_instrumental_or_gets_generated_lyrics() {
        let descriptor = minimax_media_adapter().0.operations[MUSIC_OPERATION_ID].clone();
        let codec = MiniMaxImmediateCodec {
            descriptor,
            api_type: ApiType::AudioMusic,
        };
        let context = context();
        let body = |instrumental: Option<bool>, lyrics: Option<&str>| {
            let mut request = AudioMusicRequest::new("ignored", "calm piano".to_owned());
            request.instrumental = instrumental;
            request.lyrics = lyrics.map(str::to_owned);
            let input = CodecInput {
                canonical_request: AiccCall::AudioMusic(request),
                resolved_parameters: BTreeMap::from([(
                    "provider_model_id".to_owned(),
                    json!("music-2.6"),
                )]),
            };
            let wire = codec
                .encode(&CodecCall {
                    api_type: ApiType::AudioMusic,
                    input: &input,
                    context: &context,
                })
                .unwrap();
            let HttpBody::Json(body) = wire.body else {
                panic!("expected JSON")
            };
            body
        };

        let vocal = body(None, None);
        assert_eq!(vocal["lyrics_optimizer"], true);
        assert!(vocal.get("lyrics").is_none());
        let instrumental = body(Some(true), None);
        assert_eq!(instrumental["is_instrumental"], true);
        assert!(instrumental.get("lyrics_optimizer").is_none());
        let written = body(Some(false), Some("la la"));
        assert_eq!(written["lyrics"], "la la");
        assert!(written.get("lyrics_optimizer").is_none());
    }

    #[tokio::test]
    async fn video_v2_maps_submit_status_result_and_cancel() {
        let descriptor = minimax_media_adapter().0.operations[VIDEO_V2_OPERATION_ID].clone();
        let codec = MiniMaxVideoCodec {
            descriptor,
            api_type: ApiType::VideoTextToVideo,
            v2: true,
        };
        let mut request = VideoTextToVideoRequest::new("ignored", "ocean".to_owned());
        request.duration_seconds = Some(6.0);
        request.aspect_ratio = Some("9:16".to_owned());
        request.resolution = Some("720p".to_owned());
        let parameters = BTreeMap::from([("provider_model_id".to_owned(), json!("MiniMax-H3"))]);
        let codec_input = CodecInput {
            canonical_request: AiccCall::VideoTextToVideo(request),
            resolved_parameters: parameters.clone(),
        };
        let context = context();
        let submit = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&codec_input),
            resolved_parameters: &parameters,
            context: &context,
        };
        let wire = codec.encode_native(&submit).unwrap();
        assert_eq!(wire.method, Method::POST);
        assert_eq!(wire.url, "https://api.minimax.io/v2/video_generation");
        let HttpBody::Json(body) = wire.body else {
            panic!("expected JSON")
        };
        assert_eq!(body["model"], "MiniMax-H3");
        assert_eq!(body["content"][0], json!({"type":"text","text":"ocean"}));
        assert_eq!(body["duration"], 6);
        assert_eq!(body["resolution"], "768P");
        assert_eq!(body["ratio"], "9:16");

        let NativeTaskOutput::Submitted(handle) = codec
            .decode_native(
                NativeTaskOperation::Submit,
                response(json!({"task_id":"video-1","base_resp":{"status_code":0}})),
            )
            .await
            .unwrap()
        else {
            panic!("expected submitted task")
        };
        assert_eq!(handle.remote_task_id, "video-1");
        assert!(handle.cancel_supported);

        let lifecycle = |operation| NativeTaskInput {
            operation,
            remote_task_id: Some("video-1"),
            codec_input: None,
            resolved_parameters: &parameters,
            context: &context,
        };
        assert_eq!(
            codec
                .encode_native(&lifecycle(NativeTaskOperation::Status))
                .unwrap()
                .url,
            "https://api.minimax.io/v2/query/video_generation/video-1"
        );
        let NativeTaskOutput::Status {
            state, result_ref, ..
        } = codec
            .decode_native(
                NativeTaskOperation::Status,
                response(json!({"task":{"id":"video-1","status":"succeeded"},"base_resp":{"status_code":0}})),
            )
            .await
            .unwrap()
        else {
            panic!("expected task status")
        };
        assert_eq!(state, NativeTaskState::Succeeded);
        assert_eq!(result_ref.as_deref(), Some("video-1"));

        let result = codec
            .decode_native(
                NativeTaskOperation::Result,
                response(json!({"task":{"id":"video-1","status":"succeeded","content":{"url":"https://cdn.example/video.mp4"}},"base_resp":{"status_code":0}})),
            )
            .await
            .unwrap();
        let NativeTaskOutput::Result(output) = result else {
            panic!("expected task result")
        };
        assert_eq!(output.artifacts[0].name, "video");

        let cancel = codec
            .encode_native(&lifecycle(NativeTaskOperation::Cancel))
            .unwrap();
        assert_eq!(cancel.method, Method::DELETE);
        assert_eq!(
            cancel.url,
            "https://api.minimax.io/v2/video_generation/video-1"
        );
        assert!(matches!(
            codec
                .decode_native(
                    NativeTaskOperation::Cancel,
                    response(json!({"status":"cancelled","base_resp":{"status_code":0}})),
                )
                .await
                .unwrap(),
            NativeTaskOutput::Cancelled { accepted: true }
        ));
    }
}
