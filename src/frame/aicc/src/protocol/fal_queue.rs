use super::{
    AdapterDescriptor, AdapterStatus, CodecContext, CodecRegistration, CredentialKind,
    ExecutionMode, HttpBody, HttpRequest, HttpResponse, NativeTaskCodec, NativeTaskHandle,
    NativeTaskInput, NativeTaskOperation, NativeTaskOutput, NativeTaskState, OperationBinding,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolOutput, ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{AiArtifact, AiccCall, ApiType, ResourceRef};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const FAL_QUEUE_ADAPTER_ID: &str = "fal-queue";
pub(crate) const FAL_QUEUE_OPERATION_ID: &str = "queue.submit";

const DEFAULT_MAX_REQUEST_BYTES: usize = 100 * 1024 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const FAL_API_TYPES: [ApiType; 14] = [
    ApiType::ImageTextToImage,
    ApiType::ImageImageToImage,
    ApiType::ImageInpaint,
    ApiType::ImageUpscale,
    ApiType::ImageBackgroundRemove,
    ApiType::AudioTextToSpeech,
    ApiType::AudioSpeechRecognition,
    ApiType::AudioMusic,
    ApiType::AudioEnhance,
    ApiType::VideoTextToVideo,
    ApiType::VideoImageToVideo,
    ApiType::VideoToVideo,
    ApiType::VideoExtend,
    ApiType::VideoUpscale,
];

pub(crate) fn fal_queue_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let operation = OperationDescriptor {
        operation_id: FAL_QUEUE_OPERATION_ID.to_string(),
        bindings: FAL_API_TYPES
            .into_iter()
            .map(|api_type| OperationBinding::new(api_type, [ExecutionMode::NativeTask]))
            .collect(),
        supports_cancel: true,
        supports_webhook: false,
        max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
    };
    let descriptor = AdapterDescriptor {
        protocol_family_id: "fal".to_string(),
        protocol_adapter_id: FAL_QUEUE_ADAPTER_ID.to_string(),
        interface_generation: "queue-v1".to_string(),
        base_adapter_id: None,
        status: AdapterStatus::Stable,
        operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
    };
    let native_task_codecs = FAL_API_TYPES
        .into_iter()
        .map(|api_type| {
            Arc::new(FalQueueCodec {
                descriptor: operation.clone(),
                api_type,
            }) as Arc<dyn NativeTaskCodec>
        })
        .collect();
    (
        descriptor,
        CodecRegistration {
            operation_codecs: Vec::new(),
            native_task_codecs,
        },
    )
}

#[derive(Clone)]
struct FalQueueCodec {
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for FalQueueCodec {
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
            NativeTaskOperation::Status => lifecycle_request(input, Method::GET, Some("status")),
            NativeTaskOperation::Result => lifecycle_request(input, Method::GET, None),
            NativeTaskOperation::Cancel => lifecycle_request(input, Method::PUT, Some("cancel")),
        }
    }

    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        match operation {
            NativeTaskOperation::Submit => decode_submit(response),
            NativeTaskOperation::Status => decode_status(response),
            NativeTaskOperation::Result => decode_result(response, self.api_type),
            NativeTaskOperation::Cancel => decode_cancel(response),
        }
    }
}

fn encode_submit(
    input: &NativeTaskInput<'_>,
    api_type: ApiType,
) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("fal Queue submit requires canonical input")
    })?;
    if codec_input.canonical_request.method() != api_type.typed_method() {
        return Err(ProtocolError::invalid_request(
            "fal Queue codec received the wrong canonical request",
        ));
    }
    let model_id = provider_model_id(input.resolved_parameters)?;
    let mut body = canonical_body(&codec_input.canonical_request, input.context)?;
    for (name, value) in input.resolved_parameters {
        if name != "provider_model_id" {
            if value.is_null() {
                body.remove(name);
            } else {
                body.insert(name.clone(), value.clone());
            }
        }
    }
    let mut request = HttpRequest::new(
        Method::POST,
        endpoint(&input.context.base_url, &model_id, None, None)?,
    );
    request.body = HttpBody::Json(Value::Object(body));
    finish_request(&mut request, input.context)
}

fn lifecycle_request(
    input: &NativeTaskInput<'_>,
    method: Method,
    suffix: Option<&str>,
) -> ProtocolResultValue<HttpRequest> {
    let model_id = provider_model_id(input.resolved_parameters)?;
    let request_id = input.remote_task_id.ok_or_else(|| {
        ProtocolError::invalid_request("fal Queue lifecycle requires a request ID")
    })?;
    validate_request_id(request_id)?;
    let mut request = HttpRequest::new(
        method,
        endpoint(&input.context.base_url, &model_id, Some(request_id), suffix)?,
    );
    finish_request(&mut request, input.context)
}

fn request_value<T: Serialize>(request: &T) -> ProtocolResultValue<Value> {
    serde_json::to_value(request)
        .map_err(|_| ProtocolError::invalid_request("failed to encode canonical fal request"))
}

fn canonical_body(
    call: &AiccCall,
    context: &CodecContext,
) -> ProtocolResultValue<Map<String, Value>> {
    let value = match call {
        AiccCall::ImagesGenerate(request) => request_value(request)?,
        AiccCall::ImageToImage(request) => request_value(request)?,
        AiccCall::ImageInpaint(request) => request_value(request)?,
        AiccCall::ImageUpscale(request) => request_value(request)?,
        AiccCall::ImageBackgroundRemove(request) => request_value(request)?,
        AiccCall::AudioTextToSpeech(request) => request_value(request)?,
        AiccCall::AudioSpeechRecognition(request) => request_value(request)?,
        AiccCall::AudioMusic(request) => request_value(request)?,
        AiccCall::AudioEnhance(request) => request_value(request)?,
        AiccCall::VideoTextToVideo(request) => request_value(request)?,
        AiccCall::VideoImageToVideo(request) => request_value(request)?,
        AiccCall::VideoToVideo(request) => request_value(request)?,
        AiccCall::VideoExtend(request) => request_value(request)?,
        AiccCall::VideoUpscale(request) => request_value(request)?,
        _ => {
            return Err(ProtocolError::invalid_request(
                "fal Queue does not support this canonical request",
            ));
        }
    };
    let Value::Object(mut body) = value else {
        return Err(ProtocolError::invalid_request(
            "canonical fal request must encode as an object",
        ));
    };
    for name in ["exact_model", "idempotency_key", "task_options", "output"] {
        body.remove(name);
    }
    match call {
        AiccCall::ImagesGenerate(_) => rename(&mut body, "n", "num_images"),
        AiccCall::ImageToImage(request) => {
            body.remove("images");
            body.insert(
                "image_urls".to_string(),
                Value::Array(
                    request
                        .images
                        .iter()
                        .map(|resource| fal_resource(resource, context))
                        .collect::<ProtocolResultValue<Vec<_>>>()?,
                ),
            );
        }
        AiccCall::ImageInpaint(request) => {
            replace_resource(&mut body, "image", "image_url", &request.image, context)?;
            replace_resource(&mut body, "mask", "mask_url", &request.mask, context)?;
        }
        AiccCall::ImageUpscale(request) => {
            replace_resource(&mut body, "image", "image_url", &request.image, context)?;
        }
        AiccCall::ImageBackgroundRemove(request) => {
            replace_resource(&mut body, "image", "image_url", &request.image, context)?;
        }
        AiccCall::AudioSpeechRecognition(request) => {
            replace_resource(&mut body, "audio", "audio_url", &request.audio, context)?;
        }
        AiccCall::AudioEnhance(request) => {
            replace_resource(&mut body, "audio", "audio_url", &request.audio, context)?;
        }
        AiccCall::VideoImageToVideo(request) => {
            replace_resource(&mut body, "image", "image_url", &request.image, context)?;
        }
        AiccCall::VideoToVideo(request) => {
            replace_resource(&mut body, "video", "video_url", &request.video, context)?;
        }
        AiccCall::VideoExtend(request) => {
            replace_resource(&mut body, "video", "video_url", &request.video, context)?;
        }
        AiccCall::VideoUpscale(request) => {
            replace_resource(&mut body, "video", "video_url", &request.video, context)?;
        }
        _ => {}
    }
    body.retain(|_, value| !value.is_null());
    if body
        .get("prompt")
        .and_then(Value::as_str)
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
        return Err(ProtocolError::invalid_request(
            "fal prompt must not be empty",
        ));
    }
    Ok(body)
}

fn rename(body: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(value) = body.remove(from) {
        body.insert(to.to_string(), value);
    }
}

fn replace_resource(
    body: &mut Map<String, Value>,
    from: &str,
    to: &str,
    resource: &ResourceRef,
    context: &CodecContext,
) -> ProtocolResultValue<()> {
    body.remove(from);
    body.insert(to.to_string(), fal_resource(resource, context)?);
    Ok(())
}

fn fal_resource(resource: &ResourceRef, context: &CodecContext) -> ProtocolResultValue<Value> {
    let url = match resource {
        ResourceRef::Url { url, .. } => {
            let parsed = Url::parse(url)
                .map_err(|_| ProtocolError::invalid_request("fal resource URL is invalid"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(ProtocolError::invalid_request(
                    "fal resource URL must use HTTP or HTTPS",
                ));
            }
            url.clone()
        }
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("fal resource contains invalid base64")
            })?;
            format!("data:{mime};base64,{data_base64}")
        }
        ResourceRef::NamedObject { obj_id } => {
            let materialized = context.resources.get(&obj_id.to_string()).ok_or_else(|| {
                ProtocolError::invalid_request(format!(
                    "resource `{obj_id}` was not materialized before fal encoding"
                ))
            })?;
            format!(
                "data:{};base64,{}",
                materialized.mime,
                STANDARD.encode(&materialized.bytes)
            )
        }
    };
    Ok(Value::String(url))
}

fn decode_submit(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    ensure_success(&response)?;
    let retry_after = response.retry_after;
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let mut handle = NativeTaskHandle::new(required_string(&value, "request_id")?)?;
    handle.state = if value.get("queue_position").is_some() {
        NativeTaskState::Queued
    } else {
        NativeTaskState::Submitted
    };
    handle.poll_after = retry_after.or(Some(Duration::from_millis(500)));
    handle.cancel_supported = true;
    Ok(NativeTaskOutput::Submitted(handle))
}

fn decode_status(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    ensure_success(&response)?;
    let retry_after = response.retry_after;
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let state = match required_string(&value, "status")?.as_str() {
        "IN_QUEUE" => NativeTaskState::Queued,
        "IN_PROGRESS" => NativeTaskState::Running,
        "COMPLETED" if value.get("error").is_some_and(|value| !value.is_null()) => {
            NativeTaskState::Failed
        }
        "COMPLETED" => NativeTaskState::Succeeded,
        "CANCELLED" | "CANCELED" => NativeTaskState::Cancelled,
        _ => {
            return Err(ProtocolError::invalid_response(
                "fal Queue response contains an unknown status",
            ));
        }
    };
    Ok(NativeTaskOutput::Status { state, retry_after })
}

fn decode_result(
    response: HttpResponse,
    api_type: ApiType,
) -> ProtocolResultValue<NativeTaskOutput> {
    ensure_success(&response)?;
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    if value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(fal_payload_error(
            &value,
            ProtocolErrorKind::Transport,
            None,
            None,
        ));
    }
    if api_type == ApiType::AudioSpeechRecognition {
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ProtocolError::invalid_response("fal ASR result has no text"))?;
        return Ok(NativeTaskOutput::Result(ProtocolOutput {
            value: json!({
                "text": text,
                "segments": value.get("chunks").or_else(|| value.get("segments")).cloned().unwrap_or_else(|| json!([]))
            }),
            usage: None,
            artifacts: Vec::new(),
        }));
    }
    let (field, candidates) = media_candidates(&value, api_type)?;
    let resources = candidates
        .into_iter()
        .map(decode_media_resource)
        .collect::<ProtocolResultValue<Vec<_>>>()?;
    if resources.is_empty() {
        return Err(ProtocolError::invalid_response(
            "fal result contains an empty media list",
        ));
    }
    let normalized = Value::Object(Map::from_iter([(
        field.to_string(),
        if field == "images" {
            serde_json::to_value(&resources).map_err(|_| {
                ProtocolError::invalid_response("failed to normalize fal image resources")
            })?
        } else {
            serde_json::to_value(&resources[0]).map_err(|_| {
                ProtocolError::invalid_response("failed to normalize fal media resource")
            })?
        },
    )]));
    let artifacts = resources
        .into_iter()
        .enumerate()
        .map(|(index, resource)| AiArtifact {
            name: if field == "images" {
                format!("image_{}", index + 1)
            } else {
                field.to_string()
            },
            mime: match &resource {
                ResourceRef::Url { mime_hint, .. } => mime_hint.clone(),
                ResourceRef::Base64 { mime, .. } => Some(mime.clone()),
                ResourceRef::NamedObject { .. } => None,
            },
            resource,
            metadata: None,
        })
        .collect();
    Ok(NativeTaskOutput::Result(ProtocolOutput {
        value: normalized,
        usage: None,
        artifacts,
    }))
}

fn decode_cancel(response: HttpResponse) -> ProtocolResultValue<NativeTaskOutput> {
    let request_id = Some(response.request_id.clone());
    let retry_after = response.retry_after;
    let status_code = response.status;
    let value: Value = response.json(DEFAULT_MAX_RESPONSE_BYTES)?;
    let status = value.get("status").and_then(Value::as_str);
    let accepted = status_code == StatusCode::ACCEPTED && status == Some("CANCELLATION_REQUESTED");
    if accepted
        || (matches!(status, Some("ALREADY_COMPLETED" | "NOT_FOUND"))
            && matches!(status_code, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND))
    {
        return Ok(NativeTaskOutput::Cancelled { accepted });
    }
    Err(fal_payload_error(
        &value,
        ProtocolErrorKind::Transport,
        request_id,
        retry_after,
    ))
}

fn media_candidates(
    value: &Value,
    api_type: ApiType,
) -> ProtocolResultValue<(&'static str, Vec<&Value>)> {
    match api_type {
        ApiType::ImageTextToImage | ApiType::ImageImageToImage | ApiType::ImageInpaint => value
            .get("images")
            .and_then(Value::as_array)
            .map(|items| ("images", items.iter().collect()))
            .or_else(|| value.get("image").map(|item| ("images", vec![item])))
            .ok_or_else(|| ProtocolError::invalid_response("fal result has no image output")),
        ApiType::ImageUpscale | ApiType::ImageBackgroundRemove => single(value, "image", "image"),
        ApiType::AudioTextToSpeech | ApiType::AudioMusic | ApiType::AudioEnhance => {
            single(value, "audio", "audio")
        }
        ApiType::VideoTextToVideo
        | ApiType::VideoImageToVideo
        | ApiType::VideoToVideo
        | ApiType::VideoExtend
        | ApiType::VideoUpscale => single(value, "video", "video"),
        _ => Err(ProtocolError::invalid_response(
            "fal result API type is unsupported",
        )),
    }
}

fn single<'a>(
    value: &'a Value,
    field: &'static str,
    kind: &str,
) -> ProtocolResultValue<(&'static str, Vec<&'a Value>)> {
    value
        .get(field)
        .or_else(|| value.get(format!("{field}_url")))
        .or_else(|| value.get("output"))
        .map(|item| (field, vec![item]))
        .ok_or_else(|| ProtocolError::invalid_response(format!("fal result has no {kind} output")))
}

fn decode_media_resource(value: &Value) -> ProtocolResultValue<ResourceRef> {
    let (url, mime) = match value {
        Value::String(url) => (url.as_str(), None),
        Value::Object(object) => (
            object
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| ProtocolError::invalid_response("fal media output has no URL"))?,
            object
                .get("content_type")
                .or_else(|| object.get("mime_type"))
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        _ => {
            return Err(ProtocolError::invalid_response(
                "fal media output has an invalid shape",
            ));
        }
    };
    let parsed = Url::parse(url)
        .map_err(|_| ProtocolError::invalid_response("fal media output URL is invalid"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ProtocolError::invalid_response(
            "fal media output URL must use HTTP or HTTPS",
        ));
    }
    Ok(ResourceRef::url(url.to_string(), mime))
}

fn provider_model_id(parameters: &BTreeMap<String, Value>) -> ProtocolResultValue<String> {
    let model_id = parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved `provider_model_id`"))?;
    validate_model_id(model_id)?;
    Ok(model_id.to_string())
}

fn validate_model_id(model_id: &str) -> ProtocolResultValue<()> {
    let segments = model_id.split('/').collect::<Vec<_>>();
    if model_id.len() > 512
        || segments.len() < 2
        || segments.iter().any(|segment| {
            segment.is_empty()
                || *segment == "."
                || *segment == ".."
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(ProtocolError::invalid_request(
            "fal provider model ID contains invalid path segments",
        ));
    }
    Ok(())
}

fn validate_request_id(request_id: &str) -> ProtocolResultValue<()> {
    if request_id.is_empty()
        || request_id.len() > 256
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ProtocolError::invalid_request(
            "fal request ID contains invalid path characters",
        ));
    }
    Ok(())
}

fn endpoint(
    base_url: &str,
    model_id: &str,
    request_id: Option<&str>,
    suffix: Option<&str>,
) -> ProtocolResultValue<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProtocolError::invalid_configuration("fal base URL is invalid"))?;
    let base_path = url.path().trim_end_matches('/');
    let mut path = format!("{base_path}/{model_id}");
    if let Some(request_id) = request_id {
        path.push_str("/requests/");
        path.push_str(request_id);
    }
    if let Some(suffix) = suffix {
        path.push('/');
        path.push_str(suffix);
    }
    url.set_path(&path);
    Ok(url.to_string())
}

fn finish_request(
    request: &mut HttpRequest,
    context: &CodecContext,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let credential = context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "fal Queue requires a resolved Key credential",
        )
    })?;
    if credential.audit().kind != CredentialKind::FalKey {
        return Err(ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "fal Queue requires an Authorization Key credential",
        ));
    }
    credential.apply(&mut request.headers)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request.clone())
}

fn ensure_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    let kind = match response.status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ProtocolErrorKind::Timeout,
        _ => ProtocolErrorKind::Transport,
    };
    Err(fal_payload_error(
        &value,
        kind,
        Some(response.request_id.clone()),
        response.retry_after,
    ))
}

fn fal_payload_error(
    value: &Value,
    kind: ProtocolErrorKind,
    request_id: Option<String>,
    retry_after: Option<Duration>,
) -> ProtocolError {
    let message = value
        .get("detail")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or("fal Queue request failed");
    ProtocolError::new(kind, message)
        .with_request_id(request_id)
        .with_retry_after(retry_after)
}

fn required_string(value: &Value, name: &str) -> ProtocolResultValue<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid_response(format!("fal response is missing `{name}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecInput, CodecLimits, CodecRegistry, GoldenBody, MaterializedResource,
        ProtocolContractHarness, ResolvedCredential,
    };
    use buckyos_api::{TextToImageInvokeRequest, VideoImageToVideoRequest};
    use bytes::Bytes;
    use reqwest::header::HeaderMap;

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://queue.fal.run".to_string(),
            credential: Some(
                ResolvedCredential::fal_key("secret://fal/key", "top-secret").unwrap(),
            ),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
                max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            },
        }
    }

    fn input(call: AiccCall, model_id: &str) -> CodecInput {
        CodecInput {
            canonical_request: call,
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".to_string(),
                json!(model_id),
            )]),
        }
    }

    fn native_input<'a>(
        operation: NativeTaskOperation,
        remote_task_id: Option<&'a str>,
        codec_input: &'a CodecInput,
        context: &'a CodecContext,
    ) -> NativeTaskInput<'a> {
        NativeTaskInput {
            operation,
            remote_task_id,
            codec_input: Some(codec_input),
            resolved_parameters: &codec_input.resolved_parameters,
            context,
        }
    }

    fn response(status: StatusCode, value: Value) -> HttpResponse {
        HttpResponse {
            status,
            headers: HeaderMap::new(),
            body: Bytes::from(serde_json::to_vec(&value).unwrap()),
            request_id: "wire-request".to_string(),
            retry_after: None,
        }
    }

    fn codec(api_type: ApiType) -> Arc<dyn NativeTaskCodec> {
        let (descriptor, registration) = fal_queue_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        registry
            .native_task_codec(FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID, api_type)
            .unwrap()
    }

    #[test]
    fn registers_every_media_binding_as_native_task() {
        let (descriptor, registration) = fal_queue_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        for api_type in FAL_API_TYPES {
            registry
                .native_task_codec(FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID, api_type)
                .unwrap();
        }
    }

    #[test]
    fn submit_matches_official_queue_contract_and_redacts_key() {
        let mut request = TextToImageInvokeRequest::new("fal-ai/flux/schnell@fal-main", "a sunset");
        request.n = Some(2);
        request.seed = Some(42);
        let input = input(AiccCall::ImagesGenerate(request), "fal-ai/flux/schnell");
        let context = context();
        let request = codec(ApiType::ImageTextToImage)
            .encode_native(&native_input(
                NativeTaskOperation::Submit,
                None,
                &input,
                &context,
            ))
            .unwrap();
        let golden = ProtocolContractHarness::default()
            .request(&request)
            .unwrap();
        assert_eq!(golden.method, "POST");
        assert_eq!(golden.url, "https://queue.fal.run/fal-ai/flux/schnell");
        assert_eq!(golden.headers["authorization"], "[REDACTED]");
        assert_eq!(
            golden.body,
            GoldenBody::Json(json!({"prompt":"a sunset","num_images":2,"seed":42}))
        );
        assert!(!format!("{request:?}").contains("top-secret"));
    }

    #[test]
    fn lifecycle_url_and_path_validation_are_strict() {
        let input = input(
            AiccCall::ImagesGenerate(TextToImageInvokeRequest::new("fal-ai/flux@fal-main", "cat")),
            "fal-ai/flux",
        );
        let context = context();
        let codec = codec(ApiType::ImageTextToImage);
        let request = codec
            .encode_native(&native_input(
                NativeTaskOperation::Status,
                Some("request-1"),
                &input,
                &context,
            ))
            .unwrap();
        assert_eq!(
            request.url,
            "https://queue.fal.run/fal-ai/flux/requests/request-1/status"
        );
        let error = codec
            .encode_native(&native_input(
                NativeTaskOperation::Cancel,
                Some("../request-1"),
                &input,
                &context,
            ))
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidRequest);
    }

    #[test]
    fn materialized_resource_is_encoded_as_data_url_without_debug_leak() {
        let mut context = context();
        let obj_id = ndn_lib::ObjId::new("chunk:123456").unwrap();
        context.resources.insert(
            obj_id.to_string(),
            MaterializedResource::new(Bytes::from_static(b"image"), "image/png", None).unwrap(),
        );
        let input = input(
            AiccCall::VideoImageToVideo(VideoImageToVideoRequest::new(
                "fal-ai/video@fal-main",
                ResourceRef::named_object(obj_id),
                "animate".to_string(),
            )),
            "fal-ai/video",
        );
        let request = codec(ApiType::VideoImageToVideo)
            .encode_native(&native_input(
                NativeTaskOperation::Submit,
                None,
                &input,
                &context,
            ))
            .unwrap();
        let GoldenBody::Json(body) = ProtocolContractHarness::default()
            .request(&request)
            .unwrap()
            .body
        else {
            panic!("expected JSON")
        };
        assert_eq!(body["image_url"], "data:image/png;base64,aW1hZ2U=");
        assert!(!format!("{request:?}").contains("aW1hZ2U="));
    }

    #[tokio::test]
    async fn decodes_queue_lifecycle_result_cancel_and_errors() {
        let codec = codec(ApiType::ImageTextToImage);
        let NativeTaskOutput::Submitted(handle) = codec
            .decode_native(
                NativeTaskOperation::Submit,
                response(
                    StatusCode::OK,
                    json!({"request_id":"request-1","queue_position":0}),
                ),
            )
            .await
            .unwrap()
        else {
            panic!("expected submit")
        };
        assert_eq!(handle.state, NativeTaskState::Queued);

        let NativeTaskOutput::Status { state, .. } = codec
            .decode_native(
                NativeTaskOperation::Status,
                response(StatusCode::OK, json!({"status":"IN_PROGRESS"})),
            )
            .await
            .unwrap()
        else {
            panic!("expected status")
        };
        assert_eq!(state, NativeTaskState::Running);

        let NativeTaskOutput::Result(output) = codec
            .decode_native(
                NativeTaskOperation::Result,
                response(StatusCode::OK, json!({"images":[{"url":"https://fal.media/result.png","content_type":"image/png"}]})),
            )
            .await
            .unwrap()
        else {
            panic!("expected result")
        };
        assert_eq!(output.artifacts.len(), 1);
        assert_eq!(output.value["images"][0]["kind"], "url");

        let NativeTaskOutput::Cancelled { accepted } = codec
            .decode_native(
                NativeTaskOperation::Cancel,
                response(
                    StatusCode::ACCEPTED,
                    json!({"status":"CANCELLATION_REQUESTED"}),
                ),
            )
            .await
            .unwrap()
        else {
            panic!("expected cancel")
        };
        assert!(accepted);

        let error = codec
            .decode_native(
                NativeTaskOperation::Result,
                response(StatusCode::TOO_MANY_REQUESTS, json!({"detail":"slow down"})),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert_eq!(error.message, "slow down");
        assert_eq!(error.request_id.as_deref(), Some("wire-request"));
    }

    #[tokio::test]
    async fn maps_failed_cancelled_and_unknown_statuses() {
        let codec = codec(ApiType::VideoTextToVideo);
        for (wire, expected) in [
            (
                json!({"status":"COMPLETED","error":"failed"}),
                NativeTaskState::Failed,
            ),
            (json!({"status":"CANCELLED"}), NativeTaskState::Cancelled),
        ] {
            let NativeTaskOutput::Status { state, .. } = codec
                .decode_native(NativeTaskOperation::Status, response(StatusCode::OK, wire))
                .await
                .unwrap()
            else {
                panic!("expected status")
            };
            assert_eq!(state, expected);
        }
        let error = codec
            .decode_native(
                NativeTaskOperation::Status,
                response(StatusCode::OK, json!({"status":"WAITING"})),
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidResponse);
    }

    #[tokio::test]
    async fn normalizes_asr_text_without_inventing_a_media_artifact() {
        let output = codec(ApiType::AudioSpeechRecognition)
            .decode_native(
                NativeTaskOperation::Result,
                response(
                    StatusCode::OK,
                    json!({"text":"hello","chunks":[{"text":"hello","timestamp":[0.0,1.0]}]}),
                ),
            )
            .await
            .unwrap();
        let NativeTaskOutput::Result(output) = output else {
            panic!("expected result")
        };
        assert_eq!(output.value["text"], "hello");
        assert_eq!(output.value["segments"].as_array().unwrap().len(), 1);
        assert!(output.artifacts.is_empty());
    }

    #[test]
    fn rejects_wrong_credential_and_model_path() {
        let request =
            AiccCall::ImagesGenerate(TextToImageInvokeRequest::new("fal-ai/flux@fal-main", "cat"));
        let bad_path = input(request.clone(), "fal-ai/../admin");
        let mut context = context();
        let error = codec(ApiType::ImageTextToImage)
            .encode_native(&native_input(
                NativeTaskOperation::Submit,
                None,
                &bad_path,
                &context,
            ))
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidRequest);

        context.credential = Some(ResolvedCredential::bearer("secret://wrong", "secret").unwrap());
        let valid = input(request, "fal-ai/flux");
        let error = codec(ApiType::ImageTextToImage)
            .encode_native(&native_input(
                NativeTaskOperation::Submit,
                None,
                &valid,
                &context,
            ))
            .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Authentication);
    }
}
