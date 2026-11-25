use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TOOL_REPORT: &str = "report";
/// The structured reply of a conversation in the compatible reply mode
/// ([`ReplyMode::Compat`]).
pub const TOOL_SUBMIT_REPLY: &str = "submit_reply";

/// How a UI session's final reply is given. The model changes it
/// (`agent-session ctl reply-mode`); it holds for the session, a new one
/// starts with the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplyMode {
    /// The final text is the JSON of the behavior's `[reply]` schema.
    #[default]
    Schema,
    /// The final text is what the user reads; a reply with a topic or
    /// artifacts goes through `submit_reply`.
    Compat,
}

impl ReplyMode {
    pub fn is_default(&self) -> bool {
        *self == Self::Schema
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::Compat => "compat",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPolicy {
    #[default]
    Natural,
    ExplicitReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportOutcome {
    Succeeded,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportSource {
    Tool { call_id: String },
    Behavior { step_index: u32 },
    /// `submit_reply`: the Turn's reply, not a report of the session; its
    /// `result` is `{topic}`.
    Reply { call_id: String },
}

impl ReportSource {
    /// The tool call that submitted it.
    pub fn call_id(&self) -> Option<&str> {
        match self {
            Self::Tool { call_id } | Self::Reply { call_id } => Some(call_id),
            Self::Behavior { .. } => None,
        }
    }
}

/// One artifact of a report: a file of the work directory (`path`, copied
/// to `reference`), a named object (`path` is its id) or a bucket object
/// (`path` is its `obj://bucket/...`). `obj_id` is what leaves the session:
/// the submitted id, or what the host's object store keeps (`None`: the host
/// has no object store); `url` is the host's bucket binding that retains it,
/// after which the copy at `reference` is removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReportArtifact {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl ReportArtifact {
    /// The name shown for the artifact: the file (or key) name, or the object id.
    pub fn name(&self) -> &str {
        if self.reference.is_empty() && self.url.is_none() {
            return &self.path;
        }
        std::path::Path::new(&self.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.path)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReportSubmission {
    pub id: String,
    pub run_id: String,
    pub source: ReportSource,
    pub report: String,
    #[serde(default)]
    pub artifacts: Vec<ReportArtifact>,
    #[serde(
        default,
        deserialize_with = "report_json_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub result: Option<Value>,
    pub is_end: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ReportOutcome>,
    pub accepted_at_ms: u64,
}

impl ReportSubmission {
    /// The reply of a Turn rather than a report of the session.
    pub fn is_reply(&self) -> bool {
        matches!(self.source, ReportSource::Reply { .. })
    }

    /// The report with its artifacts, by object id where there is one.
    pub fn delivery_text(&self) -> String {
        let mut text = self.report.clone();
        for a in &self.artifacts {
            let target = a.obj_id.as_deref().unwrap_or(&a.reference);
            text.push_str(&format!("\n\n[{}]({target})", a.name()));
        }
        text
    }
}

pub(crate) fn report_json_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
