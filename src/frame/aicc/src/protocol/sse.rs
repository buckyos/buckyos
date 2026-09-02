use super::{ProtocolError, ProtocolErrorKind, ProtocolResultValue, StreamingHttpResponse};
use futures_util::{stream, Stream, StreamExt};
use reqwest::StatusCode;
use std::collections::VecDeque;
use std::pin::Pin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry_millis: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SseStreamEnd {
    EndOfStream,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SseFrame {
    Event(SseEvent),
    Terminated { marker: String },
    StreamEnd(SseStreamEnd),
}

pub(crate) type SseFrameStream =
    Pin<Box<dyn Stream<Item = ProtocolResultValue<SseFrame>> + Send + 'static>>;

#[derive(Debug, Clone)]
pub(crate) struct SseConfig {
    pub max_line_bytes: usize,
    pub max_event_bytes: usize,
    pub termination_markers: Vec<String>,
}

impl Default for SseConfig {
    fn default() -> Self {
        Self {
            max_line_bytes: 256 * 1024,
            max_event_bytes: 1024 * 1024,
            termination_markers: vec!["[DONE]".to_string()],
        }
    }
}

#[derive(Debug)]
pub(crate) struct SseFramer {
    config: SseConfig,
    pending: Vec<u8>,
    data_lines: Vec<String>,
    event: Option<String>,
    id: Option<String>,
    retry_millis: Option<u64>,
    event_bytes: usize,
    first_line: bool,
    terminated: bool,
}

impl SseFramer {
    pub(crate) fn new(config: SseConfig) -> ProtocolResultValue<Self> {
        if config.max_line_bytes == 0 || config.max_event_bytes == 0 {
            return Err(ProtocolError::invalid_configuration(
                "SSE line and event limits must be greater than zero",
            ));
        }
        if config
            .termination_markers
            .iter()
            .any(|marker| marker.is_empty() || marker.contains('\n') || marker.contains('\r'))
        {
            return Err(ProtocolError::invalid_configuration(
                "SSE termination markers must be non-empty single-line strings",
            ));
        }
        Ok(Self {
            config,
            pending: Vec::new(),
            data_lines: Vec::new(),
            event: None,
            id: None,
            retry_millis: None,
            event_bytes: 0,
            first_line: true,
            terminated: false,
        })
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> ProtocolResultValue<Vec<SseFrame>> {
        if self.terminated && !chunk.is_empty() {
            return Err(ProtocolError::invalid_response(
                "SSE bytes received after termination marker",
            ));
        }
        let mut frames = Vec::new();
        let mut start = 0;
        for (index, byte) in chunk.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            self.append_line_bytes(&chunk[start..index])?;
            let mut line = std::mem::take(&mut self.pending);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.consume_line(&line, &mut frames)?;
            start = index + 1;
            if self.terminated && start < chunk.len() {
                return Err(ProtocolError::invalid_response(
                    "SSE bytes received after termination marker",
                ));
            }
        }
        self.append_line_bytes(&chunk[start..])?;
        Ok(frames)
    }

    fn append_line_bytes(&mut self, bytes: &[u8]) -> ProtocolResultValue<()> {
        if self.pending.len().saturating_add(bytes.len()) > self.config.max_line_bytes {
            return Err(ProtocolError::new(
                ProtocolErrorKind::ResponseTooLarge,
                "SSE line exceeds configured byte limit",
            ));
        }
        self.pending.extend_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn finish(mut self, end: SseStreamEnd) -> ProtocolResultValue<Vec<SseFrame>> {
        let mut frames = Vec::new();
        if !self.terminated {
            if !self.pending.is_empty() {
                let line = std::mem::take(&mut self.pending);
                self.consume_line(&line, &mut frames)?;
            }
            self.dispatch(&mut frames)?;
            frames.push(SseFrame::StreamEnd(end));
        }
        Ok(frames)
    }

    fn consume_line(
        &mut self,
        raw_line: &[u8],
        frames: &mut Vec<SseFrame>,
    ) -> ProtocolResultValue<()> {
        if raw_line.len() > self.config.max_line_bytes {
            return Err(ProtocolError::new(
                ProtocolErrorKind::ResponseTooLarge,
                "SSE line exceeds configured byte limit",
            ));
        }
        let raw_line = if self.first_line {
            self.first_line = false;
            raw_line
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(raw_line)
        } else {
            raw_line
        };
        let line = std::str::from_utf8(raw_line)
            .map_err(|_| ProtocolError::invalid_response("SSE line is not valid UTF-8"))?;
        if line.is_empty() {
            return self.dispatch(frames);
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        self.event_bytes = self
            .event_bytes
            .checked_add(line.len())
            .ok_or_else(|| ProtocolError::invalid_response("SSE event size overflow"))?;
        if self.event_bytes > self.config.max_event_bytes {
            return Err(ProtocolError::new(
                ProtocolErrorKind::ResponseTooLarge,
                "SSE event exceeds configured byte limit",
            ));
        }
        match field {
            "data" => self.data_lines.push(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            "id" if !value.contains('\0') => self.id = Some(value.to_string()),
            "retry" => {
                if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
                    self.retry_millis = value.parse::<u64>().ok();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, frames: &mut Vec<SseFrame>) -> ProtocolResultValue<()> {
        if self.data_lines.is_empty() {
            self.reset_event();
            return Ok(());
        }
        let data = self.data_lines.join("\n");
        if self
            .config
            .termination_markers
            .iter()
            .any(|marker| marker == &data)
        {
            self.terminated = true;
            frames.push(SseFrame::Terminated { marker: data });
        } else {
            frames.push(SseFrame::Event(SseEvent {
                event: self.event.take(),
                data,
                id: self.id.clone(),
                retry_millis: self.retry_millis.take(),
            }));
        }
        self.reset_event();
        Ok(())
    }

    fn reset_event(&mut self) {
        self.data_lines.clear();
        self.event = None;
        self.retry_millis = None;
        self.event_bytes = 0;
    }
}

pub(crate) async fn sse_frame_stream(
    response: StreamingHttpResponse,
    config: SseConfig,
    max_response_bytes: usize,
) -> ProtocolResultValue<SseFrameStream> {
    if max_response_bytes == 0 {
        return Err(ProtocolError::invalid_configuration(
            "streaming response body limit must be greater than zero",
        ));
    }
    if !response.status.is_success() {
        let response = response
            .into_bounded_error_response(max_response_bytes)
            .await?;
        let kind = match response.status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProtocolErrorKind::Authentication,
            StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => ProtocolErrorKind::Timeout,
            _ => ProtocolErrorKind::InvalidResponse,
        };
        return Err(ProtocolError::new(
            kind,
            format!("upstream returned HTTP status {}", response.status.as_u16()),
        )
        .with_request_id(Some(response.request_id))
        .with_retry_after(response.retry_after));
    }
    let StreamingHttpResponse {
        status: _,
        headers: _,
        body,
        request_id,
        retry_after,
    } = response;

    struct State {
        body: super::HttpByteStream,
        framer: Option<SseFramer>,
        queued: VecDeque<SseFrame>,
        request_id: String,
        retry_after: Option<std::time::Duration>,
        received: usize,
        max_response_bytes: usize,
        finished: bool,
    }

    let state = State {
        body,
        framer: Some(SseFramer::new(config)?),
        queued: VecDeque::new(),
        request_id,
        retry_after,
        received: 0,
        max_response_bytes,
        finished: false,
    };
    let frames = stream::unfold(state, |mut state| async move {
        loop {
            if let Some(frame) = state.queued.pop_front() {
                return Some((Ok(frame), state));
            }
            if state.finished {
                return None;
            }
            match state.body.next().await {
                Some(Ok(chunk)) => {
                    state.received = state.received.saturating_add(chunk.len());
                    if state.received > state.max_response_bytes {
                        state.finished = true;
                        return Some((
                            Err(ProtocolError::new(
                                ProtocolErrorKind::ResponseTooLarge,
                                "streaming HTTP response exceeds configured byte limit",
                            )
                            .with_request_id(Some(state.request_id.clone()))
                            .with_retry_after(state.retry_after)),
                            state,
                        ));
                    }
                    let framed = state
                        .framer
                        .as_mut()
                        .expect("active SSE stream has a framer")
                        .push(&chunk);
                    match framed {
                        Ok(frames) => {
                            state.finished = frames
                                .iter()
                                .any(|frame| matches!(frame, SseFrame::Terminated { .. }));
                            state.queued.extend(frames);
                        }
                        Err(error) => {
                            state.finished = true;
                            return Some((
                                Err(enrich_stream_error(
                                    error,
                                    &state.request_id,
                                    state.retry_after,
                                )),
                                state,
                            ));
                        }
                    }
                }
                Some(Err(error)) => {
                    state.finished = true;
                    return Some((
                        Err(enrich_stream_error(
                            error,
                            &state.request_id,
                            state.retry_after,
                        )),
                        state,
                    ));
                }
                None => {
                    state.finished = true;
                    let framed = state
                        .framer
                        .take()
                        .expect("active SSE stream has a framer")
                        .finish(SseStreamEnd::EndOfStream);
                    match framed {
                        Ok(frames) => state.queued.extend(frames),
                        Err(error) => {
                            return Some((
                                Err(enrich_stream_error(
                                    error,
                                    &state.request_id,
                                    state.retry_after,
                                )),
                                state,
                            ));
                        }
                    }
                }
            }
        }
    });
    Ok(Box::pin(frames))
}

fn enrich_stream_error(
    error: ProtocolError,
    request_id: &str,
    retry_after: Option<std::time::Duration>,
) -> ProtocolError {
    let request_id = error
        .request_id
        .clone()
        .or_else(|| Some(request_id.to_string()));
    let retry_after = error.retry_after.or(retry_after);
    error
        .with_request_id(request_id)
        .with_retry_after(retry_after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::{stream, StreamExt};
    use reqwest::header::HeaderMap;
    use std::time::Duration;

    fn response(
        status: StatusCode,
        chunks: Vec<ProtocolResultValue<Bytes>>,
    ) -> StreamingHttpResponse {
        StreamingHttpResponse {
            status,
            headers: HeaderMap::new(),
            body: Box::pin(stream::iter(chunks)),
            request_id: "request-stream".to_string(),
            retry_after: Some(Duration::from_secs(3)),
        }
    }

    #[test]
    fn frames_fragmented_crlf_multiline_and_ignores_comments() {
        let mut framer = SseFramer::new(SseConfig::default()).unwrap();
        assert!(framer.push(b"\xef\xbb\xbf: ping\r\nda").unwrap().is_empty());
        let frames = framer
            .push(b"ta: first\r\ndata:second\r\nevent: delta\r\nid: 7\r\nretry: 50\r\n\r\n")
            .unwrap();
        assert_eq!(
            frames,
            vec![SseFrame::Event(SseEvent {
                event: Some("delta".to_string()),
                data: "first\nsecond".to_string(),
                id: Some("7".to_string()),
                retry_millis: Some(50),
            })]
        );
    }

    #[test]
    fn reports_termination_without_interpreting_business_events() {
        let mut framer = SseFramer::new(SseConfig::default()).unwrap();
        assert_eq!(
            framer.push(b"data: [DONE]\n\n").unwrap(),
            vec![SseFrame::Terminated {
                marker: "[DONE]".to_string()
            }]
        );
        assert!(framer.push(b"data: should-not-arrive\n\n").is_err());

        let mut same_chunk = SseFramer::new(SseConfig::default()).unwrap();
        assert!(same_chunk
            .push(b"data: [DONE]\n\ndata: should-not-arrive\n\n")
            .is_err());
    }

    #[test]
    fn emits_buffered_event_and_disconnect_reason_at_stream_end() {
        let mut framer = SseFramer::new(SseConfig::default()).unwrap();
        framer.push(b"data: partial").unwrap();
        assert_eq!(
            framer.finish(SseStreamEnd::Disconnected).unwrap(),
            vec![
                SseFrame::Event(SseEvent {
                    event: None,
                    data: "partial".to_string(),
                    id: None,
                    retry_millis: None,
                }),
                SseFrame::StreamEnd(SseStreamEnd::Disconnected)
            ]
        );
    }

    #[test]
    fn enforces_line_and_event_limits() {
        let config = SseConfig {
            max_line_bytes: 8,
            max_event_bytes: 12,
            termination_markers: Vec::new(),
        };
        let mut framer = SseFramer::new(config).unwrap();
        assert_eq!(
            framer.push(b"data: 123").unwrap_err().kind,
            ProtocolErrorKind::ResponseTooLarge
        );
    }

    #[test]
    fn event_id_persists_until_replaced() {
        let mut framer = SseFramer::new(SseConfig::default()).unwrap();
        let frames = framer
            .push(b"id: 7\ndata: first\n\ndata: second\n\n")
            .unwrap();
        assert_eq!(
            frames,
            vec![
                SseFrame::Event(SseEvent {
                    event: None,
                    data: "first".to_string(),
                    id: Some("7".to_string()),
                    retry_millis: None,
                }),
                SseFrame::Event(SseEvent {
                    event: None,
                    data: "second".to_string(),
                    id: Some("7".to_string()),
                    retry_millis: None,
                })
            ]
        );
    }

    #[tokio::test]
    async fn streaming_framer_is_incremental_and_done_is_normal_termination() {
        let mut frames = sse_frame_stream(
            response(
                StatusCode::OK,
                vec![
                    Ok(Bytes::from_static(b"event: delta\nda")),
                    Ok(Bytes::from_static(b"ta: one\n\ndata: [DONE]\n\n")),
                ],
            ),
            SseConfig::default(),
            1024,
        )
        .await
        .unwrap();
        assert_eq!(
            frames.next().await.unwrap().unwrap(),
            SseFrame::Event(SseEvent {
                event: Some("delta".to_string()),
                data: "one".to_string(),
                id: None,
                retry_millis: None,
            })
        );
        assert_eq!(
            frames.next().await.unwrap().unwrap(),
            SseFrame::Terminated {
                marker: "[DONE]".to_string()
            }
        );
        assert!(frames.next().await.is_none());
    }

    #[tokio::test]
    async fn streaming_reports_clean_eof_disconnect_malformed_and_limit() {
        let mut eof = sse_frame_stream(
            response(
                StatusCode::OK,
                vec![Ok(Bytes::from_static(b"data: final\n\n"))],
            ),
            SseConfig::default(),
            1024,
        )
        .await
        .unwrap();
        assert!(matches!(
            eof.next().await.unwrap().unwrap(),
            SseFrame::Event(_)
        ));
        assert_eq!(
            eof.next().await.unwrap().unwrap(),
            SseFrame::StreamEnd(SseStreamEnd::EndOfStream)
        );

        let disconnect = ProtocolError::new(ProtocolErrorKind::Transport, "disconnected");
        let mut disconnected = sse_frame_stream(
            response(StatusCode::OK, vec![Err(disconnect)]),
            SseConfig::default(),
            1024,
        )
        .await
        .unwrap();
        let error = disconnected.next().await.unwrap().unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::Transport);
        assert_eq!(error.request_id.as_deref(), Some("request-stream"));

        let mut malformed = sse_frame_stream(
            response(
                StatusCode::OK,
                vec![Ok(Bytes::from_static(b"data: \xff\n\n"))],
            ),
            SseConfig::default(),
            1024,
        )
        .await
        .unwrap();
        assert_eq!(
            malformed.next().await.unwrap().unwrap_err().kind,
            ProtocolErrorKind::InvalidResponse
        );

        let mut oversized = sse_frame_stream(
            response(StatusCode::OK, vec![Ok(Bytes::from_static(b"data: 12345"))]),
            SseConfig {
                max_line_bytes: 8,
                max_event_bytes: 32,
                termination_markers: Vec::new(),
            },
            1024,
        )
        .await
        .unwrap();
        assert_eq!(
            oversized.next().await.unwrap().unwrap_err().kind,
            ProtocolErrorKind::ResponseTooLarge
        );

        let mut total_oversized = sse_frame_stream(
            response(
                StatusCode::OK,
                vec![
                    Ok(Bytes::from_static(b"data")),
                    Ok(Bytes::from_static(b": more\n\n")),
                ],
            ),
            SseConfig::default(),
            6,
        )
        .await
        .unwrap();
        assert_eq!(
            total_oversized.next().await.unwrap().unwrap_err().kind,
            ProtocolErrorKind::ResponseTooLarge
        );
    }

    #[tokio::test]
    async fn non_success_body_is_bounded_and_preserves_metadata() {
        let error = sse_frame_stream(
            response(
                StatusCode::TOO_MANY_REQUESTS,
                vec![Ok(Bytes::from_static(b"rate limited"))],
            ),
            SseConfig::default(),
            1024,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidResponse);
        assert_eq!(error.request_id.as_deref(), Some("request-stream"));
        assert_eq!(error.retry_after, Some(Duration::from_secs(3)));

        let server_error = sse_frame_stream(
            response(
                StatusCode::SERVICE_UNAVAILABLE,
                vec![Ok(Bytes::from_static(b"unavailable"))],
            ),
            SseConfig::default(),
            1024,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(server_error.kind, ProtocolErrorKind::InvalidResponse);
        assert!(server_error.message.contains("503"));

        let too_large = sse_frame_stream(
            response(
                StatusCode::INTERNAL_SERVER_ERROR,
                vec![Ok(Bytes::from_static(b"too large"))],
            ),
            SseConfig::default(),
            3,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(too_large.kind, ProtocolErrorKind::ResponseTooLarge);
        assert_eq!(too_large.request_id.as_deref(), Some("request-stream"));
    }
}
