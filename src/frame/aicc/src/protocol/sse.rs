use super::{ProtocolError, ProtocolErrorKind, ProtocolResultValue};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
