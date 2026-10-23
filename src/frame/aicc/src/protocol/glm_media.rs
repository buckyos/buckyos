use super::video::{
    generated_video, planned_resource, require_video_parameters, unsupported, video_generate_input,
    video_output, whole_seconds,
};
use super::{
    openai_responses_adapter, AdapterDescriptor, AdapterStatus, CodecRegistration, HttpBody,
    HttpRequest, HttpResponse, NativeTaskCodec, NativeTaskHandle, NativeTaskInput,
    NativeTaskOperation, NativeTaskOutput, NativeTaskState, OperationBinding, OperationDescriptor,
    ProtocolError, ProtocolErrorKind, ProtocolResultValue, OPENAI_AUDIO_SPEECH_OPERATION_ID,
    OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID, OPENAI_EMBEDDINGS_OPERATION_ID,
    OPENAI_IMAGES_GENERATE_OPERATION_ID,
};
use crate::video::VideoScenario;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiUsage, ApiType, KeyframePosition, ResourceRef, VideoConditionKind, VideoMetadata,
};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

const GLM_VIDEOS_OPERATION_ID: &str = "videos.generate";
pub(crate) const GLM_MEDIA_ADAPTER_ID: &str = "glm-media";
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
        bindings: vec![OperationBinding::new(
            ApiType::VideoGenerate,
            [super::ExecutionMode::NativeTask],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    registration
        .native_task_codecs
        .push(Arc::new(GlmVideoCodec {
            descriptor: video.clone(),
        }) as Arc<dyn NativeTaskCodec>);
    operations.push(video);
    (operations, registration)
}

pub(crate) fn glm_media_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (operations, registration) = glm_media_registration();
    (
        AdapterDescriptor {
            protocol_family_id: "glm".to_owned(),
            protocol_adapter_id: GLM_MEDIA_ADAPTER_ID.to_owned(),
            interface_generation: "v4".to_owned(),
            base_adapter_id: None,
            component_adapter_ids: Vec::new(),
            status: AdapterStatus::Stable,
            probe_priority: 200,
            probe_path: None,
            credential: super::AdapterCredentialContract {
                kind: super::CredentialKind::GlmJwt,
                header_name: None,
            },
            operations: operations
                .into_iter()
                .map(|operation| (operation.operation_id.clone(), operation))
                .collect(),
        },
        registration,
    )
}

#[derive(Clone)]
struct GlmVideoCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl NativeTaskCodec for GlmVideoCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // `resource_string` hands a URL straight through; inlining the payload
        // is only the fallback for bytes or a local object.
        crate::resource::ResourceInputForm::UrlOrBytes
    }

    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::VideoGenerate
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
            NativeTaskOperation::Submit => encode_submit(input),
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
                    retry_after: retry_after.or(Some(Duration::from_secs(2))),
                    result_ref: None,
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

fn encode_submit(input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input
        .codec_input
        .ok_or_else(|| ProtocolError::invalid_request("GLM video submit requires input"))?;
    require_video_parameters(input.resolved_parameters, "GLM", &[])?;
    let model = input
        .resolved_parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved provider_model_id"))?;
    let (request, plan) = video_generate_input(codec_input)?;
    if plan.negative_prompt.is_some() || plan.seed.is_some() || plan.resolution.is_some() {
        return Err(unsupported(
            "GLM video generation has no negative prompt, seed or resolution tier",
        ));
    }
    let mut body = Map::from_iter([("model".to_owned(), json!(model))]);
    if let Some(prompt) = &plan.submitted_prompt {
        body.insert("prompt".to_owned(), json!(prompt));
    }
    let mut images = Vec::new();
    for planned in &plan.inputs {
        if !matches!(
            planned.kind,
            VideoConditionKind::Keyframe | VideoConditionKind::ReferenceImage
        ) {
            return Err(unsupported(
                "GLM video generation takes keyframes or reference images only",
            ));
        }
        images.push((
            planned.position,
            resource_string(planned_resource(request, planned)?, input.context)?,
        ));
    }
    // A start/end pair goes in order; a lone start frame or reference set is
    // sent as the protocol's single URL or URL list.
    images.sort_by_key(|(position, _)| *position != Some(KeyframePosition::Start));
    match images.as_slice() {
        [] => {}
        [(_, url)] if plan.scenario != VideoScenario::References => {
            body.insert("image_url".to_owned(), json!(url));
        }
        _ => {
            body.insert(
                "image_url".to_owned(),
                json!(images.iter().map(|(_, url)| url).collect::<Vec<_>>()),
            );
        }
    }
    if let Some(seconds) = whole_seconds(&plan)? {
        body.insert("duration".to_owned(), json!(seconds));
    }
    if let Some(size) = plan.size {
        body.insert(
            "size".to_owned(),
            json!(format!("{}x{}", size.width, size.height)),
        );
    }
    if let Some(fps) = plan.fps {
        if fps.fract() != 0.0 {
            return Err(unsupported("GLM video fps must be a whole number"));
        }
        body.insert("fps".to_owned(), json!(fps as u64));
    }
    if let Some(audio) = plan.generate_audio {
        body.insert("with_audio".to_owned(), json!(audio));
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
    let mut generated = Vec::with_capacity(videos.len());
    for video in videos {
        let url = video
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| ProtocolError::invalid_response("GLM video result is missing url"))?;
        generated.push(generated_video(
            ResourceRef::url(url.to_owned(), Some("video/mp4".to_owned())),
            Some("video/mp4".to_owned()),
            VideoMetadata::default(),
        ));
    }
    Ok(NativeTaskOutput::Result(video_output(
        generated,
        AiUsage::request_units(1),
    )?))
}

fn resource_string(
    resource: &ResourceRef,
    context: &super::CodecContext,
) -> ProtocolResultValue<String> {
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
    let provider_code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        });
    let message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message").or_else(|| value.get("msg")))
        .and_then(Value::as_str)
        .unwrap_or("GLM media request failed");
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
    fn error_response_keeps_glm_business_code() {
        let response = HttpResponse {
            status: StatusCode::BAD_REQUEST,
            headers: HeaderMap::new(),
            body: Bytes::from_static(
                br#"{"error":{"code":"1210","message":"API call parameters are incorrect"}}"#,
            ),
            request_id: "glm-request-1".to_owned(),
            retry_after: None,
        };

        let error = ensure_success(&response).unwrap_err();
        assert_eq!(error.provider_code.as_deref(), Some("1210"));
        assert_eq!(error.http_status, Some(400));
        assert_eq!(error.request_id.as_deref(), Some("glm-request-1"));
    }
}
