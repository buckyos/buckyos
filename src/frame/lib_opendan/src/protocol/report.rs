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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReportArtifact {
    pub path: String,
    pub reference: String,
    pub digest: String,
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
    pub fn delivery_text(&self) -> String {
        let mut text = self.report.clone();
        for a in &self.artifacts {
            text.push_str(&format!("\n\n[{}]({})", a.path, a.reference));
        }
        text
    }
}

pub(crate) fn report_json_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}
