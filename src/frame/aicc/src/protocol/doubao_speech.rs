use super::{
    AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration, ExecutionMode, HttpBody,
    HttpRequest, HttpResponse, OperationBinding, OperationCodec, OperationDescriptor,
    ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput, ProtocolResultValue,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{AiArtifact, AiUsage, AiccCall, ApiType, ResourceRef};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, Url};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(crate) const DOUBAO_SPEECH_ADAPTER_ID: &str = "doubao-speech";
pub(crate) const DOUBAO_TTS_OPERATION_ID: &str = "tts.unidirectional";
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

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
        if request.speed.is_some() {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "Doubao Agent Plan TTS does not expose canonical speed control",
            ));
        }
        let speaker = call
            .input
            .resolved_parameters
            .get("speaker")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProtocolError::new(
                    ProtocolErrorKind::UnsupportedOperation,
                    "Doubao Agent Plan TTS requires a resolved speaker",
                )
            })?;
        let mut audio = Map::from_iter([
            ("format".to_owned(), json!("mp3")),
            ("sample_rate".to_owned(), json!(24000)),
        ]);
        if let Some(output) = &request.output {
            if let Some(media_type) = &output.media_type {
                audio.insert("format".to_owned(), json!(audio_format(media_type)?));
            }
            if let Some(sample_rate) = output.sample_rate {
                audio.insert("sample_rate".to_owned(), json!(sample_rate));
            }
        }
        let body = json!({
            "req_params": {
                "text": request.text,
                "speaker": speaker,
                "audio_params": Value::Object(audio)
            }
        });
        let mut url = Url::parse(&call.context.base_url)
            .map_err(|_| ProtocolError::invalid_configuration("Doubao TTS base URL is invalid"))?;
        let base = url.path().trim_end_matches('/');
        url.set_path(&format!("{base}/unidirectional"));
        let mut wire = HttpRequest::new(Method::POST, url.to_string());
        wire.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        wire.headers.insert(
            HeaderName::from_static("x-api-resource-id"),
            HeaderValue::from_static("seed-tts-2.0"),
        );
        wire.headers.insert(
            HeaderName::from_static("x-control-request-usage-tokens"),
            HeaderValue::from_static("true"),
        );
        apply_speech_credential(&mut wire.headers, call)?;
        wire.body = HttpBody::Json(body);
        wire.timeout = Some(call.context.limits.request_timeout);
        wire.max_request_bytes = Some(call.context.limits.max_request_bytes);
        wire.max_response_bytes = Some(call.context.limits.max_response_bytes);
        Ok(wire)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
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
                    ProtocolError::invalid_response(
                        "Doubao TTS response contains malformed JSON lines",
                    )
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
            let code = frame.get("code").and_then(Value::as_i64).unwrap_or(-1);
            if !response.status.is_success() || !matches!(code, 0 | 20_000_000) {
                let message = frame
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Doubao TTS request failed");
                return Err(ProtocolError::new(
                    super::protocol_error_kind_from_http_status(response.status),
                    message,
                )
                .with_provider_code((code >= 0).then(|| code.to_string()))
                .with_http_status(response.status.as_u16())
                .with_request_id(Some(response.request_id.clone()))
                .with_retry_after(response.retry_after));
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
                    ProtocolError::invalid_response(
                        "Doubao TTS response contains invalid base64 audio",
                    )
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
        let mime = "audio/mpeg".to_owned();
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
            "Doubao Agent Plan TTS does not support the requested audio media type",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use reqwest::{header::HeaderMap, StatusCode};

    #[tokio::test]
    async fn error_response_keeps_doubao_tts_business_code() {
        let (descriptor, registration) = doubao_speech_adapter();
        let mut registry = super::super::CodecRegistry::default();
        registry.register_codecs(descriptor, registration).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = HttpResponse {
            status: StatusCode::BAD_REQUEST,
            headers,
            body: Bytes::from_static(br#"{"code":40000000,"message":"invalid request"}"#),
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
        assert_eq!(error.provider_code.as_deref(), Some("40000000"));
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
