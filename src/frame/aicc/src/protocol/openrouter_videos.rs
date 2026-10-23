//! OpenRouter video generation (`/api/v1/videos`): submit, poll, and the
//! authenticated content download.
//!
//! The channel has no cancel endpoint, so tasks are not cancellable. A job
//! reports its total charge as `usage.cost` once it completes; the status
//! poll hands it on as `reported_cost`.

use super::video::{
    generated_video, planned_resource, require_video_parameters, unsupported, video_generate_input,
    video_output, whole_seconds,
};
use super::{
    CodecContext, ExecutionMode, HttpBody, HttpRequest, HttpResponse, NativeTaskCodec,
    NativeTaskHandle, NativeTaskInput, NativeTaskOperation, NativeTaskOutput, NativeTaskState,
    OperationBinding, OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiUsage, ApiType, KeyframePosition, ResourceRef, VideoConditionKind, VideoMetadata,
};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub(crate) const OPENROUTER_VIDEOS_OPERATION_ID: &str = "videos.create";
const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_secs(10);

pub(super) fn descriptor() -> OperationDescriptor {
    OperationDescriptor {
        operation_id: OPENROUTER_VIDEOS_OPERATION_ID.to_owned(),
        bindings: vec![OperationBinding::new(
            ApiType::VideoGenerate,
            [ExecutionMode::NativeTask],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    }
}

#[derive(Clone)]
pub(super) struct OpenRouterVideoCodec {
    pub descriptor: OperationDescriptor,
}

#[async_trait]
impl NativeTaskCodec for OpenRouterVideoCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // Images go out as URLs when they have one, otherwise as data URLs.
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

    /// OpenRouter reports the charge but not the billed seconds; they are the
    /// submitted `duration` (unknown when the channel default was used).
    fn output_video_seconds(&self, request: &HttpRequest) -> Option<u64> {
        let HttpBody::Json(body) = &request.body else {
            return None;
        };
        body.get("duration").and_then(Value::as_u64)
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        match input.operation {
            NativeTaskOperation::Submit => encode_submit(input),
            NativeTaskOperation::Status => {
                let id = job_id(input.remote_task_id)?;
                http_request(input.context, Method::GET, &format!("videos/{id}"), None)
            }
            NativeTaskOperation::Result => {
                let id = job_id(input.remote_task_id)?;
                http_request(
                    input.context,
                    Method::GET,
                    &format!("videos/{id}/content?index=0"),
                    None,
                )
            }
            NativeTaskOperation::Cancel => Err(unsupported(
                "OpenRouter video generation has no cancel endpoint",
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
            NativeTaskOperation::Submit => {
                let value: Value = response.json(MAX_RESPONSE_BYTES)?;
                let mut handle = NativeTaskHandle::new(required_string(&value, "id")?)?;
                handle.state = decode_state(&value)?;
                handle.poll_after = response.retry_after.or(Some(POLL_INTERVAL));
                handle.cancel_supported = false;
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let value: Value = response.json(MAX_RESPONSE_BYTES)?;
                let state = decode_state(&value)?;
                if state == NativeTaskState::Failed {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::ProviderRejected,
                        format!(
                            "OpenRouter video job {}: {}",
                            value
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("failed"),
                            value
                                .get("error")
                                .and_then(Value::as_str)
                                .filter(|error| !error.trim().is_empty())
                                .unwrap_or("no reason given")
                        ),
                    )
                    .with_request_id(Some(response.request_id.clone())));
                }
                let mut result_usage = None;
                if state == NativeTaskState::Succeeded {
                    let outputs = value
                        .get("unsigned_urls")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or_default();
                    if outputs != 1 {
                        return Err(ProtocolError::invalid_response(format!(
                            "OpenRouter video job finished with {outputs} outputs; exactly one is expected"
                        )));
                    }
                    result_usage = Some(AiUsage {
                        reported_cost: value
                            .pointer("/usage/cost")
                            .and_then(Value::as_f64)
                            .filter(|cost| cost.is_finite() && *cost >= 0.0),
                        ..Default::default()
                    });
                }
                Ok(NativeTaskOutput::Status {
                    state,
                    retry_after: response.retry_after.or(Some(POLL_INTERVAL)),
                    result_ref: None,
                    result_usage,
                    result_artifacts: BTreeMap::new(),
                })
            }
            NativeTaskOperation::Result => {
                if response.body.is_empty() {
                    return Err(ProtocolError::invalid_response(
                        "OpenRouter video content is empty",
                    ));
                }
                let media_type = response
                    .headers
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.split(';').next())
                    .map(str::trim)
                    .filter(|value| value.starts_with("video/"))
                    .unwrap_or("video/mp4")
                    .to_owned();
                let video = generated_video(
                    ResourceRef::base64(media_type.clone(), STANDARD.encode(&response.body)),
                    Some(media_type),
                    VideoMetadata::default(),
                );
                Ok(NativeTaskOutput::Result(video_output(
                    vec![video],
                    AiUsage::request_units(1),
                )?))
            }
            NativeTaskOperation::Cancel => Err(unsupported(
                "OpenRouter video generation has no cancel endpoint",
            )),
        }
    }
}

fn encode_submit(input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
    let codec_input = input.codec_input.ok_or_else(|| {
        ProtocolError::invalid_request("OpenRouter video submit requires canonical input")
    })?;
    require_video_parameters(input.resolved_parameters, "OpenRouter", &[])?;
    let model = input
        .resolved_parameters
        .get("provider_model_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProtocolError::invalid_request("missing resolved provider_model_id"))?;
    let (request, plan) = video_generate_input(codec_input)?;
    if plan.negative_prompt.is_some() || plan.fps.is_some() {
        return Err(unsupported(
            "OpenRouter video generation has no negative prompt or fps option",
        ));
    }
    let mut body = Map::from_iter([("model".to_owned(), json!(model))]);
    if let Some(prompt) = plan
        .submitted_prompt
        .as_ref()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        body.insert("prompt".to_owned(), json!(prompt));
    }
    let mut frames = Vec::new();
    let mut references = Vec::new();
    for planned in &plan.inputs {
        let url = image_url(planned_resource(request, planned)?, input.context)?;
        match (planned.kind, planned.position) {
            (VideoConditionKind::Keyframe, position) => frames.push(json!({
                "type": "image_url",
                "image_url": {"url": url},
                "frame_type": if position == Some(KeyframePosition::End) {
                    "last_frame"
                } else {
                    "first_frame"
                },
            })),
            (VideoConditionKind::ReferenceImage, _) => references.push(json!({
                "type": "image_url",
                "image_url": {"url": url},
            })),
            _ => {
                return Err(unsupported(
                    "OpenRouter honors reference video and audio only for some providers; this channel takes keyframes and reference images",
                ));
            }
        }
    }
    // With both present OpenRouter treats the job as image-to-video and
    // ignores the references, so the pair is never sent together.
    if !frames.is_empty() && !references.is_empty() {
        return Err(unsupported(
            "OpenRouter cannot combine frame images with reference images",
        ));
    }
    if !frames.is_empty() {
        body.insert("frame_images".to_owned(), Value::Array(frames));
    }
    if !references.is_empty() {
        body.insert("input_references".to_owned(), Value::Array(references));
    }
    if let Some(seconds) = whole_seconds(&plan)? {
        body.insert("duration".to_owned(), json!(seconds));
    }
    if let Some(size) = plan.size {
        body.insert(
            "size".to_owned(),
            json!(format!("{}x{}", size.width, size.height)),
        );
    } else if let Some(resolution) = &plan.resolution {
        body.insert("resolution".to_owned(), json!(resolution));
    }
    if let Some(ratio) = plan
        .aspect_ratio
        .as_ref()
        .filter(|_| !plan.aspect_ratio_from_input)
    {
        body.insert("aspect_ratio".to_owned(), json!(ratio));
    }
    if let Some(generate_audio) = plan.generate_audio {
        body.insert("generate_audio".to_owned(), json!(generate_audio));
    }
    if let Some(seed) = plan.seed {
        body.insert("seed".to_owned(), json!(seed));
    }
    http_request(
        input.context,
        Method::POST,
        "videos",
        Some(Value::Object(body)),
    )
}

fn image_url(resource: &ResourceRef, context: &CodecContext) -> ProtocolResultValue<String> {
    if let Some(url) = context.materialized_url(resource) {
        return Ok(url.to_owned());
    }
    match resource {
        ResourceRef::Url { url, .. } => Ok(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            STANDARD.decode(data_base64).map_err(|_| {
                ProtocolError::invalid_request("OpenRouter image contains invalid base64")
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

fn http_request(
    context: &CodecContext,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> ProtocolResultValue<HttpRequest> {
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("OpenRouter base URL is invalid"))?;
    let (path, query) = path
        .split_once('?')
        .map_or((path, None), |(path, query)| (path, Some(query)));
    let base = url.path().trim_end_matches('/').to_owned();
    url.set_path(&format!("{base}/{path}"));
    url.set_query(query);
    let mut request = HttpRequest::new(method, url.to_string());
    if let Some(body) = body {
        request
            .headers
            .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        request.body = HttpBody::Json(body);
    }
    context.finalize_request(
        &mut request,
        super::CredentialKind::Bearer,
        "OpenRouter video generation requires a credential",
        "OpenRouter video generation requires a Bearer credential",
    )
}

fn job_id(value: Option<&str>) -> ProtocolResultValue<&str> {
    value
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 256
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .ok_or_else(|| ProtocolError::invalid_request("OpenRouter video job ID is invalid"))
}

fn decode_state(value: &Value) -> ProtocolResultValue<NativeTaskState> {
    match value.get("status").and_then(Value::as_str) {
        Some("pending") => Ok(NativeTaskState::Queued),
        Some("in_progress") => Ok(NativeTaskState::Running),
        Some("completed") => Ok(NativeTaskState::Succeeded),
        Some("failed" | "expired") => Ok(NativeTaskState::Failed),
        Some("cancelled") => Ok(NativeTaskState::Cancelled),
        _ => Err(ProtocolError::invalid_response(
            "OpenRouter video job status is unknown",
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
            ProtocolError::invalid_response(format!(
                "OpenRouter video response is missing `{field}`"
            ))
        })
}

fn ensure_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let parsed = serde_json::from_slice::<Value>(&response.body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("OpenRouter video request failed");
    let code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .map(|code| match code {
            Value::String(code) => code.clone(),
            code => code.to_string(),
        });
    Err(ProtocolError::new(
        super::protocol_error_kind_from_http_status(response.status),
        message,
    )
    .with_provider_code(code)
    .with_http_status(response.status.as_u16())
    .with_request_id(Some(response.request_id.clone()))
    .with_retry_after(response.retry_after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CodecInput, CodecLimits, ResolvedCredential};
    use crate::video::test_support::{openrouter_contract as channel_contract, plan_parameters};
    use crate::video::VideoMaterialFacts;
    use buckyos_api::{
        AiccCall, ProviderStateCoordinate, VideoCondition, VideoGenerateRequest, VideoOutputSpec,
    };
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, HeaderValue};
    use reqwest::StatusCode;

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://openrouter.ai/api/v1".to_owned(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "openrouter".to_owned(),
                adapter_type: "openrouter-responses".to_owned(),
                origin_provider: "minimax".to_owned(),
                origin_model: "MiniMax-H3".to_owned(),
            },
            credential: Some(ResolvedCredential::bearer("secret://openrouter", "sk-or").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(30),
                max_request_bytes: MAX_REQUEST_BYTES,
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        }
    }

    fn response(status: StatusCode, content_type: &str, body: Bytes) -> HttpResponse {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
        HttpResponse {
            status,
            headers,
            body,
            request_id: "request-1".to_owned(),
            retry_after: None,
        }
    }

    fn json_response(value: Value) -> HttpResponse {
        response(
            StatusCode::OK,
            "application/json",
            Bytes::from(serde_json::to_vec(&value).unwrap()),
        )
    }

    fn submit(model: &str, request: VideoGenerateRequest) -> ProtocolResultValue<HttpRequest> {
        let contract = channel_contract(model);
        let parameters = plan_parameters(&contract, &request, &VideoMaterialFacts::new(), model);
        let codec = OpenRouterVideoCodec {
            descriptor: descriptor(),
        };
        let input = CodecInput {
            canonical_request: AiccCall::VideoGenerate(request),
            resolved_parameters: parameters.clone(),
        };
        let context = context();
        codec.encode_native(&NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            codec_input: Some(&input),
            resolved_parameters: &parameters,
            context: &context,
        })
    }

    fn image(name: &str) -> ResourceRef {
        ResourceRef::url(
            format!("https://media.example/{name}"),
            Some("image/png".into()),
        )
    }

    #[test]
    fn submit_maps_frames_references_and_output() {
        let mut frames = VideoGenerateRequest::new(
            "minimax/hailuo-3@openrouter-main",
            Some("open on @{a}, close on @{b}".into()),
        );
        frames.conditions = vec![
            VideoCondition::Keyframe {
                id: "a".into(),
                resource: image("a.png"),
                position: KeyframePosition::Start,
            },
            VideoCondition::Keyframe {
                id: "b".into(),
                resource: image("b.png"),
                position: KeyframePosition::End,
            },
        ];
        frames.output = Some(VideoOutputSpec {
            duration_seconds: Some(6.0),
            ..Default::default()
        });
        let wire = submit("minimax/hailuo-3", frames).unwrap();
        assert_eq!(wire.url, "https://openrouter.ai/api/v1/videos");
        assert_eq!(wire.headers["authorization"], "Bearer sk-or");
        // OpenRouter reports only the charge; the billed seconds are the submitted ones.
        let codec = OpenRouterVideoCodec {
            descriptor: descriptor(),
        };
        assert_eq!(codec.output_video_seconds(&wire), Some(6));
        let HttpBody::Json(body) = wire.body else {
            panic!("expected JSON")
        };
        assert_eq!(body["model"], "minimax/hailuo-3");
        assert_eq!(
            body["prompt"],
            "open on the first frame, close on the last frame"
        );
        assert_eq!(body["frame_images"][0]["frame_type"], "first_frame");
        assert_eq!(body["frame_images"][1]["frame_type"], "last_frame");
        assert_eq!(body["duration"], 6);
        assert!(body.get("input_references").is_none());
        assert!(body.get("aspect_ratio").is_none());

        let mut references = VideoGenerateRequest::new(
            "minimax/hailuo-3@openrouter-main",
            Some("@{hero} on a beach".into()),
        );
        references.conditions = vec![VideoCondition::ReferenceImage {
            id: "hero".into(),
            resource: image("hero.png"),
        }];
        references.output = Some(VideoOutputSpec {
            aspect_ratio: Some("9:16".into()),
            ..Default::default()
        });
        let HttpBody::Json(body) = submit("minimax/hailuo-3", references).unwrap().body else {
            panic!("expected JSON")
        };
        assert_eq!(body["prompt"], "reference image 1 on a beach");
        assert_eq!(
            body["input_references"][0]["image_url"]["url"],
            "https://media.example/hero.png"
        );
        assert_eq!(body["aspect_ratio"], "9:16");
    }

    #[test]
    fn channel_contract_rejects_what_openrouter_would_drop() {
        let contract = channel_contract("minimax/hailuo-3");
        let plan = |request: &VideoGenerateRequest| {
            crate::video::plan_video(&contract, request, &VideoMaterialFacts::new())
        };
        let mut mixed = VideoGenerateRequest::new("m@p", Some("a".into()));
        mixed.conditions = vec![
            VideoCondition::Keyframe {
                id: "a".into(),
                resource: image("a.png"),
                position: KeyframePosition::Start,
            },
            VideoCondition::ReferenceImage {
                id: "b".into(),
                resource: image("b.png"),
            },
        ];
        assert!(plan(&mixed).is_err());
        let mut video = VideoGenerateRequest::new("m@p", Some("a".into()));
        video.conditions = vec![VideoCondition::ReferenceVideo {
            id: "v".into(),
            resource: image("v.mp4"),
            time_range: None,
            include_audio: false,
        }];
        assert!(plan(&video).is_err());
        let mut seed = VideoGenerateRequest::new("m@p", Some("a".into()));
        seed.seed = Some(1);
        assert!(plan(&seed).is_err());

        let max = channel_contract("minimax/hailuo-3-max");
        let mut small = VideoGenerateRequest::new("m@p", Some("a".into()));
        small.output = Some(VideoOutputSpec {
            size: Some(buckyos_api::VideoSize::short_edge(480)),
            ..Default::default()
        });
        let plan_max = crate::video::plan_video(&max, &small, &VideoMaterialFacts::new()).unwrap();
        assert_eq!(plan_max.resolution.as_deref(), Some("480p"));
        // generate_audio=false yet the returned file carries an audio track:
        // neither silence nor sound is promised.
        let mut silent = small.clone();
        silent.output.as_mut().unwrap().audio = Some(buckyos_api::VideoAudioMode::None);
        assert!(crate::video::plan_video(&max, &silent, &VideoMaterialFacts::new()).is_err());
        let mut four = VideoGenerateRequest::new("m@p", Some("a".into()));
        four.output = Some(VideoOutputSpec {
            duration_seconds: Some(4.0),
            ..Default::default()
        });
        assert!(crate::video::plan_video(&max, &four, &VideoMaterialFacts::new()).is_err());
        let mut reference = VideoGenerateRequest::new("m@p", Some("a".into()));
        reference.conditions = vec![VideoCondition::ReferenceImage {
            id: "b".into(),
            resource: image("b.png"),
        }];
        assert!(crate::video::plan_video(&max, &reference, &VideoMaterialFacts::new()).is_err());
    }

    #[tokio::test]
    async fn lifecycle_decodes_cost_and_downloads_content_with_the_key() {
        let codec = OpenRouterVideoCodec {
            descriptor: descriptor(),
        };
        let NativeTaskOutput::Submitted(handle) = codec
            .decode_native(
                NativeTaskOperation::Submit,
                response(
                    StatusCode::ACCEPTED,
                    "application/json",
                    Bytes::from_static(br#"{"id":"gen-vid-1-abc","polling_url":"https://openrouter.ai/api/v1/videos/gen-vid-1-abc","status":"pending"}"#),
                ),
            )
            .await
            .unwrap()
        else {
            panic!("expected handle")
        };
        assert_eq!(handle.remote_task_id, "gen-vid-1-abc");
        assert!(!handle.cancel_supported);

        let parameters = BTreeMap::new();
        let context = context();
        let lifecycle = |operation| NativeTaskInput {
            operation,
            remote_task_id: Some("gen-vid-1-abc"),
            codec_input: None,
            resolved_parameters: &parameters,
            context: &context,
        };
        let status = codec
            .encode_native(&lifecycle(NativeTaskOperation::Status))
            .unwrap();
        assert_eq!(
            status.url,
            "https://openrouter.ai/api/v1/videos/gen-vid-1-abc"
        );
        let content = codec
            .encode_native(&lifecycle(NativeTaskOperation::Result))
            .unwrap();
        assert_eq!(
            content.url,
            "https://openrouter.ai/api/v1/videos/gen-vid-1-abc/content?index=0"
        );
        assert_eq!(content.headers["authorization"], "Bearer sk-or");
        assert!(codec
            .encode_native(&lifecycle(NativeTaskOperation::Cancel))
            .is_err());

        let NativeTaskOutput::Status {
            state,
            result_usage,
            ..
        } = codec
            .decode_native(
                NativeTaskOperation::Status,
                json_response(json!({
                    "id": "gen-vid-1-abc", "status": "completed",
                    "unsigned_urls": ["https://openrouter.ai/api/v1/videos/gen-vid-1-abc/content?index=0"],
                    "usage": {"cost": 0.65, "is_byok": false}
                })),
            )
            .await
            .unwrap()
        else {
            panic!("expected status")
        };
        assert_eq!(state, NativeTaskState::Succeeded);
        assert_eq!(result_usage.unwrap().reported_cost, Some(0.65));

        assert!(codec
            .decode_native(
                NativeTaskOperation::Status,
                json_response(
                    json!({"id":"gen-vid-1-abc","status":"completed","unsigned_urls":["a","b"]})
                ),
            )
            .await
            .is_err());
        let failed = codec
            .decode_native(
                NativeTaskOperation::Status,
                json_response(
                    json!({"id":"gen-vid-1-abc","status":"failed","error":"content policy"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(failed.kind, ProtocolErrorKind::ProviderRejected);
        assert!(failed.message.contains("content policy"));

        let NativeTaskOutput::Result(output) = codec
            .decode_native(
                NativeTaskOperation::Result,
                response(
                    StatusCode::OK,
                    "video/mp4",
                    Bytes::from_static(b"mp4-bytes"),
                ),
            )
            .await
            .unwrap()
        else {
            panic!("expected result")
        };
        assert_eq!(output.value["videos"][0]["resource"]["kind"], "base64");
        assert_eq!(output.value["videos"][0]["media_type"], "video/mp4");

        let unauthorized = codec
            .decode_native(
                NativeTaskOperation::Status,
                response(
                    StatusCode::UNAUTHORIZED,
                    "application/json",
                    Bytes::from_static(
                        br#"{"error":{"code":401,"message":"No auth credentials found"}}"#,
                    ),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(unauthorized.kind, ProtocolErrorKind::Authentication);
        assert_eq!(unauthorized.provider_code.as_deref(), Some("401"));
    }
}
