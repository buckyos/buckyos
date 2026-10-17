use super::{
    CodecCall, CodecContext, CredentialKind, ExecutionMode, HttpBody, HttpRequest, HttpResponse,
    NativeTaskCodec, NativeTaskHandle, NativeTaskInput, NativeTaskOperation, NativeTaskOutput,
    NativeTaskState, OperationCodec, OperationDescriptor, ProtocolError, ProtocolErrorKind,
    ProtocolExecution, ProtocolOutput, ProtocolResultValue, SseConfig, SseFrame, SseFramer,
    SseStreamEnd,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiUsage, AiccCall, ApiType, ImageToImageRequest, ResourceRef,
    TextToImageInvokeRequest,
};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const OPENROUTER_MUSIC_OPERATION_ID: &str = "chat.completions.create";
pub(crate) const OPENROUTER_IMAGES_OPERATION_ID: &str = "images.generate";

pub(crate) fn image_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        operation_id: OPENROUTER_IMAGES_OPERATION_ID.to_owned(),
        bindings: vec![
            super::OperationBinding::new(ApiType::ImageTextToImage, [ExecutionMode::Immediate]),
            super::OperationBinding::new(ApiType::ImageImageToImage, [ExecutionMode::Immediate]),
        ],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: 32 * 1024 * 1024,
        max_response_bytes: 64 * 1024 * 1024,
    }
}

pub(crate) fn image_codecs(descriptor: OperationDescriptor) -> Vec<Arc<dyn OperationCodec>> {
    [ApiType::ImageTextToImage, ApiType::ImageImageToImage]
        .into_iter()
        .map(|api_type| {
            Arc::new(OpenRouterImageCodec {
                descriptor: descriptor.clone(),
                api_type,
            }) as Arc<dyn OperationCodec>
        })
        .collect()
}

pub(crate) fn video_codecs(descriptor: OperationDescriptor) -> Vec<Arc<dyn NativeTaskCodec>> {
    [ApiType::VideoTextToVideo, ApiType::VideoImageToVideo]
        .into_iter()
        .map(|api_type| {
            Arc::new(OpenRouterVideoCodec {
                descriptor: descriptor.clone(),
                api_type,
            }) as Arc<dyn NativeTaskCodec>
        })
        .collect()
}

pub(crate) fn speech_codec(descriptor: OperationDescriptor) -> Arc<dyn OperationCodec> {
    Arc::new(OpenRouterSpeechCodec { descriptor })
}

pub(crate) fn music_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        operation_id: OPENROUTER_MUSIC_OPERATION_ID.to_owned(),
        bindings: vec![super::OperationBinding::new(
            ApiType::AudioMusic,
            [ExecutionMode::Immediate],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: 32 * 1024 * 1024,
        max_response_bytes: 64 * 1024 * 1024,
    }
}

pub(crate) fn music_codec(descriptor: OperationDescriptor) -> Arc<dyn OperationCodec> {
    Arc::new(OpenRouterMusicCodec { descriptor })
}

#[derive(Clone)]
struct OpenRouterImageCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl OperationCodec for OpenRouterImageCodec {
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
        let model = call
            .input
            .resolved_parameters
            .get("provider_model_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProtocolError::invalid_request("OpenRouter image model is missing"))?;
        let mut body = Map::from_iter([("model".to_owned(), json!(model))]);
        match (&call.input.canonical_request, self.api_type) {
            (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => {
                encode_text_to_image(request, &mut body, call.context)?;
            }
            (AiccCall::ImageToImage(request), ApiType::ImageImageToImage) => {
                encode_image_to_image(request, &mut body, call.context)?;
            }
            _ => {
                return Err(ProtocolError::invalid_request(
                    "OpenRouter image codec received the wrong canonical request",
                ));
            }
        }
        json_request(
            call.context,
            Method::POST,
            "images",
            Some(Value::Object(body)),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let data = value
            .get("data")
            .and_then(Value::as_array)
            .filter(|data| !data.is_empty())
            .ok_or_else(|| {
                ProtocolError::invalid_response("OpenRouter image response is missing data")
            })?;
        let mut images = Vec::with_capacity(data.len());
        for item in data {
            let encoded = item
                .get("b64_json")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_response("OpenRouter image result is missing b64_json")
                })?;
            STANDARD.decode(encoded).map_err(|_| {
                ProtocolError::invalid_response("OpenRouter image result is not valid base64")
            })?;
            let mime = item
                .get("media_type")
                .and_then(Value::as_str)
                .filter(|value| value.starts_with("image/"))
                .unwrap_or("image/png")
                .to_owned();
            images.push(ResourceRef::base64(mime, encoded.to_owned()));
        }
        let artifacts = images
            .iter()
            .enumerate()
            .map(|(index, resource)| AiArtifact {
                name: format!("image-{index}"),
                mime: match resource {
                    ResourceRef::Base64 { mime, .. } => Some(mime.clone()),
                    _ => None,
                },
                resource: resource.clone(),
                metadata: None,
            })
            .collect();
        let mut usage = AiUsage {
            image_units: Some(images.len() as u64),
            request_units: Some(1),
            ..AiUsage::default()
        };
        if let Some(provider_usage) = value.get("usage") {
            usage.input_tokens = provider_usage.get("prompt_tokens").and_then(Value::as_u64);
            usage.output_tokens = provider_usage
                .get("completion_tokens")
                .and_then(Value::as_u64);
            usage.total_tokens = provider_usage.get("total_tokens").and_then(Value::as_u64);
            if let Some(cost) = provider_usage
                .get("cost")
                .and_then(Value::as_f64)
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
            {
                usage.reported_cost = Some(cost);
                usage.cost = Some(buckyos_api::AiCost::new(cost, "USD"));
            }
        }
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"images": images, "provider_states": []}),
            usage: Some(usage),
            artifacts,
        }))
    }
}

fn encode_text_to_image(
    request: &TextToImageInvokeRequest,
    body: &mut Map<String, Value>,
    context: &CodecContext,
) -> ProtocolResultValue<()> {
    context.ignore_unsupported_options(
        "OpenRouter Image API",
        &[
            ("negative_prompt", request.negative_prompt.is_some()),
            ("style", request.style.is_some()),
        ],
    );
    body.insert("prompt".to_owned(), json!(request.prompt));
    insert_optional(body, "n", request.n.map(Value::from));
    insert_optional(
        body,
        "size",
        request.size.as_ref().map(|value| json!(value)),
    );
    insert_optional(
        body,
        "aspect_ratio",
        request.aspect_ratio.as_ref().map(|value| json!(value)),
    );
    insert_optional(
        body,
        "quality",
        request.quality.as_ref().map(|value| json!(value)),
    );
    insert_optional(body, "seed", request.seed.map(Value::from));
    if let Some(media_type) = request
        .output
        .as_ref()
        .and_then(|output| output.media_type.as_deref())
    {
        body.insert("output_format".to_owned(), json!(image_format(media_type)?));
    }
    Ok(())
}

fn encode_image_to_image(
    request: &ImageToImageRequest,
    body: &mut Map<String, Value>,
    context: &CodecContext,
) -> ProtocolResultValue<()> {
    if request.images.is_empty() {
        return Err(ProtocolError::invalid_request(
            "OpenRouter Image API requires at least one input reference",
        ));
    }
    context.ignore_unsupported_options(
        "OpenRouter Image API",
        &[("strength", request.strength.is_some())],
    );
    body.insert("prompt".to_owned(), json!(request.prompt));
    body.insert(
        "input_references".to_owned(),
        Value::Array(
            request
                .images
                .iter()
                .map(|resource| {
                    resource_url(resource, context)
                        .map(|url| json!({"type": "image_url", "image_url": {"url": url}}))
                })
                .collect::<ProtocolResultValue<Vec<_>>>()?,
        ),
    );
    if let Some(output) = &request.output {
        insert_optional(body, "size", output.size.as_ref().map(|value| json!(value)));
        if let Some(media_type) = output.media_type.as_deref() {
            body.insert("output_format".to_owned(), json!(image_format(media_type)?));
        }
    }
    Ok(())
}

fn image_format(media_type: &str) -> ProtocolResultValue<&'static str> {
    match media_type {
        "image/png" => Ok("png"),
        "image/jpeg" => Ok("jpeg"),
        "image/webp" => Ok("webp"),
        "image/svg+xml" => Ok("svg"),
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            format!("OpenRouter Image API does not support output media type {media_type}"),
        )),
    }
}

fn insert_optional(body: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        body.insert(key.to_owned(), value);
    }
}

#[derive(Clone)]
struct OpenRouterMusicCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for OpenRouterMusicCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::AudioMusic
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let AiccCall::AudioMusic(request) = &call.input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "OpenRouter music codec received the wrong canonical request",
            ));
        };
        let model = call
            .input
            .resolved_parameters
            .get("provider_model_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProtocolError::invalid_request("OpenRouter music model is missing"))?;
        if request
            .output
            .as_ref()
            .is_some_and(|output| output.media_type.is_some() || output.sample_rate.is_some())
        {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenRouter music does not publish configurable audio output options",
            ));
        }
        let mut prompt = request.prompt.clone();
        if let Some(duration) = request.duration_seconds {
            if !duration.is_finite() || duration <= 0.0 {
                return Err(ProtocolError::invalid_request(
                    "OpenRouter music duration must be positive",
                ));
            }
            prompt.push_str(&format!("\nTarget duration: {duration} seconds."));
        }
        if let Some(instrumental) = request.instrumental {
            prompt.push_str(if instrumental {
                "\nInstrumental only, no vocals."
            } else {
                "\nInclude vocals."
            });
        }
        if let Some(lyrics) = &request.lyrics {
            prompt.push_str("\nUse the following lyrics:\n");
            prompt.push_str(lyrics);
        }
        let mut body = Map::from_iter([
            ("model".to_owned(), json!(model)),
            (
                "messages".to_owned(),
                json!([{"role": "user", "content": prompt}]),
            ),
            ("modalities".to_owned(), json!(["audio"])),
            ("stream".to_owned(), json!(true)),
        ]);
        if let Some(seed) = request.seed {
            body.insert("seed".to_owned(), json!(seed));
        }
        json_request(
            call.context,
            Method::POST,
            "chat/completions",
            Some(Value::Object(body)),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let is_event_stream = response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
        if !is_event_stream {
            return Err(ProtocolError::invalid_response(
                "OpenRouter music response must use text/event-stream",
            ));
        }
        let mut framer = SseFramer::new(SseConfig {
            max_line_bytes: self.descriptor.max_response_bytes,
            max_event_bytes: self.descriptor.max_response_bytes,
            ..SseConfig::default()
        })?;
        let mut frames = framer.push(&response.body)?;
        frames.extend(framer.finish(SseStreamEnd::EndOfStream)?);
        let mut data = String::new();
        let mut transcript = String::new();
        let mut usage = AiUsage::request_units(1);
        let mut terminated = false;
        for frame in frames {
            match frame {
                SseFrame::Event(event) => {
                    let value: Value = serde_json::from_str(&event.data).map_err(|_| {
                        ProtocolError::invalid_response(
                            "OpenRouter music SSE data is not valid JSON",
                        )
                    })?;
                    if let Some(chunk) = value
                        .pointer("/choices/0/delta/audio/data")
                        .and_then(Value::as_str)
                    {
                        data.push_str(chunk);
                    }
                    if let Some(chunk) = value
                        .pointer("/choices/0/delta/audio/transcript")
                        .and_then(Value::as_str)
                    {
                        transcript.push_str(chunk);
                    }
                    if let Some(value_usage) = value.get("usage") {
                        usage.input_tokens =
                            value_usage.get("prompt_tokens").and_then(Value::as_u64);
                        usage.output_tokens =
                            value_usage.get("completion_tokens").and_then(Value::as_u64);
                        usage.total_tokens =
                            value_usage.get("total_tokens").and_then(Value::as_u64);
                        if let Some(cost) = value_usage
                            .get("cost")
                            .and_then(Value::as_f64)
                            .filter(|cost| cost.is_finite() && *cost >= 0.0)
                        {
                            usage.reported_cost = Some(cost);
                            usage.cost = Some(buckyos_api::AiCost::new(cost, "USD"));
                        }
                    }
                }
                SseFrame::Terminated { marker } if marker == "[DONE]" => terminated = true,
                SseFrame::Terminated { .. } => {
                    return Err(ProtocolError::invalid_response(
                        "OpenRouter music stream used an unknown termination marker",
                    ));
                }
                SseFrame::StreamEnd(_) => {}
            }
        }
        if !terminated {
            return Err(ProtocolError::invalid_response(
                "OpenRouter music stream ended before [DONE]",
            ));
        }
        if data.is_empty() {
            return Err(ProtocolError::invalid_response(
                "OpenRouter music response is missing audio data",
            ));
        }
        STANDARD.decode(&data).map_err(|_| {
            ProtocolError::invalid_response("OpenRouter music audio data is not valid base64")
        })?;
        let mime = "audio/wav".to_owned();
        let resource = ResourceRef::base64(mime.clone(), data);
        let mut value = json!({"audio": resource});
        if !transcript.is_empty() {
            value["transcript"] = json!(transcript);
        }
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value,
            usage: Some(usage),
            artifacts: vec![AiArtifact {
                name: "music".to_owned(),
                resource,
                mime: Some(mime),
                metadata: None,
            }],
        }))
    }
}

#[derive(Clone)]
struct OpenRouterSpeechCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for OpenRouterSpeechCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::AudioTextToSpeech
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let AiccCall::AudioTextToSpeech(request) = &call.input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "OpenRouter speech codec received the wrong canonical request",
            ));
        };
        let model = call
            .input
            .resolved_parameters
            .get("provider_model_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProtocolError::invalid_request("OpenRouter speech model is missing"))?;
        let mut body = Map::from_iter([
            ("model".to_string(), json!(model)),
            ("input".to_string(), json!(request.text)),
        ]);
        if let Some(value) = call.input.resolved_parameters.get("openrouter_voice") {
            if !value.is_string() {
                return Err(ProtocolError::invalid_request(
                    "OpenRouter speech openrouter_voice must be a string",
                ));
            }
            body.insert("voice".to_string(), value.clone());
        }
        for field in ["instructions"] {
            if let Some(value) = call.input.resolved_parameters.get(field) {
                if !value.is_string() {
                    return Err(ProtocolError::invalid_request(format!(
                        "OpenRouter speech {field} must be a string"
                    )));
                }
                body.insert(field.to_string(), value.clone());
            }
        }
        if let Some(speed) = request.speed {
            body.insert("speed".to_string(), json!(speed));
        }
        if let Some(value) = call
            .input
            .resolved_parameters
            .get("openrouter_response_format")
        {
            let format = value.as_str().ok_or_else(|| {
                ProtocolError::invalid_request(
                    "OpenRouter speech openrouter_response_format must be a string",
                )
            })?;
            if !matches!(format, "mp3" | "pcm") {
                return Err(ProtocolError::invalid_configuration(
                    "OpenRouter speech openrouter_response_format must be mp3 or pcm",
                ));
            }
            body.insert("response_format".to_string(), value.clone());
        } else if let Some(output) = &request.output {
            if output.sample_rate.is_some() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "OpenRouter speech does not accept an arbitrary sample rate",
                ));
            }
            if let Some(media_type) = output.media_type.as_deref() {
                let format = match media_type {
                    "audio/mpeg" => "mp3",
                    "audio/pcm" | "audio/L16" => "pcm",
                    _ => {
                        return Err(ProtocolError::new(
                            ProtocolErrorKind::UnsupportedOperation,
                            "OpenRouter speech supports only MP3 and PCM output",
                        ));
                    }
                };
                body.insert("response_format".to_string(), json!(format));
            }
        }
        json_request(
            call.context,
            Method::POST,
            "audio/speech",
            Some(Value::Object(body)),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        if response.body.is_empty() {
            return Err(ProtocolError::invalid_response(
                "OpenRouter speech response is empty",
            ));
        }
        let mime = response
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .unwrap_or("audio/mpeg")
            .to_string();
        if !mime.to_ascii_lowercase().starts_with("audio/") {
            return Err(ProtocolError::invalid_response(
                "OpenRouter speech response has an invalid content type",
            ));
        }
        let resource = ResourceRef::base64(mime.clone(), STANDARD.encode(&response.body));
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"audio": resource}),
            usage: Some(AiUsage::request_units(1)),
            artifacts: vec![AiArtifact {
                name: "speech".to_string(),
                resource,
                mime: Some(mime),
                metadata: None,
            }],
        }))
    }
}

#[derive(Clone)]
struct OpenRouterVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for OpenRouterVideoCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        crate::resource::ResourceInputForm::UrlOrBytes
    }

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
            NativeTaskOperation::Submit => encode_submit(input, self.api_type),
            NativeTaskOperation::Status => lifecycle_request(input, ""),
            NativeTaskOperation::Result => lifecycle_request(input, "/content"),
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenRouter video generation does not publish cancellation",
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
            NativeTaskOperation::Submit => decode_submit(response),
            NativeTaskOperation::Status => decode_status(response),
            NativeTaskOperation::Result => decode_result(response),
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenRouter video generation does not publish cancellation",
            )),
        }
    }
}

fn encode_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("OpenRouter video submit requires canonical input")
    })?;
    let model = input
        .resolved_parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProtocolError::invalid_request("OpenRouter video model is missing"))?;
    let mut body = Map::from_iter([("model".to_string(), json!(model))]);
    match (&codec_input.canonical_request, api_type) {
        (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => {
            body.insert("prompt".to_string(), json!(request.prompt));
            insert_video_options(
                &mut body,
                request.duration_seconds,
                request.resolution.as_deref(),
                request.generate_audio,
                request.seed,
            )?;
        }
        (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => {
            body.insert("prompt".to_string(), json!(request.prompt));
            body.insert(
                "frame_images".to_string(),
                json!([{
                    "type": "image_url",
                    "image_url": {"url": resource_url(&request.image, input.context)?},
                    "frame_type": "first_frame"
                }]),
            );
            insert_video_options(
                &mut body,
                request.duration_seconds,
                request.resolution.as_deref(),
                None,
                None,
            )?;
        }
        _ => {
            return Err(ProtocolError::invalid_request(
                "OpenRouter video codec received the wrong canonical request",
            ));
        }
    }
    json_request(
        input.context,
        Method::POST,
        "videos",
        Some(Value::Object(body)),
    )
}

fn insert_video_options(
    body: &mut Map<String, Value>,
    duration: Option<f64>,
    resolution: Option<&str>,
    generate_audio: Option<bool>,
    seed: Option<u64>,
) -> ProtocolResultValue<()> {
    if let Some(duration) = duration {
        if !duration.is_finite()
            || duration <= 0.0
            || duration.fract() != 0.0
            || duration > u64::MAX as f64
        {
            return Err(ProtocolError::invalid_request(
                "OpenRouter video duration must be a positive whole number of seconds",
            ));
        }
        body.insert("duration".to_string(), json!(duration as u64));
    }
    if let Some(resolution) = resolution {
        body.insert("resolution".to_string(), json!(resolution));
    }
    if let Some(generate_audio) = generate_audio {
        body.insert("generate_audio".to_string(), json!(generate_audio));
    }
    if let Some(seed) = seed {
        body.insert("seed".to_string(), json!(seed));
    }
    Ok(())
}

fn resource_url(resource: &ResourceRef, context: &CodecContext) -> ProtocolResultValue<String> {
    if let Some(url) = context.materialized_url(resource) {
        return Ok(url.to_string());
    }
    let resource = context.materialized_resource(resource)?;
    Ok(format!(
        "data:{};base64,{}",
        resource.mime,
        STANDARD.encode(&resource.bytes)
    ))
}

fn lifecycle_request(
    input: &NativeTaskInput<'_>,
    suffix: &str,
) -> ProtocolResultValue<HttpRequest> {
    let id = input.remote_task_id.ok_or_else(|| {
        ProtocolError::invalid_request("OpenRouter video lifecycle requires a task ID")
    })?;
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ProtocolError::invalid_request(
            "OpenRouter video task ID contains invalid path characters",
        ));
    }
    json_request(
        input.context,
        Method::GET,
        &format!("videos/{id}{suffix}"),
        None,
    )
}

fn decode_submit(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let retry_after = response.retry_after;
    let value: Value = response.json(64 * 1024 * 1024)?;
    let id = required_string(&value, "id")?;
    let mut handle = NativeTaskHandle::new(id)?;
    handle.state = decode_state(&value)?;
    handle.poll_after = retry_after.or(Some(Duration::from_secs(2)));
    handle.cancel_supported = false;
    Ok(NativeTaskOutput::Submitted(handle))
}

fn decode_status(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let retry_after = response.retry_after;
    let value: Value = response.json(64 * 1024 * 1024)?;
    let state = decode_state(&value)?;
    if state == NativeTaskState::Failed {
        return Err(ProtocolError::new(
            ProtocolErrorKind::ProviderRejected,
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("OpenRouter video task failed"),
        ));
    }
    Ok(NativeTaskOutput::Status {
        state,
        retry_after: retry_after.or(Some(Duration::from_secs(2))),
        result_ref: (state == NativeTaskState::Succeeded)
            .then(|| required_string(&value, "id"))
            .transpose()?,
        result_usage: None,
        result_artifacts: BTreeMap::new(),
    })
}

fn decode_result(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    if response.body.is_empty() {
        return Err(ProtocolError::invalid_response(
            "OpenRouter video content response is empty",
        ));
    }
    let mime = response
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("video/mp4")
        .to_string();
    if !mime.to_ascii_lowercase().starts_with("video/") {
        return Err(ProtocolError::invalid_response(
            "OpenRouter video content response has an invalid content type",
        ));
    }
    let resource = ResourceRef::base64(mime.clone(), STANDARD.encode(&response.body));
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: json!({"video": resource}),
        usage: Some(AiUsage::request_units(1)),
        artifacts: vec![AiArtifact {
            name: "video".to_string(),
            resource,
            mime: Some(mime),
            metadata: None,
        }],
    }))
}

fn decode_state(value: &Value) -> ProtocolResultValue<NativeTaskState> {
    match required_string(value, "status")?.as_str() {
        "pending" => Ok(NativeTaskState::Queued),
        "in_progress" => Ok(NativeTaskState::Running),
        "completed" => Ok(NativeTaskState::Succeeded),
        "failed" | "expired" => Ok(NativeTaskState::Failed),
        "cancelled" => Ok(NativeTaskState::Cancelled),
        _ => Err(ProtocolError::invalid_response(
            "OpenRouter video status is unknown",
        )),
    }
}

fn required_string(value: &Value, field: &str) -> ProtocolResultValue<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("OpenRouter video response is missing {field}"))
        })
}

fn json_request(
    context: &CodecContext,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> ProtocolResultValue<HttpRequest> {
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("OpenRouter base URL is invalid"))?;
    let base = url.path().trim_end_matches('/');
    url.set_path(&format!("{base}/{path}"));
    url.set_query(None);
    let mut request = HttpRequest::new(method, url.to_string());
    if let Some(body) = body {
        request.body = HttpBody::Json(body);
    }
    context.finalize_request(
        &mut request,
        CredentialKind::Bearer,
        "OpenRouter credential is missing",
        "OpenRouter requires a bearer credential",
    )
}

fn ensure_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let parsed = serde_json::from_slice::<Value>(&response.body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
        })
        .and_then(Value::as_str)
        .unwrap_or("OpenRouter media request failed");
    let provider_code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code").or_else(|| value.get("code")))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        });
    Err(ProtocolError::new(
        super::protocol_error_kind_from_http_status(response.status),
        message,
    )
    .with_provider_code(provider_code)
    .with_http_status(response.status.as_u16())
    .with_request_id(Some(response.request_id.clone()))
    .with_retry_after(response.retry_after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;

    #[test]
    fn server_error_is_retriable_transport_failure() {
        let response = HttpResponse {
            status: StatusCode::SERVICE_UNAVAILABLE,
            headers: HeaderMap::new(),
            body: Bytes::from_static(
                br#"{"error":{"code":"provider_unavailable","message":"try again"}}"#,
            ),
            request_id: "openrouter-request-1".to_owned(),
            retry_after: Some(Duration::from_secs(3)),
        };
        let error = ensure_success(&response).unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert_eq!(error.provider_code.as_deref(), Some("provider_unavailable"));
        assert_eq!(error.http_status, Some(503));
        assert_eq!(error.retry_after, Some(Duration::from_secs(3)));
    }

    #[test]
    fn video_duration_requires_whole_seconds() {
        let mut body = Map::new();
        assert!(insert_video_options(&mut body, Some(1.5), None, None, None).is_err());
        insert_video_options(&mut body, Some(4.0), None, None, None).unwrap();
        assert_eq!(body["duration"], json!(4));
    }

    #[tokio::test]
    async fn music_decodes_official_streaming_audio_shape_and_cost() {
        let codec = OpenRouterMusicCodec {
            descriptor: music_descriptor(),
        };
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([(
                CONTENT_TYPE,
                "text/event-stream".parse().unwrap(),
            )]),
            body: Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"audio\":{\"data\":\"UklG\",\"transcript\":\"BUCKYOS-\"}}}]}\n\ndata: {\"choices\":[{\"delta\":{\"audio\":{\"data\":\"RgAAAAA=\",\"transcript\":\"AICC-4827\"}}}],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":3,\"total_tokens\":7,\"cost\":0.125}}\n\ndata: [DONE]\n\n",
            ),
            request_id: "openrouter-music-1".to_owned(),
            retry_after: None,
        };
        let ProtocolExecution::Immediate(output) = codec.decode(response).await.unwrap() else {
            panic!("music must complete immediately");
        };
        assert!(output.value.get("audio").is_some());
        assert_eq!(output.value["transcript"], "BUCKYOS-AICC-4827");
        let usage = output.usage.unwrap();
        assert_eq!(usage.total_tokens, Some(7));
        assert_eq!(usage.reported_cost, Some(0.125));
        assert_eq!(usage.cost.unwrap().currency, "USD");
    }

    #[tokio::test]
    async fn music_accepts_large_single_event_audio_payload() {
        let codec = OpenRouterMusicCodec {
            descriptor: music_descriptor(),
        };
        let audio = "UklG".repeat(70_000);
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"audio\":{{\"data\":\"{audio}\"}}}}}}]}}\n\ndata: [DONE]\n\n"
        );
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([(CONTENT_TYPE, "text/event-stream".parse().unwrap())]),
            body: Bytes::from(body),
            request_id: "openrouter-music-large-event".to_owned(),
            retry_after: None,
        };
        let ProtocolExecution::Immediate(output) = codec.decode(response).await.unwrap() else {
            panic!("music must complete immediately");
        };
        let ResourceRef::Base64 { data_base64, .. } = &output.artifacts[0].resource else {
            panic!("music must return inline base64 audio");
        };
        assert_eq!(data_base64.len(), audio.len());
    }
}
