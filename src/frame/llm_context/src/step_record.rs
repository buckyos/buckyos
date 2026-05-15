//! Default XML-flavored [`StepRenderer`] implementation, paired with
//! [`crate::xml_behavior::XmlBehaviorParser`].
//!
//! The renderer is responsible for turning sedimented + hot [`StepRecord`]s
//! back into [`AiMessage`]s that the next inner LLM call sees. The waist
//! enforces strict (assistant, user) alternation per step, so this renderer
//! produces exactly one pair per step:
//!
//! - **Assistant message**: the verbatim text the LLM emitted last turn
//!   (the parsed XML lives inside `step.assistant_text`).
//! - **User message**: an `<action_result>` XML block carrying the
//!   dispatcher-side echo (success body / error message / pending marker),
//!   or `<step_ack/>` when the step had no action.
//!
//! ## History compression
//!
//! `render_history` applies a simple recency-based two-level scheme so the
//! oldest steps don't blow the prompt budget:
//!
//! - The most recent [`XmlStepRenderer::recent_full_steps`] entries render
//!   the same as [`XmlStepRenderer::render`] — full assistant text, full
//!   action body.
//! - Older entries collapse to a compact form: assistant text truncated to
//!   [`XmlStepRenderer::summary_chars`], action result body truncated to
//!   [`XmlStepRenderer::summary_chars`] / 2, success bodies replaced with
//!   `<action_result status="ok"/>` once truncated to zero.
//!
//! Schedulers needing more sophisticated tiering (e.g. the four-level
//! Min/Mini/Medium/Full scheme from the legacy opendan renderer) should
//! implement [`StepRenderer`] themselves; this default optimizes for
//! "good enough out of the box" rather than peak compression.

use buckyos_api::{AiMessage, AiRole};
use serde_json::Value;

use crate::behavior_loop::{StepRecord, StepRenderer};
use crate::observation::Observation;
use crate::xml_behavior::xml_escape;

/// Default renderer for the XML behavior protocol. Stateless beyond the
/// truncation knobs; share a single `Arc<XmlStepRenderer>` across sessions.
#[derive(Debug, Clone)]
pub struct XmlStepRenderer {
    /// Most recent N steps render at full fidelity. Older steps compress.
    /// `0` means "always compress" (only the hot `last_step` stays full,
    /// since it bypasses `render_history`).
    pub recent_full_steps: usize,
    /// Hard cap on rendered assistant_text length per compressed step.
    /// Hot / recent steps are never truncated by the renderer; truncation
    /// only applies to compressed history entries.
    pub summary_chars: usize,
    /// Hard cap on success-body length per *uncompressed* (hot / recent)
    /// step. `0` disables truncation. The hot `last_step` always goes
    /// through `render`, so this knob also caps it.
    pub max_result_chars: usize,
}

impl Default for XmlStepRenderer {
    fn default() -> Self {
        Self {
            recent_full_steps: 2,
            summary_chars: 280,
            max_result_chars: 4 * 1024,
        }
    }
}

impl XmlStepRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_recent_full_steps(mut self, n: usize) -> Self {
        self.recent_full_steps = n;
        self
    }

    pub fn with_summary_chars(mut self, n: usize) -> Self {
        self.summary_chars = n;
        self
    }

    pub fn with_max_result_chars(mut self, n: usize) -> Self {
        self.max_result_chars = n;
        self
    }

    fn render_full(&self, step: &StepRecord) -> (AiMessage, AiMessage) {
        let assistant = AiMessage::text(AiRole::Assistant, step.assistant_text.clone());
        let user_text = render_action_result_full(
            step.action.as_ref(),
            step.action_result.as_ref(),
            self.max_result_chars,
        );
        let user = AiMessage::text(AiRole::User, user_text);
        (assistant, user)
    }

    fn render_compact(&self, step: &StepRecord) -> (AiMessage, AiMessage) {
        let assistant_text = compact_assistant_text(step, self.summary_chars);
        let assistant = AiMessage::text(AiRole::Assistant, assistant_text);
        let user_text = render_action_result_compact(
            step.action.as_ref(),
            step.action_result.as_ref(),
            self.summary_chars / 2,
        );
        let user = AiMessage::text(AiRole::User, user_text);
        (assistant, user)
    }
}

impl StepRenderer for XmlStepRenderer {
    fn render(&self, step: &StepRecord) -> (AiMessage, AiMessage) {
        self.render_full(step)
    }

    fn render_history(&self, steps: Vec<StepRecord>) -> Vec<AiMessage> {
        if steps.is_empty() {
            return Vec::new();
        }
        let total = steps.len();
        let full_cutoff = total.saturating_sub(self.recent_full_steps);
        let mut out = Vec::with_capacity(total * 2);
        for (idx, step) in steps.iter().enumerate() {
            let (a, u) = if idx >= full_cutoff {
                self.render_full(step)
            } else {
                self.render_compact(step)
            };
            out.push(a);
            out.push(u);
        }
        out
    }
}

// =========================================================================
// Free helpers — kept private; behavior here is part of the protocol but
// not part of the public surface.
// =========================================================================

fn render_action_result_full(
    action: Option<&buckyos_api::AiToolCall>,
    obs: Option<&Observation>,
    max_body_chars: usize,
) -> String {
    let Some(obs) = obs else {
        return "<step_ack/>".to_string();
    };
    let tool = action.map(|a| a.name.as_str()).unwrap_or("");
    match obs {
        Observation::Success {
            call_id,
            content,
            truncated,
            ..
        } => {
            let body = stringify_content(content);
            let (body, body_truncated) = clip(body.as_str(), max_body_chars);
            let attrs = action_result_attrs(
                tool,
                call_id,
                "ok",
                *truncated || body_truncated,
            );
            format!(
                "<action_result{attrs}>{}</action_result>",
                xml_escape(&body)
            )
        }
        Observation::Error { call_id, message } => {
            let (msg, _) = clip(message.as_str(), max_body_chars.max(1024));
            let attrs = action_result_attrs(tool, call_id, "error", false);
            format!(
                "<action_result{attrs}>{}</action_result>",
                xml_escape(&msg)
            )
        }
        Observation::Pending { call_id } => {
            let attrs = action_result_attrs(tool, call_id, "pending", false);
            format!("<action_result{attrs}/>")
        }
        Observation::Cancelled { call_id, reason } => {
            let (body, _) = clip(reason.as_str(), max_body_chars.max(512));
            let attrs = action_result_attrs(tool, call_id, "cancelled", false);
            format!(
                "<action_result{attrs}>{}</action_result>",
                xml_escape(&body)
            )
        }
    }
}

fn render_action_result_compact(
    action: Option<&buckyos_api::AiToolCall>,
    obs: Option<&Observation>,
    max_body_chars: usize,
) -> String {
    let Some(obs) = obs else {
        return "<step_ack/>".to_string();
    };
    let tool = action.map(|a| a.name.as_str()).unwrap_or("");
    match obs {
        Observation::Success { call_id, content, .. } => {
            let body = stringify_content(content);
            let (body, _) = clip(body.as_str(), max_body_chars);
            let body = body.trim();
            let attrs = action_result_attrs(tool, call_id, "ok", true);
            if body.is_empty() {
                format!("<action_result{attrs}/>")
            } else {
                format!("<action_result{attrs}>{}</action_result>", xml_escape(body))
            }
        }
        Observation::Error { call_id, message } => {
            let (msg, _) = clip(message.as_str(), max_body_chars);
            let attrs = action_result_attrs(tool, call_id, "error", false);
            format!("<action_result{attrs}>{}</action_result>", xml_escape(&msg))
        }
        Observation::Pending { call_id } => {
            let attrs = action_result_attrs(tool, call_id, "pending", false);
            format!("<action_result{attrs}/>")
        }
        Observation::Cancelled { call_id, reason } => {
            let (body, _) = clip(reason.as_str(), max_body_chars);
            let attrs = action_result_attrs(tool, call_id, "cancelled", false);
            let body = body.trim();
            if body.is_empty() {
                format!("<action_result{attrs}/>")
            } else {
                format!(
                    "<action_result{attrs}>{}</action_result>",
                    xml_escape(body)
                )
            }
        }
    }
}

fn action_result_attrs(tool: &str, call_id: &str, status: &str, truncated: bool) -> String {
    let mut s = String::new();
    if !tool.is_empty() {
        s.push_str(&format!(" tool=\"{}\"", xml_escape(tool)));
    }
    if !call_id.is_empty() {
        s.push_str(&format!(" call_id=\"{}\"", xml_escape(call_id)));
    }
    s.push_str(&format!(" status=\"{status}\""));
    if truncated {
        s.push_str(" truncated=\"true\"");
    }
    s
}

fn stringify_content(content: &Value) -> String {
    match content.as_str() {
        Some(s) => s.to_string(),
        None => serde_json::to_string(content).unwrap_or_default(),
    }
}

/// Truncate to `max_chars` characters (not bytes). Returns `(clipped, was_clipped)`.
/// `max_chars == 0` means "no limit".
fn clip(input: &str, max_chars: usize) -> (String, bool) {
    if max_chars == 0 {
        return (input.to_string(), false);
    }
    let mut chars = input.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        (head, false)
    } else {
        let head = head.trim_end().to_string();
        (format!("{head}..."), true)
    }
}

/// Compact form of `assistant_text`: prefer `thought` (since that's the
/// LLM's own summary of its turn); fall back to a truncated copy of the
/// raw assistant text.
fn compact_assistant_text(step: &StepRecord, max_chars: usize) -> String {
    if let Some(thought) = step.thought.as_deref() {
        let trimmed = thought.trim();
        if !trimmed.is_empty() {
            let (clipped, _) = clip(trimmed, max_chars);
            return format!("<thinking>{}</thinking>", xml_escape(&clipped));
        }
    }
    let (clipped, _) = clip(step.assistant_text.trim(), max_chars);
    clipped
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AiContent, AiToolCall};
    use serde_json::json;
    use std::collections::HashMap;

    fn tool_call(name: &str, id: &str) -> AiToolCall {
        AiToolCall {
            name: name.to_string(),
            args: HashMap::new(),
            call_id: id.to_string(),
        }
    }

    fn assistant_text_of(msg: &AiMessage) -> String {
        assert_eq!(msg.role, AiRole::Assistant);
        plain_text(msg)
    }

    fn user_text_of(msg: &AiMessage) -> String {
        assert_eq!(msg.role, AiRole::User);
        plain_text(msg)
    }

    fn plain_text(msg: &AiMessage) -> String {
        msg.content
            .iter()
            .filter_map(|b| match b {
                AiContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn render_full_pair_preserves_assistant_text() {
        let renderer = XmlStepRenderer::new();
        let mut step = StepRecord::default();
        step.assistant_text = "<thinking>plan</thinking><action tool=\"x\">{}</action>".into();
        step.action = Some(tool_call("x", "c-1"));
        step.action_result = Some(Observation::Success {
            call_id: "c-1".into(),
            content: json!("ok"),
            bytes: 2,
            truncated: false,
        });

        let (a, u) = renderer.render(&step);
        assert!(assistant_text_of(&a).contains("<thinking>plan</thinking>"));
        let user_text = user_text_of(&u);
        assert!(user_text.contains("tool=\"x\""));
        assert!(user_text.contains("call_id=\"c-1\""));
        assert!(user_text.contains("status=\"ok\""));
        assert!(user_text.contains(">ok</action_result>"));
    }

    #[test]
    fn step_without_action_emits_ack() {
        let renderer = XmlStepRenderer::new();
        let mut step = StepRecord::default();
        step.assistant_text = "just words".into();
        let (_, u) = renderer.render(&step);
        assert_eq!(user_text_of(&u), "<step_ack/>");
    }

    #[test]
    fn error_result_carries_message() {
        let renderer = XmlStepRenderer::new();
        let mut step = StepRecord::default();
        step.action = Some(tool_call("bash", "c-9"));
        step.action_result = Some(Observation::Error {
            call_id: "c-9".into(),
            message: "permission denied".into(),
        });
        let (_, u) = renderer.render(&step);
        let user_text = user_text_of(&u);
        assert!(user_text.contains("status=\"error\""));
        assert!(user_text.contains("permission denied"));
    }

    #[test]
    fn pending_result_is_self_closing() {
        let renderer = XmlStepRenderer::new();
        let mut step = StepRecord::default();
        step.action = Some(tool_call("slow", "p-1"));
        step.action_result = Some(Observation::Pending { call_id: "p-1".into() });
        let (_, u) = renderer.render(&step);
        let user_text = user_text_of(&u);
        assert!(user_text.contains("status=\"pending\""));
        assert!(user_text.ends_with("/>"));
    }

    #[test]
    fn json_content_is_stringified() {
        let renderer = XmlStepRenderer::new();
        let mut step = StepRecord::default();
        step.action = Some(tool_call("query", "q-1"));
        step.action_result = Some(Observation::Success {
            call_id: "q-1".into(),
            content: json!({"rows": 3}),
            bytes: 0,
            truncated: false,
        });
        let (_, u) = renderer.render(&step);
        // JSON object stringified — angle brackets escaped, "rows":3 visible.
        let user_text = user_text_of(&u);
        assert!(user_text.contains("&quot;rows&quot;:3"));
    }

    #[test]
    fn xml_special_chars_in_body_are_escaped() {
        let renderer = XmlStepRenderer::new();
        let mut step = StepRecord::default();
        step.action = Some(tool_call("echo", "e-1"));
        step.action_result = Some(Observation::Success {
            call_id: "e-1".into(),
            content: json!("<b>not html</b> & friends"),
            bytes: 0,
            truncated: false,
        });
        let (_, u) = renderer.render(&step);
        let user_text = user_text_of(&u);
        assert!(user_text.contains("&lt;b&gt;not html&lt;/b&gt; &amp; friends"));
    }

    #[test]
    fn render_history_compresses_older_steps() {
        let renderer = XmlStepRenderer {
            recent_full_steps: 1,
            summary_chars: 20,
            max_result_chars: 0,
        };
        let make_step = |idx: u32, body: &str| {
            let mut step = StepRecord::default();
            step.assistant_text = format!(
                "<thinking>thought-{idx}</thinking><action tool=\"t\">{{}}</action>"
            );
            step.thought = Some(format!("thought-{idx}"));
            step.action = Some(tool_call("t", &format!("c-{idx}")));
            step.action_result = Some(Observation::Success {
                call_id: format!("c-{idx}"),
                content: json!(body),
                bytes: body.len(),
                truncated: false,
            });
            step
        };

        let steps = vec![
            make_step(0, "old body, should compress"),
            make_step(1, "newest body, full"),
        ];
        let msgs = renderer.render_history(steps);
        assert_eq!(msgs.len(), 4);

        // Step 0 (older, compressed) should use the <thinking>thought-0</thinking>
        // form rather than the original raw assistant_text.
        let a0 = plain_text(&msgs[0]);
        assert!(
            a0.contains("<thinking>thought-0</thinking>"),
            "expected compact form, got: {a0}"
        );
        assert!(!a0.contains("<action tool=\"t\">{}</action>"));

        // Step 1 (newest, full) keeps the verbatim original assistant_text.
        let a1 = plain_text(&msgs[2]);
        assert!(a1.contains("<action tool=\"t\">{}</action>"));
    }

    #[test]
    fn alternation_is_preserved() {
        let renderer = XmlStepRenderer::new();
        let make_step = |idx: u32| {
            let mut step = StepRecord::default();
            step.assistant_text = format!("turn-{idx}");
            step.action = Some(tool_call("t", &format!("c-{idx}")));
            step.action_result = Some(Observation::Success {
                call_id: format!("c-{idx}"),
                content: json!("ok"),
                bytes: 2,
                truncated: false,
            });
            step
        };
        let msgs = renderer.render_history(vec![make_step(0), make_step(1), make_step(2)]);
        // Pairs: A U A U A U
        for (idx, msg) in msgs.iter().enumerate() {
            let expected = if idx % 2 == 0 {
                AiRole::Assistant
            } else {
                AiRole::User
            };
            assert_eq!(msg.role, expected, "msg {idx} role mismatch");
        }
    }
}
