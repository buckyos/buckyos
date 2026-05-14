//! Default XML-flavored Behavior protocol.
//!
//! This is the system-default implementation of [`LLMResultParser`] paired
//! with [`crate::step_record::XmlStepRenderer`]. A worksession can swap in
//! its own parser/renderer; this pair just ships sensible defaults so the
//! common case is "build LLMContextDeps, point at this parser, go".
//!
//! ## Wire format the LLM is expected to produce
//!
//! ```xml
//! <response>
//!   <thinking>...free-form reasoning...</thinking>
//!   <observation>...reading of the previous action's result...</observation>
//!   <action tool="exec_bash" call_id="optional">
//!     {"command": "ls -la"}
//!   </action>
//!   <next_behavior>END</next_behavior>
//! </response>
//! ```
//!
//! Every tag is optional. Outer `<response>` is optional too — when present
//! we narrow the scan to its body, otherwise we scan the whole text. This
//! mirrors the legacy opendan behavior format closely enough that prior
//! prompts keep working.
//!
//! ## Tolerance
//!
//! 1. Strips ```` ```xml ```` and bare ```` ``` ```` markdown fences.
//! 2. Trims preamble / postamble around the recognized tag region.
//! 3. Missing close tags fall back to "take until next known tag or EOF".
//! 4. Action body is parsed as a JSON object when possible; otherwise the
//!    body becomes the `content` arg verbatim.
//! 5. Provider-returned `tool_calls` (when the model uses native function
//!    calling) take precedence over `<action>` parsing.
//! 6. Empty `assistant_text` + no actions + no `next_behavior` is **not**
//!    a parse error — it's a natural convergence step. The parser only
//!    fails when the response is completely empty (no text *and* no
//!    provider tool_calls).

use std::collections::HashMap;

use buckyos_api::{AiResponseSummary, AiToolCall};
use serde_json::Value;

use crate::behavior_loop::{LLMBehaviorResult, LLMResultParser};

/// Default XML behavior parser. Stateless — share one `Arc<XmlBehaviorParser>`
/// across all sessions.
#[derive(Debug, Clone, Default)]
pub struct XmlBehaviorParser {
    /// When `true`, parse succeeds only if at least one of `do_actions` /
    /// `next_behavior` is set. Defaults to `false` (lenient: a pure-text
    /// response is a valid terminal step).
    pub strict: bool,
}

impl XmlBehaviorParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn strict() -> Self {
        Self { strict: true }
    }
}

impl LLMResultParser for XmlBehaviorParser {
    fn parse(&self, response: &AiResponseSummary) -> Result<LLMBehaviorResult, String> {
        let raw_text = response.text.clone().unwrap_or_default();
        let provider_calls = response.tool_calls.clone();

        if raw_text.trim().is_empty() && provider_calls.is_empty() {
            return Err("empty response: no text and no tool_calls".to_string());
        }

        // Pick the region to scan. Strip code fences first, then narrow to
        // <response>...</response> if present.
        let unfenced = strip_code_fences(&raw_text);
        let scan_region = match extract_tag_body(&unfenced, "response") {
            Some(body) => body,
            None => unfenced.to_string(),
        };

        let thought = extract_tag_body(&scan_region, "thinking")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let observation = extract_tag_body(&scan_region, "observation")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let next_behavior = extract_tag_body(&scan_region, "next_behavior")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Provider-native tool_calls win when present; otherwise parse <action>.
        let do_actions = if !provider_calls.is_empty() {
            provider_calls
        } else {
            extract_actions(&scan_region)
        };

        if self.strict && do_actions.is_empty() && next_behavior.is_none() {
            return Err(
                "strict parse: response has neither <action> nor <next_behavior>".to_string(),
            );
        }

        Ok(LLMBehaviorResult {
            do_actions,
            next_behavior,
            assistant_text: raw_text,
            observation,
            thought,
        })
    }
}

// =========================================================================
// Helpers — small, regex-free, deliberately tolerant.
// =========================================================================

/// Strip markdown code fences. Handles `\`\`\`xml ... \`\`\`` and bare
/// `\`\`\` ... \`\`\``. If no closing fence is present, take the rest of
/// the string after the opener.
fn strip_code_fences(input: &str) -> String {
    let trimmed = input.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 || &bytes[..3] != b"```" {
        return trimmed.to_string();
    }
    let rest = &trimmed[3..];
    let after_lang = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    match after_lang.rfind("```") {
        Some(end) => after_lang[..end].trim().to_string(),
        None => after_lang.trim().to_string(),
    }
}

/// Find the first `<tag ...>` ... `</tag>` body in `input` (case-insensitive
/// on the tag name). If no closing tag is found, take everything from after
/// the opening tag to end-of-input. Returns `None` if no opening tag exists.
pub(crate) fn extract_tag_body(input: &str, tag: &str) -> Option<String> {
    let lc_input = input.to_ascii_lowercase();
    let lc_tag = tag.to_ascii_lowercase();

    let open_marker = format!("<{lc_tag}");
    let mut search_from = 0;
    let open_idx = loop {
        let rel = lc_input[search_from..].find(&open_marker)?;
        let abs = search_from + rel;
        let after = abs + open_marker.len();
        // Reject `<tagfoo` — next byte must end the tag-name token.
        match lc_input.as_bytes().get(after) {
            Some(c) if matches!(*c, b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/') => break abs,
            None => return None,
            _ => {
                search_from = after;
                continue;
            }
        }
    };

    let after_open_rel = input[open_idx..].find('>')?;
    let open_end = open_idx + after_open_rel;
    let body_start = open_end + 1;

    // Self-closing `<tag .../>` ⇒ empty body.
    if input[open_idx..open_end].trim_end().ends_with('/') {
        return Some(String::new());
    }

    let close_marker = format!("</{lc_tag}>");
    let body_end = lc_input[body_start..]
        .find(&close_marker)
        .map(|rel| body_start + rel)
        .unwrap_or(input.len());

    Some(input[body_start..body_end].to_string())
}

/// Scan the input for every `<action ...>...</action>` block and convert
/// each into an [`AiToolCall`]. Empty `tool` (or `name`) attribute ⇒ skip.
fn extract_actions(input: &str) -> Vec<AiToolCall> {
    let lc_input = input.to_ascii_lowercase();
    let open_marker = "<action";
    let close_marker = "</action>";
    let mut out: Vec<AiToolCall> = Vec::new();
    let mut auto_id: u32 = 0;
    let mut cursor = 0usize;

    while let Some(rel) = lc_input[cursor..].find(open_marker) {
        let open = cursor + rel;
        let after = open + open_marker.len();
        match lc_input.as_bytes().get(after) {
            Some(c) if matches!(*c, b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/') => {}
            None => break,
            _ => {
                cursor = after;
                continue;
            }
        }

        let Some(open_end_rel) = input[open..].find('>') else {
            break;
        };
        let open_end = open + open_end_rel;

        let opening_inner = &input[open + 1..open_end]; // "action tool=... [/]"
        let self_closing = opening_inner.trim_end().ends_with('/');
        let attrs_str = if self_closing {
            opening_inner.trim_end().trim_end_matches('/').trim_end()
        } else {
            opening_inner
        };
        let attrs = parse_attrs(attrs_str);

        let body = if self_closing {
            cursor = open_end + 1;
            String::new()
        } else {
            let body_start = open_end + 1;
            if let Some(close_rel) = lc_input[body_start..].find(close_marker) {
                let close_at = body_start + close_rel;
                cursor = close_at + close_marker.len();
                input[body_start..close_at].trim().to_string()
            } else {
                cursor = input.len();
                input[body_start..].trim().to_string()
            }
        };

        let name = attrs
            .get("tool")
            .or_else(|| attrs.get("name"))
            .cloned()
            .unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }

        let call_id = attrs.get("call_id").cloned().unwrap_or_else(|| {
            auto_id += 1;
            format!("call-{auto_id}")
        });

        let mut args: HashMap<String, Value> = attrs
            .into_iter()
            .filter(|(k, _)| !matches!(k.as_str(), "tool" | "name" | "call_id"))
            .map(|(k, v)| (k, Value::String(v)))
            .collect();

        if !body.is_empty() {
            match serde_json::from_str::<Value>(&body) {
                Ok(Value::Object(map)) => {
                    for (k, v) in map {
                        args.insert(k, v);
                    }
                }
                _ => {
                    args.entry("content".to_string())
                        .or_insert(Value::String(body));
                }
            }
        }

        out.push(AiToolCall {
            name,
            args,
            call_id,
        });
    }

    out
}

/// Parse an XML attribute string like `action tool="x" path='y' flag` into a
/// key→string map. Leading tag name is discarded. Quotes optional; unquoted
/// values terminate at the next whitespace. Flag attributes (no `=`) bind
/// to the empty string.
fn parse_attrs(input: &str) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    // Skip leading tag-name token.
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if key_start == i {
            i += 1;
            continue;
        }
        let key = input[key_start..i].to_ascii_lowercase();

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            out.insert(key, String::new());
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        let value = if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            let v_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let v = &input[v_start..i];
            if i < bytes.len() {
                i += 1;
            }
            xml_unescape(v)
        } else {
            let v_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            xml_unescape(&input[v_start..i])
        };

        out.insert(key, value);
    }

    out
}

/// Decode the five baseline XML entities. Unknown entities pass through.
pub(crate) fn xml_unescape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let tail = &rest[amp_idx..];
        match tail.find(';') {
            Some(end) => {
                let entity = &tail[1..end];
                let decoded = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => None,
                };
                match decoded {
                    Some(c) => out.push(c),
                    None => out.push_str(&tail[..=end]),
                }
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escape the five baseline XML entities. Used by the renderer.
pub(crate) fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::AiResponseSummary;
    use serde_json::json;

    fn resp(text: &str) -> AiResponseSummary {
        AiResponseSummary {
            text: Some(text.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn empty_response_is_error() {
        let parser = XmlBehaviorParser::new();
        assert!(parser.parse(&AiResponseSummary::default()).is_err());
    }

    #[test]
    fn pure_text_is_terminal_step() {
        let parser = XmlBehaviorParser::new();
        let out = parser.parse(&resp("just thinking out loud")).unwrap();
        assert_eq!(out.assistant_text, "just thinking out loud");
        assert!(out.do_actions.is_empty());
        assert!(out.next_behavior.is_none());
    }

    #[test]
    fn strict_mode_rejects_pure_text() {
        let parser = XmlBehaviorParser::strict();
        assert!(parser.parse(&resp("hello")).is_err());
    }

    #[test]
    fn basic_tags_extracted() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<response>
<thinking>step 1</thinking>
<observation>previous run produced 3 files</observation>
<next_behavior>plan_review</next_behavior>
</response>"#,
            ))
            .unwrap();
        assert_eq!(out.thought.as_deref(), Some("step 1"));
        assert_eq!(
            out.observation.as_deref(),
            Some("previous run produced 3 files")
        );
        assert_eq!(out.next_behavior.as_deref(), Some("plan_review"));
    }

    #[test]
    fn action_with_json_body() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<action tool="exec_bash" call_id="c1">
{"command": "ls -la"}
</action>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        let call = &out.do_actions[0];
        assert_eq!(call.name, "exec_bash");
        assert_eq!(call.call_id, "c1");
        assert_eq!(call.args.get("command"), Some(&json!("ls -la")));
    }

    #[test]
    fn action_with_attrs_and_text_body() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<action tool="write_file" path="foo.md">
hello world
</action>"#,
            ))
            .unwrap();
        let call = &out.do_actions[0];
        assert_eq!(call.name, "write_file");
        assert_eq!(call.args.get("path"), Some(&json!("foo.md")));
        assert_eq!(call.args.get("content"), Some(&json!("hello world")));
    }

    #[test]
    fn multiple_actions_in_one_response() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<action tool="t1">{"a":1}</action>
<action tool="t2">{"b":2}</action>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 2);
        assert_eq!(out.do_actions[0].name, "t1");
        assert_eq!(out.do_actions[1].name, "t2");
        assert_ne!(out.do_actions[0].call_id, out.do_actions[1].call_id);
    }

    #[test]
    fn self_closing_action_works() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(r#"<action tool="ping" host="localhost"/>"#))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.do_actions[0].args.get("host"), Some(&json!("localhost")));
    }

    #[test]
    fn provider_tool_calls_take_priority() {
        let parser = XmlBehaviorParser::new();
        let provider_call = AiToolCall {
            name: "real_tool".to_string(),
            args: HashMap::from([("k".to_string(), json!("v"))]),
            call_id: "native-1".to_string(),
        };
        let response = AiResponseSummary {
            text: Some(r#"<action tool="ignored">{"foo":"bar"}</action>"#.to_string()),
            tool_calls: vec![provider_call],
            ..Default::default()
        };
        let out = parser.parse(&response).unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.do_actions[0].name, "real_tool");
    }

    #[test]
    fn markdown_fences_are_stripped() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                "```xml\n<thinking>fenced</thinking>\n<next_behavior>END</next_behavior>\n```",
            ))
            .unwrap();
        assert_eq!(out.thought.as_deref(), Some("fenced"));
        assert_eq!(out.next_behavior.as_deref(), Some("END"));
    }

    #[test]
    fn missing_close_tag_recovers() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp("<thinking>unclosed body until eof"))
            .unwrap();
        assert_eq!(out.thought.as_deref(), Some("unclosed body until eof"));
    }

    #[test]
    fn preamble_outside_response_is_ignored() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                "Sure, let me think.\n<response><thinking>inside</thinking></response>\nDone.",
            ))
            .unwrap();
        assert_eq!(out.thought.as_deref(), Some("inside"));
    }

    #[test]
    fn xml_entities_in_attrs_are_decoded() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(r#"<action tool="grep" pattern="a &amp; b"/>"#))
            .unwrap();
        assert_eq!(
            out.do_actions[0].args.get("pattern"),
            Some(&json!("a & b"))
        );
    }

    #[test]
    fn action_without_tool_name_is_skipped() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(r#"<action>not a real action</action>"#))
            .unwrap();
        assert!(out.do_actions.is_empty());
    }
}
