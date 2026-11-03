//! `summary.json` — history compression state (§4.4).
//!
//! Only serves session-history compression (llm_compress): the head kept
//! verbatim, the summary of everything between the head and the start point
//! with the strings kept exactly, and the start point of the raw records in
//! the worklog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SESSION_SUMMARY_SCHEMA: &str = "opendan.session_summary/3";
/// Renderer id + version of the history rebuilt from the worklog.
/// 4: entries rebuilt as messages, nothing truncated.
pub const HISTORY_RENDERER: &str = "libopendan.history/4";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionSummary {
    pub schema: String,
    /// Summary (llm_compress) of the history between the head and
    /// `start_seq`.
    #[serde(default)]
    pub history_summary: String,
    /// Strings kept verbatim next to the summary (`<keep_exact>`): appended
    /// by each compression, never rewritten by the summarizer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep_exact: Vec<String>,
    /// The head (opening input + first round) kept verbatim ahead of the
    /// summary: the worklog entries before `head_end_seq`. `0`: no head.
    #[serde(default)]
    pub head_end_seq: u64,
    /// Byte offset of `head_end_seq` in the worklog.
    #[serde(default)]
    pub head_end_offset: u64,
    /// First worklog seq used verbatim after the summary.
    #[serde(default)]
    pub start_seq: u64,
    /// Byte offset of `start_seq` in the worklog: reverse reads stop here.
    #[serde(default)]
    pub start_offset: u64,
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
    HISTORY_RENDERER.to_string()
}

impl SessionSummary {
    /// Equivalent of a missing `summary.json` (before the first compaction).
    pub fn initial() -> Self {
        Self {
            schema: SESSION_SUMMARY_SCHEMA.to_string(),
            history_summary: String::new(),
            keep_exact: Vec::new(),
            head_end_seq: 0,
            head_end_offset: 0,
            start_seq: 0,
            start_offset: 0,
            renderer: HISTORY_RENDERER.to_string(),
            made_at_seq: 0,
            made_by: String::new(),
            updated_at_ms: 0,
        }
    }
}
