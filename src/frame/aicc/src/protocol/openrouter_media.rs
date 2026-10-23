use super::{
    CodecCall, CodecContext, CredentialKind, ExecutionMode, HttpBody, HttpRequest, HttpResponse,
    OperationCodec, OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution,
    ProtocolOutput, ProtocolResultValue, SseConfig, SseFrame, SseFramer, SseStreamEnd,
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
                    if let Some(error) = stream_failure(&value) {
                        // Whatever arrived before the failure is a truncated
                        // track, and the Provider charged for the work it did:
                        // fail the attempt and keep the reported usage.
                        return Err(error.with_usage(Some(usage)));
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

fn speech_voice(parameters: &BTreeMap<String, Value>) -> Option<Value> {
    parameters
        .get("generation_config")
        .and_then(|config| config.get("speech_config"))
        .and_then(Value::as_array)
        .and_then(|speech| speech.first())
        .and_then(|speech| speech.get("voice"))
        .cloned()
        .or_else(|| parameters.get("openrouter_voice").cloned())
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
        if let Some(value) = speech_voice(&call.input.resolved_parameters) {
            if !value.is_string() {
                return Err(ProtocolError::invalid_request(
                    "OpenRouter speech voice must be a string",
                ));
            }
            body.insert("voice".to_string(), value);
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
    fn speech_voice_prefers_the_canonical_voice_over_the_provider_default() {
        let parameters = BTreeMap::from([
            ("openrouter_voice".to_owned(), json!("Kore")),
            (
                "generation_config".to_owned(),
                json!({"speech_config": [{"voice": "Sulafat"}]}),
            ),
        ]);
        assert_eq!(speech_voice(&parameters), Some(json!("Sulafat")));
        assert_eq!(
            speech_voice(&BTreeMap::from([(
                "openrouter_voice".to_owned(),
                json!("Kore"),
            )])),
            Some(json!("Kore"))
        );
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

    #[tokio::test]
    async fn music_stream_error_frame_fails_the_attempt_and_keeps_its_usage() {
        let codec = OpenRouterMusicCodec {
            descriptor: music_descriptor(),
        };
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([(CONTENT_TYPE, "text/event-stream".parse().unwrap())]),
            body: Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"audio\":{\"data\":\"UklG\"}}}],\"usage\":{\"total_tokens\":9,\"cost\":0.25}}\n\ndata: {\"error\":{\"code\":\"upstream_timeout\",\"message\":\"upstream timed out\"}}\n\ndata: [DONE]\n\n",
            ),
            request_id: "openrouter-music-failed".to_owned(),
            retry_after: None,
        };
        let error = codec.decode(response).await.unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::ProviderRejected);
        assert!(error.message.contains("upstream timed out"), "{error:?}");
        assert_eq!(error.provider_code.as_deref(), Some("upstream_timeout"));
        let usage = error.usage.expect("a billed attempt keeps its usage");
        assert_eq!(usage.total_tokens, Some(9));
        assert_eq!(usage.reported_cost, Some(0.25));
    }

    #[tokio::test]
    async fn music_stream_finish_reason_error_fails_the_attempt() {
        let codec = OpenRouterMusicCodec {
            descriptor: music_descriptor(),
        };
        let response = HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::from_iter([(CONTENT_TYPE, "text/event-stream".parse().unwrap())]),
            body: Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"audio\":{\"data\":\"UklG\"}}}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"error\"}]}\n\ndata: [DONE]\n\n",
            ),
            request_id: "openrouter-music-finish-reason".to_owned(),
            retry_after: None,
        };
        let error = codec.decode(response).await.unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::ProviderRejected);
        assert!(error.message.contains("finish_reason=error"), "{error:?}");
        assert!(error.usage.is_some());
    }
}

/// OpenRouter reports a mid-stream failure either as a top-level `error` object
/// or as `choices[0].finish_reason == "error"`. Both mean the audio delivered so
/// far is truncated, so neither may be decoded as a successful result.
fn stream_failure(value: &Value) -> Option<ProtocolError> {
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("the stream reported an error without a message");
        let code = error.get("code").map(|code| match code.as_str() {
            Some(code) => code.to_owned(),
            None => code.to_string(),
        });
        return Some(
            ProtocolError::new(
                ProtocolErrorKind::ProviderRejected,
                format!("OpenRouter music stream failed: {message}"),
            )
            .with_provider_code(code),
        );
    }
    if value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        == Some("error")
    {
        return Some(ProtocolError::new(
            ProtocolErrorKind::ProviderRejected,
            "OpenRouter music stream finished with finish_reason=error",
        ));
    }
    None
}
