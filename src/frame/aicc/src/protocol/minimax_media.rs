use super::minimax_messages::validate_minimax_response;
use super::video::{
    generated_video, planned_resource, require_video_parameters, unsupported, video_generate_input,
    video_output, whole_seconds,
};
use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecContext, CodecRegistration, CredentialKind,
    ExecutionMode, HttpBody, HttpRequest, HttpResponse, MaterializedResource, MultipartBody,
    MultipartPart, NativeTaskCodec, NativeTaskHandle, NativeTaskInput, NativeTaskOperation,
    NativeTaskOutput, NativeTaskState, OperationBinding, OperationCodec, OperationDescriptor,
    ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput, ProtocolResultValue,
};
use crate::video::VideoScenario;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiUsage, AiccCall, ApiType, KeyframePosition, ResourceRef,
    TextToImageInvokeRequest, VideoConditionKind, VideoMetadata,
};
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
        bindings: vec![OperationBinding::new(
            ApiType::VideoGenerate,
            [ExecutionMode::NativeTask],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    };
    let video_v2 = OperationDescriptor {
        operation_id: VIDEO_V2_OPERATION_ID.to_string(),
        bindings: vec![OperationBinding::new(
            ApiType::VideoGenerate,
            [ExecutionMode::NativeTask],
        )],
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
        .map(|(descriptor, v2)| {
            Arc::new(MiniMaxVideoCodec { descriptor, v2 }) as Arc<dyn NativeTaskCodec>
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
        ApiType::VideoGenerate
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
            NativeTaskOperation::Submit => encode_video_submit(input, self.v2),
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
                let seconds = value
                    .pointer("/task/usage/output_seconds")
                    .or_else(|| value.pointer("/task/duration"))
                    .and_then(Value::as_f64);
                let video = generated_video(
                    ResourceRef::url(url.to_string(), Some("video/mp4".to_string())),
                    Some("video/mp4".to_string()),
                    VideoMetadata {
                        duration_seconds: value
                            .pointer("/task/duration")
                            .and_then(Value::as_f64)
                            .or(seconds),
                        ..Default::default()
                    },
                );
                Ok(NativeTaskOutput::Result(video_output(
                    vec![video],
                    AiUsage {
                        video_seconds: seconds,
                        ..AiUsage::request_units(1)
                    },
                )?))
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

fn encode_video_submit(input: &NativeTaskInput<'_>, v2: bool) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("MiniMax video submit requires canonical input")
    })?;
    require_video_parameters(input.resolved_parameters, "MiniMax", &[])?;
    let (request, plan) = video_generate_input(codec_input)?;
    let model = provider_model_id(input.resolved_parameters)?;
    let prompt = plan
        .submitted_prompt
        .clone()
        .filter(|prompt| !prompt.trim().is_empty())
        .ok_or_else(|| {
            ProtocolError::invalid_request("MiniMax video generation requires a non-empty prompt")
        })?;
    if plan.negative_prompt.is_some()
        || plan.seed.is_some()
        || plan.fps.is_some()
        || plan.size.is_some()
        || plan.generate_audio.is_some()
    {
        return Err(unsupported(
            "MiniMax video generation has no negative prompt, seed, fps, exact size or audio switch",
        ));
    }
    let duration = whole_seconds(&plan)?;
    let mut body = Map::from_iter([("model".to_string(), json!(model))]);
    if v2 {
        let mut content = vec![json!({"type": "text", "text": prompt})];
        for planned in &plan.inputs {
            let url = resource_string(planned_resource(request, planned)?, input.context)?;
            content.push(match (planned.kind, planned.position) {
                (VideoConditionKind::Keyframe, Some(KeyframePosition::Start)) => {
                    json!({"type": "image_url", "image_url": {"url": url}, "role": "first_frame"})
                }
                (VideoConditionKind::Keyframe, _) => {
                    json!({"type": "image_url", "image_url": {"url": url}, "role": "last_frame"})
                }
                (VideoConditionKind::ReferenceImage, _) => {
                    json!({"type": "image_url", "image_url": {"url": url}, "role": "reference_image"})
                }
                (VideoConditionKind::ReferenceVideo, _) => {
                    json!({"type": "video_url", "video_url": {"url": url}, "role": "reference_video"})
                }
                (VideoConditionKind::ReferenceAudio, _) => {
                    json!({"type": "audio_url", "audio_url": {"url": url}, "role": "reference_audio"})
                }
            });
        }
        body.insert("content".to_string(), Value::Array(content));
        body.insert(
            "duration".to_string(),
            json!(duration.ok_or_else(|| {
                ProtocolError::invalid_configuration(
                    "MiniMax V2 video needs a duration and the model declares no default",
                )
            })?),
        );
        body.insert(
            "resolution".to_string(),
            json!(plan.resolution.clone().ok_or_else(|| {
                ProtocolError::invalid_configuration(
                    "MiniMax V2 video needs a resolution and the model declares no default",
                )
            })?),
        );
        let ratio = match plan.scenario {
            VideoScenario::Keyframes => "adaptive".to_string(),
            VideoScenario::Text => plan.aspect_ratio.clone().ok_or_else(|| {
                ProtocolError::invalid_configuration(
                    "MiniMax V2 text-to-video needs a ratio and the model declares no default",
                )
            })?,
            VideoScenario::References => plan
                .aspect_ratio
                .clone()
                .unwrap_or_else(|| "adaptive".to_string()),
        };
        body.insert("ratio".to_string(), json!(ratio));
    } else {
        body.insert("prompt".to_string(), json!(prompt));
        for planned in &plan.inputs {
            if (planned.kind, planned.position)
                != (VideoConditionKind::Keyframe, Some(KeyframePosition::Start))
            {
                return Err(unsupported(
                    "MiniMax legacy video generation only takes a start keyframe",
                ));
            }
            body.insert(
                "first_frame_image".to_string(),
                json!(resource_string(
                    planned_resource(request, planned)?,
                    input.context
                )?),
            );
        }
        if let Some(duration) = duration {
            body.insert("duration".to_string(), json!(duration));
        }
        if let Some(resolution) = &plan.resolution {
            body.insert("resolution".to_string(), json!(resolution));
        }
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
    let mut images = Vec::new();
    if let Some(urls) = value.pointer("/data/image_urls").and_then(Value::as_array) {
        for url in urls {
            let url = url
                .as_str()
                .filter(|url| !url.trim().is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_response("MiniMax image URL must be a non-empty string")
                })?;
            let mime = image_mime_from_url(url).map(str::to_owned);
            images.push((ResourceRef::url(url.to_string(), mime.clone()), mime));
        }
    } else if let Some(encoded_images) = value
        .pointer("/data/image_base64")
        .and_then(Value::as_array)
    {
        for image in encoded_images {
            let data = image
                .as_str()
                .filter(|data| !data.trim().is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_response(
                        "MiniMax base64 image must be a non-empty string",
                    )
                })?;
            let bytes = STANDARD.decode(data).map_err(|_| {
                ProtocolError::invalid_response("MiniMax response contains invalid base64 image")
            })?;
            let mime = image_mime_from_bytes(&bytes).ok_or_else(|| {
                ProtocolError::invalid_response(
                    "MiniMax response contains an unrecognized base64 image",
                )
            })?;
            images.push((
                ResourceRef::base64(mime.to_string(), data.to_string()),
                Some(mime.to_string()),
            ));
        }
    }
    if images.is_empty() {
        return Err(ProtocolError::invalid_response(
            "MiniMax image response contains no images",
        ));
    }
    let image_units = images.len() as u64;
    let resources = images
        .iter()
        .map(|(resource, _)| resource.clone())
        .collect::<Vec<_>>();
    let artifacts = images
        .iter()
        .enumerate()
        .map(|(index, (resource, mime))| AiArtifact {
            name: format!("image-{}", index + 1),
            resource: resource.clone(),
            mime: mime.clone(),
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

fn image_mime_from_url(value: &str) -> Option<&'static str> {
    let path = Url::parse(value).ok()?.path().to_ascii_lowercase();
    if path.ends_with(".png") {
        Some("image/png")
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if path.ends_with(".webp") {
        Some("image/webp")
    } else if path.ends_with(".gif") {
        Some("image/gif")
    } else {
        None
    }
}

fn image_mime_from_bytes(value: &[u8]) -> Option<&'static str> {
    if value.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if value.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if value.starts_with(b"GIF87a") || value.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if value.len() >= 12 && &value[..4] == b"RIFF" && &value[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
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
        ResourceRef::Url { url, .. } => match context.materialized_resource(resource) {
            Ok(materialized) => Ok(format!(
                "data:{};base64,{}",
                materialized.mime,
                STANDARD.encode(&materialized.bytes)
            )),
            Err(_) => Ok(url.clone()),
        },
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
    use crate::video::test_support::{clip_facts, image_facts, model_contract, plan_parameters};
    use crate::video::VideoMaterialFacts;
    use buckyos_api::{
        AiccErrorCode, AudioMusicRequest, ProviderStateCoordinate, VideoAudioMode, VideoCondition,
        VideoGenerateRequest, VideoOutputSpec, VideoSize,
    };
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

    #[test]
    fn image_response_preserves_the_documented_image_format() {
        let url_output = decode_images(&json!({
            "data": {"image_urls": ["https://cdn.example/result.jpeg?token=one"]}
        }))
        .unwrap();
        assert_eq!(url_output.artifacts[0].mime.as_deref(), Some("image/jpeg"));

        let png = STANDARD.encode(b"\x89PNG\r\n\x1a\nrest");
        let base64_output = decode_images(&json!({"data": {"image_base64": [png]}})).unwrap();
        assert_eq!(
            base64_output.artifacts[0].mime.as_deref(),
            Some("image/png")
        );
    }

    fn video_submit(
        v2: bool,
        model: &str,
        request: VideoGenerateRequest,
        facts: VideoMaterialFacts,
    ) -> Value {
        let codec = MiniMaxVideoCodec {
            descriptor: minimax_media_adapter().0.operations[if v2 {
                VIDEO_V2_OPERATION_ID
            } else {
                VIDEO_OPERATION_ID
            }]
            .clone(),
            v2,
        };
        let parameters =
            plan_parameters(&model_contract("minimax", model), &request, &facts, model);
        let codec_input = CodecInput {
            canonical_request: AiccCall::VideoGenerate(request),
            resolved_parameters: parameters.clone(),
        };
        let context = context();
        let wire = codec
            .encode_native(&NativeTaskInput {
                operation: NativeTaskOperation::Submit,
                remote_task_id: None,
                codec_input: Some(&codec_input),
                resolved_parameters: &parameters,
                context: &context,
            })
            .unwrap();
        assert_eq!(wire.method, Method::POST);
        let HttpBody::Json(body) = wire.body else {
            panic!("expected JSON")
        };
        body
    }

    fn url(name: &str) -> ResourceRef {
        ResourceRef::url(format!("https://media.example/{name}"), None)
    }

    #[tokio::test]
    async fn video_v2_maps_submit_status_result_and_cancel() {
        let descriptor = minimax_media_adapter().0.operations[VIDEO_V2_OPERATION_ID].clone();
        let codec = MiniMaxVideoCodec {
            descriptor,
            v2: true,
        };
        let mut request = VideoGenerateRequest::new("MiniMax-H3@minimax", Some("ocean".into()));
        request.output = Some(VideoOutputSpec {
            duration_seconds: Some(6.0),
            aspect_ratio: Some("9:16".into()),
            size: Some(VideoSize::short_edge(768)),
            ..Default::default()
        });
        let body = video_submit(true, "MiniMax-H3", request, VideoMaterialFacts::new());
        assert_eq!(body["model"], "MiniMax-H3");
        assert_eq!(body["content"], json!([{"type":"text","text":"ocean"}]));
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

        let parameters = BTreeMap::new();
        let context = context();
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

        let NativeTaskOutput::Result(output) = codec
            .decode_native(
                NativeTaskOperation::Result,
                response(json!({"task":{"id":"video-1","status":"succeeded","duration":6,"usage":{"output_seconds":6.04},"content":{"url":"https://cdn.example/video.mp4"}},"base_resp":{"status_code":0}})),
            )
            .await
            .unwrap()
        else {
            panic!("expected task result")
        };
        assert_eq!(output.artifacts[0].name, "video");
        assert_eq!(
            output.value["videos"][0]["resource"]["url"],
            "https://cdn.example/video.mp4"
        );
        assert_eq!(
            output.value["videos"][0]["provider_metadata"]["duration_seconds"],
            6.0
        );
        assert_eq!(output.usage.unwrap().video_seconds, Some(6.04));

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

    #[test]
    fn h3_defaults_come_from_the_model_contract() {
        let body = video_submit(
            true,
            "MiniMax-H3",
            VideoGenerateRequest::new("MiniMax-H3@minimax", Some("ocean".into())),
            VideoMaterialFacts::new(),
        );
        assert_eq!(body["duration"], 4);
        assert_eq!(body["resolution"], "768P");
        assert_eq!(body["ratio"], "16:9");
        let body = video_submit(
            true,
            "MiniMax-H3-Max",
            VideoGenerateRequest::new("MiniMax-H3-Max@minimax", Some("ocean".into())),
            VideoMaterialFacts::new(),
        );
        assert_eq!(body["duration"], 5);
        assert_eq!(body["resolution"], "768P");
    }

    #[test]
    fn h3_keyframes_and_references_map_to_content_roles_and_prompt_labels() {
        let mut keyframes =
            VideoGenerateRequest::new("MiniMax-H3@minimax", Some("from @{a} to @{b}".into()));
        keyframes.conditions = vec![
            VideoCondition::Keyframe {
                id: "b".into(),
                resource: url("b.png"),
                position: KeyframePosition::End,
            },
            VideoCondition::Keyframe {
                id: "a".into(),
                resource: url("a.png"),
                position: KeyframePosition::Start,
            },
        ];
        let body = video_submit(true, "MiniMax-H3", keyframes, VideoMaterialFacts::new());
        assert_eq!(
            body["content"],
            json!([
                {"type":"text","text":"from the first frame to the last frame"},
                {"type":"image_url","image_url":{"url":"https://media.example/b.png"},"role":"last_frame"},
                {"type":"image_url","image_url":{"url":"https://media.example/a.png"},"role":"first_frame"}
            ])
        );
        assert_eq!(body["ratio"], "adaptive");

        let mut references = VideoGenerateRequest::new(
            "MiniMax-H3@minimax",
            Some("@{hero} dances like @{moves}, voice @{voice}, outfit @{dress}".into()),
        );
        references.conditions = vec![
            VideoCondition::ReferenceImage {
                id: "hero".into(),
                resource: url("hero.png"),
            },
            VideoCondition::ReferenceVideo {
                id: "moves".into(),
                resource: url("moves.mp4"),
                time_range: None,
                include_audio: false,
            },
            VideoCondition::ReferenceAudio {
                id: "voice".into(),
                resource: url("voice.wav"),
                time_range: None,
            },
            VideoCondition::ReferenceImage {
                id: "dress".into(),
                resource: url("dress.png"),
            },
        ];
        let facts = VideoMaterialFacts::from([
            ("moves".to_string(), clip_facts(5.0, true, false)),
            ("voice".to_string(), clip_facts(4.0, false, true)),
        ]);
        let body = video_submit(true, "MiniMax-H3", references, facts);
        assert_eq!(
            body["content"][0]["text"],
            "reference image 1 dances like reference video 1, voice reference audio 1, outfit reference image 2"
        );
        assert_eq!(body["content"][2]["role"], "reference_video");
        assert_eq!(body["content"][3]["type"], "audio_url");
        assert_eq!(body["ratio"], "adaptive");
    }

    #[test]
    fn h3_contract_rejects_what_the_cloud_api_cannot_honor() {
        let contract = model_contract("minimax", "MiniMax-H3");
        let reject = |request: VideoGenerateRequest, facts: VideoMaterialFacts| {
            crate::video::plan_video(&contract, &request, &facts).unwrap_err()
        };
        let base = || VideoGenerateRequest::new("MiniMax-H3@minimax", Some("ocean".into()));

        let mut seed = base();
        seed.seed = Some(7);
        assert_eq!(
            reject(seed, VideoMaterialFacts::new()).code,
            AiccErrorCode::UnsupportedOperation
        );

        let mut fractional = base();
        fractional.output = Some(VideoOutputSpec {
            duration_seconds: Some(4.5),
            ..Default::default()
        });
        assert_eq!(
            reject(fractional, VideoMaterialFacts::new())
                .details
                .unwrap()["field"],
            "/output/duration_seconds"
        );

        let mut silent = base();
        silent.output = Some(VideoOutputSpec {
            audio: Some(VideoAudioMode::None),
            ..Default::default()
        });
        assert_eq!(
            reject(silent, VideoMaterialFacts::new()).details.unwrap()["field"],
            "/output/audio"
        );

        let mut mixed = base();
        mixed.conditions = vec![
            VideoCondition::Keyframe {
                id: "a".into(),
                resource: url("a.png"),
                position: KeyframePosition::Start,
            },
            VideoCondition::ReferenceImage {
                id: "b".into(),
                resource: url("b.png"),
            },
        ];
        assert_eq!(
            reject(mixed, VideoMaterialFacts::new()).code,
            AiccErrorCode::UnsupportedOperation
        );

        let mut long = base();
        long.conditions = (0..2)
            .map(|index| VideoCondition::ReferenceVideo {
                id: format!("v{index}"),
                resource: url("v.mp4"),
                time_range: None,
                include_audio: false,
            })
            .collect();
        let facts = VideoMaterialFacts::from([
            ("v0".to_string(), clip_facts(10.0, true, false)),
            ("v1".to_string(), clip_facts(8.0, true, false)),
        ]);
        assert!(reject(long, facts).message.contains("total"));

        let mut ratio = base();
        ratio.conditions = vec![VideoCondition::Keyframe {
            id: "a".into(),
            resource: url("a.png"),
            position: KeyframePosition::Start,
        }];
        ratio.output = Some(VideoOutputSpec {
            aspect_ratio: Some("9:16".into()),
            ..Default::default()
        });
        assert_eq!(
            reject(
                ratio.clone(),
                VideoMaterialFacts::from([("a".to_string(), image_facts(1280, 720))])
            )
            .details
            .unwrap()["field"],
            "/output/aspect_ratio"
        );
        assert!(crate::video::plan_video(
            &contract,
            &ratio,
            &VideoMaterialFacts::from([("a".to_string(), image_facts(720, 1280))])
        )
        .is_ok());
    }

    #[test]
    fn legacy_video_only_takes_text_and_a_start_frame() {
        let mut request =
            VideoGenerateRequest::new("MiniMax-Hailuo-02@minimax", Some("ocean".into()));
        request.conditions = vec![VideoCondition::Keyframe {
            id: "a".into(),
            resource: url("a.png"),
            position: KeyframePosition::Start,
        }];
        let body = video_submit(
            false,
            "MiniMax-Hailuo-02",
            request,
            VideoMaterialFacts::new(),
        );
        assert_eq!(body["prompt"], "ocean");
        assert_eq!(body["first_frame_image"], "https://media.example/a.png");
        assert_eq!(body["duration"], 6);
        assert_eq!(body["resolution"], "768P");

        let contract = model_contract("minimax", "MiniMax-Hailuo-02");
        let mut long_1080 =
            VideoGenerateRequest::new("MiniMax-Hailuo-02@minimax", Some("ocean".into()));
        long_1080.output = Some(VideoOutputSpec {
            duration_seconds: Some(10.0),
            size: Some(VideoSize::short_edge(1080)),
            ..Default::default()
        });
        assert!(
            crate::video::plan_video(&contract, &long_1080, &VideoMaterialFacts::new()).is_err()
        );
    }
}
