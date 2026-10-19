use super::video::{
    generated_video, planned_resource, require_video_parameters, unsupported as video_unsupported,
    video_generate_input, video_output, whole_seconds,
};
use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration, ExecutionMode, HttpBody,
    HttpRequest, HttpResponse, NativeTaskCodec, NativeTaskHandle, NativeTaskInput,
    NativeTaskOperation, NativeTaskOutput, NativeTaskState, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput,
    ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiUsage, AiccCall, ApiType, KeyframePosition, ResourceRef, VideoConditionKind,
    VideoMetadata,
};
use reqwest::header::{HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const QWEN_MEDIA_ADAPTER_ID: &str = "qwen-media";
pub(crate) const QWEN_IMAGE_OPERATION_ID: &str = "dashscope.image_synthesis";
pub(crate) const QWEN_IMAGE_GENERATION_OPERATION_ID: &str = "dashscope.image_generation";
pub(crate) const QWEN_IMAGE_EDIT_OPERATION_ID: &str = "dashscope.image_edit";
pub(crate) const QWEN_VIDEO_OPERATION_ID: &str = "dashscope.video_synthesis";
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn qwen_media_registration() -> (Vec<OperationDescriptor>, CodecRegistration) {
    let image = task_operation(QWEN_IMAGE_OPERATION_ID, &[ApiType::ImageTextToImage]);
    let image_generation = task_operation_without_cancel(
        QWEN_IMAGE_GENERATION_OPERATION_ID,
        &[ApiType::ImageTextToImage, ApiType::ImageImageToImage],
    );
    let image_edit = immediate_operation(QWEN_IMAGE_EDIT_OPERATION_ID, ApiType::ImageImageToImage);
    let video = task_operation(QWEN_VIDEO_OPERATION_ID, &[ApiType::VideoGenerate]);
    let native_task_codecs = [
        (image.clone(), ApiType::ImageTextToImage),
        (image_generation.clone(), ApiType::ImageTextToImage),
        (image_generation.clone(), ApiType::ImageImageToImage),
        (video.clone(), ApiType::VideoGenerate),
    ]
    .into_iter()
    .map(|(descriptor, api_type)| {
        if descriptor.operation_id == QWEN_IMAGE_GENERATION_OPERATION_ID {
            Arc::new(QwenImageGenerationTaskCodec {
                descriptor,
                api_type,
            }) as Arc<dyn NativeTaskCodec>
        } else {
            Arc::new(QwenMediaCodec {
                descriptor,
                api_type,
            }) as Arc<dyn NativeTaskCodec>
        }
    })
    .collect();
    (
        vec![image, image_generation.clone(), image_edit.clone(), video],
        CodecRegistration {
            operation_codecs: vec![Arc::new(QwenImageEditCodec {
                descriptor: image_edit,
                api_type: ApiType::ImageImageToImage,
            })],
            native_task_codecs,
        },
    )
}

pub(crate) fn qwen_media_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (operations, registration) = qwen_media_registration();
    (
        AdapterDescriptor {
            protocol_family_id: "qwen".to_owned(),
            protocol_adapter_id: QWEN_MEDIA_ADAPTER_ID.to_owned(),
            interface_generation: "dashscope-v1".to_owned(),
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

fn task_operation(id: &str, api_types: &[ApiType]) -> OperationDescriptor {
    OperationDescriptor {
        operation_id: id.to_owned(),
        bindings: api_types
            .iter()
            .copied()
            .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::NativeTask]))
            .collect(),
        supports_cancel: true,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

fn task_operation_without_cancel(id: &str, api_types: &[ApiType]) -> OperationDescriptor {
    let mut descriptor = task_operation(id, api_types);
    descriptor.supports_cancel = false;
    descriptor
}

fn immediate_operation(id: &str, api_type: ApiType) -> OperationDescriptor {
    OperationDescriptor {
        operation_id: id.to_owned(),
        bindings: vec![OperationBinding::new(api_type, [ExecutionMode::Immediate])],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

#[derive(Clone)]
struct QwenImageEditCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl OperationCodec for QwenImageEditCodec {
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
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        json_request(
            call.context,
            Method::POST,
            "services/aigc/multimodal-generation/generation",
            Some(encode_qwen_image_request(
                &call.input.canonical_request,
                &call.input.resolved_parameters,
                call.context,
                self.api_type,
            )?),
            false,
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        Ok(ProtocolExecution::Immediate(decode_qwen_image_result(
            &value,
        )?))
    }
}

fn encode_qwen_image_request(
    canonical_request: &AiccCall,
    resolved_parameters: &BTreeMap<String, Value>,
    context: &super::CodecContext,
    api_type: ApiType,
) -> ProtocolResultValue<Value> {
    require_only_model(resolved_parameters)?;
    let model = provider_model_id(resolved_parameters)?;
    let (content, size, n, seed) = match canonical_request {
        AiccCall::ImagesGenerate(request) if api_type == ApiType::ImageTextToImage => {
            context.ignore_unsupported_options(
                "Qwen image generation",
                &[
                    ("aspect_ratio", request.aspect_ratio.is_some()),
                    ("quality", request.quality.is_some()),
                    ("style", request.style.is_some()),
                ],
            );
            (
                vec![json!({"text":request.prompt})],
                request.size.as_ref(),
                request.n,
                request.seed,
            )
        }
        AiccCall::ImageToImage(request) if api_type == ApiType::ImageImageToImage => {
            if request.images.is_empty() || request.images.len() > 3 {
                return Err(ProtocolError::invalid_request(
                    "Qwen image edit requires one to three images",
                ));
            }
            context.ignore_unsupported_options(
                "Qwen image edit",
                &[("strength", request.strength.is_some())],
            );
            let mut content = request
                .images
                .iter()
                .map(|image| resource_string(image, context).map(|image| json!({"image":image})))
                .collect::<ProtocolResultValue<Vec<_>>>()?;
            content.push(json!({"text":request.prompt}));
            (
                content,
                request
                    .output
                    .as_ref()
                    .and_then(|output| output.size.as_ref()),
                None,
                None,
            )
        }
        _ => {
            return Err(ProtocolError::invalid_request(
                "Qwen image generation codec received the wrong canonical request",
            ));
        }
    };
    let mut parameters = Map::new();
    if let Some(size) = size {
        parameters.insert(
            "size".to_owned(),
            json!(size.replace('x', "*").replace('X', "*")),
        );
    }
    if let Some(n) = n {
        parameters.insert("n".to_owned(), json!(n));
    }
    if let Some(seed) = seed {
        parameters.insert("seed".to_owned(), json!(seed));
    }
    Ok(
        json!({"model":model,"input":{"messages":[{"role":"user","content":content}]},"parameters":parameters}),
    )
}

fn decode_qwen_image_result(value: &Value) -> ProtocolResultValue<ProtocolOutput> {
    let content = value
        .pointer("/output/choices/0/message/content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProtocolError::invalid_response(
                "Qwen image edit response is missing output.choices content",
            )
        })?;
    let images = content
        .iter()
        .filter_map(|item| item.get("image").and_then(Value::as_str))
        .filter(|url| !url.is_empty())
        .map(|url| ResourceRef::url(url.to_owned(), Some("image/png".to_owned())))
        .collect::<Vec<_>>();
    if images.is_empty() {
        return Err(ProtocolError::invalid_response(
            "Qwen image edit response contains no images",
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
        usage: Some(AiUsage {
            image_units: Some(images.len() as u64),
            ..AiUsage::request_units(1)
        }),
        artifacts,
    })
}

#[derive(Clone)]
struct QwenImageGenerationTaskCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for QwenImageGenerationTaskCodec {
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
            NativeTaskOperation::Submit => {
                let codec_input = input.codec_input.ok_or_else(|| {
                    ProtocolError::invalid_request("Qwen image submit requires canonical input")
                })?;
                json_request(
                    input.context,
                    Method::POST,
                    "services/aigc/image-generation/generation",
                    Some(encode_qwen_image_request(
                        &codec_input.canonical_request,
                        input.resolved_parameters,
                        input.context,
                        self.api_type,
                    )?),
                    true,
                )
            }
            NativeTaskOperation::Status | NativeTaskOperation::Result => {
                let id = safe_task_id(input.remote_task_id)?;
                json_request(
                    input.context,
                    Method::GET,
                    &format!("tasks/{id}"),
                    None,
                    false,
                )
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::invalid_request(
                "Qwen image generation tasks do not expose cancellation",
            )),
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
                let mut handle =
                    NativeTaskHandle::new(required_pointer(&value, "/output/task_id")?)?;
                handle.poll_after = retry_after.or(Some(Duration::from_secs(5)));
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let state = decode_state(&required_pointer(&value, "/output/task_status")?)?;
                if state == NativeTaskState::Failed {
                    let error = value.get("output").unwrap_or(&value);
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::ProviderRejected,
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Provider image task failed"),
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
                    retry_after: retry_after.or(Some(Duration::from_secs(5))),
                    result_ref: (state == NativeTaskState::Succeeded)
                        .then(|| required_pointer(&value, "/output/task_id"))
                        .transpose()?,
                    result_artifacts: BTreeMap::new(),
                })
            }
            NativeTaskOperation::Result => {
                Ok(NativeTaskOutput::Result(decode_qwen_image_result(&value)?))
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::invalid_request(
                "Qwen image generation tasks do not expose cancellation",
            )),
        }
    }
}

#[derive(Clone)]
struct QwenMediaCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for QwenMediaCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // DashScope accepts media by URL; `resource_string` passes it through
        // and only inlines bytes that were supplied as bytes.
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
            NativeTaskOperation::Cancel,
        ])
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        match input.operation {
            NativeTaskOperation::Submit => encode_submit(input, self.api_type),
            NativeTaskOperation::Status | NativeTaskOperation::Result => {
                let id = safe_task_id(input.remote_task_id)?;
                json_request(
                    input.context,
                    Method::GET,
                    &format!("tasks/{id}"),
                    None,
                    false,
                )
            }
            NativeTaskOperation::Cancel => {
                let id = safe_task_id(input.remote_task_id)?;
                json_request(
                    input.context,
                    Method::POST,
                    &format!("tasks/{id}/cancel"),
                    None,
                    false,
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
                let mut handle =
                    NativeTaskHandle::new(required_pointer(&value, "/output/task_id")?)?;
                handle.poll_after = retry_after.or(Some(Duration::from_secs(5)));
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let state = decode_state(&required_pointer(&value, "/output/task_status")?)?;
                if state == NativeTaskState::Failed {
                    let error = value.get("output").unwrap_or(&value);
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
                    retry_after: retry_after.or(Some(Duration::from_secs(5))),
                    result_ref: (state == NativeTaskState::Succeeded)
                        .then(|| required_pointer(&value, "/output/task_id"))
                        .transpose()?,
                    result_artifacts: BTreeMap::new(),
                })
            }
            NativeTaskOperation::Result => decode_result(&value, self.api_type),
            NativeTaskOperation::Cancel => Ok(NativeTaskOutput::Cancelled { accepted: true }),
        }
    }
}

fn encode_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("Qwen media submit requires canonical input")
    })?;
    if api_type == ApiType::VideoGenerate {
        require_video_parameters(input.resolved_parameters, "Qwen", &[])?;
    } else {
        require_only_model(input.resolved_parameters)?;
    }
    let model = provider_model_id(input.resolved_parameters)?;
    let mut provider_input = Map::new();
    let mut parameters = Map::new();
    let path = match (&codec_input.canonical_request, api_type) {
        (AiccCall::ImagesGenerate(request), ApiType::ImageTextToImage) => {
            provider_input.insert("prompt".to_owned(), json!(request.prompt));
            if let Some(value) = &request.negative_prompt {
                provider_input.insert("negative_prompt".to_owned(), json!(value));
            }
            if let Some(value) = request.n {
                parameters.insert("n".to_owned(), json!(value));
            }
            if let Some(value) = &request.size {
                parameters.insert(
                    "size".to_owned(),
                    json!(value.replace('x', "*").replace('X', "*")),
                );
            }
            if let Some(value) = request.seed {
                parameters.insert("seed".to_owned(), json!(value));
            }
            "services/aigc/text2image/image-synthesis"
        }
        (AiccCall::VideoGenerate(_), ApiType::VideoGenerate) => {
            let (request, plan) = video_generate_input(codec_input)?;
            if plan.fps.is_some() {
                return Err(video_unsupported("Wan video generation has no fps option"));
            }
            if let Some(prompt) = &plan.submitted_prompt {
                provider_input.insert("prompt".to_owned(), json!(prompt));
            }
            if let Some(negative) = &plan.negative_prompt {
                provider_input.insert("negative_prompt".to_owned(), json!(negative));
            }
            for planned in &plan.inputs {
                if (planned.kind, planned.position)
                    != (VideoConditionKind::Keyframe, Some(KeyframePosition::Start))
                {
                    return Err(video_unsupported(
                        "Wan video generation only takes a start keyframe",
                    ));
                }
                provider_input.insert(
                    "img_url".to_owned(),
                    json!(resource_string(
                        planned_resource(request, planned)?,
                        input.context
                    )?),
                );
            }
            if let Some(seconds) = whole_seconds(&plan)? {
                parameters.insert("duration".to_owned(), json!(seconds));
            }
            match (plan.size, &plan.resolution) {
                (Some(size), _) => {
                    parameters.insert(
                        "size".to_owned(),
                        json!(format!("{}*{}", size.width, size.height)),
                    );
                }
                (None, Some(resolution)) => {
                    parameters.insert("resolution".to_owned(), json!(resolution));
                    if model.starts_with("wan2.7") {
                        if let Some(ratio) = &plan.aspect_ratio {
                            parameters.insert("ratio".to_owned(), json!(ratio));
                        }
                    }
                }
                (None, None) => {}
            }
            if let Some(seed) = plan.seed {
                parameters.insert("seed".to_owned(), json!(seed));
            }
            if let Some(audio) = plan.generate_audio {
                parameters.insert("audio".to_owned(), json!(audio));
            }
            "services/aigc/video-generation/video-synthesis"
        }
        _ => {
            return Err(ProtocolError::invalid_request(
                "Qwen media codec received the wrong canonical request",
            ))
        }
    };
    let body = json!({"model":model,"input":provider_input,"parameters":parameters});
    json_request(input.context, Method::POST, path, Some(body), true)
}

fn decode_result(value: &Value, api_type: ApiType) -> ProtocolResultValue<NativeTaskOutput> {
    if api_type == ApiType::ImageTextToImage {
        let results = value
            .pointer("/output/results")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProtocolError::invalid_response("Qwen image result is missing output.results")
            })?;
        let mut images = Vec::new();
        for item in results {
            if item
                .get("code")
                .and_then(Value::as_str)
                .is_some_and(|code| !code.is_empty())
            {
                continue;
            }
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ProtocolError::invalid_response("Qwen image result is missing url")
                })?;
            images.push(ResourceRef::url(
                url.to_owned(),
                Some("image/png".to_owned()),
            ));
        }
        if images.is_empty() {
            return Err(ProtocolError::invalid_response(
                "Qwen image result contains no images",
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
        return Ok(NativeTaskOutput::Result(ProtocolOutput {
            value: json!({"images":images,"provider_states":[]}),
            usage: Some(AiUsage {
                image_units: Some(images.len() as u64),
                ..AiUsage::request_units(1)
            }),
            artifacts,
        }));
    }
    let url = value
        .pointer("/output/video_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProtocolError::invalid_response("Qwen video result is missing output.video_url")
        })?;
    let seconds = value
        .pointer("/usage/duration")
        .or_else(|| value.pointer("/usage/video_duration"))
        .and_then(Value::as_f64);
    let (width, height) = super::video::size_metadata(
        value
            .pointer("/usage/size")
            .and_then(Value::as_str)
            .map(|size| size.replace('*', "x"))
            .as_deref(),
    );
    Ok(NativeTaskOutput::Result(video_output(
        vec![generated_video(
            ResourceRef::url(url.to_owned(), Some("video/mp4".to_owned())),
            Some("video/mp4".to_owned()),
            VideoMetadata {
                duration_seconds: seconds,
                width,
                height,
                ..Default::default()
            },
        )],
        AiUsage {
            video_seconds: seconds,
            ..AiUsage::request_units(1)
        },
    )?))
}

fn decode_state(status: &str) -> ProtocolResultValue<NativeTaskState> {
    match status.to_ascii_uppercase().as_str() {
        "PENDING" | "SUBMITTED" => Ok(NativeTaskState::Queued),
        "RUNNING" => Ok(NativeTaskState::Running),
        "SUCCEEDED" => Ok(NativeTaskState::Succeeded),
        "FAILED" | "UNKNOWN" => Ok(NativeTaskState::Failed),
        "CANCELED" | "CANCELLED" => Ok(NativeTaskState::Cancelled),
        _ => Err(ProtocolError::invalid_response(
            "Qwen media response contains an unknown task_status",
        )),
    }
}

fn json_request(
    context: &super::CodecContext,
    method: Method,
    path: &str,
    body: Option<Value>,
    asynchronous: bool,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("Qwen base URL is invalid"))?;
    let current = url.path().trim_end_matches('/');
    let base = current
        .strip_suffix("/compatible-mode/v1")
        .or_else(|| current.strip_suffix("/api/v1"))
        .unwrap_or(current);
    url.set_path(&format!("{base}/api/v1/{}", path.trim_start_matches('/')));
    let mut request = HttpRequest::new(method, url.to_string());
    if let Some(body) = body {
        request
            .headers
            .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        request.body = HttpBody::Json(body);
    }
    if asynchronous {
        request.headers.insert(
            HeaderName::from_static("x-dashscope-async"),
            HeaderValue::from_static("enable"),
        );
    }
    context
        .credential
        .as_ref()
        .ok_or_else(|| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Qwen media operation requires a credential",
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
            "resolved Qwen media parameter `{name}` is not supported"
        )));
    }
    Ok(())
}

fn resource_string(
    resource: &ResourceRef,
    context: &super::CodecContext,
) -> ProtocolResultValue<String> {
    if let Some(url) = context.materialized_url(resource) {
        return Ok(url.to_string());
    }
    match resource {
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("Qwen media resource contains invalid base64")
            })?;
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        ResourceRef::Url { url, .. }
            if !context
                .resources
                .contains_key(crate::resource::ResourceKey::from_ref(resource).as_str()) =>
        {
            Ok(url.clone())
        }
        ResourceRef::Url { .. } | ResourceRef::NamedObject { .. } => {
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
        .ok_or_else(|| ProtocolError::invalid_request("Qwen media task ID is invalid"))
}

fn required_pointer(value: &Value, pointer: &str) -> ProtocolResultValue<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("Qwen response is missing `{pointer}`"))
        })
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
                .get("message")
                .or_else(|| value.pointer("/error/message"))
        })
        .and_then(Value::as_str)
        .unwrap_or("Qwen media request failed");
    Err(ProtocolError::new(
        super::protocol_error_kind_from_http_status(response.status),
        message,
    )
    .with_provider_code(
        parsed
            .as_ref()
            .and_then(|value| value.get("code").or_else(|| value.pointer("/error/code")))
            .and_then(Value::as_str)
            .map(str::to_owned),
    )
    .with_http_status(response.status.as_u16())
    .with_request_id(Some(response.request_id.clone()))
    .with_retry_after(response.retry_after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, MaterializedResource,
        ResolvedCredential,
    };
    use crate::video::test_support::{model_contract, plan_parameters};
    use crate::video::VideoMaterialFacts;
    use buckyos_api::{
        ImageToImageRequest, ProviderStateCoordinate, VideoAudioMode, VideoGenerateRequest,
        VideoOutputSpec, VideoSize,
    };
    use bytes::Bytes;

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://ws.cn-beijing.maas.aliyuncs.com/compatible-mode/v1".to_owned(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "qwen".to_owned(),
                adapter_type: QWEN_MEDIA_ADAPTER_ID.to_owned(),
                origin_provider: "qwen".to_owned(),
                origin_model: "model".to_owned(),
            },
            credential: Some(ResolvedCredential::bearer("secret://qwen", "secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(10),
                max_request_bytes: MAX_REQUEST_BYTES,
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        }
    }

    #[test]
    fn uses_native_tasks_for_qwen_image_3_generation_and_editing() {
        let (descriptor, registration) = qwen_media_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        let public_context = context();
        let parameters =
            BTreeMap::from([("provider_model_id".to_owned(), json!("qwen-image-3.0-pro"))]);
        let source = ResourceRef::url(
            "https://public.example/a.png".to_owned(),
            Some("image/png".to_owned()),
        );
        let edit = CodecInput {
            canonical_request: AiccCall::ImageToImage(ImageToImageRequest::new(
                "ignored",
                vec![source],
                "edit".to_owned(),
            )),
            resolved_parameters: parameters.clone(),
        };
        let input = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&edit),
            resolved_parameters: &parameters,
            context: &public_context,
        };
        let request = registry
            .encode_native(
                QWEN_MEDIA_ADAPTER_ID,
                QWEN_IMAGE_GENERATION_OPERATION_ID,
                ApiType::ImageImageToImage,
                &input,
            )
            .unwrap();
        assert_eq!(request.url, "https://ws.cn-beijing.maas.aliyuncs.com/api/v1/services/aigc/image-generation/generation");
        assert_eq!(request.headers.get("x-dashscope-async").unwrap(), "enable");
        let HttpBody::Json(body) = &request.body else {
            panic!("Qwen image edit did not encode a JSON body")
        };
        assert_eq!(
            body["input"]["messages"][0]["content"][0]["image"],
            "https://public.example/a.png"
        );

        let protected_source = ResourceRef::url(
            "https://owner-only.example/a.png".to_owned(),
            Some("image/png".to_owned()),
        );
        let mut protected_context = context();
        protected_context.resources.insert(
            crate::resource::ResourceKey::from_ref(&protected_source).into_string(),
            MaterializedResource::new(Bytes::from_static(b"image"), "image/png", None).unwrap(),
        );
        let protected_edit = CodecInput {
            canonical_request: AiccCall::ImageToImage(ImageToImageRequest::new(
                "ignored",
                vec![protected_source],
                "edit".to_owned(),
            )),
            resolved_parameters: parameters.clone(),
        };
        let protected_input = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&protected_edit),
            resolved_parameters: &parameters,
            context: &protected_context,
        };
        let protected_request = registry
            .encode_native(
                QWEN_MEDIA_ADAPTER_ID,
                QWEN_IMAGE_GENERATION_OPERATION_ID,
                ApiType::ImageImageToImage,
                &protected_input,
            )
            .unwrap();
        let HttpBody::Json(protected_body) = &protected_request.body else {
            panic!("Qwen image edit did not encode a JSON body")
        };
        assert_eq!(
            protected_body["input"]["messages"][0]["content"][0]["image"],
            "data:image/png;base64,aW1hZ2U="
        );
        assert!(!protected_body.to_string().contains("owner-only.example"));

        let mut generate = VideoGenerateRequest::new("wan@qwen", Some("cat".to_owned()));
        generate.negative_prompt = Some("blur".to_owned());
        generate.output = Some(VideoOutputSpec {
            duration_seconds: Some(5.0),
            aspect_ratio: Some("9:16".into()),
            size: Some(VideoSize::short_edge(720)),
            audio: Some(VideoAudioMode::None),
            ..Default::default()
        });
        let video_parameters = plan_parameters(
            &model_contract("qwen", "wan2.6-t2v"),
            &generate,
            &VideoMaterialFacts::new(),
            "wan2.6-t2v",
        );
        let video = CodecInput {
            canonical_request: AiccCall::VideoGenerate(generate),
            resolved_parameters: video_parameters.clone(),
        };
        let input = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&video),
            resolved_parameters: &video_parameters,
            context: &context,
        };
        let request = registry
            .encode_native(
                QWEN_MEDIA_ADAPTER_ID,
                QWEN_VIDEO_OPERATION_ID,
                ApiType::VideoGenerate,
                &input,
            )
            .unwrap();
        assert_eq!(request.url, "https://ws.cn-beijing.maas.aliyuncs.com/api/v1/services/aigc/video-generation/video-synthesis");
        assert_eq!(request.headers.get("x-dashscope-async").unwrap(), "enable");
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON")
        };
        assert_eq!(body["input"]["negative_prompt"], "blur");
        assert_eq!(body["parameters"]["size"], "720*1280");
        assert_eq!(body["parameters"]["duration"], 5);
        assert_eq!(body["parameters"]["audio"], false);
    }
}
