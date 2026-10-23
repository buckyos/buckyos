use super::{
    openai_responses_adapter, AdapterCredentialContract, AdapterDescriptor, AdapterStatus,
    CodecCall, CodecRegistration, CodecRegistry, CredentialKind, ExecutionMode, HttpBody,
    HttpRequest, HttpResponse, NativeTaskCodec, NativeTaskInput, NativeTaskOperation,
    NativeTaskOutput, OperationBinding, OperationCodec, OperationDescriptor, ProtocolError,
    ProtocolExecution, ProtocolResultValue, OPENAI_RESPONSES_ADAPTER_ID,
};

use async_trait::async_trait;
use base64::Engine;
use buckyos_api::{AiccCall, ApiType};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(crate) const SN_OPENAI_ADAPTER_ID: &str = "sn-openai";

pub(crate) fn sn_openai_adapter() -> ProtocolResultValue<AdapterDescriptor> {
    let (mut base, _) = openai_responses_adapter();
    let (background, _) = super::openai_responses::background_responses_registration();
    base.operations
        .insert(background.operation_id.clone(), background);
    let (audio, _) = sn_audio_registration();
    base.operations.insert(audio.operation_id.clone(), audio);
    if let Some(video) = base
        .operations
        .get_mut(super::openai_responses::OPENAI_VIDEOS_OPERATION_ID)
    {
        video.supports_cancel = false;
        video.bindings.push(OperationBinding::new(
            ApiType::VideoExtend,
            [ExecutionMode::NativeTask],
        ));
    }
    Ok(AdapterDescriptor {
        protocol_family_id: base.protocol_family_id,
        protocol_adapter_id: SN_OPENAI_ADAPTER_ID.to_owned(),
        interface_generation: base.interface_generation,
        base_adapter_id: Some(OPENAI_RESPONSES_ADAPTER_ID.to_owned()),
        component_adapter_ids: Vec::new(),
        status: AdapterStatus::Stable,
        probe_priority: 200,
        probe_path: None,
        credential: AdapterCredentialContract::bearer(),
        operations: base.operations,
    })
}

pub(crate) fn register_sn_openai_adapter(registry: &mut CodecRegistry) -> ProtocolResultValue<()> {
    let (_, mut native_task_codecs) = super::openai_responses::background_responses_registration();
    let (_, base_codecs) = openai_responses_adapter();
    let operation_codecs: Vec<Arc<dyn OperationCodec>> = vec![sn_audio_registration().1];
    native_task_codecs.extend(
        base_codecs
            .native_task_codecs
            .into_iter()
            .filter(|codec| {
                codec.descriptor().operation_id
                    == super::openai_responses::OPENAI_VIDEOS_OPERATION_ID
            })
            .map(|inner| {
                let mut descriptor = inner.descriptor().clone();
                descriptor.supports_cancel = false;
                descriptor.bindings.push(OperationBinding::new(
                    ApiType::VideoExtend,
                    [ExecutionMode::NativeTask],
                ));
                let api_type = inner.api_type();
                Arc::new(SnVideoCodec {
                    inner,
                    descriptor,
                    api_type,
                }) as Arc<dyn NativeTaskCodec>
            }),
    );
    let extension_inner = native_task_codecs
        .iter()
        .find(|codec| codec.api_type() == ApiType::VideoEdit)
        .unwrap()
        .clone();
    native_task_codecs.push(Arc::new(SnVideoCodec {
        descriptor: extension_inner.descriptor().clone(),
        inner: extension_inner,
        api_type: ApiType::VideoExtend,
    }));
    registry.register_derived(
        sn_openai_adapter()?,
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

fn sn_chat_descriptor() -> OperationDescriptor {
    let (_, codecs) = super::openai_chat_completions_adapter();
    let mut descriptor = codecs.operation_codecs[0].descriptor().clone();
    descriptor.bindings.clear();
    for api_type in [ApiType::AudioSpeechRecognition] {
        descriptor
            .bindings
            .push(OperationBinding::new(api_type, [ExecutionMode::Immediate]));
    }
    descriptor
}

struct SnVideoCodec {
    inner: Arc<dyn NativeTaskCodec>,
    descriptor: OperationDescriptor,
    api_type: ApiType,
}

#[async_trait]
impl NativeTaskCodec for SnVideoCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }
    fn api_type(&self) -> ApiType {
        self.api_type
    }
    fn operations(&self) -> BTreeSet<NativeTaskOperation> {
        self.inner
            .operations()
            .into_iter()
            .filter(|operation| *operation != NativeTaskOperation::Cancel)
            .collect()
    }
    fn output_video_seconds(&self, request: &HttpRequest) -> Option<u64> {
        self.inner.output_video_seconds(request)
    }
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        self.inner.resource_input_form()
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        if input.operation != NativeTaskOperation::Submit {
            if !matches!(
                input.operation,
                NativeTaskOperation::Status | NativeTaskOperation::Result
            ) {
                return Err(ProtocolError::new(
                    super::ProtocolErrorKind::UnsupportedOperation,
                    "SN video jobs do not support cancellation",
                ));
            }
            let id = input.remote_task_id.ok_or_else(|| {
                ProtocolError::invalid_request("SN video lifecycle requires a task ID")
            })?;
            if id.is_empty()
                || id.len() > 2048
                || matches!(id, "." | "..")
                || id.chars().any(char::is_control)
            {
                return Err(ProtocolError::invalid_request(
                    "SN video task ID is invalid",
                ));
            }
            let mut url = reqwest::Url::parse(&input.context.base_url)
                .map_err(|_| ProtocolError::invalid_request("SN video base URL is invalid"))?;
            {
                let mut path = url.path_segments_mut().map_err(|_| {
                    ProtocolError::invalid_request("SN video base URL cannot contain path segments")
                })?;
                path.pop_if_empty().push("videos").push(id);
                if input.operation == NativeTaskOperation::Result {
                    path.push("content");
                }
            }
            let mut wire = HttpRequest::new(reqwest::Method::GET, url.to_string());
            return input.context.finalize_request(
                &mut wire,
                CredentialKind::Bearer,
                "SN video requires credentials",
                "SN video requires bearer credentials",
            );
        }
        if let Some(codec) = input.codec_input {
            if let AiccCall::VideoExtend(request) = &codec.canonical_request {
                let resource = input.context.materialized_resource(&request.video)?;
                let id = resource.provider_artifact_id.as_deref().or(request.continuation_handle.as_deref()).filter(|id| !id.is_empty() && id.len() <= 2048 && !id.chars().any(char::is_control)).ok_or_else(|| ProtocolError::invalid_request("SN video extension requires a source video ID or explicit continuation handle"))?;
                if resource.provider_artifact_id.is_some()
                    && request
                        .continuation_handle
                        .as_deref()
                        .is_some_and(|handle| handle != id)
                {
                    return Err(ProtocolError::invalid_request(
                        "SN video continuation handle must match the source artifact",
                    ));
                }
                let mut body = serde_json::json!({"model":input.resolved_parameters.get("provider_model_id").ok_or_else(|| ProtocolError::invalid_request("SN video requires a model"))?,"video":{"id":id},"prompt":request.prompt});
                if let Some(seconds) = request.duration_seconds {
                    if !seconds.is_finite() || seconds <= 0.0 || seconds.fract() != 0.0 {
                        return Err(ProtocolError::invalid_request(
                            "SN video duration must be a positive whole number of seconds",
                        ));
                    }
                    body["seconds"] = serde_json::json!(format!("{seconds:.0}"));
                }
                if request.resolution.is_some() {
                    return Err(ProtocolError::invalid_request(
                        "SN extension inherits the source resolution",
                    ));
                }
                let mut wire = HttpRequest::new(
                    reqwest::Method::POST,
                    format!(
                        "{}/videos/extensions",
                        input.context.base_url.trim_end_matches('/')
                    ),
                );
                wire.body = HttpBody::Json(body);
                return input.context.finalize_request(
                    &mut wire,
                    CredentialKind::Bearer,
                    "SN video requires credentials",
                    "SN video requires bearer credentials",
                );
            }
            if matches!(&codec.canonical_request, AiccCall::VideoEdit(_)) {
                let mut wire = self.inner.encode_native(input)?;
                if let HttpBody::Json(body) = &mut wire.body {
                    body["model"] = input
                        .resolved_parameters
                        .get("provider_model_id")
                        .cloned()
                        .ok_or_else(|| {
                            ProtocolError::invalid_request("SN video requires a model")
                        })?;
                }
                return Ok(wire);
            }
        }
        self.inner.encode_native(input)
    }

    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        let mut output = self.inner.decode_native(operation, response).await?;
        if let NativeTaskOutput::Submitted(handle) = &mut output {
            handle.cancel_supported = false;
        }
        Ok(output)
    }
}

fn sn_audio_registration() -> (OperationDescriptor, Arc<dyn OperationCodec>) {
    let (_, codecs) = super::openai_chat_completions_adapter();
    let inner = codecs
        .operation_codecs
        .into_iter()
        .find(|codec| codec.api_type() == ApiType::Llm)
        .unwrap();
    let (_, audio_codecs) = openai_responses_adapter();
    let error_codec = audio_codecs
        .operation_codecs
        .into_iter()
        .find(|codec| codec.api_type() == ApiType::AudioSpeechRecognition)
        .unwrap();
    let descriptor = sn_chat_descriptor();
    (
        descriptor.clone(),
        Arc::new(SnAudioTranscriptionCodec {
            inner,
            error_codec,
            descriptor,
        }),
    )
}

struct SnAudioTranscriptionCodec {
    inner: Arc<dyn OperationCodec>,
    error_codec: Arc<dyn OperationCodec>,
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for SnAudioTranscriptionCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }
    fn api_type(&self) -> ApiType {
        ApiType::AudioSpeechRecognition
    }
    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        let AiccCall::AudioSpeechRecognition(request) = &call.input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "SN audio chat requires a transcription request",
            ));
        };
        if request
            .timestamps
            .as_deref()
            .is_some_and(|timestamps| timestamps != "none")
            || request.diarization == Some(true)
            || request
                .output_formats
                .as_ref()
                .is_some_and(|formats| formats.iter().any(|format| format != "json"))
        {
            return Err(ProtocolError::invalid_request(
                "SN audio chat returns plain transcripts without timestamp or speaker annotations",
            ));
        }
        if call
            .input
            .resolved_parameters
            .keys()
            .any(|key| key != "provider_model_id")
        {
            return Err(ProtocolError::invalid_request(
                "SN audio chat has unsupported provider parameters",
            ));
        }
        let model = call
            .input
            .resolved_parameters
            .get("provider_model_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProtocolError::invalid_request("SN audio chat requires a provider model ID")
            })?;
        let audio = call.context.materialized_resource(&request.audio)?;
        let format = match audio.mime.as_str() {
            "audio/wav" | "audio/x-wav" => "wav",
            "audio/mpeg" | "audio/mp3" => "mp3",
            "audio/flac" => "flac",
            "audio/ogg" => "ogg",
            "audio/aac" => "aac",
            "audio/mp4" | "audio/x-m4a" => "m4a",
            "audio/webm" => "webm",
            _ => {
                return Err(ProtocolError::invalid_request(
                    "SN audio chat does not support this audio format",
                ))
            }
        };
        let prompt = format!(
            "Transcribe the speech verbatim. Return only the transcript, without commentary.{}",
            request
                .language
                .as_ref()
                .map(|language| format!(" Language hint: {language}."))
                .unwrap_or_default()
        );
        let mut wire = HttpRequest::new(
            reqwest::Method::POST,
            format!(
                "{}/chat/completions",
                call.context.base_url.trim_end_matches('/')
            ),
        );
        wire.body = HttpBody::Json(
            serde_json::json!({"model": model, "messages": [{"role": "user", "content": [
                {"type": "input_audio", "input_audio": {"data": base64::engine::general_purpose::STANDARD.encode(&audio.bytes), "format": format}},
                {"type": "text", "text": prompt}
            ]}]}),
        );
        call.context.finalize_request(
            &mut wire,
            CredentialKind::Bearer,
            "SN audio chat requires credentials",
            "SN audio chat requires bearer credentials",
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        if !response.status.is_success() {
            return self.error_codec.decode(response).await;
        }
        let mut execution = self.inner.decode(response).await?;
        let ProtocolExecution::Immediate(output) = &mut execution else {
            return Err(ProtocolError::invalid_response(
                "SN audio chat returned an unexpected stream",
            ));
        };
        let text = output
            .value
            .pointer("/message/content")
            .and_then(serde_json::Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter(|part| {
                        part.get("type").and_then(serde_json::Value::as_str) == Some("text")
                    })
                    .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                ProtocolError::invalid_response("SN audio chat returned no transcript")
            })?;
        output.value = serde_json::json!({"text": text, "segments": [], "artifacts": {}});
        Ok(execution)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CodecContext, CodecInput, CodecLimits, HttpBody, ResolvedCredential};
    use buckyos_api::{ProviderStateCoordinate, VideoGenerateRequest, VideoOutputSpec, VideoSize};
    use crate::video::test_support::{model_contract, plan_parameters};
    use crate::video::VideoMaterialFacts;
    use std::collections::BTreeMap;

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://sn.example/api/v1/ai".into(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "sn".into(),
                adapter_type: SN_OPENAI_ADAPTER_ID.into(),
                origin_provider: "sn".into(),
                origin_model: "wan-3.0".into(),
            },
            credential: Some(ResolvedCredential::bearer("secret://sn", "test-key").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: std::time::Duration::from_secs(30),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }

    #[test]
    fn video_duration_preserves_sn_values_without_changing_openai() {
        let mut registry = CodecRegistry::default();
        let (adapter, codecs) = openai_responses_adapter();
        registry.register_codecs(adapter, codecs).unwrap();
        register_sn_openai_adapter(&mut registry).unwrap();
        let context = context();
        assert!(
            !registry.adapter(SN_OPENAI_ADAPTER_ID).unwrap().operations["videos.create"]
                .supports_cancel
        );
        for duration in [2.0, 15.0, 30.0] {
            let mut request =
                VideoGenerateRequest::new("wan-3.0@sn", Some("A moving paper plane".into()));
            request.output = Some(VideoOutputSpec {
                duration_seconds: Some(duration),
                size: Some(VideoSize::exact(1080, 1920)),
                ..Default::default()
            });
            let contract = model_contract("qwen", "wan3.0-video");
            let parameters = plan_parameters(&contract, &request, &VideoMaterialFacts::new(), "wan-3.0");
            let codec_input = CodecInput {
                canonical_request: AiccCall::VideoGenerate(request),
                resolved_parameters: parameters.clone(),
            };
            let input = NativeTaskInput {
                operation: NativeTaskOperation::Submit,
                remote_task_id: None,
                codec_input: Some(&codec_input),
                resolved_parameters: &parameters,
                context: &context,
            };
            let wire = registry
                .encode_native(
                    SN_OPENAI_ADAPTER_ID,
                    "videos.create",
                    ApiType::VideoGenerate,
                    &input,
                )
                .unwrap();
            assert_eq!(wire.url, "https://sn.example/api/v1/ai/videos");
            let HttpBody::Multipart(body) = wire.body else {
                panic!("expected multipart video request")
            };
            assert!(body
                .parts()
                .iter()
                .any(|part| part.name == "seconds"
                    && part.bytes == format!("{duration:.0}").as_bytes()));
            assert!(body
                .parts()
                .iter()
                .any(|part| part.name == "size" && part.bytes.as_ref() == b"1080x1920"));
            let original = registry
                .encode_native(
                    OPENAI_RESPONSES_ADAPTER_ID,
                    "videos.create",
                    ApiType::VideoGenerate,
                    &input,
                )
                .unwrap();
            let HttpBody::Multipart(body) = original.body else {
                panic!("expected multipart video request")
            };
            assert!(body.parts().iter().any(|part| part.name == "seconds" && part.bytes == format!("{duration:.0}").as_bytes()));
        }
    }

    #[test]
    fn video_edits_and_extensions_retain_model_and_source_identity() {
        let mut registry = CodecRegistry::default();
        let (adapter, codecs) = openai_responses_adapter();
        registry.register_codecs(adapter, codecs).unwrap();
        register_sn_openai_adapter(&mut registry).unwrap();
        let source = buckyos_api::ResourceRef::base64("video/mp4".into(), "dmlkZW8=".into());
        let mut context = context();
        let mut resource = crate::protocol::MaterializedResource::new(
            bytes::Bytes::from_static(b"video"),
            "video/mp4",
            None,
        )
        .unwrap();
        resource.provider_artifact_id = Some("video_source.v2".into());
        context.resources.insert(
            crate::resource::ResourceKey::from_ref(&source).into_string(),
            resource,
        );
        for (api_type, call, path) in [
            (
                ApiType::VideoEdit,
                AiccCall::VideoEdit(buckyos_api::VideoEditRequest::new(
                    "wan-3.0@sn",
                    source.clone(),
                    "edit motion".into(),
                )),
                "videos/edits",
            ),
            (
                ApiType::VideoExtend,
                AiccCall::VideoExtend(buckyos_api::VideoExtendRequest::new(
                    "wan-3.0@sn",
                    source.clone(),
                    "continue motion".into(),
                )),
                "videos/extensions",
            ),
        ] {
            let input = CodecInput {
                canonical_request: call,
                resolved_parameters: BTreeMap::new(),
            };
            let parameters =
                BTreeMap::from([("provider_model_id".into(), serde_json::json!("wan-3.0"))]);
            let native = NativeTaskInput {
                operation: NativeTaskOperation::Submit,
                remote_task_id: None,
                codec_input: Some(&input),
                resolved_parameters: &parameters,
                context: &context,
            };
            let wire = registry
                .encode_native(SN_OPENAI_ADAPTER_ID, "videos.create", api_type, &native)
                .unwrap();
            assert_eq!(wire.url, format!("https://sn.example/api/v1/ai/{path}"));
            let HttpBody::Json(body) = wire.body else {
                panic!("expected JSON source identity")
            };
            assert_eq!(body["model"], "wan-3.0");
            assert_eq!(body["video"]["id"], "video_source.v2");
        }
    }

    #[tokio::test]
    async fn audio_chat_transcribes_bytes_and_preserves_usage() {
        let codec = sn_audio_registration().1;
        let mut context = context();
        let audio = buckyos_api::ResourceRef::base64("audio/wav".into(), "UklGRm1vY2s=".into());
        context.resources.insert(
            crate::resource::ResourceKey::from_ref(&audio).into_string(),
            crate::protocol::MaterializedResource::new(
                bytes::Bytes::from_static(b"RIFFmock"),
                "audio/wav",
                None,
            )
            .unwrap(),
        );
        let mut request =
            buckyos_api::AudioSpeechRecognitionRequest::new("voxtral-small-24b-2507@sn", audio);
        request.timestamps = Some("none".into());
        let input = CodecInput {
            canonical_request: AiccCall::AudioSpeechRecognition(request),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".into(),
                serde_json::json!("voxtral-small-24b-2507"),
            )]),
        };
        let wire = codec
            .encode(&CodecCall {
                api_type: ApiType::AudioSpeechRecognition,
                input: &input,
                context: &context,
            })
            .unwrap();
        assert_eq!(wire.url, "https://sn.example/api/v1/ai/chat/completions");
        let HttpBody::Json(body) = wire.body else {
            panic!("expected JSON audio chat")
        };
        assert_eq!(
            body["messages"][0]["content"][0],
            serde_json::json!({"type":"input_audio","input_audio":{"data":"UklGRm1vY2s=","format":"wav"}})
        );
        let response = HttpResponse {status: reqwest::StatusCode::OK, headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from_static(br#"{"id":"chat_1","object":"chat.completion","model":"voxtral-small-24b-2507","choices":[{"index":0,"message":{"role":"assistant","content":"A spoken sentence."},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}}"#),
            request_id:"audio-chat-test".into(), retry_after:None};
        let mut failure = response.clone();
        failure.status = reqwest::StatusCode::UNAUTHORIZED;
        failure.body = bytes::Bytes::from_static(
            br#"{"code":"unauthorized","message":"invalid or missing credentials"}"#,
        );
        assert_eq!(
            codec
                .decode(failure)
                .await
                .unwrap_err()
                .provider_code
                .as_deref(),
            Some("unauthorized")
        );
        let ProtocolExecution::Immediate(output) = codec.decode(response).await.unwrap() else {
            panic!("expected transcript")
        };
        assert_eq!(output.value["text"], "A spoken sentence.");
        assert_eq!(output.usage.unwrap().total_tokens, Some(7));
        context.credential = None;
        assert!(codec
            .encode(&CodecCall {
                api_type: ApiType::AudioSpeechRecognition,
                input: &input,
                context: &context
            })
            .is_err());
    }

    #[tokio::test]
    async fn video_lifecycle_encodes_sn_ids_and_disables_remote_cancel() {
        let mut registry = CodecRegistry::default();
        let (adapter, codecs) = openai_responses_adapter();
        registry.register_codecs(adapter, codecs).unwrap();
        register_sn_openai_adapter(&mut registry).unwrap();
        let context = context();
        let parameters = BTreeMap::new();
        for (operation, suffix) in [
            (NativeTaskOperation::Status, ""),
            (NativeTaskOperation::Result, "/content"),
        ] {
            let input = NativeTaskInput {
                operation,
                remote_task_id: Some("video_a/b+=.1"),
                codec_input: None,
                resolved_parameters: &parameters,
                context: &context,
            };
            let wire = registry
                .encode_native(
                    SN_OPENAI_ADAPTER_ID,
                    "videos.create",
                    ApiType::VideoGenerate,
                    &input,
                )
                .unwrap();
            assert_eq!(
                wire.url,
                format!("https://sn.example/api/v1/ai/videos/video_a%2Fb+=.1{suffix}")
            );
        }
        let response = HttpResponse {
            status: reqwest::StatusCode::OK,
            headers: reqwest::header::HeaderMap::new(),
            body: bytes::Bytes::from_static(
                br#"{"id":"video_a/b+=.1","object":"video","status":"queued"}"#,
            ),
            request_id: "video-test".into(),
            retry_after: None,
        };
        let NativeTaskOutput::Submitted(handle) = registry
            .decode_native(
                SN_OPENAI_ADAPTER_ID,
                "videos.create",
                ApiType::VideoGenerate,
                NativeTaskOperation::Submit,
                response,
            )
            .await
            .unwrap()
        else {
            panic!("expected submitted video")
        };
        assert!(!handle.cancel_supported);
    }
}
