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
                body.insert("size".to_owned(), json!(ratio));
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
                handle.poll_after = retry_after.or(Some(Duration::from_secs(3)));
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let state = decode_state(required_string(&value, "status")?.as_str())?;
                Ok(NativeTaskOutput::Status {
                    state,
                    retry_after,
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
    require_only_model(input.resolved_parameters)?;
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
            );
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
            );
        }
        (AiccCall::VideoToVideo(request), ApiType::VideoToVideo) => {
            content.push(json!({"type":"text","text":request.prompt}));
            content.push(json!({"type":"video_url","video_url":{"url":resource_string(&request.video, input.context)?}}));
        }
        (AiccCall::VideoExtend(request), ApiType::VideoExtend) => {
            content.push(json!({"type":"text","text":request.prompt}));
            content.push(json!({"type":"video_url","video_url":{"url":resource_string(&request.video, input.context)?}}));
            video_options(
                &mut parameters,
                request.duration_seconds,
                None,
                request.resolution.as_deref(),
                None,
                None,
            );
            parameters.insert("operation".to_owned(), json!("extend"));
            if let Some(handle) = &request.continuation_handle {
                parameters.insert("continuation_handle".to_owned(), json!(handle));
            }
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
) {
    if let Some(value) = duration {
        body.insert("duration".to_owned(), json!(value));
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
            images.push(ResourceRef::url(
                url.to_owned(),
                Some("image/png".to_owned()),
            ));
        } else if let Some(encoded) = item
            .get("b64_json")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            STANDARD.decode(encoded).map_err(|_| {
                ProtocolError::invalid_response("Doubao image response contains invalid base64")
            })?;
            images.push(ResourceRef::base64(
                "image/png".to_owned(),
                encoded.to_owned(),
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
        .map(|(index, resource)| AiArtifact {
            name: format!("image-{}", index + 1),
            resource: resource.clone(),
            mime: Some("image/png".to_owned()),
            metadata: None,
        })
        .collect();
    Ok(ProtocolOutput {
        value: json!({"images":images,"provider_states":[]}),
        usage: Some(AiUsage::request_units(data.len() as u64)),
        artifacts,
    })
}

fn decode_video_result(value: &Value) -> ProtocolResultValue<NativeTaskOutput> {
    let url = value
        .pointer("/content/video_url/url")
        .or_else(|| value.get("video_url"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProtocolError::invalid_response("Doubao video result is missing video_url")
        })?;
    let resource = ResourceRef::url(url.to_owned(), Some("video/mp4".to_owned()));
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: json!({"video":resource}),
        usage: Some(AiUsage::request_units(1)),
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
    use buckyos_api::{ProviderStateCoordinate, TextToImageInvokeRequest, VideoTextToVideoRequest};

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_owned(),
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
    fn uses_ark_v3_image_and_content_task_endpoints() {
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
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
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
            "https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks"
        );
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
}
