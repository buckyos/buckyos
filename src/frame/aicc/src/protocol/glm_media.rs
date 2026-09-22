use super::{
    openai_responses_adapter, CodecRegistration, HttpBody, HttpRequest, HttpResponse,
    NativeTaskCodec, NativeTaskHandle, NativeTaskInput, NativeTaskOperation, NativeTaskOutput,
    NativeTaskState, OperationBinding, OperationDescriptor, ProtocolError, ProtocolErrorKind,
    ProtocolOutput, ProtocolResultValue, OPENAI_AUDIO_SPEECH_OPERATION_ID,
    OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID, OPENAI_EMBEDDINGS_OPERATION_ID,
    OPENAI_IMAGES_GENERATE_OPERATION_ID,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{AiArtifact, AiUsage, AiccCall, ApiType, ResourceRef};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

const GLM_VIDEOS_OPERATION_ID: &str = "videos.generate";
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn glm_media_registration() -> (Vec<OperationDescriptor>, CodecRegistration) {
    let compatible_ids = BTreeSet::from([
        OPENAI_EMBEDDINGS_OPERATION_ID,
        OPENAI_IMAGES_GENERATE_OPERATION_ID,
        OPENAI_AUDIO_SPEECH_OPERATION_ID,
        OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID,
    ]);
    let (openai, openai_codecs) = openai_responses_adapter();
    let mut operations = openai
        .operations
        .into_values()
        .filter(|operation| compatible_ids.contains(operation.operation_id.as_str()))
        .collect::<Vec<_>>();
    let mut registration = CodecRegistration {
        operation_codecs: openai_codecs
            .operation_codecs
            .into_iter()
            .filter(|codec| compatible_ids.contains(codec.descriptor().operation_id.as_str()))
            .collect(),
        native_task_codecs: Vec::new(),
    };

    let video = OperationDescriptor {
        operation_id: GLM_VIDEOS_OPERATION_ID.to_owned(),
        bindings: [ApiType::VideoTextToVideo, ApiType::VideoImageToVideo]
            .into_iter()
            .map(|api_type| OperationBinding::new(api_type, [super::ExecutionMode::NativeTask]))
            .collect(),
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    registration.native_task_codecs.extend([
        Arc::new(GlmVideoCodec {
            descriptor: video.clone(),
            api_type: ApiType::VideoTextToVideo,
        }) as Arc<dyn NativeTaskCodec>,
        Arc::new(GlmVideoCodec {
            descriptor: video.clone(),
            api_type: ApiType::VideoImageToVideo,
        }) as Arc<dyn NativeTaskCodec>,
    ]);
    operations.push(video);
    (operations, registration)
}

#[derive(Clone)]
struct GlmVideoCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for GlmVideoCodec {
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
            NativeTaskOperation::Status | NativeTaskOperation::Result => {
                let task_id = safe_task_id(input.remote_task_id)?;
                json_request(
                    input.context,
                    Method::GET,
                    &format!("async-result/{task_id}"),
                    None,
                )
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "GLM video generation does not support cancellation",
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
                let mut handle = NativeTaskHandle::new(required_string(&value, "id")?)?;
                handle.poll_after = retry_after.or(Some(Duration::from_secs(2)));
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let state = decode_state(required_string(&value, "task_status")?.as_str())?;
                Ok(NativeTaskOutput::Status {
                    state,
                    retry_after,
                    result_ref: (state == NativeTaskState::Succeeded)
                        .then(|| required_string(&value, "id"))
                        .transpose()?,
                    result_artifacts: Default::default(),
                })
            }
            NativeTaskOperation::Result => decode_result(&value),
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "GLM video generation does not support cancellation",
            )),
        }
    }
}

fn encode_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input
        .codec_input
        .ok_or_else(|| ProtocolError::invalid_request("GLM video submit requires input"))?;
    require_parameter_subset(input.resolved_parameters)?;
    let model = input
        .resolved_parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved provider_model_id"))?;
    let mut body = match (&codec_input.canonical_request, api_type) {
        (AiccCall::VideoTextToVideo(request), ApiType::VideoTextToVideo) => Map::from_iter([
            ("model".to_owned(), json!(model)),
            ("prompt".to_owned(), json!(request.prompt)),
        ]),
        (AiccCall::VideoImageToVideo(request), ApiType::VideoImageToVideo) => Map::from_iter([
            ("model".to_owned(), json!(model)),
            ("prompt".to_owned(), json!(request.prompt)),
            (
                "image_url".to_owned(),
                json!(resource_string(&request.image, input.context)?),
            ),
        ]),
        _ => {
            return Err(ProtocolError::invalid_request(
                "GLM video codec received the wrong canonical request",
            ));
        }
    };
    match &codec_input.canonical_request {
        AiccCall::VideoTextToVideo(request) => {
            if let Some(duration) = request.duration_seconds {
                body.insert("duration".to_owned(), json!(duration));
            }
            if let Some(resolution) = &request.resolution {
                body.insert("size".to_owned(), json!(resolution));
            }
        }
        AiccCall::VideoImageToVideo(request) => {
            if let Some(duration) = request.duration_seconds {
                body.insert("duration".to_owned(), json!(duration));
            }
            if let Some(resolution) = &request.resolution {
                body.insert("size".to_owned(), json!(resolution));
            }
        }
        _ => {}
    }
    json_request(
        input.context,
        Method::POST,
        "videos/generations",
        Some(Value::Object(body)),
    )
}

fn json_request(
    context: &super::CodecContext,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("GLM base URL is invalid"))?;
    let base = url.path().trim_end_matches('/');
    url.set_path(&format!("{base}/{}", path.trim_start_matches('/')));
    let mut request = HttpRequest::new(method, url.to_string());
    if let Some(body) = body {
        request
            .headers
            .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        request.body = HttpBody::Json(body);
    }
    let credential = context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "GLM media operation requires a resolved credential",
        )
    })?;
    credential.apply(&mut request.headers)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request)
}

fn decode_state(status: &str) -> ProtocolResultValue<NativeTaskState> {
    match status.to_ascii_uppercase().as_str() {
        "SUBMITTED" | "QUEUED" => Ok(NativeTaskState::Queued),
        "PROCESSING" | "RUNNING" => Ok(NativeTaskState::Running),
        "SUCCESS" | "SUCCEEDED" => Ok(NativeTaskState::Succeeded),
        "FAIL" | "FAILED" => Ok(NativeTaskState::Failed),
        _ => Err(ProtocolError::invalid_response(
            "GLM video response contains an unknown task_status",
        )),
    }
}

fn decode_result(value: &Value) -> ProtocolResultValue<NativeTaskOutput> {
    let videos = value
        .get("video_result")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProtocolError::invalid_response("GLM video result is missing video_result")
        })?;
    let mut resources = Vec::with_capacity(videos.len());
    let mut artifacts = Vec::with_capacity(videos.len());
    for (index, video) in videos.iter().enumerate() {
        let url = video
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| ProtocolError::invalid_response("GLM video result is missing url"))?;
        let resource = ResourceRef::url(url.to_owned(), Some("video/mp4".to_owned()));
        resources.push(resource.clone());
        artifacts.push(AiArtifact {
            name: format!("video-{index}"),
            resource,
            mime: Some("video/mp4".to_owned()),
            metadata: video.get("cover_image_url").cloned(),
        });
    }
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: json!({"videos": resources}),
        usage: Some(AiUsage::request_units(1)),
        artifacts,
    }))
}

fn resource_string(
    resource: &ResourceRef,
    context: &super::CodecContext,
) -> ProtocolResultValue<String> {
    match resource {
        ResourceRef::Url { url, .. } => Ok(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("GLM video image contains invalid base64")
            })?;
            Ok(format!("data:{mime};base64,{data_base64}"))
        }
        ResourceRef::NamedObject { .. } => {
            let materialized = context.materialized_resource(resource)?;
            Ok(format!(
                "data:{};base64,{}",
                materialized.mime,
                STANDARD.encode(&materialized.bytes)
            ))
        }
    }
}

fn require_parameter_subset(
    parameters: &std::collections::BTreeMap<String, Value>,
) -> ProtocolResultValue<()> {
    if let Some(name) = parameters
        .keys()
        .find(|name| name.as_str() != "provider_model_id")
    {
        return Err(ProtocolError::invalid_request(format!(
            "resolved GLM video parameter `{name}` is not supported"
        )));
    }
    Ok(())
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
        .ok_or_else(|| ProtocolError::invalid_request("GLM video task ID is invalid"))
}

fn required_string(value: &Value, field: &str) -> ProtocolResultValue<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ProtocolError::invalid_response(format!("GLM response is missing `{field}`"))
        })
}

fn ensure_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let parsed = serde_json::from_slice::<Value>(&response.body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message").or_else(|| value.get("msg")))
        .and_then(Value::as_str)
        .unwrap_or("GLM media request failed");
    Err(ProtocolError::new(
        super::protocol_error_kind_from_http_status(response.status),
        message,
    )
    .with_request_id(Some(response.request_id.clone()))
    .with_retry_after(response.retry_after))
}
