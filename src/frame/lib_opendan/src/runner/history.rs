//! The history part of the next llm_context (§4.4).
//!
//! The history is the worklog rebuilt as messages, nothing truncated
//! (renderer `libopendan.history/4`): assistant responses with their tool
//! calls, tool results, user input, the final report delivery. A history
//! over its budget is compressed by `agent_tool::llm_compress` — the only
//! compression — into `head + [summary] + keep_exact + raw tail`; summary.json
//! keeps the summary block and where the head ends and the raw records start.
//!
//! A rebuild reads the head (`[0, head_end_offset)`), the summary block, then
//! the worklog **backwards** from the committed end down to `start_offset`,
//! stopping early when the budget runs out (the range is then read once more
//! in full and compressed). The worklog is never rewritten.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use agent_tool::llm_compress::{self, CompressOptions, CompressReport, CompressedSummary};
use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiRole, AiToolCall};
use llm_context::deps::{
    ByteHeuristicTokenizer, LLMContextDeps, LlmClient, ToolCallCtx, ToolDispatchError,
    ToolManager,
};
use llm_context::observation::Observation;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::lock::Lease;
use crate::protocol::*;
use crate::session::Session;

/// Fixed line after the summary block; part of the rebuilt history only,
/// never of the worklog.
pub const COMPRESSED_NOTE: &str = "Everything before this point was compressed; files in the session directory are intact — re-read only what you need";

/// How the rebuilt messages carry tool calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryShape {
    /// Function call contexts: native tool calls, each answered by its tool
    /// result.
    Native,
    /// Behavior contexts (no native tools): calls listed in the assistant
    /// text, results in a `<<last_step_action_results>>` user message.
    Text,
}

/// What history compression summarizes with.
#[derive(Clone)]
pub struct Summarizer {
    pub llm: Arc<dyn LlmClient>,
    /// Model alias of the summary calls (`llm.summarize` through AICC).
    pub model: String,
}

pub(super) async fn build_chat_history(
    session: &mut Session,
    lease: &Lease,
    summarizer: &Summarizer,
    budget: u32,
) -> Result<Vec<AiMessage>> {
    let sm = session.summary()?;
    if sm.start_offset == 0 && sm.history_summary.is_empty() {
        if let Some(run_id) = session.state.last_run.as_deref() {
            let (record, snapshot) = session.dir.runs().load_checked(run_id)?;
            let ended_turn = crate::session::runs::finish_info(&record)
                .is_some_and(|f| !f["turn_end"].is_null());
            if record.config.loop_model == agent_tool::xllm::LoopModel::FunctionCall && ended_turn {
                let snapshot = snapshot.ok_or_else(|| {
                    OpenDanError::blocked("the previous chat run has no snapshot", Some(run_id))
                })?;
                if super::receipts::snapshot_host_meta(&snapshot).process_entry
                    == session.state.process_entry
                {
                    let messages: Vec<_> = snapshot
                        .state
                        .accumulated
                        .into_iter()
                        .skip_while(|m| m.role == AiRole::System)
                        // A budget line spoke of that run's budget only.
                        .filter(|m| {
                            m.role != AiRole::User
                                || !super::budget::is_budget_line(&m.text_content())
                        })
                        .collect();
                    if estimate(&messages) <= u64::from(budget) {
                        return Ok(messages);
                    }
                }
            }
        }
    }
    build_history(session, lease, Some(summarizer), HistoryShape::Native, budget).await
}

/// Rough token estimate (bytes / 4), same heuristic as the waist.
pub fn tokens(s: &str) -> u32 {
    ((s.len() as u64 + 3) / 4).min(u32::MAX as u64) as u32
}

/// The waist's estimate of messages.
pub fn estimate(messages: &[AiMessage]) -> u64 {
    llm_context::context_window::estimate_messages(&ByteHeuristicTokenizer, messages)
}

/// Run a transcript entry belongs to (`None`: session level entries).
fn entry_run(body: &WorklogBody) -> Option<&str> {
    match body {
        WorklogBody::InputBatch { run_id, .. }
        | WorklogBody::UserMessage { run_id, .. }
        | WorklogBody::AssistantMessage { run_id, .. }
        | WorklogBody::ReportDelivery { run_id, .. }
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
/// the child's whole transcript mixed into its own. A final report shows
/// once, as its delivery: the report call's arguments become a reference.
#[derive(Default)]
struct ReturnedChildren {
    children: HashSet<String>,
    deliveries: HashMap<String, ReportSource>,
}

impl ReturnedChildren {
    fn note(&mut self, body: &WorklogBody) {
        if let WorklogBody::ReportDelivery {
            run_id, submission, ..
        } = body
        {
            self.deliveries
                .insert(run_id.clone(), submission.source.clone());
        }
        if let WorklogBody::Outcome { run_id, kind, .. } = body {
            if kind == "process_done" {
                self.children.insert(run_id.clone());
            }
        }
    }

    /// The entry is part of a returned child's transcript (not its result).
    fn hides(&self, body: &WorklogBody) -> bool {
        if matches!(body, WorklogBody::Outcome { kind, .. } if kind == "process_done") {
            return false;
        }
        if let WorklogBody::Step {
            run_id, step_index, ..
        } = body
        {
            if self.deliveries.get(run_id)
                == Some(&ReportSource::Behavior {
                    step_index: *step_index,
                })
            {
                return true;
            }
        }
        entry_run(body).is_some_and(|r| self.children.contains(r))
    }

    fn delivered_call(&self, run_id: &str, call_id: &str) -> bool {
        matches!(self.deliveries.get(run_id), Some(ReportSource::Tool { call_id: c }) if c == call_id)
    }
}

/// Worklog position (`offset`, `seq`) of the entry a message starts at.
type Pos = (u64, u64);

/// Worklog entries (oldest first) → messages.
struct Render<'a> {
    shape: HistoryShape,
    children: &'a ReturnedChildren,
    messages: Vec<AiMessage>,
    positions: Vec<Option<Pos>>,
    /// Native: calls of the last assistant message without a result yet.
    open: Vec<String>,
    /// Input and notes that came while calls were open.
    deferred: Vec<(Pos, AiMessage)>,
    /// Text results being collected: position, body.
    results: Option<(Pos, String)>,
    results_header: String,
    notes: Option<(Pos, String)>,
}

impl<'a> Render<'a> {
    fn new(shape: HistoryShape, children: &'a ReturnedChildren) -> Self {
        Self {
            shape,
            children,
            messages: Vec::new(),
            positions: Vec::new(),
            open: Vec::new(),
            deferred: Vec::new(),
            results: None,
            results_header: format!("{}>>", llm_compress::STEP_RESULTS_PREFIX),
            notes: None,
        }
    }

    fn push(&mut self, pos: Option<Pos>, m: AiMessage) {
        self.messages.push(m);
        self.positions.push(pos);
    }

    /// A user message; held back while calls wait for their results.
    fn user(&mut self, pos: Pos, m: AiMessage) {
        if self.open.is_empty() {
            self.push(Some(pos), m);
        } else {
            self.deferred.push((pos, m));
        }
    }

    fn flush_results(&mut self) {
        if let Some((pos, body)) = self.results.take() {
            self.push(
                Some(pos),
                AiMessage::text(
                    AiRole::User,
                    format!("{}\n{body}<</last_step_action_results>>", self.results_header),
                ),
            );
        }
    }

    fn flush_notes(&mut self) {
        if let Some((pos, text)) = self.notes.take() {
            self.user(pos, AiMessage::text(AiRole::User, text));
        }
    }

    /// Calls left without a result get one, then the held back messages
    /// follow.
    fn close_calls(&mut self) {
        for call_id in std::mem::take(&mut self.open) {
            self.push(
                None,
                AiMessage::new(
                    AiRole::Tool,
                    vec![AiContent::tool_result_text(
                        call_id,
                        "[no result recorded]",
                        true,
                    )],
                ),
            );
        }
        for (pos, m) in std::mem::take(&mut self.deferred) {
            self.push(Some(pos), m);
        }
    }

    fn note(&mut self, pos: Pos, line: String) {
        match &mut self.notes {
            Some((_, text)) => {
                text.push('\n');
                text.push_str(&line);
            }
            None => self.notes = Some((pos, line)),
        }
    }

    /// A model response: what was pending before it goes first.
    fn response(&mut self, pos: Pos, m: AiMessage) {
        self.flush_results();
        self.close_calls();
        self.flush_notes();
        self.push(Some(pos), m);
    }

    fn entry(&mut self, pos: Pos, body: &WorklogBody) {
        if let WorklogBody::ActionResult {
            call_id,
            status,
            result,
            ..
        } = body
        {
            if let Some(i) = self.open.iter().position(|c| c == call_id) {
                self.open.remove(i);
                self.push(
                    Some(pos),
                    AiMessage::new(
                        AiRole::Tool,
                        vec![AiContent::tool_result_text(
                            call_id.clone(),
                            result.clone(),
                            status == "error",
                        )],
                    ),
                );
                if self.open.is_empty() {
                    self.close_calls();
                }
            } else {
                let body = self.results.get_or_insert_with(|| (pos, String::new()));
                body.1.push_str(&format!(
                    "- #{call_id} {status}\n```output\n{result}\n```\n"
                ));
            }
            return;
        }
        self.flush_results();
        match body {
            WorklogBody::UserMessage { content, brief, .. } => {
                self.flush_notes();
                let text = brief.clone().unwrap_or_else(|| content.clone());
                self.user(pos, AiMessage::text(AiRole::User, text));
            }
            WorklogBody::AssistantMessage {
                run_id,
                assistant,
                tool_calls,
                ..
            } => {
                let calls: Vec<ActionEntry> = tool_calls
                    .iter()
                    .map(|c| {
                        let mut c = c.clone();
                        if self.children.delivered_call(run_id, &c.call_id) {
                            c.args = json!({"delivery": "report_delivery"});
                        }
                        c
                    })
                    .collect();
                match self.shape {
                    HistoryShape::Native => {
                        let mut content = Vec::new();
                        if !assistant.trim().is_empty() {
                            content.push(AiContent::text(assistant.clone()));
                        }
                        for c in &calls {
                            content.push(AiContent::ToolUse {
                                call_id: c.call_id.clone(),
                                name: c.tool.clone(),
                                args: args_map(&c.args),
                            });
                        }
                        if content.is_empty() {
                            return;
                        }
                        self.response(pos, AiMessage::new(AiRole::Assistant, content));
                        self.open = calls.into_iter().map(|c| c.call_id).collect();
                    }
                    HistoryShape::Text => {
                        let text = with_calls(assistant, &calls);
                        if text.is_empty() {
                            return;
                        }
                        self.response(pos, AiMessage::text(AiRole::Assistant, text));
                        self.results_header = format!("{}>>", llm_compress::STEP_RESULTS_PREFIX);
                    }
                }
            }
            WorklogBody::Step {
                step_index,
                behavior,
                assistant,
                actions,
                correction,
                ..
            } => {
                let mut text = if assistant.trim().is_empty() {
                    with_calls("", actions)
                } else {
                    assistant.clone()
                };
                if *correction {
                    text = format!("[correction] {text}");
                }
                self.response(pos, AiMessage::text(AiRole::Assistant, text));
                self.results_header = format!(
                    "{} behavior=\"{}\" step=\"{step_index}\">>",
                    llm_compress::STEP_RESULTS_PREFIX,
                    behavior.as_deref().unwrap_or_default()
                );
            }
            WorklogBody::ReportDelivery { assistant, .. } => {
                self.response(pos, AiMessage::text(AiRole::Assistant, assistant.clone()));
            }
            WorklogBody::Outcome {
                kind,
                next_behavior,
                report,
                ..
            } => {
                // A run's normal end and the rewrite boundary are not part
                // of what the model saw.
                if matches!(kind.as_str(), "done" | "wait" | "context_rewritten") {
                    return;
                }
                self.note(
                    pos,
                    format!(
                        "[outcome {kind}{}]{}",
                        next_behavior
                            .as_ref()
                            .map(|b| format!(" → {b}"))
                            .unwrap_or_default(),
                        report
                            .as_ref()
                            .map(|r| format!(" {r}"))
                            .unwrap_or_default()
                    ),
                );
            }
            WorklogBody::Created { objective, .. } => {
                self.note(pos, format!("[created] {objective}"))
            }
            WorklogBody::TurnStarted {
                turn,
                inputs,
                events,
                hook,
                ..
            } => self.note(
                pos,
                format!(
                    "── turn {turn}{}{} ──",
                    hook.as_ref().map(|h| format!(" ({h})")).unwrap_or_default(),
                    input_list(inputs, events)
                ),
            ),
            WorklogBody::InputBatch {
                inputs,
                events,
                hook,
                ..
            } => self.note(
                pos,
                format!(
                    "── {}{} ──",
                    hook.as_deref().unwrap_or("input"),
                    input_list(inputs, events)
                ),
            ),
            WorklogBody::TurnEnded { turn, status, .. } => {
                if let Some(status) = serde_json::to_value(status)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                {
                    self.note(pos, format!("── turn {turn} {status} ──"));
                }
            }
            WorklogBody::Decide { decision, by, .. } => {
                self.note(pos, format!("[decide] {decision} by {by}"))
            }
            WorklogBody::InputRejected { input, reason, .. } => {
                self.note(pos, format!("[rejected {}] {reason}", input.id()))
            }
            WorklogBody::EventDropped { input, reason } => {
                self.note(pos, format!("[event dropped {}] {reason}", input.id()))
            }
            WorklogBody::ControlApplied { command, .. } => {
                self.note(pos, format!("[control] {command}"))
            }
            WorklogBody::Compaction { .. } | WorklogBody::ActionResult { .. } => {}
        }
    }

    fn finish(mut self) -> (Vec<AiMessage>, Vec<Option<Pos>>) {
        self.flush_results();
        self.close_calls();
        self.flush_notes();
        self.close_calls();
        (self.messages, self.positions)
    }
}

fn args_map(args: &Value) -> HashMap<String, Value> {
    match args {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Null => HashMap::new(),
        other => HashMap::from([("value".to_string(), other.clone())]),
    }
}

/// An assistant text with its calls listed (text shape).
fn with_calls(assistant: &str, calls: &[ActionEntry]) -> String {
    let mut s = assistant.trim().to_string();
    for c in calls {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&format!("→ {}({}) #{}", c.tool, c.args, c.call_id));
    }
    s
}

fn input_list(inputs: &[InputRef], events: &[String]) -> String {
    let ins: Vec<String> = inputs.iter().map(|i| i.id()).collect();
    format!(
        "{}{}",
        if ins.is_empty() {
            String::new()
        } else {
            format!(" inputs: {}", ins.join(","))
        },
        if events.is_empty() {
            String::new()
        } else {
            format!(" events: {}", events.join(","))
        }
    )
}

fn render(
    entries: &[(u64, WorklogEntry)],
    shape: HistoryShape,
    children: &ReturnedChildren,
) -> (Vec<AiMessage>, Vec<Option<Pos>>) {
    let mut r = Render::new(shape, children);
    for (offset, e) in entries {
        r.entry((*offset, e.seq), &e.body);
    }
    r.finish()
}

/// Upper estimate of what an entry renders to (the early stop of a
/// budgeted reverse read).
fn entry_tokens(body: &WorklogBody) -> u64 {
    u64::from(tokens(&serde_json::to_string(body).unwrap_or_default())) + 4
}

fn dropped(session: &Session, body: &WorklogBody) -> bool {
    session
        .config
        .prompt
        .history_render
        .drop_kinds
        .iter()
        .any(|k| k == body.kind())
}

/// Raw records after `summary.start_offset`, rebuilt as messages.
#[derive(Debug, Clone, Default)]
pub struct HistoryWindow {
    /// Oldest first.
    pub messages: Vec<AiMessage>,
    positions: Vec<Option<Pos>>,
    pub tokens: u64,
    /// `true` when the reverse read reached `start_offset`.
    pub reached_start: bool,
    /// Byte offset / seq of the oldest entry read.
    pub oldest_offset: Option<u64>,
    pub oldest_seq: Option<u64>,
    /// Bytes read from the worklog (bounded by the window, not file size).
    pub bytes_read: u64,
}

/// Reverse-read the committed worklog down to `summary.start_offset`,
/// stopping early once the entries exceed `budget` tokens (`None`: read all).
pub fn read_window(
    session: &Session,
    summary: &SessionSummary,
    shape: HistoryShape,
    budget: Option<u64>,
) -> Result<HistoryWindow> {
    let wl = session.dir.worklog();
    let end = session.state.worklog.committed_bytes;
    let mut r = wl.reverse(end, summary.start_offset)?;
    let mut children = ReturnedChildren::default();
    let mut rev = Vec::new();
    let mut used = 0u64;
    let mut reached_start = true;
    while let Some((offset, e)) = r.next_json::<WorklogEntry>()? {
        // Read backwards: a child's `process_done` comes before its entries.
        children.note(&e.body);
        if children.hides(&e.body) || dropped(session, &e.body) {
            continue;
        }
        let t = entry_tokens(&e.body);
        if budget.is_some_and(|b| used + t > b) {
            reached_start = false;
            break;
        }
        used += t;
        rev.push((offset, e));
    }
    rev.reverse();
    let (messages, positions) = render(&rev, shape, &children);
    Ok(HistoryWindow {
        tokens: estimate(&messages),
        messages,
        positions,
        reached_start,
        oldest_offset: rev.first().map(|(o, _)| *o),
        oldest_seq: rev.first().map(|(_, e)| e.seq),
        bytes_read: r.bytes_read,
    })
}

/// The whole history: head + summary block + raw records.
struct Assembled {
    messages: Vec<AiMessage>,
    positions: Vec<Option<Pos>>,
    tokens: u64,
    /// Read to the start point and within the budget.
    complete: bool,
}

fn assemble(
    session: &Session,
    sm: &SessionSummary,
    shape: HistoryShape,
    budget: Option<u64>,
) -> Result<Assembled> {
    let mut messages = Vec::new();
    let mut positions = Vec::new();
    if sm.head_end_offset > 0 {
        let entries: Vec<(u64, WorklogEntry)> = session
            .dir
            .worklog()
            .read_range(0, sm.head_end_offset)?
            .into_iter()
            .filter(|(_, e)| !dropped(session, &e.body))
            .collect();
        let mut children = ReturnedChildren::default();
        for (_, e) in &entries {
            children.note(&e.body);
        }
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|(_, e)| !children.hides(&e.body))
            .collect();
        let (m, p) = render(&entries, shape, &children);
        messages.extend(m);
        positions.extend(p);
    }
    if !sm.history_summary.trim().is_empty() {
        messages.push(llm_compress::summary_message(
            &CompressedSummary {
                summary: sm.history_summary.clone(),
                keep_exact: sm.keep_exact.clone(),
            },
            Some(COMPRESSED_NOTE),
        ));
        positions.push(None);
    }
    let fixed = estimate(&messages);
    let window = read_window(session, sm, shape, budget.map(|b| b.saturating_sub(fixed)))?;
    messages.extend(window.messages);
    positions.extend(window.positions);
    let tokens = estimate(&messages);
    Ok(Assembled {
        complete: window.reached_start && budget.map_or(true, |b| tokens <= b),
        messages,
        positions,
        tokens,
    })
}

struct NoTools;

#[async_trait]
impl ToolManager for NoTools {
    async fn call_tool(
        &self,
        call: AiToolCall,
        _ctx: ToolCallCtx,
    ) -> std::result::Result<Observation, ToolDispatchError> {
        Ok(Observation::Error {
            call_id: call.call_id,
            message: "history compression calls no tools".into(),
            host: None,
        })
    }
}

/// Compress the whole history with llm_compress to `target` tokens and
/// keep the result in summary.json (the start point moves to the raw tail,
/// the head end to where the head stops), then append the audit entry and
/// commit. Returns the compressed history.
async fn compact(
    session: &mut Session,
    lease: &Lease,
    summarizer: &Summarizer,
    sm: &SessionSummary,
    full: Assembled,
    target: u32,
    made_by: &str,
) -> Result<(Vec<AiMessage>, CompressReport)> {
    let started = Instant::now();
    let mut event = json!({
        "session_id": session.sid(),
        "reason": made_by,
        "model": summarizer.model,
        "target_tokens": target,
        "history_tokens_before": full.tokens,
        "start_seq_before": sm.start_seq,
        "head_end_seq_before": sm.head_end_seq,
        "summary_tokens_before": tokens(&sm.history_summary),
    });
    log::info!("opendan.context.compaction.started {}", event);
    let deps = LLMContextDeps::new(summarizer.llm.clone(), Arc::new(NoTools));
    let opts = CompressOptions::new(target, summarizer.model.clone()).with_note(COMPRESSED_NOTE);
    let out = match llm_compress::compress_history(&full.messages, &deps, &opts).await {
        Ok(out) => out,
        Err(error) => {
            event["error"] = json!(error.to_string());
            event["elapsed_ms"] = json!(started.elapsed().as_millis());
            log::warn!("opendan.context.compaction.failed {}", event);
            return Err(OpenDanError::Llm(format!("compress history: {error}")));
        }
    };
    event["strategy"] = json!(out.report.strategy);
    event["history_tokens_after"] = json!(out.report.tokens_after);
    event["reached_target"] = json!(out.report.reached_target);
    let Some(summary) = out.summary.clone() else {
        event["elapsed_ms"] = json!(started.elapsed().as_millis());
        log::warn!("opendan.context.compaction.skipped {}", event);
        return Ok((out.messages, out.report));
    };
    let committed_end = (
        session.state.worklog.committed_bytes,
        session.state.worklog.committed_seq + 1,
    );
    let at = |i: usize| full.positions.get(i).copied().flatten();
    let (head_end_offset, head_end_seq) = if out.head_end == 0 {
        (0, 0)
    } else {
        at(out.head_end).unwrap_or((sm.head_end_offset, sm.head_end_seq))
    };
    let (start_offset, start_seq) = at(out.tail_start).unwrap_or(committed_end);
    let new = SessionSummary {
        schema: SESSION_SUMMARY_SCHEMA.to_string(),
        history_summary: summary.summary,
        keep_exact: summary.keep_exact,
        head_end_seq,
        head_end_offset,
        start_seq,
        start_offset,
        renderer: HISTORY_RENDERER.to_string(),
        made_at_seq: session.state.worklog.committed_seq,
        made_by: made_by.to_string(),
        updated_at_ms: crate::now_ms(),
    };
    // The clues shown before are no longer in the history (only in its
    // summary): the session's observation may show them again.
    if let Some(m) = session.state.memory.as_mut() {
        m.bump_context_generation();
    }
    let persisted: Result<()> = (|| {
        session.write_summary(lease, &new)?;
        session.append_worklog(
            lease,
            vec![WorklogBody::Compaction {
                summary_start_seq: start_seq,
                made_by: made_by.to_string(),
            }],
        )?;
        session.commit_state(lease)
    })();
    event["start_seq_after"] = json!(start_seq);
    event["head_end_seq_after"] = json!(head_end_seq);
    event["summary_tokens_after"] = json!(out.report.summary_tokens);
    event["summary_calls"] = json!(out.report.summary_calls);
    event["keep_exact"] = json!(out.report.keep_exact);
    event["memory_candidates"] = json!(out.memory_candidates);
    event["range_rounds"] = json!(out.report.range_rounds);
    event["tail_rounds"] = json!(out.report.tail_rounds);
    event["elapsed_ms"] = json!(started.elapsed().as_millis());
    match persisted {
        Ok(()) => log::info!("opendan.context.compaction.completed {}", event),
        Err(error) => {
            event["error"] = json!(error.to_string());
            log::warn!("opendan.context.compaction.failed {}", event);
            return Err(error);
        }
    }
    Ok((out.messages, out.report))
}

/// History messages for a new llm_context within `budget` tokens:
/// compressed first when they do not fit.
pub async fn build_history(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&Summarizer>,
    shape: HistoryShape,
    budget: u32,
) -> Result<Vec<AiMessage>> {
    Ok(build(session, lease, summarizer, shape, budget, None)
        .await?
        .0)
}

async fn build(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&Summarizer>,
    shape: HistoryShape,
    budget: u32,
    line: Option<&LimitLine>,
) -> Result<(Vec<AiMessage>, Option<CompressReport>)> {
    let sm = session.summary()?;
    let window = assemble(session, &sm, shape, Some(u64::from(budget)))?;
    let mut event = json!({
        "session_id": session.sid(),
        "phase": if line.is_some() { "context_limit" } else { "build_history" },
        "reason": if line.is_some() { "context_limit" } else { "history_budget" },
        "budget_tokens": budget,
        "summary_tokens": tokens(&sm.history_summary),
        "history_tokens": window.tokens,
        "history_tokens_lower_bound": !window.complete,
    });
    if let Some(line) = line {
        event["window_tokens"] = json!(line.window_tokens);
        event["target_tokens"] = json!(line.target_tokens);
        event["prefix_tokens"] = json!(line.prefix_tokens);
    }
    if window.complete {
        event["skip_reason"] = json!("within_history_budget");
        if line.is_some() {
            log::info!("opendan.context.compaction.skipped {}", event);
        } else {
            log::debug!("opendan.context.compaction.skipped {}", event);
        }
        return Ok((window.messages, None));
    }
    log::info!("opendan.context.compaction.triggered {}", event);
    let Some(s) = summarizer else {
        event["failure_reason"] = json!("missing_summarizer");
        log::warn!("opendan.context.compaction.unavailable {}", event);
        return Err(OpenDanError::Other(
            "history exceeds the budget and no summarizer is configured".into(),
        ));
    };
    let full = assemble(session, &sm, shape, None)?;
    let (messages, report) = compact(session, lease, s, &sm, full, budget, "context_limit").await?;
    if !report.reached_target {
        log::warn!(
            "opendan.context.compaction.budget_exceeded {}",
            json!({
                "session_id": session.sid(),
                "budget_tokens": budget,
                "history_tokens": report.tokens_after,
                "strategy": report.strategy,
            })
        );
    }
    Ok((messages, Some(report)))
}

/// Where a mid-run rewrite at the context limit brings the request back to
/// (work context budget TODO §3.1): `target = ⌈window × ratio⌉`, of which
/// the history gets what the system prefix (system messages, tool schemas)
/// leaves, never less than the session's history budget (also used as is
/// when the window is unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimitLine {
    pub window_tokens: Option<u32>,
    pub target_tokens: Option<u32>,
    pub prefix_tokens: u32,
    /// Budget of the rebuilt history (head, summary block, raw records).
    pub history_budget: u32,
}

impl LimitLine {
    pub fn new(window: Option<u32>, ratio: f32, prefix_tokens: u64, floor: u32) -> Self {
        let prefix_tokens = prefix_tokens.min(u64::from(u32::MAX)) as u32;
        // The f32 ratio to six decimals: 0.3 means 0.3, not 0.30000001.
        let ratio = (f64::from(ratio.min(1.0)) * 1e6).round() / 1e6;
        let target = window
            .filter(|_| ratio.is_finite() && ratio > 0.0)
            .map(|w| (f64::from(w) * ratio).ceil() as u32);
        Self {
            window_tokens: window,
            target_tokens: target,
            prefix_tokens,
            history_budget: target
                .map(|t| t.saturating_sub(prefix_tokens).max(floor))
                .unwrap_or(floor),
        }
    }
}

/// Mid-run compaction at the context limit (§4.4): the run's history so far
/// is already in the worklog. The history is rebuilt within `budget`
/// (`line.history_budget` on a first rewrite, less on the next ones),
/// compressed by llm_compress when it does not fit.
pub async fn compact_for_limit(
    session: &mut Session,
    lease: &Lease,
    summarizer: &Summarizer,
    shape: HistoryShape,
    line: &LimitLine,
    budget: u32,
) -> Result<(Vec<AiMessage>, Option<CompressReport>)> {
    build(session, lease, Some(summarizer), shape, budget, Some(line)).await
}

/// After a run: compress when the history occupies more than `ratio` of the
/// budget, down to about half of the budget.
pub async fn maybe_compact(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&Summarizer>,
    budget: u32,
    ratio: f32,
) -> Result<bool> {
    let mut event = json!({
        "session_id": session.sid(),
        "phase": "after_run",
        "reason": "ratio",
        "budget_tokens": budget,
        "ratio": ratio,
        "threshold_tokens": ratio * budget as f32,
    });
    let Some(s) = summarizer else {
        event["failure_reason"] = json!("missing_summarizer");
        log::warn!("opendan.context.compaction.unavailable {}", event);
        return Ok(false);
    };
    let sm = session.summary()?;
    let window = assemble(session, &sm, HistoryShape::Native, Some(u64::from(budget)))?;
    event["history_tokens"] = json!(window.tokens);
    event["history_tokens_lower_bound"] = json!(!window.complete);
    if window.complete && (window.tokens as f32) <= ratio * budget as f32 {
        event["skip_reason"] = json!("below_threshold");
        log::debug!("opendan.context.compaction.skipped {}", event);
        return Ok(false);
    }
    log::info!("opendan.context.compaction.triggered {}", event);
    let full = assemble(session, &sm, HistoryShape::Native, None)?;
    let (_, report) = compact(session, lease, s, &sm, full, budget / 2, "ratio").await?;
    Ok(report.strategy == "llm")
}
