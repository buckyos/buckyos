//! `summary.json` — history compression state (§4.4).
//!
//! Only serves session-history compression: a summary of everything before
//! the start point, the start point in the worklog, and the mechanical
//! compression parameters used to render the raw records after it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::config::MechanicalCompress;

pub const SESSION_SUMMARY_SCHEMA: &str = "opendan.session_summary/1";
/// Renderer id + version; determinism is only promised within one version.
pub const MECHANICAL_RENDERER: &str = "libopendan.mechanical/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionSummary {
    pub schema: String,
    /// Summary (LLM generated) of all history before `start_seq`.
    #[serde(default)]
    pub history_summary: String,
    /// First worklog seq used verbatim.
    #[serde(default)]
    pub start_seq: u64,
    /// Byte offset of `start_seq` in the worklog: reverse reads stop here.
    #[serde(default)]
    pub start_offset: u64,
    #[serde(default)]
    pub mechanical: MechanicalCompress,
    #[serde(default = "default_renderer")]
    pub renderer: String,
    #[serde(default)]
    pub made_at_seq: u64,
    /// `context_limit | ratio | manual`.
    #[serde(default)]
    pub made_by: String,
    #[serde(default)]
    pub updated_at_ms: u64,
}

fn default_renderer() -> String {
    MECHANICAL_RENDERER.to_string()
}

impl SessionSummary {
    /// Equivalent of a missing `summary.json` (before the first compaction).
    pub fn initial(mechanical: MechanicalCompress) -> Self {
        Self {
            schema: SESSION_SUMMARY_SCHEMA.to_string(),
            history_summary: String::new(),
            start_seq: 0,
            start_offset: 0,
            mechanical,
            renderer: MECHANICAL_RENDERER.to_string(),
            made_at_seq: 0,
            made_by: String::new(),
            updated_at_ms: 0,
        }
    }
}
