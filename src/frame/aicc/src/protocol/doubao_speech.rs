use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration, ExecutionMode, HttpBody,
    HttpRequest, HttpResponse, OperationBinding, OperationCodec, OperationDescriptor,
    ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput, ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use buckyos_api::{AiArtifact, AiUsage, AiccCall, ApiType, ResourceRef};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Url};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const DOUBAO_SPEECH_ADAPTER_ID: &str = "doubao-speech";
pub(crate) const DOUBAO_TTS_OPERATION_ID: &str = "tts.unidirectional";
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn doubao_speech_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let (operation, registration) = doubao_speech_registration();
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
            operations: [(operation.operation_id.clone(), operation)]
                .into_iter()
                .collect(),
        },
        registration,
    )
}

pub(super) fn doubao_speech_registration() -> (OperationDescriptor, CodecRegistration) {
    let operation = OperationDescriptor {
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
    let codec = Arc::new(DoubaoTtsCodec {
        descriptor: operation.clone(),
    }) as Arc<dyn OperationCodec>;
    (
        operation,
        CodecRegistration {
            operation_codecs: vec![codec],
            native_task_codecs: Vec::new(),
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
            }
            _ => {
                return Err(ProtocolError::invalid_configuration(
                    "Doubao TTS wire profile is missing or unsupported",
                ));
            }
        }
        apply_speech_credential(&mut wire.headers, call)?;
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
    call: &CodecCall<'_>,
) -> ProtocolResultValue<()> {
    let credential = call.context.credential.as_ref().ok_or_else(|| {
        ProtocolError::new(
            ProtocolErrorKind::Authentication,
            "Doubao TTS requires an API key",
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
            "Doubao TTS credential is invalid",
        )
    })?;
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(secret).map_err(|_| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Doubao TTS credential is invalid",
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
        CodecContext, CodecInput, CodecLimits, CodecRegistry, ResolvedCredential,
    };
    use buckyos_api::{AudioTextToSpeechRequest, ProviderStateCoordinate};
    use bytes::Bytes;
    use reqwest::{StatusCode, header::HeaderMap};
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn context(base_url: &str) -> CodecContext {
        CodecContext {
            base_url: base_url.to_owned(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "doubao".to_owned(),
                adapter_type: DOUBAO_SPEECH_ADAPTER_ID.to_owned(),
                origin_provider: "doubao".to_owned(),
                origin_model: "doubao-seed-tts-2.0".to_owned(),
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

    #[test]
    fn standard_and_agent_plan_tts_headers_follow_configured_wire_profile() {
        let (descriptor, registration) = doubao_speech_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();

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
        assert!(
            !standard
                .headers
                .contains_key("x-control-request-usage-tokens")
        );

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
        assert!(!agent_plan.headers.contains_key("x-api-request-id"));
    }

    #[test]
    fn standard_tts_lowers_supported_canonical_parameters() {
        let (descriptor, registration) = doubao_speech_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
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
        let (descriptor, registration) = doubao_speech_adapter();
        let mut registry = super::super::CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
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
        let (descriptor, registration) = doubao_speech_adapter();
        let mut registry = super::super::CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
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
}
