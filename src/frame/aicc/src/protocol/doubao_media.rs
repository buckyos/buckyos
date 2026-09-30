use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration, ExecutionMode, HttpBody,
    HttpRequest, HttpResponse, NativeTaskCodec, NativeTaskHandle, NativeTaskInput,
    NativeTaskOperation, NativeTaskOutput, NativeTaskState, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput,
    ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{AiArtifact, AiUsage, AiccCall, ApiType, ResourceRef};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const DOUBAO_MEDIA_ADAPTER_ID: &str = "doubao-media";
pub(crate) const DOUBAO_IMAGE_OPERATION_ID: &str = "ark.images.generate";
pub(crate) const DOUBAO_VIDEO_OPERATION_ID: &str = "ark.contents.generate";
pub(crate) const DOUBAO_MULTIMODAL_EMBEDDING_OPERATION_ID: &str = "ark.embeddings.multimodal";
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn doubao_media_registration() -> (Vec<OperationDescriptor>, CodecRegistration) {
    let embedding = immediate_operation(
        DOUBAO_MULTIMODAL_EMBEDDING_OPERATION_ID,
        &[ApiType::EmbeddingMultimodal],
    );
    let image = immediate_operation(
        DOUBAO_IMAGE_OPERATION_ID,
        &[ApiType::ImageTextToImage, ApiType::ImageImageToImage],
    );
    let video = OperationDescriptor {
        operation_id: DOUBAO_VIDEO_OPERATION_ID.to_owned(),
        bindings: [
            ApiType::VideoTextToVideo,
            ApiType::VideoImageToVideo,
            ApiType::VideoToVideo,
            ApiType::VideoExtend,
        ]
        .into_iter()
        .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::NativeTask]))
        .collect(),
        supports_cancel: true,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let operation_codecs = [ApiType::ImageTextToImage, ApiType::ImageImageToImage]
        .into_iter()
        .map(|api_type| {
            Arc::new(DoubaoImageCodec {
                descriptor: image.clone(),
                api_type,
            }) as Arc<dyn OperationCodec>
        })
        .chain(std::iter::once(Arc::new(DoubaoMultimodalEmbeddingCodec {
            descriptor: embedding.clone(),
        }) as Arc<dyn OperationCodec>))
        .collect();
    let native_task_codecs = [
        ApiType::VideoTextToVideo,
        ApiType::VideoImageToVideo,
        ApiType::VideoToVideo,
        ApiType::VideoExtend,
    ]
    .into_iter()
    .map(|api_type| {
        Arc::new(DoubaoVideoCodec {
            descriptor: video.clone(),
            api_type,
        }) as Arc<dyn NativeTaskCodec>
    })
    .collect();
    (
        vec![embedding, image, video],
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

#[derive(Clone)]
struct DoubaoMultimodalEmbeddingCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for DoubaoMultimodalEmbeddingCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::EmbeddingMultimodal
    }

    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        require_only_model(&call.input.resolved_parameters)?;
        let AiccCall::EmbeddingMultimodal(request) = &call.input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "Doubao multimodal embedding codec received the wrong canonical request",
            ));
        };
        if request.items.len() != 1 {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Doubao multimodal embedding requires exactly one canonical sequence item",
            ));
        }
        if request.normalize == Some(false) {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Doubao multimodal embedding does not expose normalization control",
            ));
        }
        let item = &request.items[0];
        if item.text.is_none() && item.image.is_none() {
            return Err(ProtocolError::invalid_request(
                "Doubao multimodal embedding item must contain text or image",
            ));
        }
        let mut input = Vec::new();
        if let Some(image) = &item.image {
            input.push(json!({"type":"image_url","image_url":{"url":resource_string(image, call.context)?}}));
        }
        if let Some(text) = &item.text {
            input.push(json!({"type":"text","text":text}));
        }
        let mut body = Map::from_iter([
            (
                "model".to_owned(),
                json!(provider_model_id(&call.input.resolved_parameters)?),
            ),
            ("encoding_format".to_owned(), json!("float")),
            ("input".to_owned(), Value::Array(input)),
        ]);
        if let Some(dimensions) = request.dimensions {
            body.insert("dimensions".to_owned(), json!(dimensions));
        }
        json_request(
            call.context,
            Method::POST,
            "embeddings/multimodal",
            Some(Value::Object(body)),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        let data = value.get("data").ok_or_else(|| {
            ProtocolError::invalid_response("Doubao embedding response is missing data")
        })?;
        let embedding = data
            .get("embedding")
            .or_else(|| {
                data.as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item.get("embedding"))
            })
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProtocolError::invalid_response("Doubao embedding response is missing embedding")
            })?;
        if embedding.is_empty() || embedding.iter().any(|item| item.as_f64().is_none()) {
            return Err(ProtocolError::invalid_response(
                "Doubao embedding vector must contain numeric values",
            ));
        }
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("doubao-embedding-vision");
        let usage = value.get("usage");
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({
                "data": [{
                    "index": 0,
                    "id": Value::Null,
                    "embedding": embedding,
                    "embedding_space_id": format!("{model}:{}", embedding.len())
                }],
                "data_resource": Value::Null
            }),
            usage: usage.map(|usage| AiUsage {
                input_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
                total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
                ..AiUsage::request_units(1)
            }),
            artifacts: Vec::new(),
        }))
    }
}

pub(crate) fn doubao_media_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (operations, registration) = doubao_media_registration();
    (
        AdapterDescriptor {
            protocol_family_id: "doubao".to_owned(),
            protocol_adapter_id: DOUBAO_MEDIA_ADAPTER_ID.to_owned(),
            interface_generation: "ark-v3".to_owned(),
            base_adapter_id: None,
            component_adapter_ids: Vec::new(),
            status: AdapterStatus::Stable,
            probe_priority: 200,
            probe_path: None,
            credential: super::AdapterCredentialContract::bearer(),
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
        operation_id: id.to_owned(),
        bindings: api_types
            .iter()
            .copied()
            .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::Immediate]))
            .collect(),
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

#[derive(Clone)]
struct DoubaoImageCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl OperationCodec for DoubaoImageCodec {
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
        let mut body = match (&call.input.canonical_request, self.api_type) {
            (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => Map::from_iter([
                ("model".to_owned(), json!(model)),
                ("prompt".to_owned(), json!(request.prompt)),
                ("response_format".to_owned(), json!("url")),
            ]),
            (AiccCall::ImageToImage(request), ApiType::ImageImageToImage) => {
                let images = request
                    .images
                    .iter()
                    .map(|resource| resource_string(resource, call.context))
                    .collect::<ProtocolResultValue<Vec<_>>>()?;
                Map::from_iter([
                    ("model".to_owned(), json!(model)),
                    ("prompt".to_owned(), json!(request.prompt)),
                    ("image".to_owned(), json!(images)),
                    ("response_format".to_owned(), json!("url")),
                ])
            }
            _ => {
                return Err(ProtocolError::invalid_request(
                    "Doubao image codec received the wrong canonical request",
                ))
            }
        };
        if let AiccCall::ImagesGenerate(request) = &call.input.canonical_request {
            if let Some(size) = &request.size {
                body.insert("size".to_owned(), json!(size));
            } else if let Some(ratio) = &request.aspect_ratio {
                body.insert("size".to_owned(), json!(seedream_size(ratio)?));
            }
            if let Some(seed) = request.seed {
                body.insert("seed".to_owned(), json!(seed));
            }
            if let Some(n) = request.n {
                let enabled = n > 1;
                body.insert(
                    "sequential_image_generation".to_owned(),
                    json!(if enabled { "auto" } else { "disabled" }),
                );
                if enabled {
                    body.insert(
                        "sequential_image_generation_options".to_owned(),
                        json!({"max_images":n}),
                    );
                }
            }
        }
        json_request(
            call.context,
            Method::POST,
            "images/generations",
            Some(Value::Object(body)),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        Ok(ProtocolExecution::Immediate(decode_images(&value)?))
    }
}

#[derive(Clone)]
struct DoubaoVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for DoubaoVideoCodec {
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
            NativeTaskOperation::Status | NativeTaskOperation::Result => {
                let task_id = safe_task_id(input.remote_task_id)?;
                json_request(
                    input.context,
                    Method::GET,
                    &format!("contents/generations/tasks/{task_id}"),
                    None,
                )
            }
            NativeTaskOperation::Cancel => {
                let task_id = safe_task_id(input.remote_task_id)?;
                json_request(
                    input.context,
                    Method::DELETE,
                    &format!("contents/generations/tasks/{task_id}"),
                    None,
                )
            }
        }
    }

    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        ensure_success(&response)?;
        let retry_after = response.retry_after;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        match operation {
            NativeTaskOperation::Submit => {
                let mut handle = NativeTaskHandle::new(required_string(&value, "id")?)?;
                handle.cancel_supported = true;
                handle.poll_after = retry_after.or(Some(Duration::from_secs(3)));
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let state = decode_state(required_string(&value, "status")?.as_str())?;
                if state == NativeTaskState::Failed {
                    let error = value.get("error").unwrap_or(&value);
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::ProviderRejected,
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Provider video task failed"),
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
                Ok(NativeTaskOutput::Status {
                    result_usage: None,
                    state,
                    retry_after: retry_after.or(Some(Duration::from_secs(3))),
                    result_ref: (state == NativeTaskState::Succeeded)
                        .then(|| required_string(&value, "id"))
                        .transpose()?,
                    result_artifacts: BTreeMap::new(),
                })
            }
            NativeTaskOperation::Result => decode_video_result(&value),
            NativeTaskOperation::Cancel => Ok(NativeTaskOutput::Cancelled { accepted: true }),
        }
    }
}

fn encode_video_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("Doubao video submit requires canonical input")
    })?;
    require_video_parameters(input.resolved_parameters)?;
    let model = provider_model_id(input.resolved_parameters)?;
    let mut content = Vec::new();
    let mut parameters = Map::new();
    match (&codec_input.canonical_request, api_type) {
        (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => {
            content.push(json!({"type":"text","text":request.prompt}));
            video_options(
                &mut parameters,
                request.duration_seconds,
                request.aspect_ratio.as_deref(),
                request.resolution.as_deref(),
                request.seed,
                request.generate_audio,
            )?;
        }
        (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => {
            content.push(json!({"type":"text","text":request.prompt}));
            content.push(json!({"type":"image_url","image_url":{"url":resource_string(&request.image, input.context)?},"role":"first_frame"}));
            video_options(
                &mut parameters,
                request.duration_seconds,
                request.aspect_ratio.as_deref(),
                request.resolution.as_deref(),
                None,
                None,
            )?;
        }
        (AiccCall::VideoToVideo(request), ApiType::VideoToVideo) => {
            content.push(json!({"type":"text","text":request.prompt}));
            content.push(json!({"type":"video_url","video_url":{"url":resource_string(&request.video, input.context)?},"role":"reference_video"}));
        }
        (AiccCall::VideoExtend(request), ApiType::VideoExtend) => {
            content.push(json!({"type":"text","text":request.prompt}));
            content.push(json!({"type":"video_url","video_url":{"url":resource_string(&request.video, input.context)?},"role":"reference_video"}));
            video_options(
                &mut parameters,
                request.duration_seconds,
                None,
                request.resolution.as_deref(),
                None,
                None,
            )?;
        }
        _ => {
            return Err(ProtocolError::invalid_request(
                "Doubao video codec received the wrong canonical request",
            ))
        }
    }
    let mut body = Map::from_iter([
        ("model".to_owned(), json!(model)),
        ("content".to_owned(), Value::Array(content)),
    ]);
    body.extend(parameters);
    if let Some(task_type) = input
        .resolved_parameters
        .get("omni_reference_task_type")
        .and_then(Value::as_str)
    {
        body.insert("omni_reference_task_type".to_owned(), json!(task_type));
    }
    json_request(
        input.context,
        Method::POST,
        "contents/generations/tasks",
        Some(Value::Object(body)),
    )
}

fn video_options(
    body: &mut Map<String, Value>,
    duration: Option<f64>,
    ratio: Option<&str>,
    resolution: Option<&str>,
    seed: Option<u64>,
    audio: Option<bool>,
) -> ProtocolResultValue<()> {
    if let Some(value) = duration {
        body.insert(
            "duration".to_owned(),
            json!(super::adapter::integer_seconds(value)?),
        );
    }
    if let Some(value) = ratio {
        body.insert("ratio".to_owned(), json!(value));
    }
    if let Some(value) = resolution {
        body.insert("resolution".to_owned(), json!(value));
    }
    if let Some(value) = seed {
        body.insert("seed".to_owned(), json!(value));
    }
    if let Some(value) = audio {
        body.insert("generate_audio".to_owned(), json!(value));
    }
    Ok(())
}

fn seedream_size(ratio: &str) -> ProtocolResultValue<&'static str> {
    match ratio {
        "1:1" => Ok("2048x2048"),
        "4:3" => Ok("2304x1728"),
        "3:4" => Ok("1728x2304"),
        "16:9" => Ok("2848x1600"),
        "9:16" => Ok("1600x2848"),
        "3:2" => Ok("2496x1664"),
        "2:3" => Ok("1664x2496"),
        "21:9" => Ok("3136x1344"),
        _ => Err(ProtocolError::invalid_request(
            "unsupported Seedream aspect ratio; specify size in pixels",
        )),
    }
}

fn decode_images(value: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::invalid_response("Doubao image response is missing data"))?;
    let mut images = Vec::new();
    for item in data {
        if let Some(url) = item
            .get("url")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let mime = image_mime_from_url(url).map(str::to_owned);
            images.push((ResourceRef::url(url.to_owned(), mime.clone()), mime));
        } else if let Some(encoded) = item
            .get("b64_json")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let bytes = STANDARD.decode(encoded).map_err(|_| {
                ProtocolError::invalid_response("Doubao image response contains invalid base64")
            })?;
            let mime = image_mime_from_bytes(&bytes).ok_or_else(|| {
                ProtocolError::invalid_response(
                    "Doubao image response contains an unrecognized base64 image",
                )
            })?;
            images.push((
                ResourceRef::base64(mime.to_owned(), encoded.to_owned()),
                Some(mime.to_owned()),
            ));
        }
    }
    if images.is_empty() {
        return Err(ProtocolError::invalid_response(
            "Doubao image response contains no images",
        ));
    }
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
    let resources = images
        .into_iter()
        .map(|(resource, _)| resource)
        .collect::<Vec<_>>();
    Ok(ProtocolOutput {
        value: json!({"images":resources,"provider_states":[]}),
        usage: Some(AiUsage {
            image_units: Some(resources.len() as u64),
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

fn decode_video_result(value: &Value) -> ProtocolResultValue<NativeTaskOutput> {
    let url = value
        .pointer("/content/video_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProtocolError::invalid_response("Doubao video result is missing video_url")
        })?;
    let usage = value.get("usage");
    let resource = ResourceRef::url(url.to_owned(), Some("video/mp4".to_owned()));
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: json!({"video":resource}),
        usage: Some(AiUsage {
            output_tokens: usage
                .and_then(|usage| usage.get("completion_tokens"))
                .and_then(Value::as_u64),
            total_tokens: usage
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64),
            ..AiUsage::request_units(1)
        }),
        artifacts: vec![AiArtifact {
            name: "video".to_owned(),
            resource,
            mime: Some("video/mp4".to_owned()),
            metadata: None,
        }],
    }))
}

fn decode_state(status: &str) -> ProtocolResultValue<NativeTaskState> {
    match status.to_ascii_lowercase().as_str() {
        "queued" | "submitted" => Ok(NativeTaskState::Queued),
        "running" | "processing" => Ok(NativeTaskState::Running),
        "succeeded" | "success" => Ok(NativeTaskState::Succeeded),
        "failed" | "expired" => Ok(NativeTaskState::Failed),
        "cancelled" | "canceled" => Ok(NativeTaskState::Cancelled),
        _ => Err(ProtocolError::invalid_response(
            "Doubao video response contains an unknown status",
        )),
    }
}

fn json_request(
    context: &super::CodecContext,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("Doubao base URL is invalid"))?;
    let base = url.path().trim_end_matches('/');
    url.set_path(&format!("{base}/{}", path.trim_start_matches('/')));
    let mut request = HttpRequest::new(method, url.to_string());
    if let Some(body) = body {
        request
            .headers
            .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        request.body = HttpBody::Json(body);
    }
    context
        .credential
        .as_ref()
        .ok_or_else(|| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Doubao media operation requires a credential",
            )
        })?
        .apply(&mut request.headers)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request)
}

fn provider_model_id(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<String> {
    parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved provider_model_id"))
}

fn require_only_model(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<()> {
    if let Some(name) = parameters
        .keys()
        .find(|name| name.as_str() != "provider_model_id")
    {
        return Err(ProtocolError::invalid_request(format!(
            "resolved Doubao media parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

fn require_video_parameters(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<()> {
    for (name, value) in parameters {
        if name == "provider_model_id" {
            continue;
        }
        if name == "omni_reference_task_type"
            && matches!(value.as_str(), Some("reference" | "edit" | "extend"))
        {
            continue;
        }
        return Err(ProtocolError::invalid_request(format!(
            "resolved Doubao video parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

fn resource_string(
    resource: &ResourceRef,
    context: &super::CodecContext,
) -> ProtocolResultValue<String> {
    match resource {
        ResourceRef::Url { url, .. } => Ok(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("Doubao media resource contains invalid base64")
            })?;
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        ResourceRef::NamedObject { .. } => {
            let value = context.materialized_resource(resource)?;
            Ok(format!(
                "data:{};base64,{}",
                value.mime,
                STANDARD.encode(&value.bytes)
            ))
        }
    }
}

fn safe_task_id(value: Option<&str>) -> ProtocolResultValue<&str> {
    value
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 512
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
        .ok_or_else(|| ProtocolError::invalid_request("Doubao media task ID is invalid"))
}

fn required_string(value: &Value, field: &str) -> ProtocolResultValue<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("Doubao response is missing `{field}`"))
        })
}

fn ensure_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let parsed = serde_json::from_slice::<Value>(&response.body).ok();
    let provider_code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code").or_else(|| value.get("code")))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        });
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
        })
        .and_then(Value::as_str)
        .unwrap_or("Doubao media request failed");
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
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, ResolvedCredential,
    };
    use buckyos_api::{
        ProviderStateCoordinate, ResourceRef, TextToImageInvokeRequest, VideoExtendRequest,
        VideoTextToVideoRequest,
    };

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://ark.cn-beijing.volces.com/api/plan/v3".to_owned(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "doubao".to_owned(),
                adapter_type: DOUBAO_MEDIA_ADAPTER_ID.to_owned(),
                origin_provider: "doubao".to_owned(),
                origin_model: "model".to_owned(),
            },
            credential: Some(ResolvedCredential::bearer("secret://ark", "secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(10),
                max_request_bytes: MAX_REQUEST_BYTES,
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        }
    }

    #[test]
    fn uses_agent_plan_image_and_content_task_endpoints() {
        let (descriptor, registration) = doubao_media_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        let context = context();
        let parameters =
            BTreeMap::from([("provider_model_id".to_owned(), json!("doubao-seedream-4-0"))]);
        let image = CodecInput {
            canonical_request: AiccCall::ImagesGenerate(TextToImageInvokeRequest::new(
                "ignored", "cat",
            )),
            resolved_parameters: parameters.clone(),
        };
        let request = registry
            .encode(
                DOUBAO_MEDIA_ADAPTER_ID,
                DOUBAO_IMAGE_OPERATION_ID,
                ApiType::ImageTextToImage,
                &image,
                &context,
            )
            .unwrap();
        assert_eq!(
            request.url,
            "https://ark.cn-beijing.volces.com/api/plan/v3/images/generations"
        );

        let video = CodecInput {
            canonical_request: AiccCall::VideoTextToVideo(VideoTextToVideoRequest::new(
                "ignored",
                "cat".to_owned(),
            )),
            resolved_parameters: parameters.clone(),
        };
        let input = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&video),
            resolved_parameters: &parameters,
            context: &context,
        };
        let request = registry
            .encode_native(
                DOUBAO_MEDIA_ADAPTER_ID,
                DOUBAO_VIDEO_OPERATION_ID,
                ApiType::VideoTextToVideo,
                &input,
            )
            .unwrap();
        assert_eq!(
            request.url,
            "https://ark.cn-beijing.volces.com/api/plan/v3/contents/generations/tasks"
        );
    }

    #[test]
    fn standard_seedance_extend_uses_official_reference_task_fields() {
        let (descriptor, registration) = doubao_media_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        let mut context = context();
        context.base_url = "https://ark.cn-beijing.volces.com/api/v3".to_owned();
        let parameters = BTreeMap::from([
            (
                "provider_model_id".to_owned(),
                json!("doubao-seedance-2-5-260628"),
            ),
            ("omni_reference_task_type".to_owned(), json!("extend")),
        ]);
        let video = CodecInput {
            canonical_request: AiccCall::VideoExtend(VideoExtendRequest::new(
                "ignored",
                ResourceRef::url("https://resource.example/video.mp4".to_owned(), None),
                "Continue the scene".to_owned(),
            )),
            resolved_parameters: parameters.clone(),
        };
        let input = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&video),
            resolved_parameters: &parameters,
            context: &context,
        };
        let request = registry
            .encode_native(
                DOUBAO_MEDIA_ADAPTER_ID,
                DOUBAO_VIDEO_OPERATION_ID,
                ApiType::VideoExtend,
                &input,
            )
            .unwrap();
        assert_eq!(
            request.url,
            "https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks"
        );
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON body")
        };
        assert_eq!(body["omni_reference_task_type"], "extend");
        assert_eq!(body["content"][1]["role"], "reference_video");
        assert!(body.get("operation").is_none());
        assert!(body.get("continuation_handle").is_none());
    }

    #[test]
    fn video_duration_is_encoded_as_an_integer() {
        let mut body = Map::new();
        video_options(&mut body, Some(4.0), None, None, None, None).unwrap();
        assert_eq!(body["duration"], json!(4));
        assert_eq!(serde_json::to_string(&body["duration"]).unwrap(), "4");
        assert!(video_options(&mut Map::new(), Some(4.5), None, None, None, None).is_err());
    }

    #[tokio::test]
    async fn submitted_video_task_preserves_cancel_support() {
        let codec = DoubaoVideoCodec {
            descriptor: doubao_media_registration()
                .0
                .into_iter()
                .find(|operation| operation.operation_id == DOUBAO_VIDEO_OPERATION_ID)
                .unwrap(),
            api_type: ApiType::VideoTextToVideo,
        };
        let response = HttpResponse {
            status: reqwest::StatusCode::OK,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from_static(br#"{"id":"video-1","status":"queued"}"#),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let NativeTaskOutput::Submitted(handle) = codec
            .decode_native(NativeTaskOperation::Submit, response)
            .await
            .unwrap()
        else {
            panic!("expected submitted task")
        };
        assert!(handle.cancel_supported);
    }

    #[test]
    fn error_response_keeps_doubao_business_code() {
        let response = HttpResponse {
            status: reqwest::StatusCode::BAD_REQUEST,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from_static(
                br#"{"error":{"code":"InvalidParameter","message":"invalid input"}}"#,
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let error = ensure_success(&response).unwrap_err();
        assert_eq!(error.provider_code.as_deref(), Some("InvalidParameter"));
    }

    #[test]
    fn image_response_preserves_jpeg_mime_from_provider_url() {
        let output = decode_images(&json!({
            "data": [{"url": "https://example.test/generated/image.jpeg?signature=opaque"}]
        }))
        .unwrap();
        assert_eq!(output.value["images"][0]["mime_hint"], "image/jpeg");
        assert_eq!(output.artifacts[0].mime.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn video_result_decodes_official_content_url_and_usage() {
        let NativeTaskOutput::Result(output) = decode_video_result(&json!({
            "id": "video-1",
            "status": "succeeded",
            "content": {"video_url": "https://example.test/generated/video.mp4"},
            "usage": {"completion_tokens": 120, "total_tokens": 120}
        }))
        .unwrap()
        else {
            panic!("expected video result")
        };
        assert_eq!(
            output.value["video"]["url"],
            "https://example.test/generated/video.mp4"
        );
        let usage = output.usage.unwrap();
        assert_eq!(usage.output_tokens, Some(120));
        assert_eq!(usage.total_tokens, Some(120));
    }

    #[test]
    fn video_result_rejects_non_official_nested_url_shape() {
        assert!(decode_video_result(&json!({
            "status": "succeeded",
            "content": {"video_url": {"url": "https://example.test/video.mp4"}}
        }))
        .is_err());
    }
}
