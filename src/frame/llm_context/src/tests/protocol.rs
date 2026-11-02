//! A minimal behavior protocol for the loop tests. The waist ships no
//! parser or renderer of its own; this fixture understands just enough of an
//! XML response to drive the behavior loop:
//!
//! ```xml
//! <response>
//!   <observation>…</observation><intent>…</intent>
//!   <actions><shell>cmd</shell></actions>
//!   <report end="true">…</report><result>{"k":1}</result>
//!   <next_behavior>…</next_behavior>
//! </response>
//! ```
//!
//! The `<response>` wrapper is optional; a response without any of these
//! tags is an empty step (a yield); an empty response is a parse error.

use std::collections::HashMap;

use buckyos_api::{AiMessage, AiResponse, AiRole, AiToolCall};
use serde_json::Value;

use crate::behavior_loop::{LLMBehaviorResult, LLMResultParser, StepRecord, StepRenderer};
use crate::observation::Observation;

pub(crate) struct XmlBehaviorParser;

impl XmlBehaviorParser {
    pub(crate) fn new() -> Self {
        Self
    }
}

fn unwrap_cdata(body: &str) -> String {
    let t = body.trim();
    match t
        .strip_prefix("<![CDATA[")
        .and_then(|rest| rest.strip_suffix("]]>"))
    {
        Some(inner) => inner.to_string(),
        None => t.to_string(),
    }
}

/// `(attrs, body)` of the first `<tag …>body</tag>` (or `<tag …/>`).
fn element<'a>(text: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = text[from..].find(&open) {
        let start = from + i;
        let after = &text[start + open.len()..];
        match after.chars().next() {
            Some('>') | Some(' ') | Some('/') => {}
            _ => {
                from = start + open.len();
                continue;
            }
        }
        let gt = after.find('>')?;
        let attrs = after[..gt].trim();
        if attrs.ends_with('/') {
            return Some((attrs.trim_end_matches('/').trim(), ""));
        }
        let body = &after[gt + 1..];
        let close = format!("</{tag}>");
        let end = body.find(&close).unwrap_or(body.len());
        return Some((attrs, &body[..end]));
    }
    None
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = attrs.find(&key)?;
    let rest = &attrs[i + key.len()..];
    Some(rest[..rest.find('"')?].to_string())
}

impl LLMResultParser for XmlBehaviorParser {
    fn parse(&self, response: &AiResponse) -> Result<LLMBehaviorResult, String> {
        let text = response.message.text_content();
        if text.trim().is_empty() && response.message.tool_calls().is_empty() {
            return Err("empty response".into());
        }
        let mut result = LLMBehaviorResult {
            assistant_text: text.clone(),
            ..Default::default()
        };
        result.observation = element(&text, "observation").map(|(_, b)| unwrap_cdata(b));
        result.thought = element(&text, "intent").map(|(_, b)| unwrap_cdata(b));
        result.next_behavior = element(&text, "next_behavior")
            .map(|(_, b)| unwrap_cdata(b))
            .filter(|s| !s.is_empty());
        if let Some((attrs, body)) = element(&text, "report") {
            result.self_report = Some(unwrap_cdata(body));
            result.report_end = match attr(attrs, "end").as_deref() {
                None | Some("false") => false,
                Some("true") => true,
                Some(other) => return Err(format!("report end must be true or false, got {other}")),
            };
        }
        if let Some((_, body)) = element(&text, "result") {
            let value: Value = serde_json::from_str(&unwrap_cdata(body))
                .map_err(|e| format!("result is not JSON: {e}"))?;
            result.report_result = Some(value);
        }
        let native = response.message.tool_calls();
        if !native.is_empty() {
            result.do_actions = native;
        } else if let Some((_, mut body)) = element(&text, "actions") {
            while let Some((_, cmd)) = element(body, "shell") {
                let mut args = HashMap::new();
                args.insert("command".to_string(), Value::String(unwrap_cdata(cmd)));
                result.do_actions.push(AiToolCall {
                    name: "shell".into(),
                    args,
                    call_id: String::new(),
                });
                let close = body.find("</shell>").map(|i| i + "</shell>".len());
                match close {
                    Some(i) => body = &body[i..],
                    None => break,
                }
            }
        }
        Ok(result)
    }
}

/// Plain-text rendering of steps: the assistant text and one line per
/// action result.
pub(crate) struct XmlStepRenderer;

impl XmlStepRenderer {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn without_timestamps(self) -> Self {
        self
    }
}

fn result_line(obs: &Observation) -> String {
    match obs {
        Observation::Success { content, .. } => match content.as_str() {
            Some(s) => s.to_string(),
            None => content.to_string(),
        },
        Observation::Error { message, .. } => format!("Error: {message}"),
        Observation::Cancelled {
            reason,
            effect_unknown,
            ..
        } => {
            if *effect_unknown {
                format!("Cancelled (result unknown): {reason}")
            } else {
                format!("Cancelled: {reason}")
            }
        }
        Observation::Unresolved {
            reason,
            effect_unknown,
            ..
        } => {
            if *effect_unknown {
                format!("Unresolved (result unknown): {reason}")
            } else {
                format!("Not executed: {reason}")
            }
        }
    }
}

impl StepRenderer for XmlStepRenderer {
    fn render(&self, step: &StepRecord) -> (AiMessage, AiMessage) {
        let assistant = step
            .assistant_message
            .clone()
            .unwrap_or_else(|| AiMessage::text(AiRole::Assistant, step.assistant_text.clone()));
        let user = step.next_user_message.clone().unwrap_or_else(|| {
            let lines: Vec<String> = step.action_results.iter().map(result_line).collect();
            AiMessage::text(
                AiRole::User,
                format!(
                    "<<last_step_action_results behavior=\"{}\" step=\"{}\">>\n{}\n<</last_step_action_results>>",
                    step.meta.behavior_name,
                    step.meta.step_index,
                    lines.join("\n")
                ),
            )
        });
        (assistant, user)
    }

    fn render_inherited(&self, step: &StepRecord) -> AiMessage {
        AiMessage::text(
            AiRole::User,
            format!(
                "<step_record behavior=\"{}\" index=\"{}\">{}</step_record>",
                step.meta.behavior_name,
                step.meta.step_index,
                step.thought.as_deref().unwrap_or(step.assistant_text.trim())
            ),
        )
    }
}
