use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TOOL_REPORT: &str = "report";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPolicy {
    #[default]
    Natural,
    ExplicitReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReportSource {
    Tool { call_id: String },
    Behavior { step_index: u32 },
}

/// One artifact of a report: a file of the work directory (`path`, copied
/// to `reference`) or a named object (`path` is its id). `obj_id` is what
/// leaves the session: the submitted id, or the file put into the host's
/// object store (`None`: the host has no object store).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReportArtifact {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj_id: Option<String>,
}

impl ReportArtifact {
    /// The name shown for the artifact: the file name, or the object id.
    pub fn name(&self) -> &str {
        if self.reference.is_empty() {
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
    pub accepted_at_ms: u64,
}

impl ReportSubmission {
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
