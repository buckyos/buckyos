use super::{ProtocolError, ProtocolErrorKind, ProtocolResultValue};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Method, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(crate) struct HttpTransportConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub proxy: Option<String>,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_json_bytes: usize,
    pub request_id_header: HeaderName,
}

impl std::fmt::Debug for HttpTransportConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpTransportConfig")
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("proxy_configured", &self.proxy.is_some())
            .field("max_request_bytes", &self.max_request_bytes)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("max_json_bytes", &self.max_json_bytes)
            .field("request_id_header", &self.request_id_header)
            .finish()
    }
}

impl Default for HttpTransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(120),
            proxy: None,
            max_request_bytes: 32 * 1024 * 1024,
            max_response_bytes: 32 * 1024 * 1024,
            max_json_bytes: 8 * 1024 * 1024,
            request_id_header: HeaderName::from_static("x-request-id"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MultipartPart {
    pub name: String,
    pub bytes: Bytes,
    pub file_name: Option<String>,
    pub mime: Option<String>,
}

impl std::fmt::Debug for MultipartPart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MultipartPart")
            .field("name", &self.name)
            .field("byte_len", &self.bytes.len())
            .field("file_name", &self.file_name)
            .field("mime", &self.mime)
            .finish()
    }
}

impl MultipartPart {
    pub(crate) fn bytes(name: impl Into<String>, bytes: impl Into<Bytes>) -> Self {
        Self {
            name: name.into(),
            bytes: bytes.into(),
            file_name: None,
            mime: None,
        }
    }

    pub(crate) fn file(
        name: impl Into<String>,
        bytes: impl Into<Bytes>,
        file_name: impl Into<String>,
        mime: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            bytes: bytes.into(),
            file_name: Some(file_name.into()),
            mime: Some(mime.into()),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MultipartBody {
    parts: Vec<MultipartPart>,
    max_parts: usize,
    max_bytes: usize,
    total_bytes: usize,
}

impl std::fmt::Debug for MultipartBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MultipartBody")
            .field("part_count", &self.parts.len())
            .field("max_parts", &self.max_parts)
            .field("max_bytes", &self.max_bytes)
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

impl MultipartBody {
    pub(crate) fn new(max_parts: usize, max_bytes: usize) -> ProtocolResultValue<Self> {
        if max_parts == 0 || max_bytes == 0 {
            return Err(ProtocolError::invalid_configuration(
                "multipart limits must be greater than zero",
            ));
        }
        Ok(Self {
            parts: Vec::new(),
            max_parts,
            max_bytes,
            total_bytes: 0,
        })
    }

    pub(crate) fn push(&mut self, part: MultipartPart) -> ProtocolResultValue<()> {
        if part.name.trim().is_empty() {
            return Err(ProtocolError::invalid_request(
                "multipart part name must not be empty",
            ));
        }
        if self.parts.len() >= self.max_parts {
            return Err(ProtocolError::invalid_request(
                "multipart part count exceeds configured limit",
            ));
        }
        let total = self
            .total_bytes
            .checked_add(part.bytes.len())
            .ok_or_else(|| ProtocolError::invalid_request("multipart body size overflow"))?;
        if total > self.max_bytes {
            return Err(ProtocolError::invalid_request(
                "multipart body exceeds configured byte limit",
            ));
        }
        self.total_bytes = total;
        self.parts.push(part);
        Ok(())
    }

    pub(crate) fn parts(&self) -> &[MultipartPart] {
        &self.parts
    }

    pub(crate) fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn into_form(self) -> ProtocolResultValue<reqwest::multipart::Form> {
        let mut form = reqwest::multipart::Form::new();
        for part in self.parts {
            let mut wire_part = reqwest::multipart::Part::bytes(part.bytes.to_vec());
            if let Some(file_name) = part.file_name {
                wire_part = wire_part.file_name(file_name);
            }
            if let Some(mime) = part.mime {
                wire_part = wire_part.mime_str(&mime).map_err(|_| {
                    ProtocolError::invalid_request("multipart MIME type is invalid")
                })?;
            }
            form = form.part(part.name, wire_part);
        }
        Ok(form)
    }
}

#[derive(Clone)]
pub(crate) enum HttpBody {
    Empty,
    Json(Value),
    Bytes {
        bytes: Bytes,
        content_type: Option<HeaderValue>,
    },
    Multipart(MultipartBody),
}

impl std::fmt::Debug for HttpBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("HttpBody::Empty"),
            Self::Json(_) => formatter.write_str("HttpBody::Json { value: [REDACTED] }"),
            Self::Bytes {
                bytes,
                content_type,
            } => formatter
                .debug_struct("HttpBody::Bytes")
                .field("byte_len", &bytes.len())
                .field("content_type_configured", &content_type.is_some())
                .finish(),
            Self::Multipart(body) => std::fmt::Debug::fmt(body, formatter),
        }
    }
}

#[derive(Clone)]
pub(crate) struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: HttpBody,
    pub timeout: Option<Duration>,
    pub max_request_bytes: Option<usize>,
    pub max_response_bytes: Option<usize>,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field(
                "header_names",
                &self
                    .headers
                    .keys()
                    .map(HeaderName::as_str)
                    .collect::<Vec<_>>(),
            )
            .field("body", &self.body)
            .field("timeout", &self.timeout)
            .field("max_request_bytes", &self.max_request_bytes)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

impl HttpRequest {
    pub(crate) fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: HeaderMap::new(),
            body: HttpBody::Empty,
            timeout: None,
            max_request_bytes: None,
            max_response_bytes: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub request_id: String,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpResponse")
            .field("status", &self.status)
            .field(
                "header_names",
                &self
                    .headers
                    .keys()
                    .map(HeaderName::as_str)
                    .collect::<Vec<_>>(),
            )
            .field("body_len", &self.body.len())
            .field("request_id", &self.request_id)
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

pub(crate) type HttpByteStream =
    Pin<Box<dyn futures_util::Stream<Item = ProtocolResultValue<Bytes>> + Send + 'static>>;

pub(crate) struct StreamingHttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: HttpByteStream,
    pub request_id: String,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Debug for StreamingHttpResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamingHttpResponse")
            .field("status", &self.status)
            .field(
                "header_names",
                &self
                    .headers
                    .keys()
                    .map(HeaderName::as_str)
                    .collect::<Vec<_>>(),
            )
            .field("body", &"<stream>")
            .field("request_id", &self.request_id)
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

impl HttpResponse {
    pub(crate) fn json<T: DeserializeOwned>(&self, max_bytes: usize) -> ProtocolResultValue<T> {
        decode_json(&self.body, max_bytes).map_err(|error| {
            error
                .with_request_id(Some(self.request_id.clone()))
                .with_retry_after(self.retry_after)
        })
    }
}

pub(crate) struct HttpTransport {
    client: reqwest::Client,
    config: HttpTransportConfig,
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpTransport")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    pub(crate) fn new(config: HttpTransportConfig) -> ProtocolResultValue<Self> {
        if config.connect_timeout.is_zero()
            || config.request_timeout.is_zero()
            || config.max_request_bytes == 0
            || config.max_response_bytes == 0
            || config.max_json_bytes == 0
        {
            return Err(ProtocolError::invalid_configuration(
                "HTTP timeouts and body limits must be greater than zero",
            ));
        }
        let mut builder = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout);
        if let Some(proxy) = &config.proxy {
            builder =
                builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| {
                    ProtocolError::invalid_configuration("HTTP proxy URL is invalid")
                })?);
        }
        let client = builder
            .build()
            .map_err(|_| ProtocolError::invalid_configuration("failed to construct HTTP client"))?;
        Ok(Self { client, config })
    }

    pub(crate) async fn send(&self, request: HttpRequest) -> ProtocolResultValue<HttpResponse> {
        let response = self.send_streaming(request).await?;
        let StreamingHttpResponse {
            status,
            headers,
            mut body,
            request_id,
            retry_after,
        } = response;
        let mut collected = BytesMut::new();
        while let Some(chunk) = body.next().await {
            collected.extend_from_slice(&chunk?);
        }
        Ok(HttpResponse {
            status,
            headers,
            body: collected.freeze(),
            request_id,
            retry_after,
        })
    }

    pub(crate) async fn send_streaming(
        &self,
        mut request: HttpRequest,
    ) -> ProtocolResultValue<StreamingHttpResponse> {
        let request_id =
            match request.headers.get(&self.config.request_id_header) {
                Some(value) => validate_request_id(value.to_str().map_err(|_| {
                    ProtocolError::invalid_request("request ID header is not UTF-8")
                })?)?,
                None => {
                    let value = generate_request_id();
                    request.headers.insert(
                        self.config.request_id_header.clone(),
                        HeaderValue::from_str(&value).map_err(|_| {
                            ProtocolError::invalid_request("generated request ID is invalid")
                        })?,
                    );
                    value
                }
            };

        let mut builder = self
            .client
            .request(request.method, &request.url)
            .headers(request.headers);
        if let Some(timeout) = request.timeout {
            if timeout.is_zero() {
                return Err(ProtocolError::invalid_request(
                    "request timeout must be greater than zero",
                ));
            }
            builder = builder.timeout(timeout);
        }
        let request_limit = request
            .max_request_bytes
            .unwrap_or(self.config.max_request_bytes)
            .min(self.config.max_request_bytes);
        if request_limit == 0 {
            return Err(ProtocolError::invalid_request(
                "request byte limit must be greater than zero",
            ));
        }
        builder = match request.body {
            HttpBody::Empty => builder,
            HttpBody::Json(value) => {
                let body = encode_json(&value, request_limit.min(self.config.max_json_bytes))?;
                builder.header(CONTENT_TYPE, "application/json").body(body)
            }
            HttpBody::Bytes {
                bytes,
                content_type,
            } => {
                if bytes.len() > request_limit {
                    return Err(ProtocolError::invalid_request(
                        "HTTP request exceeds configured byte limit",
                    ));
                }
                let builder = builder.body(bytes);
                match content_type {
                    Some(content_type) => builder.header(CONTENT_TYPE, content_type),
                    None => builder,
                }
            }
            HttpBody::Multipart(form) => {
                if form.total_bytes() > request_limit {
                    return Err(ProtocolError::invalid_request(
                        "HTTP request exceeds configured byte limit",
                    ));
                }
                builder.multipart(form.into_form()?)
            }
        };

        let response = builder.send().await.map_err(|error| {
            let kind = if error.is_timeout() {
                ProtocolErrorKind::Timeout
            } else {
                ProtocolErrorKind::Transport
            };
            ProtocolError::new(kind, "HTTP request failed")
                .with_request_id(Some(request_id.clone()))
        })?;
        let status = response.status();
        let headers = response.headers().clone();
        let response_request_id = headers
            .get(&self.config.request_id_header)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .unwrap_or(&request_id)
            .to_string();
        let retry_after = parse_retry_after(headers.get(RETRY_AFTER), SystemTime::now());
        let limit = request
            .max_response_bytes
            .unwrap_or(self.config.max_response_bytes)
            .min(self.config.max_response_bytes);
        if limit == 0 {
            return Err(ProtocolError::invalid_request(
                "response byte limit must be greater than zero",
            ));
        }
        if response
            .content_length()
            .is_some_and(|content_length| content_length > limit as u64)
        {
            return Err(ProtocolError::new(
                ProtocolErrorKind::ResponseTooLarge,
                "HTTP response exceeds configured byte limit",
            )
            .with_request_id(Some(response_request_id))
            .with_retry_after(retry_after));
        }
        let response_request_id_for_stream = response_request_id.clone();
        let stream =
            response
                .bytes_stream()
                .scan((0_usize, false), move |(received, ended), chunk| {
                    let request_id = response_request_id_for_stream.clone();
                    let result = {
                        if *ended {
                            None
                        } else if let Ok(chunk) = chunk {
                            if received.saturating_add(chunk.len()) > limit {
                                *ended = true;
                                Some(Err(ProtocolError::new(
                                    ProtocolErrorKind::ResponseTooLarge,
                                    "HTTP response exceeds configured byte limit",
                                )
                                .with_request_id(Some(request_id))
                                .with_retry_after(retry_after)))
                            } else {
                                *received += chunk.len();
                                Some(Ok(chunk))
                            }
                        } else {
                            *ended = true;
                            Some(Err(ProtocolError::new(
                                ProtocolErrorKind::Transport,
                                "HTTP response stream disconnected",
                            )
                            .with_request_id(Some(request_id))
                            .with_retry_after(retry_after)))
                        }
                    };
                    futures_util::future::ready(result)
                });
        Ok(StreamingHttpResponse {
            status,
            headers,
            body: Box::pin(stream),
            request_id: response_request_id,
            retry_after,
        })
    }
}

pub(crate) fn encode_json<T: Serialize>(
    value: &T,
    max_bytes: usize,
) -> ProtocolResultValue<Vec<u8>> {
    if max_bytes == 0 {
        return Err(ProtocolError::invalid_configuration(
            "JSON byte limit must be greater than zero",
        ));
    }
    let bytes = serde_json::to_vec(value)
        .map_err(|_| ProtocolError::invalid_request("JSON serialization failed"))?;
    if bytes.len() > max_bytes {
        return Err(ProtocolError::invalid_request(
            "JSON body exceeds configured byte limit",
        ));
    }
    Ok(bytes)
}

pub(crate) fn decode_json<T: DeserializeOwned>(
    bytes: &[u8],
    max_bytes: usize,
) -> ProtocolResultValue<T> {
    if max_bytes == 0 {
        return Err(ProtocolError::invalid_configuration(
            "JSON byte limit must be greater than zero",
        ));
    }
    if bytes.len() > max_bytes {
        return Err(ProtocolError::new(
            ProtocolErrorKind::ResponseTooLarge,
            "JSON body exceeds configured byte limit",
        ));
    }
    serde_json::from_slice(bytes)
        .map_err(|_| ProtocolError::invalid_response("JSON response is invalid"))
}

fn generate_request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("aicc-{nanos:x}-{sequence:x}")
}

fn validate_request_id(value: &str) -> ProtocolResultValue<String> {
    if value.is_empty() || value.len() > 256 {
        return Err(ProtocolError::invalid_request(
            "request ID must contain between 1 and 256 bytes",
        ));
    }
    Ok(value.to_string())
}

pub(crate) fn parse_retry_after(value: Option<&HeaderValue>, now: SystemTime) -> Option<Duration> {
    let value = value?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let target = parse_imf_fixdate(value)?;
    Some(target.duration_since(now).unwrap_or_default())
}

fn parse_imf_fixdate(value: &str) -> Option<SystemTime> {
    let mut fields = value.split_ascii_whitespace();
    let weekday = fields.next()?;
    if !weekday.ends_with(',') {
        return None;
    }
    let day = fields.next()?.parse::<u32>().ok()?;
    let month = match fields.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = fields.next()?.parse::<i32>().ok()?;
    let mut time = fields.next()?.split(':');
    let hour = time.next()?.parse::<u32>().ok()?;
    let minute = time.next()?.parse::<u32>().ok()?;
    let second = time.next()?.parse::<u32>().ok()?;
    if time.next().is_some() || fields.next()? != "GMT" || fields.next().is_some() {
        return None;
    }
    if !(1970..=9999).contains(&year)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    let seconds = (days as u64)
        .checked_mul(86_400)?
        .checked_add(hour as u64 * 3_600 + minute as u64 * 60 + second as u64)?;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let adjusted_year = year - i32::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146_097 + day_of_era - 719_468) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct StrictBody {
        ok: bool,
    }

    #[test]
    fn bounded_json_rejects_oversize_and_unknown_fields() {
        assert_eq!(
            decode_json::<StrictBody>(br#"{"ok":true}"#, 64).unwrap(),
            StrictBody { ok: true }
        );
        assert_eq!(
            decode_json::<StrictBody>(br#"{"ok":true}"#, 4)
                .unwrap_err()
                .kind,
            ProtocolErrorKind::ResponseTooLarge
        );
        assert_eq!(
            decode_json::<StrictBody>(br#"{"ok":true,"extra":1}"#, 64)
                .unwrap_err()
                .kind,
            ProtocolErrorKind::InvalidResponse
        );
    }

    #[test]
    fn multipart_enforces_count_and_size_before_network_io() {
        let mut body = MultipartBody::new(1, 4).unwrap();
        body.push(MultipartPart::bytes("input", Bytes::from_static(b"1234")))
            .unwrap();
        assert_eq!(
            body.push(MultipartPart::bytes("extra", Bytes::new()))
                .unwrap_err()
                .kind,
            ProtocolErrorKind::InvalidRequest
        );
    }

    #[test]
    fn transport_debug_redacts_proxy_headers_bodies_and_urls() {
        let config = HttpTransportConfig {
            proxy: Some("http://proxy-user:proxy-secret@example.invalid".to_string()),
            ..HttpTransportConfig::default()
        };
        assert!(!format!("{config:?}").contains("proxy-secret"));

        let mut request = HttpRequest::new(
            Method::POST,
            "https://example.invalid/?api_key=query-secret",
        );
        request.headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer header-secret"),
        );
        request.body = HttpBody::Json(serde_json::json!({"secret":"body-secret"}));
        let rendered = format!("{request:?}");
        for secret in ["query-secret", "header-secret", "body-secret"] {
            assert!(!rendered.contains(secret));
        }
    }

    #[tokio::test]
    async fn transport_rejects_oversized_request_before_network_io() {
        let transport = HttpTransport::new(HttpTransportConfig::default()).unwrap();
        let mut request = HttpRequest::new(Method::POST, "http://127.0.0.1:1/unreachable");
        request.max_request_bytes = Some(3);
        request.body = HttpBody::Bytes {
            bytes: Bytes::from_static(b"1234"),
            content_type: None,
        };
        assert_eq!(
            transport.send(request).await.unwrap_err().kind,
            ProtocolErrorKind::InvalidRequest
        );
    }

    #[test]
    fn retry_after_supports_delta_seconds_and_http_dates() {
        let now = UNIX_EPOCH + Duration::from_secs(1_447_849_550);
        assert_eq!(
            parse_retry_after(Some(&HeaderValue::from_static("120")), now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            parse_retry_after(
                Some(&HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT")),
                UNIX_EPOCH + Duration::from_secs(1_445_412_470),
            ),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            parse_retry_after(Some(&HeaderValue::from_static("not-a-date")), now),
            None
        );
    }

    #[tokio::test]
    async fn http_transport_propagates_request_id_retry_after_and_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            assert!(request.contains("x-request-id: client-request"));
            stream
                .write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\ncontent-length: 12\r\nretry-after: 3\r\nx-request-id: server-request\r\nconnection: close\r\n\r\nrate-limited",
                )
                .await
                .unwrap();
        });

        let transport = HttpTransport::new(HttpTransportConfig::default()).unwrap();
        let mut request = HttpRequest::new(Method::GET, format!("http://{address}/test"));
        request.headers.insert(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_static("client-request"),
        );
        let response = transport.send(request).await.unwrap();
        assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.request_id, "server-request");
        assert_eq!(response.retry_after, Some(Duration::from_secs(3)));
        assert_eq!(response.body, Bytes::from_static(b"rate-limited"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn streaming_transport_enforces_limit_without_buffering_entire_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let config = HttpTransportConfig {
            max_response_bytes: 6,
            ..HttpTransportConfig::default()
        };
        let transport = HttpTransport::new(config).unwrap();
        let mut response = transport
            .send_streaming(HttpRequest::new(
                Method::GET,
                format!("http://{address}/stream"),
            ))
            .await
            .unwrap();
        let mut received = 0;
        let mut saw_limit = false;
        while let Some(chunk) = response.body.next().await {
            match chunk {
                Ok(chunk) => received += chunk.len(),
                Err(error) => {
                    assert_eq!(error.kind, ProtocolErrorKind::ResponseTooLarge);
                    saw_limit = true;
                }
            }
        }
        assert!(received <= 6);
        assert!(saw_limit);
        assert!(response.body.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn per_request_timeout_is_classified_without_exposing_url() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let transport = HttpTransport::new(HttpTransportConfig::default()).unwrap();
        let mut request = HttpRequest::new(
            Method::GET,
            format!("http://{address}/?secret=query-secret"),
        );
        request.timeout = Some(Duration::from_millis(10));
        let error = transport.send(request).await.unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Timeout);
        assert!(!format!("{error:?}").contains("query-secret"));
        server.await.unwrap();
    }
}
