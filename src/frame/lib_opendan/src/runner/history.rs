//! The history part of the next llm_context (§4.4).
//!
//! Read order is fixed: `summary.json` first, then the worklog **backwards**
//! from the committed end down to `summary.start_offset`, stopping early when
//! the budget runs out (then compaction moves the start point first, so the
//! summary and the raw records never leave a gap). The runtime never scans
//! the whole worklog.
//!
//! Rendering is deterministic for a renderer version
//! (`libopendan.mechanical/2`): same summary.json + worklog → same bytes.

use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};

use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::InferenceAbortToken;

use crate::error::{OpenDanError, Result};
use crate::lock::Lease;
use crate::protocol::*;
use crate::session::Session;

/// Rough token estimate (bytes / 4), same heuristic as the waist.
pub fn tokens(s: &str) -> u32 {
    ((s.len() as u64 + 3) / 4).min(u32::MAX as u64) as u32
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max).collect();
    format!("{t}…")
}

fn input_list(inputs: &[InputRef], changes: &[String]) -> String {
    let ins: Vec<String> = inputs.iter().map(|i| i.id()).collect();
    format!(
        "{}{}",
        if ins.is_empty() {
            String::new()
        } else {
            format!(" inputs: {}", ins.join(","))
        },
        if changes.is_empty() {
            String::new()
        } else {
            format!(" changes: {}", changes.join(","))
        }
    )
}

/// `true` for the entries that count as one model response in
/// `MechanicalCompress.recent_full_responses`.
fn is_response(body: &WorklogBody) -> bool {
    matches!(
        body,
        WorklogBody::Step { .. } | WorklogBody::AssistantMessage { .. }
    )
}

fn render_calls(s: &mut String, calls: &[ActionEntry], limit: impl Fn(&str, u32) -> String) {
    for a in calls {
        s.push_str(&format!(
            "\n  → {}({}) #{}",
            a.tool,
            limit(&a.args.to_string(), 400),
            a.call_id
        ));
    }
}

/// Render one worklog entry. `age` counts model responses (`step` /
/// `assistant_message`) already rendered, newest first: the
/// `recent_full_responses` newest responses and the entries after them are
/// full, older ones are truncated to `summary_chars`.
pub fn render_entry(e: &WorklogEntry, age: u32, cfg: &MechanicalCompress) -> Option<String> {
    if cfg.drop_kinds.iter().any(|k| k == e.body.kind()) {
        return None;
    }
    let full = age < cfg.recent_full_responses;
    let limit = |s: &str, full_max: u32| -> String {
        if full {
            trunc(s, full_max as usize)
        } else {
            trunc(s, cfg.summary_chars as usize)
        }
    };
    Some(match &e.body {
        WorklogBody::Created { objective, .. } => format!("[created] {objective}"),
        WorklogBody::TurnStarted {
            turn,
            inputs,
            changes,
            hook,
            ..
        } => format!(
            "── turn {turn}{}{} ──",
            hook.as_ref().map(|h| format!(" ({h})")).unwrap_or_default(),
            input_list(inputs, changes)
        ),
        WorklogBody::InputBatch {
            inputs,
            changes,
            hook,
            ..
        } => format!(
            "── {}{} ──",
            hook.as_deref().unwrap_or("input"),
            input_list(inputs, changes)
        ),
        WorklogBody::UserMessage { content, .. } => {
            format!("[input] {}", limit(content, cfg.max_result_chars))
        }
        WorklogBody::AssistantMessage {
            assistant,
            tool_calls,
            ..
        } => {
            let mut s = format!("[assistant] {}", limit(assistant, cfg.max_result_chars));
            render_calls(&mut s, tool_calls, &limit);
            s
        }
        WorklogBody::Step {
            step_index,
            assistant,
            actions,
            behavior,
            correction,
            ..
        } => {
            let mut s = format!(
                "[step {step_index}{}{}] {}",
                behavior
                    .as_ref()
                    .map(|b| format!(" {b}"))
                    .unwrap_or_default(),
                if *correction { " correction" } else { "" },
                limit(assistant, cfg.max_result_chars)
            );
            render_calls(&mut s, actions, &limit);
            s
        }
        WorklogBody::ActionResult {
            call_id,
            status,
            result,
            ..
        } => format!(
            "[result #{call_id} {status}] {}",
            limit(result, cfg.max_result_chars)
        ),
        WorklogBody::Outcome {
            kind,
            next_behavior,
            report,
            ..
        } => format!(
            "[outcome {kind}{}]{}",
            next_behavior
                .as_ref()
                .map(|b| format!(" → {b}"))
                .unwrap_or_default(),
            report
                .as_ref()
                .map(|r| format!(" {}", limit(r, cfg.max_result_chars)))
                .unwrap_or_default()
        ),
        WorklogBody::TurnEnded { turn, status, .. } => {
            format!(
                "── turn {turn} {} ──",
                serde_json::to_value(status).ok()?.as_str()?
            )
        }
        WorklogBody::Compaction { .. } => return None,
        WorklogBody::Decide { decision, by, .. } => format!("[decide] {decision} by {by}"),
        WorklogBody::InputRejected { input, reason } => {
            format!("[rejected {}] {reason}", input.id())
        }
        WorklogBody::ChangeDropped { change, reason } => format!("[change dropped {change}] {reason}"),
        WorklogBody::ControlApplied { command, .. } => format!("[control] {command}"),
    })
}

/// Run a transcript entry belongs to (`None`: session level entries).
fn entry_run(body: &WorklogBody) -> Option<&str> {
    match body {
        WorklogBody::InputBatch { run_id, .. }
        | WorklogBody::UserMessage { run_id, .. }
        | WorklogBody::AssistantMessage { run_id, .. }
        | WorklogBody::Step { run_id, .. }
        | WorklogBody::ActionResult { run_id, .. }
        | WorklogBody::Outcome { run_id, .. } => Some(run_id),
        _ => None,
    }
}

/// Sub contexts that returned (`process_done`): their transcript stays in
/// the worklog for audit, but a history built for another context only
/// carries what they handed back — the `process_done` outcome with its
/// result. Without this a caller rebuilt from the session history would get
/// the child's whole transcript mixed into its own.
#[derive(Default)]
struct ReturnedChildren(std::collections::HashSet<String>);

impl ReturnedChildren {
    fn note(&mut self, body: &WorklogBody) {
        if let WorklogBody::Outcome { run_id, kind, .. } = body {
            if kind == "process_done" {
                self.0.insert(run_id.clone());
            }
        }
    }

    /// The entry is part of a returned child's transcript (not its result).
    fn hides(&self, body: &WorklogBody) -> bool {
        if matches!(body, WorklogBody::Outcome { kind, .. } if kind == "process_done") {
            return false;
        }
        entry_run(body).is_some_and(|r| self.0.contains(r))
    }
}

/// Rendered history window.
#[derive(Debug, Clone, Default)]
pub struct HistoryWindow {
    /// Rendered entries, oldest first.
    pub lines: Vec<String>,
    pub tokens: u32,
    /// `true` when the reverse read reached `start_offset`.
    pub reached_start: bool,
    /// Byte offset of the oldest rendered entry.
    pub oldest_offset: Option<u64>,
    pub oldest_seq: Option<u64>,
    /// Bytes read from the worklog (bounded by the window, not file size).
    pub bytes_read: u64,
}

/// Reverse-read the committed worklog within `budget` tokens.
pub fn read_window(
    session: &Session,
    summary: &SessionSummary,
    budget: u32,
) -> Result<HistoryWindow> {
    let wl = session.dir.worklog();
    let end = session.state.worklog.committed_bytes;
    let mut r = wl.reverse(end, summary.start_offset)?;
    let mut out = HistoryWindow {
        reached_start: true,
        ..Default::default()
    };
    let mut rev: Vec<String> = Vec::new();
    let mut age = 0u32;
    let mut pending_age_bump = false;
    let mut children = ReturnedChildren::default();
    loop {
        let Some((offset, e)) = r.next_json::<WorklogEntry>()? else {
            break;
        };
        // Read backwards: a child's `process_done` comes before its entries.
        children.note(&e.body);
        if children.hides(&e.body) {
            continue;
        }
        if pending_age_bump {
            age += 1;
            pending_age_bump = false;
        }
        let Some(text) = render_entry(&e, age, &summary.mechanical) else {
            // Dropped kinds still count as read.
            continue;
        };
        let t = tokens(&text) + 1;
        if out.tokens + t > budget {
            out.reached_start = false;
            break;
        }
        out.tokens += t;
        out.oldest_offset = Some(offset);
        out.oldest_seq = Some(e.seq);
        rev.push(text);
        if is_response(&e.body) {
            pending_age_bump = true;
        }
    }
    out.bytes_read = r.bytes_read;
    rev.reverse();
    out.lines = rev;
    Ok(out)
}

/// The history message (summary + raw records), `None` when empty.
pub fn history_message(summary: &SessionSummary, window: &HistoryWindow) -> Option<AiMessage> {
    if summary.history_summary.trim().is_empty() && window.lines.is_empty() {
        return None;
    }
    let mut s = String::from("<session_history>\n");
    if !summary.history_summary.trim().is_empty() {
        s.push_str(&format!(
            "<summary upto_seq=\"{}\">\n{}\n</summary>\n",
            summary.start_seq,
            summary.history_summary.trim()
        ));
    }
    for l in &window.lines {
        s.push_str(l);
        s.push('\n');
    }
    s.push_str("</session_history>");
    Some(AiMessage::text(AiRole::User, s))
}

/// Produces the summary text of compaction.
#[async_trait]
pub trait Summarizer: Send + Sync {
    async fn summarize(&self, previous: &str, segment: &str) -> Result<String>;
}

pub const SUMMARIZE_MARKER: &str = "[libopendan:summarize]";

/// LLM based summarizer (uses the session's model).
pub struct LlmSummarizer {
    pub llm: Arc<dyn LlmClient>,
    pub model: String,
}

#[async_trait]
impl Summarizer for LlmSummarizer {
    async fn summarize(&self, previous: &str, segment: &str) -> Result<String> {
        let prompt = format!(
            "{SUMMARIZE_MARKER}\nSummarize the work history below into a concise, factual summary that preserves decisions, results, file names, open problems and user requests. Merge it with the previous summary.\n\n<previous_summary>\n{previous}\n</previous_summary>\n\n<history>\n{segment}\n</history>"
        );
        let req = LlmInferenceRequest {
            trace_id: None,
            messages: vec![
                AiMessage::text(
                    AiRole::System,
                    "You compress agent work logs. Reply with the summary text only.",
                ),
                AiMessage::text(AiRole::User, prompt),
            ],
            model_alias: self.model.clone(),
            fallbacks: Vec::new(),
            temperature: None,
            max_completion_tokens: Some(2048),
            force_json: false,
            json_schema: None,
            provider_options: None,
            disable_capabilities: Vec::new(),
            tool_specs: Vec::new(),
            allow_tool_calls: false,
            abort: InferenceAbortToken::noop(),
        };
        let resp = self
            .llm
            .infer(req)
            .await
            .map_err(|e| OpenDanError::Llm(format!("summarize: {e}")))?;
        let text = resp.message.text_content();
        if text.trim().is_empty() {
            return Err(OpenDanError::Llm("summarize: empty summary".into()));
        }
        Ok(text.trim().to_string())
    }
}

/// Compaction (§4.4): summarize `[start_offset, cut_offset)` into a new
/// summary.json (start point moves to `cut_offset`), then append the audit
/// entry and commit. The worklog is never rewritten.
pub async fn compact(
    session: &mut Session,
    lease: &Lease,
    summarizer: &dyn Summarizer,
    cut_offset: u64,
    cut_seq: u64,
    made_by: &str,
) -> Result<SessionSummary> {
    let sm = session.summary()?;
    if cut_offset <= sm.start_offset {
        return Ok(sm);
    }
    // The only forward read: the bounded range being summarized.
    let segment_entries = session.dir.worklog().read_range(sm.start_offset, cut_offset)?;
    let full = MechanicalCompress {
        recent_full_responses: u32::MAX,
        ..sm.mechanical.clone()
    };
    let mut children = ReturnedChildren::default();
    for (_, e) in &segment_entries {
        children.note(&e.body);
    }
    let segment: Vec<String> = segment_entries
        .iter()
        .filter(|(_, e)| !children.hides(&e.body))
        .filter_map(|(_, e)| render_entry(e, 0, &full))
        .collect();
    let summary_text = summarizer
        .summarize(&sm.history_summary, &segment.join("\n"))
        .await?;
    let new = SessionSummary {
        schema: SESSION_SUMMARY_SCHEMA.to_string(),
        history_summary: summary_text,
        start_seq: cut_seq,
        start_offset: cut_offset,
        mechanical: sm.mechanical.clone(),
        renderer: MECHANICAL_RENDERER.to_string(),
        made_at_seq: session.state.worklog.committed_seq,
        made_by: made_by.to_string(),
        updated_at_ms: crate::now_ms(),
    };
    session.write_summary(lease, &new)?;
    session.append_worklog(
        lease,
        vec![WorklogBody::Compaction {
            summary_start_seq: cut_seq,
            made_by: made_by.to_string(),
        }],
    )?;
    session.commit_state(lease)?;
    Ok(new)
}

/// History messages for a new llm_context: compacts first when the budget is
/// exhausted before the start point (no gap between summary and records).
pub async fn build_history(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&dyn Summarizer>,
    budget: u32,
) -> Result<(Option<AiMessage>, HistoryWindow)> {
    let mut sm = session.summary()?;
    let summary_cost = tokens(&sm.history_summary);
    let mut window = read_window(session, &sm, budget.saturating_sub(summary_cost))?;
    if !window.reached_start {
        let Some(s) = summarizer else {
            return Err(OpenDanError::Other(
                "history exceeds the budget and no summarizer is configured".into(),
            ));
        };
        // New start = the oldest entry that still fits.
        let cut = window
            .oldest_offset
            .unwrap_or(session.state.worklog.committed_bytes);
        let cut_seq = window
            .oldest_seq
            .unwrap_or(session.state.worklog.committed_seq + 1);
        sm = compact(session, lease, s, cut, cut_seq, "context_limit").await?;
        let summary_cost = tokens(&sm.history_summary);
        window = read_window(session, &sm, budget.saturating_sub(summary_cost))?;
        if !window.reached_start {
            return Err(OpenDanError::Other(
                "history still exceeds the budget after compaction".into(),
            ));
        }
    }
    Ok((history_message(&sm, &window), window))
}

/// Mid-run compaction at the context limit (§4.4): the run's history so far
/// is already in the worklog. The start point moves so that at most `keep`
/// tokens of raw records remain, then the history message is rebuilt within
/// `budget`.
pub async fn compact_for_limit(
    session: &mut Session,
    lease: &Lease,
    summarizer: &dyn Summarizer,
    budget: u32,
    keep: u32,
) -> Result<Option<AiMessage>> {
    let sm = session.summary()?;
    let window = read_window(session, &sm, keep)?;
    if !window.reached_start {
        let cut = window
            .oldest_offset
            .unwrap_or(session.state.worklog.committed_bytes);
        let cut_seq = window
            .oldest_seq
            .unwrap_or(session.state.worklog.committed_seq + 1);
        compact(session, lease, summarizer, cut, cut_seq, "context_limit").await?;
    }
    Ok(build_history(session, lease, Some(summarizer), budget)
        .await?
        .0)
}

/// After a run: compact when the history occupies more than `ratio` of the
/// budget, keeping about half of the budget as raw records.
pub async fn maybe_compact(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&dyn Summarizer>,
    budget: u32,
    ratio: f32,
) -> Result<bool> {
    let Some(s) = summarizer else {
        return Ok(false);
    };
    let sm = session.summary()?;
    let full = read_window(session, &sm, budget)?;
    let used = full.tokens + tokens(&sm.history_summary);
    if full.reached_start && (used as f32) <= ratio * budget as f32 {
        return Ok(false);
    }
    let keep = read_window(session, &sm, budget / 2)?;
    let (Some(cut), Some(cut_seq)) = (keep.oldest_offset, keep.oldest_seq) else {
        return Ok(false);
    };
    if cut <= sm.start_offset {
        return Ok(false);
    }
    compact(session, lease, s, cut, cut_seq, "ratio").await?;
    Ok(true)
}
