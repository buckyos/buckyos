use serde_json::{json, Value as Json};

use super::types::{Observation, ObservationSource};

pub struct Sanitizer;

impl Sanitizer {
    pub fn sanitize_observation(
        source: ObservationSource,
        name: &str,
        raw_json: Json,
        max_bytes: usize,
    ) -> Observation {
        let mut serialized = serde_json::to_string(&raw_json)
            .unwrap_or_else(|_| "{\"_err\":\"serialize_failed\"}".to_string());
        serialized = strip_ansi(&serialized);
        let (trimmed, truncated) = truncate_utf8(&serialized, max_bytes);

        Observation {
            source,
            name: name.to_string(),
            content: json!({
                "data": trimmed,
                "untrusted": true
            }),
            ok: true,
            truncated,
            bytes: trimmed.len(),
        }
    }

    pub fn tool_error_observation(name: &str, err: String, max_bytes: usize) -> Observation {
        let (trimmed, truncated) = truncate_utf8(&err, max_bytes);
        Observation {
            source: ObservationSource::Tool,
            name: name.to_string(),
            content: json!({
                "error": trimmed,
                "untrusted": true
            }),
            ok: false,
            truncated,
            bytes: err.len().min(max_bytes),
        }
    }

    pub fn format_observations(obs: &[Observation], max_bytes: usize) -> String {
        let value = serde_json::to_value(obs).unwrap_or_else(|_| json!([]));
        let compact = serde_json::to_string(&value).unwrap_or_else(|_| "[]".to_string());
        truncate_utf8(&compact, max_bytes).0
    }
}

pub fn sanitize_json_compact(value: &Json) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

pub fn sanitize_text(text: &str) -> String {
    text.chars()
        .filter(|c| *c != '\u{0000}')
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0;

    while idx < chars.len() {
        let c = chars[idx];
        if c == '\u{1b}' {
            idx += 1;
            if idx < chars.len() && chars[idx] == '[' {
                idx += 1;
                while idx < chars.len() {
                    let ch = chars[idx];
                    idx += 1;
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
        idx += 1;
    }

    out
}

pub fn truncate_utf8(input: &str, max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (input.to_string(), false);
    }

    if max_bytes == 0 {
        return (String::new(), true);
    }

    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }

    (input[..end].to_string(), true)
}
