//! The history part of the next llm_context (§4.4).
//!
//! The history is the worklog rebuilt as messages (renderer
//! `libopendan.history/5`): assistant responses with their tool calls, tool
//! results, user input, the final report delivery. Tool results are shown as
//! the model saw them, except when the history is compressed or the
//! provider's prompt cache is gone (`prompt_cache_idle` since the last run):
//! then the older ones are shown by position ([`Tiering::ByPosition`]) — the
//! latest [`FULL_ROUNDS`] rounds in full, the [`SUMMARY_ROUNDS`] before them
//! by their summary, older ones by their title, each with how to read the
//! original again. A history still over its budget is compressed by
//! `agent_tool::llm_compress` into `head + [summary] + keep_exact + raw
//! tail`; summary.json keeps the summary block and where the head ends and
//! the raw records start. When the summarizer fails, a history that fits
//! once the older tool results are clipped (`llm_compress::trim_tool_results`)
//! is used for this context only; summary.json is not touched.
//!
//! What a history shows in full ([`Built::shown`]) seeds the context's
//! result meta and its hint ledger (prompt mechanics TODO D1, D4).
//!
//! A rebuild reads the head (`[0, head_end_offset)`), the summary block, then
//! the worklog **backwards** from the committed end down to `start_offset`,
//! stopping early when the budget runs out (the range is then read once more
//! in full and compressed). The worklog is never rewritten.

use std::collections::{BTreeMap, HashMap, HashSet};
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

/// Opening of the fixed lines after the summary block; part of the rebuilt
/// history only, never of the worklog.
pub const COMPRESSED_NOTE: &str = "Everything before this point was compressed; files in the session directory are intact — re-read only what you need";

/// Recorded perceptions listed after a summary, the newest, and how much of
/// each.
const COMPRESSED_PERCEPTIONS: usize = 40;
const COMPRESSED_PERCEPTION_CHARS: usize = 300;

/// The fixed lines after the summary block of `session`: how to read the
/// original words again, and what it recorded. They change only with a
/// compression.
fn compressed_note(session: &Session, perceptions: &[String]) -> String {
    let mut note = COMPRESSED_NOTE.to_string();
    let sid = session.sid();
    if !session.config.runtime.is_node() {
        note.push_str(&format!(
            ". The original messages are kept: `agent-session read-session {sid} --turn <n>` shows those of one turn (`--after <cursor>` goes on from its `next_after`)"
        ));
    }
    if !perceptions.is_empty() {
        note.push_str("\nNotes you recorded in this session (`agent-session recall --read <ref>` shows one whole):");
        for p in perceptions {
            note.push_str(&format!("\n- {p}"));
        }
    }
    note
}

/// What the session recorded, from its worklog (oldest first, the newest
/// [`COMPRESSED_PERCEPTIONS`]).
fn recorded_perceptions(session: &Session) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut r = session
        .dir
        .worklog()
        .reverse(session.state.worklog.committed_bytes, 0)?;
    while let Some((_, e)) = r.next_json::<WorklogEntry>()? {
        let WorklogBody::ControlApplied { command, detail, .. } = e.body else {
            continue;
        };
        let Some(reference) = detail.get("reference").and_then(Value::as_str) else {
            continue;
        };
        if command != "perceive" {
            continue;
        }
        let text: String = detail
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(COMPRESSED_PERCEPTION_CHARS)
            .collect();
        let turn = detail.get("turn").and_then(Value::as_u64).unwrap_or(0);
        out.push(format!("{reference} (turn {turn}): {text}"));
        if out.len() == COMPRESSED_PERCEPTIONS {
            break;
        }
    }
    out.reverse();
    Ok(out)
}

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

/// How a rebuilt history shows older tool results (prompt mechanics TODO C1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tiering {
    /// Every result as the model saw it.
    #[default]
    Full,
    /// By distance from the end (see the module doc).
    ByPosition,
}

/// Rounds at the end whose results a [`Tiering::ByPosition`] history shows
/// in full.
pub const FULL_ROUNDS: u64 = 8;
/// Rounds before them whose results it shows by their summary; older ones
/// by their title.
pub const SUMMARY_ROUNDS: u64 = 16;
/// What a result without a summary keeps in the summary tier.
const SUMMARY_TIER_CHARS: usize = 600;
/// What a result without a title keeps in the title tier.
const TITLE_TIER_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Full,
    Summary,
    Title,
}

/// A history built for a new context, and what it shows in full: the keys
/// of each result it shows as the model saw it (`ResultMeta::keys`), by
/// call id.
#[derive(Debug, Clone, Default)]
pub struct Built {
    pub messages: Vec<AiMessage>,
    pub report: Option<CompressReport>,
    pub shown: BTreeMap<String, ResultMeta>,
}

/// A result shown in full: the message it is in, its call and keys.
#[derive(Debug, Clone)]
struct Shown {
    message: usize,
    call_id: String,
    keys: ResultMeta,
}

fn shown_map<'a>(shown: impl Iterator<Item = &'a Shown>) -> BTreeMap<String, ResultMeta> {
    shown
        .filter(|s| s.keys.has_keys())
        .map(|s| (s.call_id.clone(), s.keys.clone()))
        .collect()
}

/// The rounds and results of the entries a history is rendered from.
struct TierCtx {
    tiering: Tiering,
    rounds: u64,
    round_of: HashMap<String, u64>,
    results: HashMap<String, (String, ResultMeta, u64)>,
    /// The command that reads a Turn's original messages again.
    recover: Option<String>,
}

impl TierCtx {
    fn new<'a>(
        bodies: impl Iterator<Item = &'a WorklogBody>,
        tiering: Tiering,
        recover: Option<String>,
    ) -> Self {
        let mut ctx = Self {
            tiering,
            rounds: 0,
            round_of: HashMap::new(),
            results: HashMap::new(),
            recover,
        };
        for body in bodies {
            match body {
                WorklogBody::AssistantMessage { tool_calls, .. } if !tool_calls.is_empty() => {
                    ctx.rounds += 1
                }
                WorklogBody::Step { .. } => ctx.rounds += 1,
                WorklogBody::ActionResult {
                    call_id,
                    result,
                    meta,
                    turn,
                    ..
                } => {
                    ctx.round_of.insert(call_id.clone(), ctx.rounds);
                    ctx.results
                        .insert(call_id.clone(), (result.clone(), meta.clone(), *turn));
                }
                _ => {}
            }
        }
        ctx
    }

    fn tier(&self, call_id: &str) -> Tier {
        if self.tiering == Tiering::Full {
            return Tier::Full;
        }
        let Some(round) = self.round_of.get(call_id) else {
            return Tier::Full;
        };
        let distance = self.rounds.saturating_sub(*round);
        if distance < FULL_ROUNDS {
            Tier::Full
        } else if distance < FULL_ROUNDS + SUMMARY_ROUNDS {
            Tier::Summary
        } else {
            Tier::Title
        }
    }

    /// The text result `call_id` shows, and its keys when it is the one the
    /// model saw. A dedup hint whose earlier result is not shown in full
    /// shows that result instead.
    fn show(
        &self,
        call_id: &str,
        result: &str,
        meta: &ResultMeta,
        turn: u64,
    ) -> (String, Option<ResultMeta>) {
        let (result, meta, turn) = match &meta.dedup_of {
            Some(of) => match self.results.get(of) {
                Some(_) if self.tier(of) == Tier::Full => return (result.to_string(), None),
                Some((r, m, t)) => (r.as_str(), m, *t),
                None => {
                    return (
                        format!("{result} That result is no longer in this history: call again if you need it."),
                        None,
                    )
                }
            },
            None => (result, meta, turn),
        };
        let tier = self.tier(call_id);
        if tier != Tier::Full {
            let short = self.shorten(tier, result, meta, turn);
            if short.len() < result.len() {
                return (short, None);
            }
        }
        (result.to_string(), Some(meta.keys()))
    }

    fn shorten(&self, tier: Tier, result: &str, meta: &ResultMeta, turn: u64) -> String {
        let clip = |s: &str, n: usize| -> String {
            let mut out: String = s.chars().take(n).collect();
            if out.len() < s.len() {
                out.push('…');
            }
            out
        };
        let nonempty = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        let body = match tier {
            Tier::Summary => nonempty(&meta.summary)
                .or_else(|| nonempty(&meta.title))
                .unwrap_or_else(|| clip(result.trim(), SUMMARY_TIER_CHARS)),
            _ => nonempty(&meta.title)
                .or_else(|| meta.summary.lines().next().and_then(nonempty))
                .unwrap_or_else(|| clip(result.trim(), TITLE_TIER_CHARS)),
        };
        match &self.recover {
            Some(cmd) => format!("{body}\n[shortened; the full result: `{cmd} --turn {turn}`]"),
            None => format!("{body}\n[shortened: call again if you need it in full]"),
        }
    }
}

/// What history compression summarizes with.
#[derive(Clone)]
pub struct Summarizer {
    pub llm: Arc<dyn LlmClient>,
    /// Model alias of the summary calls (`llm.summarize` through AICC).
    pub model: String,
}

/// The lines of a history between Turns: compressed once it exceeds
/// `trigger`, down to `target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLine {
    pub trigger: u32,
    pub target: u32,
}

/// Least room a window's trigger leaves for the history, whatever the prefix
/// takes.
const MIN_TRIGGER_TOKENS: u32 = 2_048;

impl HistoryLine {
    /// A conversation follows its model's window, with the lines of the
    /// mid-run rewrite (`context_yield_ratio` starts it, the target ratio is
    /// where it lands), less what the request carries besides
    /// (`prefix_tokens`). `None`: the session states a history budget of its
    /// own, is no conversation, or the window is unknown — the fixed budget
    /// holds.
    pub fn of_window(
        cfg: &SessionConfig,
        options: &super::RunnerOptions,
        limits: &agent_tool::xllm::RunLimits,
        prefix_tokens: u64,
    ) -> Option<Self> {
        if cfg.session.kind != SessionKind::Ui || cfg.prompt.history_budget_tokens.is_some() {
            return None;
        }
        let window = limits.context_window_tokens?;
        let ratio = limits
            .context_yield_ratio
            .unwrap_or(agent_tool::xllm::DEFAULT_CONTEXT_YIELD_RATIO);
        let prefix = prefix_tokens.min(u64::from(u32::MAX)) as u32;
        let trigger = ((f64::from(window) * f64::from(ratio)).floor() as u32)
            .saturating_sub(prefix)
            .max(MIN_TRIGGER_TOKENS);
        let line = LimitLine::new(
            Some(window),
            cfg.prompt.context_target_ratio.unwrap_or(options.context_target_ratio),
            prefix_tokens,
            options.history_budget_tokens,
        );
        Some(Self {
            trigger,
            // Well below the line that triggers it, also in a small window.
            target: line.history_budget.min(trigger / 2),
        })
    }

    /// A fixed budget: what fits it stays, what does not is brought to it.
    pub fn fixed(budget: u32) -> Self {
        Self {
            trigger: budget,
            target: budget,
        }
    }
}

/// The history of a chat's next run: the previous run's messages as they
/// were while they fit and the provider's prompt cache holds them, else
/// rebuilt (`cold`: the cache is gone, older results are shown by position).
/// What a compression replaced is not in them: the previous run ended after
/// it. `rebuild`: the context starts anew, its history brought to the target.
pub(super) async fn build_chat_history(
    session: &mut Session,
    lease: &Lease,
    summarizer: &Summarizer,
    line: HistoryLine,
    rebuild: bool,
    cold: bool,
) -> Result<Built> {
    let sm = session.summary()?;
    let fits = if rebuild { line.target } else { line.trigger };
    if let Some(run_id) = session.state.last_run.as_deref().filter(|_| !cold) {
        let (record, snapshot) = session.dir.runs().load_checked(run_id)?;
        let ended_turn = crate::session::runs::finish_info(&record)
            .is_some_and(|f| !f["turn_end"].is_null());
        let current = sm.updated_at_ms <= record.updated_at_ms;
        if record.config.loop_model == agent_tool::xllm::LoopModel::FunctionCall && ended_turn && current {
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
                if estimate(&messages) <= u64::from(fits) {
                    // The same messages: what that run showed in full.
                    let calls: HashSet<&str> = messages
                        .iter()
                        .flat_map(|m| m.content.iter())
                        .filter_map(|c| match c {
                            AiContent::ToolResult { call_id, .. } => Some(call_id.as_str()),
                            _ => None,
                        })
                        .collect();
                    let shown = crate::session::runs::run_results(&record)
                        .into_iter()
                        .filter(|(c, m)| calls.contains(c.as_str()) && m.has_keys())
                        .map(|(c, m)| (c, m.keys()))
                        .collect();
                    return Ok(Built {
                        messages,
                        report: None,
                        shown,
                    });
                }
            }
        }
    }
    let made_by = if rebuild { "prompt_refrozen" } else { "context_limit" };
    build(
        session,
        lease,
        Some(summarizer),
        HistoryShape::Native,
        HistoryLine { trigger: fits, ..line },
        made_by,
        None,
        cold,
    )
    .await
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
        self.deliveries.get(run_id).and_then(ReportSource::call_id) == Some(call_id)
    }
}

/// Worklog position (`offset`, `seq`) of the entry a message starts at.
type Pos = (u64, u64);

/// Worklog entries (oldest first) → messages.
struct Render<'a> {
    shape: HistoryShape,
    children: &'a ReturnedChildren,
    tiers: &'a TierCtx,
    /// Results shown in full (the text shape's wait for their message).
    shown: Vec<Shown>,
    results_shown: Vec<(String, ResultMeta)>,
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
    fn new(shape: HistoryShape, children: &'a ReturnedChildren, tiers: &'a TierCtx) -> Self {
        Self {
            shape,
            children,
            tiers,
            shown: Vec::new(),
            results_shown: Vec::new(),
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
            let message = self.messages.len();
            for (call_id, keys) in std::mem::take(&mut self.results_shown) {
                self.shown.push(Shown {
                    message,
                    call_id,
                    keys,
                });
            }
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
            meta,
            turn,
            ..
        } = body
        {
            let (result, keys) = self.tiers.show(call_id, result, meta, *turn);
            if let Some(i) = self.open.iter().position(|c| c == call_id) {
                self.open.remove(i);
                if let Some(keys) = keys {
                    self.shown.push(Shown {
                        message: self.messages.len(),
                        call_id: call_id.clone(),
                        keys,
                    });
                }
                self.push(
                    Some(pos),
                    AiMessage::new(
                        AiRole::Tool,
                        vec![AiContent::tool_result_text(
                            call_id.clone(),
                            result,
                            status == "error",
                        )],
                    ),
                );
                if self.open.is_empty() {
                    self.close_calls();
                }
            } else {
                if let Some(keys) = keys {
                    self.results_shown.push((call_id.clone(), keys));
                }
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
                // A run's normal end, a message it stopped for and the rewrite
                // boundary are not part of what the model saw.
                if matches!(kind.as_str(), "done" | "wait" | "message" | "context_rewritten") {
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
            WorklogBody::ControlApplied { command, detail, .. } if command == "retract" => {
                if detail["outcome"] == "stopped" {
                    self.note(
                        pos,
                        "[control] the user retracted the input of this turn: do not act on it. What the turn already did is not undone.".to_string(),
                    )
                }
            }
            WorklogBody::ControlApplied { command, .. } => {
                self.note(pos, format!("[control] {command}"))
            }
            WorklogBody::Compaction { .. } | WorklogBody::ActionResult { .. } => {}
        }
    }

    fn finish(mut self) -> Rendered {
        self.flush_results();
        self.close_calls();
        self.flush_notes();
        self.close_calls();
        Rendered {
            messages: self.messages,
            positions: self.positions,
            shown: self.shown,
        }
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

struct Rendered {
    messages: Vec<AiMessage>,
    positions: Vec<Option<Pos>>,
    shown: Vec<Shown>,
}

fn render(
    entries: &[(u64, WorklogEntry)],
    shape: HistoryShape,
    children: &ReturnedChildren,
    tiers: &TierCtx,
) -> Rendered {
    let mut r = Render::new(shape, children, tiers);
    for (offset, e) in entries {
        r.entry((*offset, e.seq), &e.body);
    }
    r.finish()
}

/// The command that reads a Turn of `session` again (`None` on a node
/// runtime, which has no `agent-session`).
fn recover_command(session: &Session) -> Option<String> {
    (!session.config.runtime.is_node())
        .then(|| format!("agent-session read-session {}", session.sid()))
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
    let w = window_entries(session, summary, budget)?;
    let tiers = TierCtx::new(w.entries.iter().map(|(_, e)| &e.body), Tiering::Full, None);
    let r = render(&w.entries, shape, &w.children, &tiers);
    Ok(w.into_window(r))
}

/// The raw records a window reads (oldest first).
struct WindowEntries {
    entries: Vec<(u64, WorklogEntry)>,
    children: ReturnedChildren,
    reached_start: bool,
    bytes_read: u64,
}

impl WindowEntries {
    fn into_window(self, r: Rendered) -> HistoryWindow {
        HistoryWindow {
            tokens: estimate(&r.messages),
            messages: r.messages,
            positions: r.positions,
            reached_start: self.reached_start,
            oldest_offset: self.entries.first().map(|(o, _)| *o),
            oldest_seq: self.entries.first().map(|(_, e)| e.seq),
            bytes_read: self.bytes_read,
        }
    }
}

fn window_entries(
    session: &Session,
    summary: &SessionSummary,
    budget: Option<u64>,
) -> Result<WindowEntries> {
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
    Ok(WindowEntries {
        entries: rev,
        children,
        reached_start,
        bytes_read: r.bytes_read,
    })
}

/// Where the history of a Goal Session with a fresh context starts: the
/// committed worklog before the open Activation is not read (none open: none
/// of it). `None`: the whole history.
fn activation_floor(session: &Session) -> Option<u64> {
    if session.config.session.kind != SessionKind::Goal {
        return None;
    }
    let g = session.state.goal.as_ref()?;
    match &g.activation {
        Some(a) => (a.context == GoalContext::Fresh).then_some(a.worklog_offset),
        None => (g.context == GoalContext::Fresh).then_some(session.state.worklog.committed_bytes),
    }
}

/// The compression the history starts from. With a fresh context one made
/// before the open Activation is not part of it.
fn current_summary(session: &Session) -> Result<SessionSummary> {
    let mut sm = session.summary()?;
    if let Some(floor) = activation_floor(session) {
        if sm.start_offset <= floor {
            sm.history_summary.clear();
            sm.keep_exact.clear();
            sm.head_end_offset = 0;
            sm.head_end_seq = 0;
            sm.start_offset = floor;
        }
    }
    Ok(sm)
}

/// The whole history: head + summary block + raw records.
struct Assembled {
    messages: Vec<AiMessage>,
    positions: Vec<Option<Pos>>,
    shown: Vec<Shown>,
    tokens: u64,
    /// Read to the start point and within the budget.
    complete: bool,
}

fn assemble(
    session: &Session,
    sm: &SessionSummary,
    shape: HistoryShape,
    budget: Option<u64>,
    tiering: Tiering,
) -> Result<Assembled> {
    let floor = activation_floor(session);
    let mut messages = Vec::new();
    let mut positions = Vec::new();
    let mut shown = Vec::new();
    let mut head = Vec::new();
    let mut head_children = ReturnedChildren::default();
    if sm.head_end_offset > floor.unwrap_or(0) {
        let entries: Vec<(u64, WorklogEntry)> = session
            .dir
            .worklog()
            .read_range(floor.unwrap_or(0), sm.head_end_offset)?
            .into_iter()
            .filter(|(_, e)| !dropped(session, &e.body))
            .collect();
        for (_, e) in &entries {
            head_children.note(&e.body);
        }
        head = entries
            .into_iter()
            .filter(|(_, e)| !head_children.hides(&e.body))
            .collect();
    }
    let summary = (!sm.history_summary.trim().is_empty()).then(|| {
        llm_compress::summary_message(
            &CompressedSummary {
                summary: sm.history_summary.clone(),
                keep_exact: sm.keep_exact.clone(),
            },
            Some(&compressed_note(session, &sm.perceptions)),
        )
    });
    // Shown in full, the head is what it is whatever follows it (a repeat
    // only names an earlier result).
    let head_full = (tiering == Tiering::Full).then(|| {
        let tiers = TierCtx::new(head.iter().map(|(_, e)| &e.body), tiering, None);
        render(&head, shape, &head_children, &tiers)
    });
    let fixed = head_full.as_ref().map_or(0, |r| estimate(&r.messages)) + estimate(summary.as_slice());
    // The raw records: within what the budget leaves, unless they are shown
    // by position (then all of them, they are shortened afterwards).
    let window_budget = match tiering {
        Tiering::Full => budget.map(|b| b.saturating_sub(fixed)),
        Tiering::ByPosition => None,
    };
    let window = window_entries(session, sm, window_budget)?;
    let tiers = TierCtx::new(
        head.iter().chain(window.entries.iter()).map(|(_, e)| &e.body),
        tiering,
        recover_command(session),
    );
    let head = head_full.unwrap_or_else(|| render(&head, shape, &head_children, &tiers));
    messages.extend(head.messages);
    positions.extend(head.positions);
    shown.extend(head.shown);
    if let Some(summary) = summary {
        messages.push(summary);
        positions.push(None);
    }
    let base = messages.len();
    let r = render(&window.entries, shape, &window.children, &tiers);
    shown.extend(r.shown.iter().cloned().map(|mut s| {
        s.message += base;
        s
    }));
    let window = window.into_window(r);
    messages.extend(window.messages);
    positions.extend(window.positions);
    let tokens = estimate(&messages);
    Ok(Assembled {
        complete: window.reached_start && budget.map_or(true, |b| tokens <= b),
        messages,
        positions,
        shown,
        tokens,
    })
}

impl Assembled {
    fn built(self, report: Option<CompressReport>) -> Built {
        Built {
            shown: shown_map(self.shown.iter()),
            messages: self.messages,
            report,
        }
    }
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
) -> Result<Built> {
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
    let perceptions = recorded_perceptions(session)?;
    let opts = CompressOptions::new(target, summarizer.model.clone())
        .with_note(compressed_note(session, &perceptions));
    let out = match llm_compress::compress_history(&full.messages, &deps, &opts).await {
        Ok(out) => out,
        Err(error) => {
            event["error"] = json!(error.to_string());
            event["elapsed_ms"] = json!(started.elapsed().as_millis());
            log::warn!("opendan.context.compaction.failed {}", event);
            // The turn goes on with older tool results clipped; summary.json
            // is left as it was, so the next build summarizes again.
            let trimmed = llm_compress::trim_tool_results(&full.messages, &deps, target);
            if trimmed.report.reached_target {
                log::warn!(
                    "opendan.context.compaction.fallback {}",
                    json!({
                        "session_id": session.sid(),
                        "reason": made_by,
                        "target_tokens": target,
                        "history_tokens_before": trimmed.report.tokens_before,
                        "history_tokens_after": trimmed.report.tokens_after,
                        "trimmed_results": trimmed.report.trimmed_results,
                    })
                );
                // What was clipped is not shown in full any more.
                let shown = shown_map(full.shown.iter().filter(|r| {
                    full.messages.get(r.message) == trimmed.messages.get(r.message)
                }));
                return Ok(Built {
                    messages: trimmed.messages,
                    report: Some(trimmed.report),
                    shown,
                });
            }
            return Err(OpenDanError::Llm(format!("compress history: {error}")));
        }
    };
    event["strategy"] = json!(out.report.strategy);
    event["history_tokens_after"] = json!(out.report.tokens_after);
    event["reached_target"] = json!(out.report.reached_target);
    let Some(summary) = out.summary.clone() else {
        event["elapsed_ms"] = json!(started.elapsed().as_millis());
        log::warn!("opendan.context.compaction.skipped {}", event);
        return Ok(Built {
            messages: out.messages,
            report: Some(out.report),
            shown: shown_map(full.shown.iter()),
        });
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
        perceptions,
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
    // The summary replaced the results between the head and the raw tail.
    let shown = shown_map(
        full.shown
            .iter()
            .filter(|r| r.message < out.head_end || r.message >= out.tail_start),
    );
    Ok(Built {
        messages: out.messages,
        report: Some(out.report),
        shown,
    })
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
    Ok(build_history_for(session, lease, summarizer, shape, budget, false)
        .await?
        .messages)
}

/// [`build_history`] with what it shows in full. `cold`: the provider's
/// prompt cache is gone, older results are shown by position anyway.
pub async fn build_history_for(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&Summarizer>,
    shape: HistoryShape,
    budget: u32,
    cold: bool,
) -> Result<Built> {
    build(
        session,
        lease,
        summarizer,
        shape,
        HistoryLine::fixed(budget),
        "context_limit",
        None,
        cold,
    )
    .await
}

/// The history within `lines`: as the model saw it while it fits and the
/// cache is warm; else older results by position first (no summary call when
/// that fits); else compressed.
async fn build(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&Summarizer>,
    shape: HistoryShape,
    lines: HistoryLine,
    made_by: &str,
    line: Option<&LimitLine>,
    cold: bool,
) -> Result<Built> {
    let budget = lines.trigger;
    let sm = current_summary(session)?;
    let window = assemble(session, &sm, shape, Some(u64::from(budget)), Tiering::Full)?;
    let mut event = json!({
        "session_id": session.sid(),
        "phase": if line.is_some() { "context_limit" } else { "build_history" },
        "reason": if line.is_some() { "context_limit" } else { "history_budget" },
        "budget_tokens": budget,
        "compress_to_tokens": lines.target,
        "summary_tokens": tokens(&sm.history_summary),
        "history_tokens": window.tokens,
        "history_tokens_lower_bound": !window.complete,
        "cache_cold": cold,
    });
    if let Some(line) = line {
        event["window_tokens"] = json!(line.window_tokens);
        event["target_tokens"] = json!(line.target_tokens);
        event["prefix_tokens"] = json!(line.prefix_tokens);
    }
    if window.complete && !cold {
        event["skip_reason"] = json!("within_history_budget");
        if line.is_some() {
            log::info!("opendan.context.compaction.skipped {}", event);
        } else {
            log::debug!("opendan.context.compaction.skipped {}", event);
        }
        return Ok(window.built(None));
    }
    let degraded = assemble(session, &sm, shape, None, Tiering::ByPosition)?;
    event["degraded_tokens"] = json!(degraded.tokens);
    if degraded.tokens <= u64::from(budget) {
        event["skip_reason"] = json!("degraded_within_budget");
        log::info!("opendan.context.compaction.skipped {}", event);
        return Ok(degraded.built(None));
    }
    log::info!("opendan.context.compaction.triggered {}", event);
    let Some(s) = summarizer else {
        event["failure_reason"] = json!("missing_summarizer");
        log::warn!("opendan.context.compaction.unavailable {}", event);
        return Err(OpenDanError::Other(
            "history exceeds the budget and no summarizer is configured".into(),
        ));
    };
    let built = compact(session, lease, s, &sm, degraded, lines.target, made_by).await?;
    if let Some(report) = built.report.as_ref().filter(|r| !r.reached_target) {
        log::warn!(
            "opendan.context.compaction.budget_exceeded {}",
            json!({
                "session_id": session.sid(),
                "budget_tokens": lines.target,
                "history_tokens": report.tokens_after,
                "strategy": report.strategy,
            })
        );
    }
    Ok(built)
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
) -> Result<Built> {
    // A rewrite renews the prompt: older results are shown by position.
    build(
        session,
        lease,
        Some(summarizer),
        shape,
        HistoryLine::fixed(budget),
        "context_limit",
        Some(line),
        true,
    )
    .await
}

/// After a run: compress a history over `line.trigger` to `line.target`
/// (a fixed budget: over `compact_ratio` of it, to half of it).
pub async fn maybe_compact(
    session: &mut Session,
    lease: &Lease,
    summarizer: Option<&Summarizer>,
    line: HistoryLine,
) -> Result<bool> {
    let mut event = json!({
        "session_id": session.sid(),
        "phase": "after_run",
        "reason": "ratio",
        "threshold_tokens": line.trigger,
        "compress_to_tokens": line.target,
    });
    let Some(s) = summarizer else {
        event["failure_reason"] = json!("missing_summarizer");
        log::warn!("opendan.context.compaction.unavailable {}", event);
        return Ok(false);
    };
    let sm = current_summary(session)?;
    let window = assemble(
        session,
        &sm,
        HistoryShape::Native,
        Some(u64::from(line.trigger)),
        Tiering::Full,
    )?;
    event["history_tokens"] = json!(window.tokens);
    event["history_tokens_lower_bound"] = json!(!window.complete);
    if window.complete {
        event["skip_reason"] = json!("below_threshold");
        log::debug!("opendan.context.compaction.skipped {}", event);
        return Ok(false);
    }
    // Older results by position first: when that fits, no summary is made
    // (the next history is built the same way).
    let degraded = assemble(session, &sm, HistoryShape::Native, None, Tiering::ByPosition)?;
    event["degraded_tokens"] = json!(degraded.tokens);
    if degraded.tokens <= u64::from(line.trigger) {
        event["skip_reason"] = json!("degraded_below_threshold");
        log::info!("opendan.context.compaction.skipped {}", event);
        return Ok(false);
    }
    log::info!("opendan.context.compaction.triggered {}", event);
    let built = compact(session, lease, s, &sm, degraded, line.target, "ratio").await?;
    Ok(built.report.is_some_and(|r| r.strategy == "llm"))
}
