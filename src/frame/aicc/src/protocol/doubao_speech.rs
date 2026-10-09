use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecContext, CodecRegistration, ExecutionMode,
    HttpBody, HttpRequest, HttpResponse, NativeTaskCodec, NativeTaskHandle, NativeTaskInput,
    NativeTaskOperation, NativeTaskOutput, NativeTaskState, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput,
    ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    AiArtifact, AiUsage, AiccCall, ApiType, AudioSpeechRecognitionRequest, ResourceRef,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const DOUBAO_SPEECH_ADAPTER_ID: &str = "doubao-speech";
pub(crate) const DOUBAO_TTS_OPERATION_ID: &str = "tts.unidirectional";
/// 录音文件识别极速版：一次 HTTP 请求即返回识别结果。
pub(crate) const DOUBAO_ASR_FLASH_OPERATION_ID: &str = "asr.recognize.flash";
/// 录音文件识别标准版：`submit` + `query` 异步任务，音频只能以 URL 提交。
pub(crate) const DOUBAO_ASR_TASK_OPERATION_ID: &str = "asr.recognize.submit";

/// `X-Api-Resource-Id` 默认值。标准版对应豆包录音文件识别模型 2.0。
const DOUBAO_ASR_FLASH_RESOURCE_ID: &str = "volc.bigasr.auc_turbo";
const DOUBAO_ASR_TASK_RESOURCE_ID: &str = "volc.seedasr.auc";
const DOUBAO_ASR_MODEL_NAME: &str = "bigmodel";
/// AUC 业务状态码：任务完成。
const AUC_STATUS_SUCCESS: &str = "20000000";
/// AUC 业务状态码：任务在队列中 / 正在处理中。
const AUC_STATUS_QUEUED: &str = "20000002";
const AUC_STATUS_PROCESSING: &str = "20000001";
/// 标准版 submit 之后的轮询间隔。
const ASR_TASK_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// 请求 ID 与任务 ID 共用同一个头，长度上限取自官方文档建议的 UUID。
const MAX_TASK_ID_BYTES: usize = 256;

const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// 极速版允许把音频内联进请求体（官方上限 100MB，base64 后约 4/3）。
const ASR_FLASH_MAX_REQUEST_BYTES: usize = 160 * 1024 * 1024;

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn doubao_speech_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (operations, registration) = doubao_speech_registration();
    (
        AdapterDescriptor {
            protocol_family_id: "doubao".to_owned(),
            protocol_adapter_id: DOUBAO_SPEECH_ADAPTER_ID.to_owned(),
            interface_generation: "openspeech-plan-v3".to_owned(),
            base_adapter_id: None,
            component_adapter_ids: Vec::new(),
            status: AdapterStatus::Stable,
            probe_priority: 200,
            probe_path: None,
            credential: super::AdapterCredentialContract::named_header("x-api-key"),
            operations: operations
                .into_iter()
                .map(|operation| (operation.operation_id.clone(), operation))
                .collect(),
        },
        registration,
    )
}

pub(super) fn doubao_speech_registration() -> (Vec<OperationDescriptor>, CodecRegistration) {
    let tts = OperationDescriptor {
        operation_id: DOUBAO_TTS_OPERATION_ID.to_owned(),
        bindings: vec![OperationBinding::new(
            ApiType::AudioTextToSpeech,
            [ExecutionMode::Immediate],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let asr_flash = OperationDescriptor {
        operation_id: DOUBAO_ASR_FLASH_OPERATION_ID.to_owned(),
        bindings: vec![OperationBinding::new(
            ApiType::AudioSpeechRecognition,
            [ExecutionMode::Immediate],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: ASR_FLASH_MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let asr_task = OperationDescriptor {
        operation_id: DOUBAO_ASR_TASK_OPERATION_ID.to_owned(),
        bindings: vec![OperationBinding::new(
            ApiType::AudioSpeechRecognition,
            [ExecutionMode::NativeTask],
        )],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: MAX_REQUEST_BYTES,
        max_response_bytes: MAX_RESPONSE_BYTES,
    };
    let operation_codecs = vec![
        Arc::new(DoubaoTtsCodec {
            descriptor: tts.clone(),
        }) as Arc<dyn OperationCodec>,
        Arc::new(DoubaoAsrFlashCodec {
            descriptor: asr_flash.clone(),
        }) as Arc<dyn OperationCodec>,
    ];
    let native_task_codecs = vec![Arc::new(DoubaoAsrTaskCodec {
        descriptor: asr_task.clone(),
    }) as Arc<dyn NativeTaskCodec>];
    (
        vec![tts, asr_flash, asr_task],
        CodecRegistration {
            operation_codecs,
            native_task_codecs,
        },
    )
}

#[derive(Clone)]
struct DoubaoTtsCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for DoubaoTtsCodec {
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
                "Doubao TTS codec received the wrong canonical request",
            ));
        };
        let wire_profile = call
            .input
            .resolved_parameters
            .get("doubao_tts_wire_profile")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProtocolError::invalid_configuration(
                    "Doubao TTS wire profile is missing or unsupported",
                )
            })?;
        let speaker = call
            .input
            .resolved_parameters
            .get("speaker")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Doubao TTS requires a resolved speaker",
                )
            })?;
        let mut format = "mp3";
        let mut sample_rate = 24000;
        if let Some(output) = &request.output {
            if let Some(media_type) = &output.media_type {
                format = audio_format(media_type)?;
                if format == "ogg_opus" && output.sample_rate.is_none() {
                    sample_rate = 48000;
                }
            }
            if let Some(requested_sample_rate) = output.sample_rate {
                sample_rate = requested_sample_rate;
            }
        }
        validate_audio_params(format, sample_rate)?;
        let mut audio = Map::from_iter([
            ("format".to_owned(), json!(format)),
            ("sample_rate".to_owned(), json!(sample_rate)),
        ]);
        if let Some(speed) = request.speed {
            if wire_profile != "standard" {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Doubao Agent Plan TTS does not expose canonical speed control",
                ));
            }
            audio.insert("speech_rate".to_owned(), json!(speech_rate(speed)?));
        }
        let mut req_params = Map::from_iter([
            ("text".to_owned(), json!(request.text)),
            ("speaker".to_owned(), json!(speaker)),
            ("audio_params".to_owned(), Value::Object(audio)),
        ]);
        if let Some(model) = call
            .input
            .resolved_parameters
            .get("doubao_tts_model")
            .and_then(Value::as_str)
        {
            req_params.insert("model".to_owned(), json!(model));
        }
        if wire_profile == "standard" {
            if let Some(language) = &request.voice.language {
                let additions = serde_json::to_string(&json!({
                    "explicit_language": explicit_language(language)?
                }))
                .map_err(|_| {
                    ProtocolError::invalid_configuration(
                        "Doubao TTS additions could not be serialized",
                    )
                })?;
                req_params.insert("additions".to_owned(), json!(additions));
            }
            if let Some(instructions) = request
                .voice
                .instructions
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                req_params.insert("context_texts".to_owned(), json!([instructions]));
            }
        }
        let body = json!({"req_params": Value::Object(req_params)});
        let mut url = Url::parse(&call.context.base_url)
            .map_err(|_| ProtocolError::invalid_configuration("Doubao TTS base URL is invalid"))?;
        let base = url.path().trim_end_matches('/');
        url.set_path(&format!("{base}/unidirectional"));
        let mut wire = HttpRequest::new(Method::POST, url.to_string());
        wire.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let resource_id = call
            .input
            .resolved_parameters
            .get("doubao_tts_resource_id")
            .and_then(Value::as_str)
            .unwrap_or("seed-tts-2.0");
        wire.headers.insert(
            HeaderName::from_static("x-api-resource-id"),
            HeaderValue::from_str(resource_id).map_err(|_| {
                ProtocolError::invalid_configuration("Doubao TTS resource ID is invalid")
            })?,
        );
        match wire_profile {
            "standard" => {
                wire.headers.insert(
                    HeaderName::from_static("x-api-request-id"),
                    HeaderValue::from_str(&request_id()).map_err(|_| {
                        ProtocolError::invalid_configuration("Doubao TTS request ID is invalid")
                    })?,
                );
                wire.headers.insert(
                    HeaderName::from_static("x-control-require-usage-tokens-return"),
                    HeaderValue::from_static("*"),
                );
            }
            "agent_plan" => {
                wire.headers.insert(
                    HeaderName::from_static("x-control-request-usage-tokens"),
                    HeaderValue::from_static("true"),
                );
                wire.headers.insert(
                    HeaderName::from_static("x-control-require-usage-tokens-return"),
                    HeaderValue::from_static("*"),
                );
            }
            _ => {
                return Err(ProtocolError::invalid_configuration(
                    "Doubao TTS wire profile is missing or unsupported",
                ));
            }
        }
        apply_speech_credential(&mut wire.headers, call.context)?;
        wire.body = HttpBody::Json(body);
        wire.timeout = Some(call.context.limits.request_timeout);
        wire.max_request_bytes = Some(call.context.limits.max_request_bytes);
        wire.max_response_bytes = Some(call.context.limits.max_response_bytes);
        Ok(wire)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        decode_audio(response, "mp3")
    }

    async fn decode_with_input(
        &self,
        response: HttpResponse,
        input: &super::CodecInput,
    ) -> ProtocolResultValue<ProtocolExecution> {
        let AiccCall::AudioTextToSpeech(request) = &input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "Doubao TTS requires audio.tts",
            ));
        };
        let format = request
            .output
            .as_ref()
            .and_then(|output| output.media_type.as_deref())
            .map(audio_format)
            .transpose()?
            .unwrap_or("mp3");
        decode_audio(response, format)
    }
}

fn decode_audio(response: HttpResponse, format: &str) -> ProtocolResultValue<ProtocolExecution> {
    if !response.status.is_success() {
        let value = serde_json::from_slice::<Value>(&response.body).unwrap_or(Value::Null);
        let header = value.get("header").and_then(Value::as_object);
        let request_id = header
            .and_then(|header| header.get("reqid"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| response.request_id.clone());
        return Err(ProtocolError::new(
            super::protocol_error_kind_from_http_status(response.status),
            value
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| {
                    header
                        .and_then(|header| header.get("message"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("Doubao TTS request failed"),
        )
        .with_provider_code(
            value
                .get("code")
                .or_else(|| header.and_then(|header| header.get("code")))
                .map(Value::to_string),
        )
        .with_http_status(response.status.as_u16())
        .with_request_id(Some(request_id))
        .with_retry_after(response.retry_after));
    }
    let content_type = response
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !matches!(content_type, Some("text/plain" | "application/json")) {
        return Err(ProtocolError::invalid_response(
            "Doubao TTS response has an invalid content type",
        ));
    }
    let body = std::str::from_utf8(&response.body).map_err(|_| {
        ProtocolError::invalid_response("Doubao TTS response is not UTF-8 JSON lines")
    })?;
    let frames = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line).map_err(|_| {
                ProtocolError::invalid_response("Doubao TTS response contains malformed JSON lines")
            })
        })
        .collect::<ProtocolResultValue<Vec<_>>>()?;
    if frames.is_empty() {
        return Err(ProtocolError::invalid_response(
            "Doubao TTS response contains no JSON frames",
        ));
    }
    let mut audio = Vec::new();
    let mut usage = None;
    let mut completed = false;
    for frame in frames {
        let header = frame.get("header").and_then(Value::as_object);
        let code = frame
            .get("code")
            .and_then(Value::as_i64)
            .or_else(|| {
                header
                    .and_then(|header| header.get("code"))
                    .and_then(Value::as_i64)
            })
            .unwrap_or(-1);
        if !response.status.is_success() || !matches!(code, 0 | 20_000_000) {
            let message = frame
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| {
                    header
                        .and_then(|header| header.get("message"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("Doubao TTS request failed");
            let request_id = header
                .and_then(|header| header.get("reqid"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| response.request_id.clone());
            return Err(
                ProtocolError::new(ProtocolErrorKind::ProviderRejected, message)
                    .with_provider_code((code >= 0).then(|| code.to_string()))
                    .with_http_status(response.status.as_u16())
                    .with_request_id(Some(request_id))
                    .with_retry_after(response.retry_after),
            );
        }
        if let Some(value) = frame.get("usage") {
            usage = Some(value.clone());
        }
        if code == 20_000_000 {
            completed = true;
            continue;
        }
        if let Some(data) = frame
            .get("data")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            audio.extend(STANDARD.decode(data).map_err(|_| {
                ProtocolError::invalid_response("Doubao TTS response contains invalid base64 audio")
            })?);
        }
        if frame.get("usage").is_some() {
            usage = frame.get("usage").cloned();
        }
    }
    if !completed {
        return Err(ProtocolError::invalid_response(
            "Doubao TTS response is missing the completion frame",
        ));
    }
    if audio.is_empty() {
        return Err(ProtocolError::invalid_response(
            "Doubao TTS response is missing audio data",
        ));
    }
    let data = STANDARD.encode(audio);
    let mime = match format {
        "wav" => "audio/wav",
        "pcm" => "audio/pcm",
        "ogg_opus" => "audio/ogg",
        _ => "audio/mpeg",
    }
    .to_owned();
    let resource = ResourceRef::base64(mime.clone(), data);
    let characters = usage
        .as_ref()
        .and_then(|value| value.get("text_words"))
        .and_then(Value::as_u64);
    Ok(ProtocolExecution::Immediate(ProtocolOutput {
        value: json!({"audio": resource}),
        usage: Some(AiUsage {
            characters,
            ..AiUsage::request_units(1)
        }),
        artifacts: vec![AiArtifact {
            name: "speech".to_owned(),
            resource,
            mime: Some(mime),
            metadata: Some(Value::Object(Map::from_iter([(
                "provider_usage".to_owned(),
                usage.unwrap_or(Value::Null),
            )]))),
        }],
    }))
}

/// 录音文件识别极速版：`audio.url` / `audio.data` 二选一，一次请求返回结果。
#[derive(Clone)]
struct DoubaoAsrFlashCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for DoubaoAsrFlashCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // `audio.url` 与 `audio.data` 都可接受，所以 URL 形态直接透传、不做下载。
        crate::resource::ResourceInputForm::UrlOrBytes
    }

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
                "Doubao ASR codec received the wrong canonical request",
            ));
        };
        let audio = resolve_asr_audio(&request.audio, call.context, true)?;
        let body = asr_body(call.context, audio, request)?;
        let resource_id = asr_resource_id(
            call.input.resolved_parameters.get("doubao_asr_resource_id"),
            DOUBAO_ASR_FLASH_RESOURCE_ID,
        );
        asr_request(
            call.context,
            "recognize/flash",
            resource_id,
            &request_id(),
            true,
            Some(body),
        )
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        ensure_auc_success(&response)?;
        let value: Value = response.json(self.descriptor.max_response_bytes)?;
        Ok(ProtocolExecution::Immediate(asr_output(&value, &response)?))
    }
}

/// 录音文件识别标准版：`submit` → `query` 异步任务，音频只能以 URL 提交。
#[derive(Clone)]
struct DoubaoAsrTaskCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl NativeTaskCodec for DoubaoAsrTaskCodec {
    fn resource_input_form(&self) -> crate::resource::ResourceInputForm {
        // 标准版 `audio.url` 必填、没有内联字节的位置：`Base64` 会先落进本地 NDN
        // 拿到对象 URL，`NamedObject` 直接用对象 URL 交给豆包自取。
        crate::resource::ResourceInputForm::UrlOnly
    }

    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    fn api_type(&self) -> ApiType {
        ApiType::AudioSpeechRecognition
    }

    fn operations(&self) -> BTreeSet<NativeTaskOperation> {
        BTreeSet::from([
            NativeTaskOperation::Submit,
            NativeTaskOperation::Status,
            NativeTaskOperation::Result,
        ])
    }

    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
        let resource_id = asr_resource_id(
            input.resolved_parameters.get("doubao_asr_resource_id"),
            DOUBAO_ASR_TASK_RESOURCE_ID,
        );
        match input.operation {
            NativeTaskOperation::Submit => {
                let codec_input = input.codec_input.ok_or_else(|| {
                    ProtocolError::invalid_request("Doubao ASR submit requires canonical input")
                })?;
                let AiccCall::AudioSpeechRecognition(request) = &codec_input.canonical_request
                else {
                    return Err(ProtocolError::invalid_request(
                        "Doubao ASR codec received the wrong canonical request",
                    ));
                };
                let audio = resolve_asr_audio(&request.audio, input.context, false)?;
                let body = asr_body(input.context, audio, request)?;
                asr_request(
                    input.context,
                    "submit",
                    resource_id,
                    &request_id(),
                    true,
                    Some(body),
                )
            }
            // `query` 用 submit 时的同一个任务 ID 定位结果，请求体是空 JSON；
            // 它不携带音频，所以恢复路径不需要（也没有）资源字节。
            NativeTaskOperation::Status | NativeTaskOperation::Result => asr_request(
                input.context,
                "query",
                resource_id,
                safe_task_id(input.remote_task_id)?,
                false,
                Some(json!({})),
            ),
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Doubao ASR task does not support cancellation",
            )),
        }
    }

    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        match operation {
            NativeTaskOperation::Submit => {
                ensure_auc_success(&response)?;
                let mut handle = NativeTaskHandle::new(submit_task_id(&response)?)?;
                handle.poll_after = Some(ASR_TASK_POLL_INTERVAL);
                Ok(NativeTaskOutput::Submitted(handle))
            }
            NativeTaskOperation::Status => {
                let code = auc_status_code(&response).ok_or_else(|| {
                    ProtocolError::invalid_response(
                        "Doubao ASR status response is missing X-Api-Status-Code",
                    )
                })?;
                let state = match code.as_str() {
                    AUC_STATUS_SUCCESS => NativeTaskState::Succeeded,
                    AUC_STATUS_QUEUED => NativeTaskState::Queued,
                    AUC_STATUS_PROCESSING => NativeTaskState::Running,
                    _ => return Err(auc_error(&response, Some(code.as_str()))),
                };
                Ok(NativeTaskOutput::Status {
                    state,
                    retry_after: response.retry_after.or(Some(ASR_TASK_POLL_INTERVAL)),
                    result_ref: None,
                    // The AUC status/query response carries only a status code in
                    // its headers; usage arrives with the result, so there is
                    // nothing to report while polling.
                    result_usage: None,
                    result_artifacts: BTreeMap::new(),
                })
            }
            NativeTaskOperation::Result => {
                ensure_auc_success(&response)?;
                let value: Value = response.json(self.descriptor.max_response_bytes)?;
                Ok(NativeTaskOutput::Result(asr_output(&value, &response)?))
            }
            NativeTaskOperation::Cancel => Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Doubao ASR task does not support cancellation",
            )),
        }
    }
}

/// 音频在 AUC 线格式里的两种形态。
enum AsrAudio {
    /// 一个豆包自己会去拉取的绝对 URL。
    Url {
        url: String,
        format: Option<&'static str>,
    },
    /// 内联的 base64 音频。
    Bytes {
        data_base64: String,
        format: Option<&'static str>,
    },
}

impl AsrAudio {
    fn into_object(self) -> Map<String, Value> {
        match self {
            Self::Url { url, format } => {
                let mut audio = Map::from_iter([("url".to_owned(), json!(url))]);
                if let Some(format) = format {
                    audio.insert("format".to_owned(), json!(format));
                }
                audio
            }
            Self::Bytes {
                data_base64,
                format,
            } => {
                let mut audio = Map::from_iter([("data".to_owned(), json!(data_base64))]);
                if let Some(format) = format {
                    audio.insert("format".to_owned(), json!(format));
                }
                audio
            }
        }
    }
}

/// 把规范化音频还原成本 Codec 在 `resource_input_form` 里声明过的形态。
///
/// `UrlOnly`（标准版）只会走到前两个分支：`ResourceRef::Url` 由物化阶段原样透传，
/// 其余形态则因为「协议只要 URL」而被提前发布成 NDN 对象，`materialized_url` 给回
/// 对象 URL。`allow_bytes` 只对 `UrlOrBytes`（极速版）为真，用于内联字节。
fn resolve_asr_audio(
    resource: &ResourceRef,
    context: &CodecContext,
    allow_bytes: bool,
) -> ProtocolResultValue<AsrAudio> {
    if let Some(url) = context.materialized_url(resource) {
        let mime = context
            .materialized_resource(resource)
            .ok()
            .map(|materialized| materialized.mime.clone());
        let format = asr_audio_format(mime.as_deref(), Some(url));
        return Ok(AsrAudio::Url {
            url: url.to_owned(),
            format,
        });
    }
    match resource {
        ResourceRef::Url { url, mime_hint } => Ok(AsrAudio::Url {
            url: url.clone(),
            format: asr_audio_format(mime_hint.as_deref(), Some(url)),
        }),
        _ if allow_bytes => {
            let materialized = context.materialized_resource(resource)?;
            Ok(AsrAudio::Bytes {
                data_base64: STANDARD.encode(&materialized.bytes),
                format: asr_audio_format(
                    Some(&materialized.mime),
                    materialized.file_name.as_deref(),
                ),
            })
        }
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Doubao ASR task submission requires an absolute audio URL",
        )),
    }
}

fn asr_body(
    context: &CodecContext,
    audio: AsrAudio,
    request: &AudioSpeechRecognitionRequest,
) -> ProtocolResultValue<Value> {
    let mut audio = audio.into_object();
    if let Some(language) = request
        .language
        .as_deref()
        .filter(|value| !value.is_empty())
        .and_then(asr_language)
    {
        audio.insert("language".to_owned(), json!(language));
    }
    Ok(json!({
        "user": {"uid": asr_uid(context)},
        "audio": Value::Object(audio),
        "request": Value::Object(asr_request_options(request)?),
    }))
}

fn asr_request_options(
    request: &AudioSpeechRecognitionRequest,
) -> ProtocolResultValue<Map<String, Value>> {
    if request.output_formats.as_ref().is_some_and(|formats| {
        formats
            .iter()
            .any(|format| !matches!(format.as_str(), "json" | "verbose_json"))
    }) {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Doubao ASR canonical output supports JSON only",
        ));
    }
    let mut options = Map::from_iter([("model_name".to_owned(), json!(DOUBAO_ASR_MODEL_NAME))]);
    let timestamps_requested = if let Some(timestamps) = request
        .timestamps
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        if !matches!(timestamps, "segment" | "word" | "both") {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Doubao ASR timestamps must be segment, word, or both",
            ));
        }
        // 分句时间戳来自 `show_utterances` 返回的 utterance 边界。
        true
    } else {
        false
    };
    if let Some(diarization) = request.diarization {
        options.insert("enable_speaker_info".to_owned(), json!(diarization));
    }
    if timestamps_requested || request.diarization == Some(true) {
        options.insert("show_utterances".to_owned(), json!(true));
    }
    Ok(options)
}

/// AUC 用 `uid` 在同一 app 下区分调用方；用凭证的匿名引用既稳定又不泄漏密钥。
fn asr_uid(context: &CodecContext) -> String {
    context
        .credential
        .as_ref()
        .map(|credential| credential.audit().anonymous_ref.as_str().to_owned())
        .unwrap_or_else(|| "aicc".to_owned())
}

fn asr_resource_id<'a>(configured: Option<&'a Value>, default: &'a str) -> &'a str {
    configured
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
}

/// 拼一个 AUC 请求：`{base}/{path}` + openspeech 头。
fn asr_request(
    context: &CodecContext,
    path: &str,
    resource_id: &str,
    task_id: &str,
    sequence: bool,
    body: Option<Value>,
) -> ProtocolResultValue<HttpRequest> {
    context.validate()?;
    let mut url = Url::parse(&context.base_url)
        .map_err(|_| ProtocolError::invalid_configuration("Doubao ASR base URL is invalid"))?;
    let base = url.path().trim_end_matches('/');
    url.set_path(&format!("{base}/{}", path.trim_start_matches('/')));
    let mut request = HttpRequest::new(Method::POST, url.to_string());
    request
        .headers
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    request.headers.insert(
        HeaderName::from_static("x-api-resource-id"),
        HeaderValue::from_str(resource_id).map_err(|_| {
            ProtocolError::invalid_configuration("Doubao ASR resource ID is invalid")
        })?,
    );
    let task_id_header = HeaderValue::from_str(task_id)
        .map_err(|_| ProtocolError::invalid_configuration("Doubao ASR task ID is invalid"))?;
    request.headers.insert(
        HeaderName::from_static("x-api-request-id"),
        task_id_header.clone(),
    );
    if sequence {
        request.headers.insert(
            HeaderName::from_static("x-api-sequence"),
            HeaderValue::from_static("-1"),
        );
    }
    // AUC 的任务 ID 由调用方自选，而 submit 的响应体是空的：ID 必须自己绕一圈回来。
    // 传输层会把请求自带的 request-ID 头回填到 `HttpResponse::request_id`，所以同一个
    // 值也写到传输层使用的头上，`decode_native(Submit)` 再把它读回来当作任务 ID。
    // 若服务端同时在响应里回显 `X-Api-Request-Id`，则优先使用回显值。
    request
        .headers
        .insert(HeaderName::from_static("x-request-id"), task_id_header);
    if let Some(body) = body {
        request.body = HttpBody::Json(body);
    }
    apply_speech_credential(&mut request.headers, context)?;
    request.timeout = Some(context.limits.request_timeout);
    request.max_request_bytes = Some(context.limits.max_request_bytes);
    request.max_response_bytes = Some(context.limits.max_response_bytes);
    Ok(request)
}

fn auc_status_code(response: &HttpResponse) -> Option<String> {
    response
        .headers
        .get("x-api-status-code")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn auc_message(response: &HttpResponse) -> Option<&str> {
    response
        .headers
        .get("x-api-message")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// AUC 把业务错误码放在响应头里，HTTP 状态码可能仍是 200，所以以头为准。
fn auc_error(response: &HttpResponse, code: Option<&str>) -> ProtocolError {
    let message = auc_message(response).unwrap_or("Doubao ASR request failed");
    let logid = response
        .headers
        .get("x-tt-logid")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| response.request_id.clone());
    ProtocolError::new(
        super::protocol_error_kind_from_http_status(response.status),
        message,
    )
    .with_provider_code(code.map(str::to_owned))
    .with_http_status(response.status.as_u16())
    .with_request_id(Some(logid))
    .with_retry_after(response.retry_after)
}

fn ensure_auc_success(response: &HttpResponse) -> ProtocolResultValue<()> {
    let code = auc_status_code(response);
    if response.status.is_success()
        && code
            .as_deref()
            .is_none_or(|code| code == AUC_STATUS_SUCCESS)
    {
        return Ok(());
    }
    Err(auc_error(response, code.as_deref()))
}

/// submit 的响应体为空，任务 ID 只能从响应头或传输层回填的 request ID 取回。
fn submit_task_id(response: &HttpResponse) -> ProtocolResultValue<String> {
    response
        .headers
        .get("x-api-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            let request_id = response.request_id.trim();
            (!request_id.is_empty()).then(|| request_id.to_owned())
        })
        .ok_or_else(|| {
            ProtocolError::invalid_response("Doubao ASR submit response is missing the task ID")
        })
}

fn asr_output(value: &Value, response: &HttpResponse) -> ProtocolResultValue<ProtocolOutput> {
    let result = value
        .get("result")
        .ok_or_else(|| ProtocolError::invalid_response("Doubao ASR response is missing result"))?;
    let text = result
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let segments = result
        .get("utterances")
        .and_then(Value::as_array)
        .map(|utterances| {
            utterances
                .iter()
                .enumerate()
                .map(|(index, utterance)| {
                    let start = utterance
                        .get("start_time")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    let end = utterance
                        .get("end_time")
                        .and_then(Value::as_f64)
                        .unwrap_or(start);
                    json!({
                        "id": index.to_string(),
                        "start_seconds": start / 1000.0,
                        "end_seconds": end / 1000.0,
                        "text": utterance.get("text").and_then(Value::as_str).unwrap_or(""),
                        "speaker": utterance.pointer("/additions/speaker").cloned().unwrap_or(Value::Null),
                        "confidence": Value::Null,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let duration_ms = value
        .pointer("/audio_info/duration")
        .and_then(Value::as_f64);
    let logid = response
        .headers
        .get("x-tt-logid")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(response.request_id.as_str());
    Ok(ProtocolOutput {
        value: json!({
            "text": text,
            "segments": segments,
            "artifacts": {},
            "diagnostic": {
                "duration_ms": duration_ms,
                "logid": logid,
            }
        }),
        usage: Some(AiUsage {
            audio_seconds: duration_ms.map(|ms| ms / 1000.0),
            ..AiUsage::request_units(1)
        }),
        artifacts: Vec::new(),
    })
}

/// AUC 只接受 wav / mp3 / ogg（另有裸流 raw）；格式既能从 `ResourceRef` 的 MIME
/// 推断，也能从文件名或 URL 后缀推断。
fn asr_audio_format(mime: Option<&str>, name_hint: Option<&str>) -> Option<&'static str> {
    if let Some(mime) = mime {
        let mime = mime
            .split(';')
            .next()
            .unwrap_or(mime)
            .trim()
            .to_ascii_lowercase();
        match mime.as_str() {
            "audio/mpeg" | "audio/mp3" | "audio/mpeg3" => return Some("mp3"),
            "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => return Some("wav"),
            "audio/ogg" | "audio/opus" => return Some("ogg"),
            "audio/pcm" | "audio/l16" | "audio/raw" => return Some("raw"),
            _ => {}
        }
    }
    let name = name_hint?.to_ascii_lowercase();
    let extension = name.rsplit_once('.')?.1;
    let extension = extension.split(['?', '#']).next().unwrap_or(extension);
    match extension {
        "mp3" => Some("mp3"),
        "wav" | "wave" => Some("wav"),
        "ogg" | "opus" => Some("ogg"),
        "pcm" | "raw" | "l16" => Some("raw"),
        _ => None,
    }
}

/// 官方支持的语种标签；无法识别时返回 `None`，让模型自行判断。
fn asr_language(language: &str) -> Option<&'static str> {
    match language.to_ascii_lowercase().replace('_', "-").as_str() {
        "zh" | "zh-cn" | "cmn" => Some("zh-CN"),
        "yue" | "yue-cn" | "cant" => Some("yue-CN"),
        "en" | "en-us" | "en-gb" => Some("en-US"),
        "ja" | "ja-jp" => Some("ja-JP"),
        "ko" | "ko-kr" => Some("ko-KR"),
        "de" | "de-de" => Some("de-DE"),
        "fr" | "fr-fr" => Some("fr-FR"),
        "es" | "es-es" | "es-mx" => Some("es-MX"),
        "pt" | "pt-br" | "pt-pt" => Some("pt-BR"),
        "id" | "id-id" => Some("id-ID"),
        "ms" | "ms-my" => Some("ms-MY"),
        "th" | "th-th" => Some("th-TH"),
        "ar" | "ar-sa" => Some("ar-SA"),
        "it" | "it-it" => Some("it-IT"),
        "ru" | "ru-ru" => Some("ru-RU"),
        "tr" | "tr-tr" => Some("tr-TR"),
        "vi" | "vi-vn" => Some("vi-VN"),
        "pl" | "pl-pl" => Some("pl-PL"),
        "nl" | "nl-nl" => Some("nl-NL"),
        "el" | "el-gr" => Some("el-GR"),
        "uk" | "uk-ua" => Some("uk-UA"),
        "ro" | "ro-ro" => Some("ro-RO"),
        "fil" | "fil-ph" | "tl" => Some("fil-PH"),
        "bn" | "bn-bd" => Some("bn-BD"),
        "ne" | "ne-np" => Some("ne-NP"),
        _ => None,
    }
}

fn safe_task_id(value: Option<&str>) -> ProtocolResultValue<&str> {
    value
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_TASK_ID_BYTES
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
        .ok_or_else(|| ProtocolError::invalid_request("Doubao ASR task ID is invalid"))
}

fn request_id() -> String {
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut digest = Sha256::new();
    digest.update(timestamp.to_le_bytes());
    digest.update(sequence.to_le_bytes());
    digest.update(std::process::id().to_le_bytes());
    let mut bytes: [u8; 16] = digest.finalize()[..16]
        .try_into()
        .expect("fixed digest slice");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes(bytes[0..4].try_into().expect("fixed UUID field")),
        u16::from_be_bytes(bytes[4..6].try_into().expect("fixed UUID field")),
        u16::from_be_bytes(bytes[6..8].try_into().expect("fixed UUID field")),
        u16::from_be_bytes(bytes[8..10].try_into().expect("fixed UUID field")),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    )
}

fn apply_speech_credential(
    headers: &mut HeaderMap,
    context: &CodecContext,
) -> ProtocolResultValue<()> {
    let credential = context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "Doubao speech requires an API key",
        )
    })?;
    let mut source = HeaderMap::new();
    credential.apply(&mut source)?;
    let secret = if let Some(value) = source.get(AUTHORIZATION) {
        value
            .to_str()
            .ok()
            .and_then(|value| value.strip_prefix("Bearer "))
    } else {
        source.values().next().and_then(|value| value.to_str().ok())
    }
    .filter(|value| !value.is_empty())
    .ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "Doubao speech credential is invalid",
        )
    })?;
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(secret).map_err(|_| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Doubao speech credential is invalid",
            )
        })?,
    );
    Ok(())
}

fn audio_format(media_type: &str) -> ProtocolResultValue<&'static str> {
    match media_type.split(';').next().unwrap_or(media_type).trim() {
        "audio/mpeg" | "audio/mp3" => Ok("mp3"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok("wav"),
        "audio/pcm" | "audio/L16" => Ok("pcm"),
        "audio/ogg" | "audio/opus" => Ok("ogg_opus"),
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Doubao TTS does not support the requested audio media type",
        )),
    }
}

fn validate_audio_params(format: &str, sample_rate: u32) -> ProtocolResultValue<()> {
    const SAMPLE_RATES: [u32; 7] = [8000, 16000, 22050, 24000, 32000, 44100, 48000];
    if !SAMPLE_RATES.contains(&sample_rate) || (format == "ogg_opus" && sample_rate != 48000) {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Doubao TTS does not support the requested audio sample rate",
        ));
    }
    Ok(())
}

fn speech_rate(speed: f64) -> ProtocolResultValue<i64> {
    if !speed.is_finite() || !(0.5..=2.0).contains(&speed) {
        return Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            "Doubao TTS speed must be between 0.5 and 2.0",
        ));
    }
    Ok(((speed - 1.0) * 100.0).round() as i64)
}

fn explicit_language(language: &str) -> ProtocolResultValue<&'static str> {
    match language.to_ascii_lowercase().as_str() {
        "zh" | "zh-cn" => Ok("zh-cn"),
        "en" | "en-us" | "en-gb" => Ok("en"),
        "ja" | "ja-jp" => Ok("ja"),
        "es" | "es-es" => Ok("es-es"),
        "es-mx" => Ok("es-mx"),
        "id" | "id-id" => Ok("id"),
        "pt" | "pt-pt" => Ok("pt"),
        "pt-br" => Ok("pt-br"),
        "ko" | "ko-kr" => Ok("ko"),
        "it" | "it-it" => Ok("it"),
        "de" | "de-de" => Ok("de"),
        "fr" | "fr-fr" => Ok("fr"),
        "th" | "th-th" => Ok("th"),
        "vi" | "vi-vn" => Ok("vi"),
        "ru" | "ru-ru" => Ok("ru"),
        "fil" | "fil-ph" => Ok("fil"),
        "ms" | "ms-my" => Ok("ms"),
        "ar" | "ar-sa" => Ok("ar"),
        "pl" | "pl-pl" => Ok("pl"),
        "tr" | "tr-tr" => Ok("tr"),
        "sv" | "sv-se" => Ok("sv"),
        "nl" | "nl-nl" => Ok("nl"),
        "no" | "nb-no" | "nn-no" => Ok("no"),
        "uk" | "uk-ua" => Ok("uk"),
        "fi" | "fi-fi" => Ok("fi"),
        "da" | "da-dk" => Ok("da"),
        "cs" | "cs-cz" => Ok("cs"),
        "hu" | "hu-hu" => Ok("hu"),
        "el" | "el-gr" => Ok("el"),
        "ro" | "ro-ro" => Ok("ro"),
        "hi" | "hi-in" => Ok("hi"),
        _ => Err(ProtocolError::new(
            ProtocolErrorKind::UnsupportedOperation,
            format!("Doubao TTS does not support language `{language}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, MaterializedResource,
        ResolvedCredential,
    };
    use buckyos_api::{
        AudioSpeechRecognitionRequest, AudioTextToSpeechRequest, ProviderStateCoordinate,
    };
    use bytes::Bytes;
    use reqwest::{header::HeaderMap, StatusCode};
    use std::collections::BTreeMap;
    use std::time::Duration;

    const ASR_BASE_URL: &str = "https://openspeech.bytedance.com/api/v3/auc/bigmodel";

    fn context(base_url: &str) -> CodecContext {
        CodecContext {
            base_url: base_url.to_owned(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "doubao-speech".to_owned(),
                adapter_type: DOUBAO_SPEECH_ADAPTER_ID.to_owned(),
                origin_provider: "doubao".to_owned(),
                origin_model: "doubao-seed-asr-2.0".to_owned(),
            },
            credential: Some(ResolvedCredential::bearer("secret://ark", "secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(10),
                max_request_bytes: ASR_FLASH_MAX_REQUEST_BYTES,
                max_response_bytes: MAX_RESPONSE_BYTES,
            },
        }
    }

    fn input(wire_profile: &str) -> CodecInput {
        input_with_request(
            wire_profile,
            json!({
                "exact_model": "doubao-seed-tts-2.0@doubao-main",
                "text": "hello",
                "voice": {"language": "zh-CN"}
            }),
        )
    }

    fn input_with_request(wire_profile: &str, request: Value) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::AudioTextToSpeech(
                AudioTextToSpeechRequest::from_json(request).unwrap(),
            ),
            resolved_parameters: BTreeMap::from([
                ("provider_model_id".to_owned(), json!("doubao-seed-tts-2.0")),
                ("speaker".to_owned(), json!("zh_female_test")),
                ("doubao_tts_wire_profile".to_owned(), json!(wire_profile)),
                ("doubao_tts_resource_id".to_owned(), json!("seed-tts-2.0")),
            ]),
        }
    }

    fn asr_input(request: Value, model: &str) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::AudioSpeechRecognition(
                AudioSpeechRecognitionRequest::from_json(request).unwrap(),
            ),
            resolved_parameters: BTreeMap::from([("provider_model_id".to_owned(), json!(model))]),
        }
    }

    fn speech_registry() -> CodecRegistry {
        let (descriptor, registration) = doubao_speech_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        registry
    }

    #[test]
    fn asr_diarization_requests_utterances_without_timestamps() {
        let request = AudioSpeechRecognitionRequest::from_json(json!({
            "exact_model": "doubao-seed-asr-2.0-fast@doubao-main",
            "audio": ResourceRef::url("https://cdn.example.com/a.mp3".to_owned(), None),
            "diarization": true
        }))
        .unwrap();
        let options = asr_request_options(&request).unwrap();
        assert_eq!(options.get("enable_speaker_info"), Some(&json!(true)));
        assert_eq!(options.get("show_utterances"), Some(&json!(true)));

        let request = AudioSpeechRecognitionRequest::from_json(json!({
            "exact_model": "doubao-seed-asr-2.0-fast@doubao-main",
            "audio": ResourceRef::url("https://cdn.example.com/a.mp3".to_owned(), None),
            "diarization": false
        }))
        .unwrap();
        let options = asr_request_options(&request).unwrap();
        assert_eq!(options.get("enable_speaker_info"), Some(&json!(false)));
        assert!(!options.contains_key("show_utterances"));
    }

    #[test]
    fn standard_and_agent_plan_tts_headers_follow_configured_wire_profile() {
        let registry = speech_registry();

        let standard = registry
            .encode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_TTS_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                &input("standard"),
                &context("https://openspeech.bytedance.com/api/v3/tts"),
            )
            .unwrap();
        assert_eq!(
            standard
                .headers
                .get("x-control-require-usage-tokens-return")
                .unwrap(),
            "*"
        );
        assert!(standard.headers.contains_key("x-api-request-id"));
        assert!(!standard
            .headers
            .contains_key("x-control-request-usage-tokens"));

        let agent_plan = registry
            .encode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_TTS_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                &input("agent_plan"),
                &context("https://openspeech.bytedance.com/api/v3/plan/tts"),
            )
            .unwrap();
        assert_eq!(
            agent_plan
                .headers
                .get("x-control-request-usage-tokens")
                .unwrap(),
            "true"
        );
        assert_eq!(
            agent_plan
                .headers
                .get("x-control-require-usage-tokens-return")
                .unwrap(),
            "*"
        );
        assert!(!agent_plan.headers.contains_key("x-api-request-id"));
    }

    #[test]
    fn standard_tts_lowers_supported_canonical_parameters() {
        let registry = speech_registry();
        let request = registry
            .encode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_TTS_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                &input_with_request(
                    "standard",
                    json!({
                        "exact_model": "doubao-seed-tts-2.0@doubao-main",
                        "text": "hello",
                        "voice": {
                            "language": "en-US",
                            "instructions": "Speak calmly"
                        },
                        "speed": 1.25,
                        "output": {"media_type": "audio/ogg", "sample_rate": 48000}
                    }),
                ),
                &context("https://openspeech.bytedance.com/api/v3/tts"),
            )
            .unwrap();
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON request body");
        };
        assert_eq!(body["req_params"]["audio_params"]["format"], "ogg_opus");
        assert_eq!(body["req_params"]["audio_params"]["sample_rate"], 48000);
        assert_eq!(body["req_params"]["audio_params"]["speech_rate"], 25);
        assert_eq!(body["req_params"]["context_texts"], json!(["Speak calmly"]));
        assert_eq!(
            body["req_params"]["additions"],
            json!("{\"explicit_language\":\"en\"}")
        );
    }

    #[test]
    fn standard_tts_rejects_unsupported_speed_and_language() {
        assert!(speech_rate(0.49).is_err());
        assert!(speech_rate(2.01).is_err());
        assert!(explicit_language("eo").is_err());
        assert!(validate_audio_params("mp3", 24000).is_ok());
        assert!(validate_audio_params("ogg_opus", 48000).is_ok());
        assert!(validate_audio_params("mp3", 12000).is_err());
        assert!(validate_audio_params("ogg_opus", 24000).is_err());
    }

    #[tokio::test]
    async fn error_response_keeps_doubao_tts_business_code() {
        let registry = speech_registry();
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = HttpResponse {
            status: StatusCode::UNAUTHORIZED,
            headers,
            body: Bytes::from_static(
                br#"{"header":{"reqid":"provider-request-1","code":45000010,"message":"Invalid X-Api-Key"}}"#,
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let error = registry
            .decode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_TTS_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                response,
            )
            .await
            .unwrap_err();
        assert_eq!(error.provider_code.as_deref(), Some("45000010"));
        assert_eq!(error.request_id.as_deref(), Some("provider-request-1"));
        assert!(error.to_string().contains("Invalid X-Api-Key"));
    }

    #[tokio::test]
    async fn decodes_official_text_plain_json_line_audio_chunks() {
        let registry = speech_registry();
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        let response = HttpResponse {
            status: StatusCode::OK,
            headers,
            body: Bytes::from_static(
                b"{\"code\":0,\"message\":\"\",\"data\":\"SUQz\"}\n{\"code\":0,\"message\":\"\",\"data\":\"BA==\"}\n{\"code\":20000000,\"message\":\"OK\",\"data\":null}\n",
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let result = registry
            .decode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_TTS_OPERATION_ID,
                ApiType::AudioTextToSpeech,
                response,
            )
            .await
            .unwrap();
        let ProtocolExecution::Immediate(output) = result else {
            panic!("expected immediate TTS output");
        };
        assert_eq!(output.artifacts.len(), 1);
        assert_eq!(output.usage.unwrap().request_units, Some(1));
    }

    #[test]
    fn flash_asr_inlines_bytes_and_accepts_the_url_form() {
        let registry = speech_registry();
        let resource = ResourceRef::base64("audio/wav".to_owned(), STANDARD.encode(b"RIFFdata"));
        // The codec looks resources up by the canonical `ResourceKey`; reuse the
        // key the materialization stage would have published.
        let mut base = context(ASR_BASE_URL);
        base.resources.insert(
            crate::resource::ResourceKey::from_ref(&resource)
                .as_str()
                .to_owned(),
            MaterializedResource::new(Bytes::from_static(b"RIFFdata"), "audio/wav", None).unwrap(),
        );
        let request = registry
            .encode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_FLASH_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &asr_input(
                    json!({
                        "exact_model": "doubao-seed-asr-flash@doubao-main",
                        "audio": resource,
                        "language": "zh-CN",
                        "timestamps": "segment",
                        "diarization": true
                    }),
                    "doubao-seed-asr-flash",
                ),
                &base,
            )
            .unwrap();
        assert_eq!(request.url, format!("{ASR_BASE_URL}/recognize/flash"));
        assert_eq!(
            request.headers["x-api-resource-id"],
            "volc.bigasr.auc_turbo"
        );
        assert_eq!(request.headers["x-api-sequence"], "-1");
        let HttpBody::Json(body) = request.body else {
            panic!("expected JSON request body");
        };
        assert_eq!(body["audio"]["data"], STANDARD.encode(b"RIFFdata"));
        assert_eq!(body["audio"]["format"], "wav");
        assert_eq!(body["audio"]["language"], "zh-CN");
        assert_eq!(body["request"]["model_name"], "bigmodel");
        assert_eq!(body["request"]["show_utterances"], true);
        assert_eq!(body["request"]["enable_speaker_info"], true);
        assert!(body["user"]["uid"].is_string());

        let url_form = registry
            .encode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_FLASH_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &asr_input(
                    json!({
                        "exact_model": "doubao-seed-asr-flash@doubao-main",
                        "audio": ResourceRef::url("https://cdn.example.com/a.mp3".to_owned(), None)
                    }),
                    "doubao-seed-asr-flash",
                ),
                &context(ASR_BASE_URL),
            )
            .unwrap();
        let HttpBody::Json(body) = url_form.body else {
            panic!("expected JSON request body");
        };
        assert_eq!(body["audio"]["url"], "https://cdn.example.com/a.mp3");
        assert_eq!(body["audio"]["format"], "mp3");
        assert!(body["audio"]["data"].is_null());
    }

    #[tokio::test]
    async fn flash_asr_decodes_official_result_and_usage() {
        let registry = speech_registry();
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-status-code"),
            HeaderValue::from_static("20000000"),
        );
        headers.insert(
            HeaderName::from_static("x-api-message"),
            HeaderValue::from_static("OK"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = HttpResponse {
            status: StatusCode::OK,
            headers,
            body: Bytes::from_static(
                br#"{"audio_info":{"duration":2663},"result":{"text":"hello","utterances":[{"text":"hello","start_time":280,"end_time":2400,"additions":{"speaker":"1"}}]}}"#,
            ),
            request_id: "request-1".to_owned(),
            retry_after: None,
        };
        let result = registry
            .decode(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_FLASH_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                response,
            )
            .await
            .unwrap();
        let ProtocolExecution::Immediate(output) = result else {
            panic!("expected immediate ASR output");
        };
        assert_eq!(output.value["text"], "hello");
        assert_eq!(output.value["segments"][0]["start_seconds"], 0.28);
        assert_eq!(output.value["segments"][0]["end_seconds"], 2.4);
        assert_eq!(output.value["segments"][0]["speaker"], "1");
        assert_eq!(output.value["segments"][0]["id"], "0");
        assert_eq!(output.usage.unwrap().audio_seconds, Some(2.663));
    }

    #[tokio::test]
    async fn task_asr_submits_a_url_and_round_trips_the_chosen_task_id() {
        let registry = speech_registry();
        let request = asr_input(
            json!({
                "exact_model": "doubao-seed-asr-2.0@doubao-main",
                "audio": ResourceRef::url("https://ndn.example.com/ndn/cyfile:aa".to_owned(), Some("audio/mpeg".to_owned()))
            }),
            "doubao-seed-asr-2.0",
        );
        let submit = registry
            .encode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Submit,
                    remote_task_id: None,
                    codec_input: Some(&request),
                    resolved_parameters: &request.resolved_parameters,
                    context: &context(ASR_BASE_URL),
                },
            )
            .unwrap();
        assert_eq!(submit.url, format!("{ASR_BASE_URL}/submit"));
        assert_eq!(submit.headers["x-api-resource-id"], "volc.seedasr.auc");
        assert_eq!(submit.headers["x-api-sequence"], "-1");
        let chosen_id = submit.headers["x-api-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        // The transport echoes the request's own request-ID header; the codec
        // sets it to the same value so the empty submit body can still yield the
        // task ID.
        assert_eq!(submit.headers["x-request-id"], chosen_id);
        let HttpBody::Json(body) = &submit.body else {
            panic!("expected JSON request body");
        };
        assert_eq!(
            body["audio"]["url"],
            "https://ndn.example.com/ndn/cyfile:aa"
        );
        assert_eq!(body["audio"]["format"], "mp3");
        assert!(body["audio"]["data"].is_null());
        assert_eq!(body["request"]["model_name"], "bigmodel");

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-status-code"),
            HeaderValue::from_static("20000000"),
        );
        headers.insert(
            HeaderName::from_static("x-api-message"),
            HeaderValue::from_static("OK"),
        );
        let encoded_id = chosen_id.clone();
        let submitted = registry
            .decode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                NativeTaskOperation::Submit,
                HttpResponse {
                    status: StatusCode::OK,
                    headers,
                    body: Bytes::new(),
                    request_id: encoded_id,
                    retry_after: None,
                },
            )
            .await
            .unwrap();
        let NativeTaskOutput::Submitted(handle) = submitted else {
            panic!("expected a submitted native task");
        };
        assert_eq!(handle.remote_task_id, chosen_id);
        assert_eq!(handle.poll_after, Some(ASR_TASK_POLL_INTERVAL));
        assert!(!handle.cancel_supported);
    }

    #[tokio::test]
    async fn task_asr_polls_with_the_same_task_id_and_decodes_the_result() {
        let registry = speech_registry();
        let parameters =
            BTreeMap::from([("provider_model_id".to_owned(), json!("doubao-seed-asr-2.0"))]);
        let query = registry
            .encode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Status,
                    remote_task_id: Some("67ee89ba-7050-4c04-a3d7-ac61a63499b3"),
                    codec_input: None,
                    resolved_parameters: &parameters,
                    context: &context(ASR_BASE_URL),
                },
            )
            .unwrap();
        assert_eq!(query.url, format!("{ASR_BASE_URL}/query"));
        assert_eq!(
            query.headers["x-api-request-id"],
            "67ee89ba-7050-4c04-a3d7-ac61a63499b3"
        );
        assert!(!query.headers.contains_key("x-api-sequence"));
        let HttpBody::Json(body) = query.body else {
            panic!("expected JSON request body");
        };
        assert_eq!(body, json!({}));

        let status = |code: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                HeaderName::from_static("x-api-status-code"),
                HeaderValue::from_static(code),
            );
            headers.insert(
                HeaderName::from_static("x-api-message"),
                HeaderValue::from_static("OK"),
            );
            HttpResponse {
                status: StatusCode::OK,
                headers,
                body: Bytes::new(),
                request_id: "request-1".to_owned(),
                retry_after: None,
            }
        };
        let running = registry
            .decode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                NativeTaskOperation::Status,
                status("20000001"),
            )
            .await
            .unwrap();
        assert!(matches!(
            running,
            NativeTaskOutput::Status {
                state: NativeTaskState::Running,
                ..
            }
        ));
        let queued = registry
            .decode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                NativeTaskOperation::Status,
                status("20000002"),
            )
            .await
            .unwrap();
        assert!(matches!(
            queued,
            NativeTaskOutput::Status {
                state: NativeTaskState::Queued,
                ..
            }
        ));
        let finished = registry
            .decode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                NativeTaskOperation::Status,
                status("20000000"),
            )
            .await
            .unwrap();
        assert!(matches!(
            finished,
            NativeTaskOutput::Status {
                state: NativeTaskState::Succeeded,
                result_ref: None,
                ..
            }
        ));

        let failed = registry
            .decode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                NativeTaskOperation::Status,
                status("45000002"),
            )
            .await
            .unwrap_err();
        assert_eq!(failed.provider_code.as_deref(), Some("45000002"));

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-status-code"),
            HeaderValue::from_static("20000000"),
        );
        headers.insert(
            HeaderName::from_static("x-api-message"),
            HeaderValue::from_static("OK"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let result = registry
            .decode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                NativeTaskOperation::Result,
                HttpResponse {
                    status: StatusCode::OK,
                    headers,
                    body: Bytes::from_static(
                        br#"{"audio_info":{"duration":10000},"result":{"text":"\u8fd9\u662f\u5b57\u8282","utterances":[{"text":"\u8fd9\u662f","start_time":0,"end_time":1705}]}}"#,
                    ),
                    request_id: "request-1".to_owned(),
                    retry_after: None,
                },
            )
            .await
            .unwrap();
        let NativeTaskOutput::Result(output) = result else {
            panic!("expected a native task result");
        };
        assert_eq!(output.value["text"], "这是字节");
        assert_eq!(output.value["segments"][0]["end_seconds"], 1.705);
    }

    #[test]
    fn task_asr_refuses_to_inline_audio_bytes() {
        let registry = speech_registry();
        let resource = ResourceRef::base64("audio/mpeg".to_owned(), STANDARD.encode(b"ID3"));
        let mut base = context(ASR_BASE_URL);
        base.resources.insert(
            crate::resource::ResourceKey::from_ref(&resource)
                .as_str()
                .to_owned(),
            MaterializedResource::new(Bytes::from_static(b"ID3"), "audio/mpeg", None).unwrap(),
        );
        let request = asr_input(
            json!({
                "exact_model": "doubao-seed-asr-2.0@doubao-main",
                "audio": resource
            }),
            "doubao-seed-asr-2.0",
        );
        let error = registry
            .encode_native(
                DOUBAO_SPEECH_ADAPTER_ID,
                DOUBAO_ASR_TASK_OPERATION_ID,
                ApiType::AudioSpeechRecognition,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Submit,
                    remote_task_id: None,
                    codec_input: Some(&request),
                    resolved_parameters: &request.resolved_parameters,
                    context: &base,
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("requires an absolute audio URL"));
    }

    #[test]
    fn asr_language_and_format_hints_cover_the_documented_values() {
        assert_eq!(asr_language("zh"), Some("zh-CN"));
        assert_eq!(asr_language("EN_us"), Some("en-US"));
        assert_eq!(asr_language("klingon"), None);
        assert_eq!(asr_audio_format(Some("audio/mpeg"), None), Some("mp3"));
        assert_eq!(asr_audio_format(None, Some("clip.WAV")), Some("wav"));
        assert_eq!(
            asr_audio_format(None, Some("https://host/a.opus?x=1")),
            Some("ogg")
        );
        assert_eq!(
            asr_audio_format(None, Some("https://host/ndn/cyfile:aa")),
            None
        );
    }
}
