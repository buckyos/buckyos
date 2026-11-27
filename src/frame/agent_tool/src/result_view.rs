//! Rendering view of the AgentToolResult envelope carried in
//! `Observation.host`.
//!
//! The waist keeps the envelope as an opaque value; this view is how the
//! renderers of this crate read it. Deserialization is lenient (unknown
//! fields are ignored, every field is optional) so a renderer never fails on
//! an envelope written by a newer tool.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatusView {
    Success,
    Error,
    Pending,
}

impl Default for ToolResultStatusView {
    fn default() -> Self {
        Self::Success
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolResultView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_tool_protocol: Option<String>,
    #[serde(default)]
    pub status: ToolResultStatusView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_args: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_after: Option<u64>,
    /// Hints shown with the result (`<runtime_hints>`), after the ledger.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<HintView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup: Option<DedupView>,
}

/// The envelope's `dedup` (lenient view of `ResultDedup`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DedupView {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub hint: String,
}

/// The text a result shows in full, the same in both loops: the output,
/// else a string detail, the summary, the title; a structured detail only
/// when none of them is there (compact JSON within the per-result budget).
/// An error shows its summary with its output.
pub fn full_text(
    is_error: bool,
    output: Option<&str>,
    detail: Option<&Value>,
    summary: &str,
    title: &str,
) -> String {
    let output = output.filter(|s| !s.trim().is_empty());
    let summary_t = summary.trim();
    let title = title.trim();
    let detail_text = || match detail {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    };
    let detail_json = || match detail {
        Some(Value::Null) | None => None,
        Some(Value::Object(m)) if m.is_empty() => None,
        Some(d) => Some(crate::text_budget::clip_to_default_budget(&d.to_string(), None).0),
    };
    if is_error {
        return match (summary_t.is_empty(), output.map(str::trim)) {
            (false, Some(out)) => format!("{summary_t}\n{out}"),
            (false, None) => summary_t.to_string(),
            (true, Some(out)) => out.to_string(),
            (true, None) => detail_text()
                .or_else(|| (!title.is_empty()).then(|| title.to_string()))
                .or_else(detail_json)
                .unwrap_or_else(|| "tool error".to_string()),
        };
    }
    if let Some(out) = output {
        return out.to_string();
    }
    detail_text()
        .or_else(|| (!summary_t.is_empty()).then(|| summary.to_string()))
        .or_else(|| (!title.is_empty()).then(|| title.to_string()))
        .or_else(detail_json)
        .unwrap_or_else(|| "Success".to_string())
}

/// One hint of the envelope (lenient view of `ResultHint`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HintView {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<String>,
}

impl ToolResultView {
    /// Read the envelope an observation carries (`None` when it carries
    /// none or it is not an object).
    pub fn from_host(host: &Option<Value>) -> Option<Self> {
        let value = host.as_ref().filter(|v| v.is_object())?;
        let mut view: Self = serde_json::from_value(value.clone()).ok()?;
        if matches!(&view.detail, Some(Value::Null))
            || matches!(&view.detail, Some(Value::Object(m)) if m.is_empty())
        {
            view.detail = None;
        }
        Some(view)
    }

    /// The view as an envelope value for `Observation.host`.
    pub fn into_host(mut self) -> Value {
        if self.agent_tool_protocol.is_none() {
            self.agent_tool_protocol = Some(crate::AGENT_TOOL_PROTOCOL_VERSION.to_string());
        }
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// The result in full ([`full_text`]).
    pub fn full_text(&self) -> String {
        full_text(
            self.status == ToolResultStatusView::Error,
            self.output.as_deref(),
            self.detail.as_ref(),
            &self.summary,
            &self.title,
        )
    }

    /// The `once` keys of the hints shown with the result.
    pub fn once_keys(&self) -> Vec<String> {
        self.hints
            .iter()
            .filter_map(|h| h.once.clone())
            .filter(|k| !k.is_empty())
            .collect()
    }

    /// The texts of the hints shown with the result.
    pub fn hint_texts(&self) -> Vec<String> {
        self.hints
            .iter()
            .map(|h| h.text.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    }

    pub fn command_line_text(&self) -> Option<String> {
        self.cmd_name.as_ref().map(|cmd_name| {
            match self
                .cmd_args
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(cmd_args) => format!("{cmd_name} {cmd_args}"),
                None => cmd_name.clone(),
            }
        })
    }
}
