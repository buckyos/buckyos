//! Default XML-flavored Behavior protocol — v2.
//!
//! This is the system-default implementation of [`LLMResultParser`] paired
//! with [`crate::step_record::XmlStepRenderer`]. A worksession can swap in
//! its own parser/renderer; this pair just ships sensible defaults so the
//! common case is "build LLMContextDeps, point at this parser, go".
//!
//! ## Wire format the LLM is expected to produce (v2)
//!
//! See `doc/opendan/Agent Actions.md` for the full spec.
//!
//! ```xml
//! <response>
//!   <observation>...reading of the previous action's result...</observation>
//!   <thinking>...free-form reasoning...</thinking>
//!
//!   <actions>
//!     <exec_bash>cargo test</exec_bash>
//!     <write_file path="src/foo.rs"><![CDATA[
//! pub fn bar() -> u32 { 42 }
//! ]]></write_file>
//!     <sendmsg target="user"><![CDATA[已开始测试...]]></sendmsg>
//!   </actions>
//!
//!   <report><![CDATA[本步骤完成总结...]]></report>
//! </response>
//! ```
//!
//! ## Recognized Action tags (first-class, hardcoded allowlist)
//!
//! `exec_bash`, `write_file`, `edit_file`, `read`, `sendmsg`,
//! `subscribe_event`, `unsubscribe_event`. Any other element inside
//! `<actions>` is silently skipped — the v2 Action set is prompt-coupled,
//! not registry-driven.
//!
//! ## `<sendmsg>` / `<report>` handling
//!
//! These tags are **not** turned into `AiToolCall`s. They are captured into
//! dedicated slots on [`LLMBehaviorResult`]:
//!
//! - `<sendmsg target="...">` inside `<actions>` → `messages_to_send` (ordered)
//! - `<report>` outside `<actions>`              → `self_report` (last one wins)
//!
//! This keeps process messaging and LLMContext last-state updates separate:
//! `<sendmsg>` is the side-effect action shape, while `<report>` updates
//! `last_report`.
//!
//! ## Tolerance
//!
//! 1. Strips ```` ```xml ```` and bare ```` ``` ```` markdown fences.
//! 2. Trims preamble / postamble around the recognized tag region.
//! 3. Missing close tags fall back to "take until next known tag or EOF".
//! 4. Bodies are CDATA-aware. CDATA inner content is taken verbatim
//!    (leading/trailing newlines preserved — they may be part of file
//!    content). Non-CDATA bodies go through `xml_unescape`.
//! 5. Provider-returned `tool_calls` (when the model uses native function
//!    calling) take precedence over `<actions>` parsing.
//! 6. Empty response (no text, no tool_calls) is the only hard error.

use std::collections::HashMap;

use buckyos_api::{AiResponse, AiToolCall};
use serde_json::Value;

use crate::behavior_loop::{LLMBehaviorResult, LLMResultParser, SendMessageRecord};

/// Prompt snippet that teaches an LLM to emit the XML Behavior v2 result
/// format accepted by [`XmlBehaviorParser`].
///
/// This is intentionally protocol-shaped rather than behavior-shaped:
/// behavior prompts should still decide when to use each action and which
/// `next_behavior` values are valid.
pub const XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT: &str = r#"

The response MUST be parseable by XMLParser. Example:
```
<response>
  <observation>follow process rules</observation>
  <thinking>follow process rules</thinking>
  <actions>
    <write_file path="test.py"><![CDATA[
print("test")
    ]]></write_file>
    <exec_bash><![CDATA[
python3 test.py
    ]]></exec_bash>
    <exec_bash><![CDATA[
ifconfig
    ]]></exec_bash>
    <edit_file path="src/foo.rs">
      <old_string><![CDATA[println!("hello");]]></old_string>
      <new_string><![CDATA[println!("hi");]]></new_string>
    </edit_file>
    <read uri="src/foo.rs" offset="0" limit="200"/>
    <sendmsg><![CDATA[
Message to the user; optional; SHOULD only be provided when there is important progress or user input is required.
    ]]></sendmsg>
  </actions>
  <report><![CDATA[
    follow process rules.
  ]]></report>
  <next_behavior>...</next_behavior>
</response>
```

## next_behavior

- MUST set `<next_behavior>` only when `<actions>` is empty; action execution results MUST be observed before changing behavior.
- `<next_behavior>` is the key field used to maintain the behavior state machine and MUST follow the process rules.

## <actions> Usage Rules

- Actions are executed in order, and later actions can depend on the results of earlier actions.
- You SHOULD always prefer `write_file` / `edit_file` / `read` over `exec_bash`. MUST NOT use `echo`, `cat`, or heredoc to write files
- `<edit_file>` MUST provide `old_string` and `new_string`. `new_string` MUST be different from `old_string`. `old_string` MUST match exactly once in the target file, otherwise the edit fails.
- `<exec_bash>` MUST contain online bash commands in its body, and each `<exec_bash>` SHOULD complete exactly one task.

"#;

/// Hardcoded allowlist of v2 first-class Action tag names. Everything in
/// `<actions>` that isn't one of these is silently skipped.
///
/// `sendmsg` lives in this set because it shares the same XML scanning path,
/// but it's never dispatched as an action — see the `<sendmsg>` handling
/// note above.
pub const V2_ACTION_TAGS: &[&str] = &[
    "exec_bash",
    "write_file",
    "edit_file",
    "read",
    "subscribe_event",
    "unsubscribe_event",
    "sendmsg",
];

const SELF_REPORT_TAGS: &[&str] = &["report"];
const EDIT_FILE_ARG_TAGS: &[&str] = &["old_string", "new_string"];

/// True if `name` is a v2 first-class Action tag (i.e. handled by the XML
/// behavior parser rather than by `ToolManager`'s provider-native tool
/// surface). Used by policy gates to decide which whitelist to consult.
///
/// `sendmsg` is intentionally excluded — it's a v2 tag for the parser but
/// never becomes a dispatchable invocation, so policy gating for it is a
/// non-event.
pub fn is_v2_action_tag(name: &str) -> bool {
    matches!(
        name,
        "exec_bash" | "write_file" | "edit_file" | "read" | "subscribe_event" | "unsubscribe_event"
    )
}

/// Per-tag body→arg mapping. Tags not in the table either expect no body
/// (e.g. `<read uri="..."/>`) or are special-cased (`sendmsg`).
fn body_arg_name(tag: &str) -> Option<&'static str> {
    match tag {
        "exec_bash" => Some("command"),
        "write_file" => Some("content"),
        _ => None,
    }
}

/// Default XML behavior parser. Stateless — share one `Arc<XmlBehaviorParser>`
/// across all sessions.
#[derive(Debug, Clone, Default)]
pub struct XmlBehaviorParser {
    /// When `true`, parse succeeds only if at least one of `do_actions` /
    /// `self_report` / `messages_to_send` / `next_behavior` is set. Defaults
    /// to `false` (lenient: a pure-text response is a valid terminal step).
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
    fn parse(&self, response: &AiResponse) -> Result<LLMBehaviorResult, String> {
        let raw_text = response.message.text_content();
        let provider_calls = response.message.tool_calls();

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

        // Provider-native tool_calls win when present; otherwise parse the
        // v2 XML actions. Reports are extracted from the XML even when
        // provider tool_calls are present — they're a separate slot.
        let (mut do_actions, self_report, messages_to_send) = extract_v2_actions(&scan_region);
        if !provider_calls.is_empty() {
            do_actions = provider_calls;
        }

        if self.strict
            && do_actions.is_empty()
            && next_behavior.is_none()
            && self_report.is_none()
            && messages_to_send.is_empty()
        {
            return Err(
                "strict parse: response has no <actions>, <sendmsg>, <report>, or <next_behavior>"
                    .to_string(),
            );
        }

        Ok(LLMBehaviorResult {
            do_actions,
            next_behavior,
            assistant_text: raw_text,
            observation,
            thought,
            self_report,
            messages_to_send,
        })
    }
}

// =========================================================================
// v2 Action extraction
// =========================================================================

/// Walk the scan region and split out (regular actions, self_report,
/// messages_to_send). Document order is preserved within `do_actions` and
/// `messages_to_send` independently.
///
/// Strategy:
/// 1. Find `<actions>` container body — if missing, treat the whole region
///    as the container (tolerant: LLM may forget the wrapper).
/// 2. Inside the container, walk left-to-right with [`scan_v2_action_tags`];
///    each known tag becomes an `AiToolCall` except `<sendmsg>`, which is
///    captured as `messages_to_send`.
/// 3. Scan the response outside `<actions>` for `<report>` tags. Self Report
///    is a last-state update, not an action.
fn extract_v2_actions(
    scan_region: &str,
) -> (Vec<AiToolCall>, Option<String>, Vec<SendMessageRecord>) {
    let actions_body =
        extract_tag_body(scan_region, "actions").unwrap_or_else(|| scan_region.to_string());

    let mut do_actions: Vec<AiToolCall> = Vec::new();
    let mut messages_to_send: Vec<SendMessageRecord> = Vec::new();
    let mut auto_id: u32 = 0;

    // Pass 1: walk the actions container. Dispatchable actions become
    // AiToolCalls; `<sendmsg>` is a parser-side message side effect.
    for raw in scan_v2_action_tags(&actions_body) {
        if raw.tag == "sendmsg" {
            let target = raw
                .attrs
                .get("target")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if let Some(target) = target {
                messages_to_send.push(SendMessageRecord {
                    target,
                    body: raw.body,
                });
            }
            continue;
        }
        auto_id += 1;
        let call_id = raw
            .attrs
            .get("call_id")
            .cloned()
            .unwrap_or_else(|| format!("call-{auto_id}"));

        let mut args: HashMap<String, Value> = raw
            .attrs
            .into_iter()
            .filter(|(k, _)| k != "call_id")
            .map(|(k, v)| (k, Value::String(v)))
            .collect();

        if raw.tag == "edit_file" {
            for child in scan_known_tags(&raw.body, EDIT_FILE_ARG_TAGS) {
                args.insert(child.tag.to_string(), Value::String(child.body));
            }
        } else if let Some(arg_name) = body_arg_name(raw.tag) {
            if !raw.body.is_empty() {
                args.insert(arg_name.to_string(), Value::String(raw.body));
            }
        }

        do_actions.push(AiToolCall {
            name: raw.tag.to_string(),
            args,
            call_id,
        });
    }

    // Pass 2: scan for `<report>` outside `<actions>`. Self Report updates
    // the LLMContext last state; it is not an action and has no target mode.
    let mut self_report: Option<String> = None;
    let action_ranges = find_tag_ranges(scan_region, "actions");
    for raw in scan_self_report_tags(scan_region) {
        if is_in_ranges(raw.start, &action_ranges) {
            continue;
        }
        self_report = Some(raw.body);
    }

    (do_actions, self_report, messages_to_send)
}

/// One v2 Action tag occurrence, in document order.
struct RawActionTag {
    tag: &'static str,
    attrs: HashMap<String, String>,
    body: String,
    start: usize,
}

/// Walk `input` left-to-right, yielding every recognized v2 Action tag in
/// document order. Unknown tag names are skipped (treated as opaque text).
fn scan_v2_action_tags(input: &str) -> Vec<RawActionTag> {
    scan_known_tags(input, V2_ACTION_TAGS)
}

fn scan_self_report_tags(input: &str) -> Vec<RawActionTag> {
    scan_known_tags(input, SELF_REPORT_TAGS)
}

fn scan_known_tags(input: &str, tags: &'static [&'static str]) -> Vec<RawActionTag> {
    let lc_input = input.to_ascii_lowercase();
    let bytes = lc_input.as_bytes();
    let mut out: Vec<RawActionTag> = Vec::new();
    let mut cursor = 0usize;

    while cursor < input.len() {
        let Some(rel) = lc_input[cursor..].find('<') else {
            break;
        };
        let open = cursor + rel;

        // Try matching against each known tag name. Byte-level comparison
        // (not str slicing!) — when `tag.len()` extends past the tag name
        // into following content like `<report>本...</report>`, a str slice
        // can land mid-multibyte-char and panic. Bytes are safe and faster.
        let mut matched: Option<&'static str> = None;
        for &tag in tags {
            let needed = 1 + tag.len();
            if open + needed > bytes.len() {
                continue;
            }
            if &bytes[open + 1..open + 1 + tag.len()] != tag.as_bytes() {
                continue;
            }
            // After the tag name must come a terminator.
            match bytes.get(open + 1 + tag.len()) {
                Some(c) if matches!(*c, b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/') => {
                    matched = Some(tag);
                    break;
                }
                _ => {}
            }
        }
        let Some(tag) = matched else {
            cursor = open + 1;
            continue;
        };

        // Locate '>' that closes the opening tag.
        let Some(close_open_rel) = input[open..].find('>') else {
            break;
        };
        let open_end = open + close_open_rel;
        let opening_inner = &input[open + 1..open_end]; // "tag attr=val ... [/]"
        let self_closing = opening_inner.trim_end().ends_with('/');
        let attrs_str = if self_closing {
            opening_inner.trim_end().trim_end_matches('/').trim_end()
        } else {
            opening_inner
        };
        let attrs = parse_attrs(attrs_str);

        if self_closing {
            out.push(RawActionTag {
                tag,
                attrs,
                body: String::new(),
                start: open,
            });
            cursor = open_end + 1;
            continue;
        }

        // CDATA-aware close-tag search.
        let body_start = open_end + 1;
        let close_marker = format!("</{tag}>");
        let (body_raw, after_close) =
            extract_body_cdata_aware(input, &lc_input, body_start, &close_marker);
        let body = normalize_action_body(&body_raw);
        out.push(RawActionTag {
            tag,
            attrs,
            body,
            start: open,
        });
        cursor = after_close;
    }

    out
}

fn find_tag_ranges(input: &str, tag: &str) -> Vec<(usize, usize)> {
    let lc_input = input.to_ascii_lowercase();
    let bytes = lc_input.as_bytes();
    let open_prefix = format!("<{tag}");
    let close_marker = format!("</{tag}>");
    let mut ranges = Vec::new();
    let mut cursor = 0usize;

    while cursor < input.len() {
        let Some(rel) = lc_input[cursor..].find(&open_prefix) else {
            break;
        };
        let open = cursor + rel;
        match bytes.get(open + open_prefix.len()) {
            Some(c) if matches!(*c, b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/') => {}
            _ => {
                cursor = open + 1;
                continue;
            }
        }
        let Some(close_open_rel) = input[open..].find('>') else {
            break;
        };
        let open_end = open + close_open_rel;
        let opening_inner = &input[open + 1..open_end];
        let self_closing = opening_inner.trim_end().ends_with('/');
        let end = if self_closing {
            open_end + 1
        } else {
            let (_, after_close) =
                extract_body_cdata_aware(input, &lc_input, open_end + 1, &close_marker);
            after_close
        };
        ranges.push((open, end));
        cursor = end;
    }

    ranges
}

fn is_in_ranges(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|(start, end)| pos >= *start && pos < *end)
}

/// Scan forward from `body_start` for `close_marker_lc`, skipping over any
/// `<![CDATA[...]]>` regions so a literal close-tag inside CDATA does not
/// terminate the body prematurely. Returns `(raw_body, position_after_close)`.
/// If no close tag is found, returns body = everything-to-EOF.
fn extract_body_cdata_aware(
    input: &str,
    lc_input: &str,
    body_start: usize,
    close_marker_lc: &str,
) -> (String, usize) {
    let mut search_from = body_start;
    loop {
        if search_from >= input.len() {
            return (input[body_start..].to_string(), input.len());
        }
        let next_cdata = lc_input[search_from..]
            .find("<![cdata[")
            .map(|r| search_from + r);
        let next_close = lc_input[search_from..]
            .find(close_marker_lc)
            .map(|r| search_from + r);

        match (next_cdata, next_close) {
            (Some(c), Some(cl)) if c < cl => {
                // CDATA opens before close — skip past its `]]>` and retry.
                let after_open = c + "<![CDATA[".len();
                match lc_input[after_open..].find("]]>") {
                    Some(rel) => {
                        search_from = after_open + rel + "]]>".len();
                    }
                    None => {
                        // Unterminated CDATA — bail by taking the rest.
                        return (input[body_start..].to_string(), input.len());
                    }
                }
            }
            (_, Some(cl)) => {
                let body = input[body_start..cl].to_string();
                return (body, cl + close_marker_lc.len());
            }
            (Some(_), None) | (None, None) => {
                return (input[body_start..].to_string(), input.len());
            }
        }
    }
}

/// Trim outer whitespace; if the body is a single CDATA wrapper, take its
/// inner contents **verbatim** (no further trim — preserves leading/trailing
/// newlines that may be part of file content). Otherwise apply `xml_unescape`
/// for the `&lt;` / `&gt;` / `&amp;` form.
fn normalize_action_body(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lc = trimmed.to_ascii_lowercase();
    if lc.starts_with("<![cdata[") && lc.ends_with("]]>") && trimmed.len() >= "<![CDATA[]]>".len() {
        let inner_start = "<![CDATA[".len();
        let inner_end = trimmed.len() - "]]>".len();
        return trimmed[inner_start..inner_end].to_string();
    }
    xml_unescape(trimmed)
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
///
/// This helper is **not** CDATA-aware — it's used for short, scalar-style
/// tags (`<thinking>`, `<observation>`, `<next_behavior>`, `<actions>`,
/// `<response>`) where CDATA collisions are not expected.
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

/// Parse an XML attribute string like `tag attr="x" path='y' flag` into a
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
    use buckyos_api::AiResponse;
    use serde_json::json;

    fn resp(text: &str) -> AiResponse {
        AiResponse::text(text)
    }

    #[test]
    fn empty_response_is_error() {
        let parser = XmlBehaviorParser::new();
        assert!(parser.parse(&AiResponse::default()).is_err());
    }

    #[test]
    fn pure_text_is_terminal_step() {
        let parser = XmlBehaviorParser::new();
        let out = parser.parse(&resp("just thinking out loud")).unwrap();
        assert_eq!(out.assistant_text, "just thinking out loud");
        assert!(out.do_actions.is_empty());
        assert!(out.next_behavior.is_none());
        assert!(out.self_report.is_none());
    }

    #[test]
    fn strict_mode_rejects_pure_text() {
        let parser = XmlBehaviorParser::strict();
        assert!(parser.parse(&resp("hello")).is_err());
    }

    #[test]
    fn basic_scalar_tags_extracted() {
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

    // ---- v2 first-class Action tags ----

    #[test]
    fn exec_bash_body_maps_to_command_arg() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions><exec_bash>ls -la | head -5</exec_bash></actions>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        let call = &out.do_actions[0];
        assert_eq!(call.name, "exec_bash");
        assert_eq!(call.args.get("command"), Some(&json!("ls -la | head -5")));
    }

    #[test]
    fn write_file_path_attr_and_cdata_body() {
        let parser = XmlBehaviorParser::new();
        let raw = "<actions><write_file path=\"src/foo.rs\"><![CDATA[\npub fn bar() -> u32 { 42 }\n]]></write_file></actions>";
        let out = parser.parse(&resp(raw)).unwrap();
        assert_eq!(out.do_actions.len(), 1);
        let call = &out.do_actions[0];
        assert_eq!(call.name, "write_file");
        assert_eq!(call.args.get("path"), Some(&json!("src/foo.rs")));
        // CDATA inner is verbatim — leading/trailing newlines preserved.
        assert_eq!(
            call.args.get("content"),
            Some(&json!("\npub fn bar() -> u32 { 42 }\n"))
        );
    }

    #[test]
    fn cdata_body_with_literal_close_tag_inside() {
        // CDATA must shield a literal `</write_file>` from being matched as
        // the close tag.
        let parser = XmlBehaviorParser::new();
        let raw = "<actions><write_file path=\"x.md\"><![CDATA[</write_file>literal</write_file>]]></write_file></actions>";
        let out = parser.parse(&resp(raw)).unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(
            out.do_actions[0].args.get("content"),
            Some(&json!("</write_file>literal</write_file>"))
        );
    }

    #[test]
    fn edit_file_child_tags_map_to_old_and_new_string_args() {
        let parser = XmlBehaviorParser::new();
        let raw = r#"<actions>
<edit_file path="src/foo.rs">
  <old_string><![CDATA[println!("hello");]]></old_string>
  <new_string><![CDATA[println!("hi");]]></new_string>
</edit_file>
</actions>"#;
        let out = parser.parse(&resp(raw)).unwrap();
        assert_eq!(out.do_actions.len(), 1);
        let call = &out.do_actions[0];
        assert_eq!(call.name, "edit_file");
        assert_eq!(call.args.get("path"), Some(&json!("src/foo.rs")));
        assert_eq!(
            call.args.get("old_string"),
            Some(&json!("println!(\"hello\");"))
        );
        assert_eq!(
            call.args.get("new_string"),
            Some(&json!("println!(\"hi\");"))
        );
        assert!(call.args.get("content").is_none());
    }

    #[test]
    fn read_uri_attr_self_closing() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions><read uri="x" offset="0" limit="1024"/></actions>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        let call = &out.do_actions[0];
        assert_eq!(call.name, "read");
        assert_eq!(call.args.get("uri"), Some(&json!("x")));
        assert_eq!(call.args.get("offset"), Some(&json!("0")));
        assert_eq!(call.args.get("limit"), Some(&json!("1024")));
    }

    #[test]
    fn multiple_actions_preserve_document_order() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions>
<exec_bash>echo first</exec_bash>
<write_file path="a">A</write_file>
<exec_bash>echo third</exec_bash>
</actions>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 3);
        assert_eq!(out.do_actions[0].name, "exec_bash");
        assert_eq!(
            out.do_actions[0].args.get("command"),
            Some(&json!("echo first"))
        );
        assert_eq!(out.do_actions[1].name, "write_file");
        assert_eq!(out.do_actions[2].name, "exec_bash");
        assert_eq!(
            out.do_actions[2].args.get("command"),
            Some(&json!("echo third"))
        );
    }

    #[test]
    fn unknown_tags_inside_actions_are_skipped() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions>
<unknown_thing>nope</unknown_thing>
<exec_bash>ok</exec_bash>
</actions>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.do_actions[0].name, "exec_bash");
    }

    #[test]
    fn missing_actions_wrapper_is_tolerated() {
        // LLM forgets `<actions>` wrapper — known action tags directly under
        // `<response>` still get picked up.
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(r#"<response><exec_bash>ls</exec_bash></response>"#))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.do_actions[0].name, "exec_bash");
    }

    // ---- <sendmsg> / <report> handling ----

    #[test]
    fn self_report_outside_actions() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<response>
<actions><exec_bash>echo done</exec_bash></actions>
<report><![CDATA[本步骤完成]]></report>
<next_behavior>END</next_behavior>
</response>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.self_report.as_deref(), Some("本步骤完成"));
        assert!(out.messages_to_send.is_empty());
        assert_eq!(out.next_behavior.as_deref(), Some("END"));
    }

    #[test]
    fn sendmsg_in_actions_becomes_message_send() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions><sendmsg target="user">进度更新</sendmsg></actions>"#,
            ))
            .unwrap();
        assert!(out.do_actions.is_empty()); // <sendmsg> is NOT a do_action
        assert!(out.self_report.is_none());
        assert_eq!(out.messages_to_send.len(), 1);
        assert_eq!(out.messages_to_send[0].target, "user");
        assert_eq!(out.messages_to_send[0].body, "进度更新");
    }

    #[test]
    fn report_inside_actions_is_ignored() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions><report target="user">旧协议消息</report></actions>"#,
            ))
            .unwrap();
        assert!(out.do_actions.is_empty());
        assert!(out.self_report.is_none());
        assert!(out.messages_to_send.is_empty());
    }

    #[test]
    fn both_self_report_and_send_message_in_same_step() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<response>
<actions>
<sendmsg target="user">中途反馈</sendmsg>
<exec_bash>echo work</exec_bash>
</actions>
<report>最终总结</report>
</response>"#,
            ))
            .unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.do_actions[0].name, "exec_bash");
        assert_eq!(out.self_report.as_deref(), Some("最终总结"));
        assert_eq!(out.messages_to_send.len(), 1);
        assert_eq!(out.messages_to_send[0].target, "user");
        assert_eq!(out.messages_to_send[0].body, "中途反馈");
    }

    #[test]
    fn multiple_self_reports_last_one_wins() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(r#"<report>first</report><report>second</report>"#))
            .unwrap();
        assert_eq!(out.self_report.as_deref(), Some("second"));
    }

    // ---- precedence / strictness ----

    #[test]
    fn provider_tool_calls_replace_xml_actions_but_keep_reports() {
        // Provider-native tool_calls override XML do_actions, but the report
        // slot is independent and survives.
        let parser = XmlBehaviorParser::new();
        let provider_call = AiToolCall {
            name: "real_tool".to_string(),
            args: HashMap::from([("k".to_string(), json!("v"))]),
            call_id: "native-1".to_string(),
        };
        let response = AiResponse {
            message: AiResponse::message_from_parts(
                Some(
                    r#"<actions><exec_bash>ignored</exec_bash></actions><report>kept</report>"#
                        .to_string(),
                ),
                vec![provider_call],
                vec![],
            ),
            ..Default::default()
        };
        let out = parser.parse(&response).unwrap();
        assert_eq!(out.do_actions.len(), 1);
        assert_eq!(out.do_actions[0].name, "real_tool");
        assert_eq!(out.self_report.as_deref(), Some("kept"));
    }

    #[test]
    fn strict_mode_accepts_self_report_only() {
        // A response with only `<report>` is a valid strict-mode step.
        let parser = XmlBehaviorParser::strict();
        let out = parser
            .parse(&resp(r#"<report>checkpoint</report>"#))
            .unwrap();
        assert_eq!(out.self_report.as_deref(), Some("checkpoint"));
    }

    // ---- tolerance / quirks ----

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
            .parse(&resp(r#"<actions><read uri="file:///a&amp;b"/></actions>"#))
            .unwrap();
        assert_eq!(
            out.do_actions[0].args.get("uri"),
            Some(&json!("file:///a&b"))
        );
    }

    #[test]
    fn non_cdata_body_unescapes_entities() {
        let parser = XmlBehaviorParser::new();
        let out = parser
            .parse(&resp(
                r#"<actions><exec_bash>echo &lt;hi&gt;</exec_bash></actions>"#,
            ))
            .unwrap();
        assert_eq!(
            out.do_actions[0].args.get("command"),
            Some(&json!("echo <hi>"))
        );
    }
}
