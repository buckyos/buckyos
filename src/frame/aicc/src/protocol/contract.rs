use super::{
    parse_retry_after, HttpBody, HttpRequest, HttpResponse, ProtocolError, ProtocolErrorKind,
    ProtocolResultValue, SseConfig, SseFrame, SseFramer, SseStreamEnd,
};
use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, RETRY_AFTER};
use reqwest::StatusCode;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GoldenBody {
    Empty,
    Json(Value),
    Bytes(Vec<u8>),
    Multipart(Vec<GoldenMultipartPart>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoldenMultipartPart {
    pub name: String,
    pub bytes: Vec<u8>,
    pub file_name: Option<String>,
    pub mime: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GoldenRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: GoldenBody,
    pub timeout: Option<Duration>,
    pub max_request_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GoldenError {
    pub kind: ProtocolErrorKind,
    pub message: String,
    pub request_id: Option<String>,
    pub retry_after: Option<Duration>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProtocolContractHarness {
    sse: SseConfig,
    sensitive_headers: Vec<HeaderName>,
}

impl ProtocolContractHarness {
    pub(crate) fn new(sse: SseConfig) -> Self {
        Self {
            sse,
            sensitive_headers: vec![AUTHORIZATION],
        }
    }

    pub(crate) fn redact_header(mut self, header: HeaderName) -> Self {
        if !self.sensitive_headers.contains(&header) {
            self.sensitive_headers.push(header);
        }
        self
    }

    pub(crate) fn request(&self, request: &HttpRequest) -> ProtocolResultValue<GoldenRequest> {
        let mut headers = BTreeMap::new();
        for (name, value) in &request.headers {
            let value = if self.sensitive_headers.contains(name) {
                REDACTED.to_string()
            } else {
                value
                    .to_str()
                    .map_err(|_| {
                        ProtocolError::invalid_request(
                            "golden request header contains non-UTF-8 bytes",
                        )
                    })?
                    .to_string()
            };
            headers.insert(name.as_str().to_string(), value);
        }
        let body = match &request.body {
            HttpBody::Empty => GoldenBody::Empty,
            HttpBody::Json(value) => GoldenBody::Json(value.clone()),
            HttpBody::Bytes { bytes, .. } => GoldenBody::Bytes(bytes.to_vec()),
            HttpBody::Multipart(body) => GoldenBody::Multipart(
                body.parts()
                    .iter()
                    .map(|part| GoldenMultipartPart {
                        name: part.name.clone(),
                        bytes: part.bytes.to_vec(),
                        file_name: part.file_name.clone(),
                        mime: part.mime.clone(),
                    })
                    .collect(),
            ),
        };
        Ok(GoldenRequest {
            method: request.method.to_string(),
            url: request.url.clone(),
            headers,
            body,
            timeout: request.timeout,
            max_request_bytes: request.max_request_bytes,
        })
    }

    pub(crate) fn response(
        &self,
        status: StatusCode,
        headers: &[(&str, &str)],
        body: impl Into<Bytes>,
        request_id: impl Into<String>,
        now: SystemTime,
    ) -> ProtocolResultValue<HttpResponse> {
        let mut header_map = HeaderMap::new();
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| ProtocolError::invalid_response("golden header name is invalid"))?;
            let value = HeaderValue::from_str(value)
                .map_err(|_| ProtocolError::invalid_response("golden header value is invalid"))?;
            header_map.append(name, value);
        }
        let retry_after = parse_retry_after(header_map.get(RETRY_AFTER), now);
        Ok(HttpResponse {
            status,
            headers: header_map,
            body: body.into(),
            request_id: request_id.into(),
            retry_after,
        })
    }

    pub(crate) fn sse(
        &self,
        chunks: &[&[u8]],
        end: SseStreamEnd,
    ) -> ProtocolResultValue<Vec<SseFrame>> {
        let mut framer = SseFramer::new(self.sse.clone())?;
        let mut frames = Vec::new();
        for chunk in chunks {
            frames.extend(framer.push(chunk)?);
        }
        frames.extend(framer.finish(end)?);
        Ok(frames)
    }

    pub(crate) fn error(&self, error: &ProtocolError) -> GoldenError {
        GoldenError {
            kind: error.kind,
            message: error.message.clone(),
            request_id: error.request_id.clone(),
            retry_after: error.retry_after,
        }
    }

    pub(crate) fn assert_no_secrets(
        &self,
        rendered: &str,
        secrets: &[&str],
    ) -> ProtocolResultValue<()> {
        if secrets
            .iter()
            .any(|secret| !secret.is_empty() && rendered.contains(secret))
        {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidResponse,
                "golden diagnostic contains credential material",
            ));
        }
        Ok(())
    }
}

impl Default for ProtocolContractHarness {
    fn default() -> Self {
        Self::new(SseConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{MultipartBody, MultipartPart, ResolvedCredential, SseEvent};
    use reqwest::Method;
    use serde_json::json;
    use std::time::UNIX_EPOCH;

    #[test]
    fn golden_request_response_sse_and_error_are_deterministic() {
        let harness =
            ProtocolContractHarness::default().redact_header(HeaderName::from_static("x-api-key"));
        let mut request = HttpRequest::new(Method::POST, "https://api.example/v1/responses");
        request.body = HttpBody::Json(json!({"model":"model-1","input":"hello"}));
        ResolvedCredential::named_header("secret://key", "x-api-key", "top-secret")
            .unwrap()
            .apply(&mut request.headers)
            .unwrap();
        let golden = harness.request(&request).unwrap();
        assert_eq!(golden.headers["x-api-key"], REDACTED);
        assert_eq!(
            golden.body,
            GoldenBody::Json(json!({"model":"model-1","input":"hello"}))
        );

        let response = harness
            .response(
                StatusCode::TOO_MANY_REQUESTS,
                &[("retry-after", "2")],
                Bytes::from_static(br#"{"error":"rate_limit"}"#),
                "request-1",
                UNIX_EPOCH,
            )
            .unwrap();
        assert_eq!(response.retry_after, Some(Duration::from_secs(2)));

        let frames = harness
            .sse(
                &[b"event: delta\nda".as_slice(), b"ta: one\n\n".as_slice()],
                SseStreamEnd::EndOfStream,
            )
            .unwrap();
        assert_eq!(
            frames,
            vec![
                SseFrame::Event(SseEvent {
                    event: Some("delta".to_string()),
                    data: "one".to_string(),
                    id: None,
                    retry_millis: None,
                }),
                SseFrame::StreamEnd(SseStreamEnd::EndOfStream)
            ]
        );

        let error = ProtocolError::new(ProtocolErrorKind::Transport, "wire failed")
            .with_request_id(Some("request-1".to_string()))
            .with_retry_after(Some(Duration::from_secs(2)));
        assert_eq!(
            harness.error(&error),
            GoldenError {
                kind: ProtocolErrorKind::Transport,
                message: "wire failed".to_string(),
                request_id: Some("request-1".to_string()),
                retry_after: Some(Duration::from_secs(2)),
            }
        );
        harness
            .assert_no_secrets(&format!("{golden:?} {error:?}"), &["top-secret"])
            .unwrap();
    }

    #[test]
    fn golden_multipart_preserves_part_order_and_metadata() {
        let mut multipart = MultipartBody::new(2, 16).unwrap();
        multipart
            .push(MultipartPart::bytes("prompt", Bytes::from_static(b"hello")))
            .unwrap();
        multipart
            .push(MultipartPart::file(
                "image",
                Bytes::from_static(b"png"),
                "input.png",
                "image/png",
            ))
            .unwrap();
        let mut request = HttpRequest::new(Method::POST, "https://api.example/upload");
        request.body = HttpBody::Multipart(multipart);
        let snapshot = ProtocolContractHarness::default()
            .request(&request)
            .unwrap();
        let GoldenBody::Multipart(parts) = snapshot.body else {
            panic!("expected multipart golden body")
        };
        assert_eq!(parts[0].name, "prompt");
        assert_eq!(parts[1].file_name.as_deref(), Some("input.png"));
        assert_eq!(parts[1].mime.as_deref(), Some("image/png"));
    }
}
