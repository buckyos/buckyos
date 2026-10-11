//! The runner side of a Goal Session (`session.goal`): one Activation is one
//! logical Turn.
//!
//! - **Goal State**: the session owns it (`state.goal.state`), changes it in
//!   its own commits and publishes it to `state/goals/<id>/state.json`
//!   afterwards; Activation records go to `activations.jsonl` the same way
//!   ([`flush`]). A crash between the commit and the publish is caught up by
//!   the next drive.
//! - **Gate**: the Goal's status decides whether a new Activation may open
//!   (paused: no) and when the session ends (archived / achieved / a newer
//!   epoch) — at the next commit point, never in the middle of a run.
//! - **Opening**: the first batch of an Activation carries the Goal header
//!   (definition, state, open Work, budget, reasons) read and frozen before
//!   the batch commit; the receipt carries the opened Activation, so a
//!   recovery applies it once.
//! - **Closing**: whatever ends the Turn (final report, a question, budget,
//!   error, stop) ends the Activation only; its result is checked against
//!   the Goal result format and written into the Goal State in the commit
//!   that closes the Turn.

use std::collections::BTreeSet;
use std::sync::Arc;

use llm_context::{escape_xml_attr, escape_xml_text};
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::{Session, SessionDir};

use super::shared::{commit_and_report, Shared};

/// What the Goal allows the session to do now.
#[derive(Debug, Clone)]
pub(super) enum Gate {
    Open(Box<GoalDefinition>),
    /// No new Activation; an open one runs to its end.
    Paused(Box<GoalDefinition>),
    /// The session ends (at the next commit point).
    End(GoalEnding),
}

pub(super) fn binding(cfg: &SessionConfig) -> Option<&GoalBinding> {
    if cfg.session.kind == SessionKind::Goal {
        cfg.session.goal.as_ref()
    } else {
        None
    }
}

pub(super) fn is_goal(cfg: &SessionConfig) -> bool {
    binding(cfg).is_some()
}

/// Make sure the session holds its Goal State (first drive of an epoch).
pub(super) async fn load(sh: &Arc<Shared>) -> Result<()> {
    let (b, has) = {
        let s = sh.session.lock().await;
        match binding(&s.config) {
            Some(b) => (b.clone(), s.state.goal.is_some()),
            None => return Ok(()),
        }
    };
    if has {
        return Ok(());
    }
    let goal = sh
        .agent()
        .goals()
        .get(&b.goal_id)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("goal {}", b.goal_id)))?;
    let state = match sh.agent().goals().state(&b.goal_id).await? {
        Some(st) if st.epoch == b.epoch => st,
        Some(st) if st.epoch > b.epoch => {
            // A later epoch owns the Goal State already: this one only
            // ends (the gate says so); it starts from what it left.
            sh.agent()
                .goals()
                .epoch_state(&b.goal_id, b.epoch)
                .await?
                .unwrap_or_else(|| GoalState::initial(&goal, b.epoch, crate::now_ms()))
        }
        Some(st) => {
            return Err(OpenDanError::GoalConflict(format!(
                "the goal state of {} is still at epoch {}; the host prepares epoch {} before its session runs",
                b.goal_id, st.epoch, b.epoch
            )))
        }
        None => GoalState::initial(&goal, b.epoch, crate::now_ms()),
    };
    let mut s = sh.session.lock().await;
    let published = state.rev;
    s.state.goal = Some(GoalSessionState {
        goal_id: b.goal_id.clone(),
        epoch: b.epoch,
        state,
        published_rev: published,
        unwritten: Vec::new(),
        activation: None,
        ending: None,
    });
    commit_and_report(sh, &mut s).await
}

/// The Goal's word on this session right now.
pub(super) async fn gate(sh: &Shared) -> Result<Option<Gate>> {
    let b = {
        let s = sh.session.lock().await;
        match binding(&s.config) {
            Some(b) => b.clone(),
            None => return Ok(None),
        }
    };
    let Some(goal) = sh.agent().goals().get(&b.goal_id).await? else {
        return Ok(Some(Gate::End(GoalEnding {
            end: EpochEnd::Archived,
            outcome: Outcome::Stopped,
            reason: format!("goal {} no longer exists", b.goal_id),
            cascade: true,
        })));
    };
    if goal.session.epoch != b.epoch {
        let reason = goal
            .epoch(goal.session.epoch)
            .and_then(|e| e.reason.clone())
            .unwrap_or_default();
        return Ok(Some(Gate::End(GoalEnding {
            end: EpochEnd::Restarted,
            outcome: Outcome::Stopped,
            reason: format!("superseded by epoch {}: {reason}", goal.session.epoch),
            cascade: false,
        })));
    }
    Ok(Some(match goal.status {
        GoalStatus::Active => Gate::Open(Box::new(goal)),
        GoalStatus::Paused => Gate::Paused(Box::new(goal)),
        GoalStatus::Archived => Gate::End(GoalEnding {
            end: EpochEnd::Archived,
            outcome: Outcome::Stopped,
            reason: goal.status_reason.clone().unwrap_or_else(|| "archived".into()),
            cascade: true,
        }),
        GoalStatus::Achieved => Gate::End(GoalEnding {
            end: EpochEnd::Achieved,
            outcome: Outcome::Succeeded,
            reason: goal.status_reason.clone().unwrap_or_else(|| "achieved".into()),
            cascade: false,
        }),
    }))
}

/// The Goal ends this session: recorded, then carried out by the stop path
/// (`stop_requested`) at the next commit point.
pub(super) async fn begin_ending(sh: &Shared, ending: GoalEnding) -> Result<()> {
    let mut s = sh.session.lock().await;
    if s.state.is_finished() || s.state.goal.as_ref().is_some_and(|g| g.ending.is_some()) {
        return Ok(());
    }
    let rev = s.state.rev;
    s.append_worklog(
        &sh.lease,
        vec![WorklogBody::ControlApplied {
            input: InputRef {
                src: "_runner".into(),
                index: 0,
                key: format!("goal_end@{rev}"),
                kind: INPUT_TYPE_CONTROL.into(),
            },
            command: "goal_end".into(),
            detail: serde_json::to_value(&ending).unwrap_or(Value::Null),
        }],
    )?;
    if let Some(g) = s.state.goal.as_mut() {
        g.ending = Some(ending);
    }
    s.state.stop_requested = true;
    commit_and_report(sh, &mut s).await
}

pub(super) fn ending(state: &SessionState) -> Option<&GoalEnding> {
    state.goal.as_ref().and_then(|g| g.ending.as_ref())
}

/// Tokens and run ids recorded in `usage.jsonl` after `offset`.
fn usage_since(dir: &SessionDir, offset: u64) -> (u64, BTreeSet<String>) {
    let path = dir.file(USAGE_FILE);
    let Ok(bytes) = std::fs::read(&path) else {
        return (0, BTreeSet::new());
    };
    let start = (offset as usize).min(bytes.len());
    let mut tokens = 0;
    let mut runs = BTreeSet::new();
    for line in bytes[start..].split(|b| *b == b'\n') {
        if let Ok(r) = serde_json::from_slice::<UsageRecord>(line) {
            tokens += r.total_tokens;
            if let Some(id) = r.run_id {
                runs.insert(id);
            }
        }
    }
    (tokens, runs)
}

fn usage_len(dir: &SessionDir) -> u64 {
    std::fs::metadata(dir.file(USAGE_FILE))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// Why the open Activation may not go on (`creating`: a new run would be
/// started).
pub(super) async fn activation_exhausted(sh: &Shared, creating: bool) -> Option<String> {
    let s = sh.session.lock().await;
    let open = s.state.goal.as_ref()?.activation.as_ref()?;
    let (tokens, runs) = usage_since(&sh.dir, open.usage_offset);
    if let Some(max) = open.budget.max_total_tokens {
        if tokens >= max {
            return Some(format!("{tokens} of {max} tokens of the activation used"));
        }
    }
    if let Some(max) = open.budget.max_runs {
        let live = s.state.live_run.as_ref().map(|l| l.run_id.clone());
        let n = runs.len() + usize::from(creating && live.is_none_or(|l| !runs.contains(&l)));
        if creating && n > max as usize {
            return Some(format!("{} of {max} runs of the activation used", runs.len()));
        }
    }
    None
}

/// Token cap of a run started now: what the open Activation has left, or
/// its whole budget for the run that opens one.
pub(super) async fn run_token_cap(sh: &Shared) -> Option<u64> {
    let (b, open) = {
        let s = sh.session.lock().await;
        let b = binding(&s.config)?.clone();
        (b, s.state.goal.as_ref().and_then(|g| g.activation.clone()))
    };
    match open {
        Some(a) => {
            let max = a.budget.max_total_tokens?;
            let (used, _) = usage_since(&sh.dir, a.usage_offset);
            Some(max.saturating_sub(used).max(1))
        }
        None => {
            let goal = sh.agent().goals().get(&b.goal_id).await.ok().flatten()?;
            let class = sh.session.lock().await.config.session.class.clone();
            let d = crate::goal::defaults(sh.agent_root.as_deref(), &class);
            crate::goal::effective_budget(&goal, &d)
                .activation
                .max_total_tokens
        }
    }
}

/// A Work of the Goal as the header shows it.
#[derive(Debug, Clone, Serialize)]
struct WorkView {
    session_id: String,
    objective: String,
    run_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<Outcome>,
    one_line_status: String,
    epoch: u64,
    activation: u64,
    goal_revision: u64,
    /// Created for an older revision of the Goal.
    outdated: bool,
}

/// What a Goal header is made of (template variable `goal`).
#[derive(Debug, Clone, Serialize)]
pub(super) struct GoalView {
    goal_id: String,
    epoch: u64,
    activation: u64,
    revision: u64,
    revision_seen: u64,
    /// Definition fields revised since the last Activation.
    changed: Vec<String>,
    status: GoalStatus,
    definition: Value,
    state: Value,
    works: Vec<WorkView>,
    budget: Value,
    reasons: Vec<ActivationReason>,
    text: String,
}

/// The Activation a batch opens, and its Goal header.
pub(super) struct Opening {
    pub(super) open: OpenActivation,
    pub(super) text: String,
    pub(super) brief: String,
}

async fn works_of(sh: &Shared, goal: &GoalDefinition, st: &GoalState) -> Result<Vec<WorkView>> {
    let mut out = Vec::new();
    for e in sh
        .agent()
        .sessions()
        .query(&RegistryQuery {
            goal_id: Some(goal.goal_id.clone()),
            ..Default::default()
        })
        .await?
    {
        if e.kind == SessionKind::Goal || st.is_absorbed(&e.session_id) {
            continue;
        }
        let o = e.origin.as_ref().and_then(|o| o.goal.clone());
        let rev = o.as_ref().map(|g| g.revision).unwrap_or(0);
        out.push(WorkView {
            session_id: e.session_id.clone(),
            objective: e.objective.chars().take(300).collect(),
            run_state: e.status.run_state.as_str().to_string(),
            outcome: e.status.outcome,
            one_line_status: e.status.one_line_status.chars().take(300).collect(),
            epoch: o.as_ref().map(|g| g.epoch).unwrap_or(0),
            activation: o.as_ref().map(|g| g.activation_seq).unwrap_or(0),
            goal_revision: rev,
            outdated: rev < goal.revision,
        });
    }
    Ok(out)
}

fn xml_list(out: &mut String, tag: &str, item: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("<{tag}>\n"));
    for i in items {
        out.push_str(&format!("<{item}>{}</{item}>\n", escape_xml_text(i)));
    }
    out.push_str(&format!("</{tag}>\n"));
}

fn render_builtin(
    goal: &GoalDefinition,
    st: &GoalState,
    open: &OpenActivation,
    changed: &[String],
    works: &[WorkView],
    budget: &GoalBudget,
    usage: &GoalUsage,
) -> String {
    let mut s = format!(
        "<goal_activation goal=\"{}\" epoch=\"{}\" activation=\"{}\" revision=\"{}\" status=\"{}\">\n",
        escape_xml_attr(&goal.goal_id),
        st.epoch,
        open.seq,
        goal.revision,
        goal.status.as_str()
    );
    // Why this Activation runs.
    s.push_str("<reasons>\n");
    for r in &open.reasons {
        let mut attrs = format!("kind=\"{}\"", crate::goal::event_name(r.kind));
        if let Some(n) = r.missed {
            attrs.push_str(&format!(" missed_slots=\"{n}\""));
        }
        if let Some(x) = &r.reference {
            attrs.push_str(&format!(" ref=\"{}\"", escape_xml_attr(x)));
        }
        s.push_str(&format!(
            "<reason {attrs}>{}</reason>\n",
            escape_xml_text(r.note.as_deref().unwrap_or_default())
        ));
    }
    s.push_str("</reasons>\n");
    // The definition.
    s.push_str(&format!(
        "<definition revision=\"{}\"{}>\n",
        goal.revision,
        if changed.is_empty() {
            String::new()
        } else {
            format!(
                " changed_since_last_activation=\"{}\"",
                escape_xml_attr(&changed.join(", "))
            )
        }
    ));
    s.push_str(&format!("<title>{}</title>\n", escape_xml_text(&goal.title)));
    s.push_str(&format!(
        "<objective>{}</objective>\n",
        escape_xml_text(&goal.objective)
    ));
    s.push_str(&format!(
        "<lifecycle>{}</lifecycle>\n",
        match goal.lifecycle {
            GoalLifecycle::Achieve => "achieve",
            GoalLifecycle::Maintain => "maintain",
        }
    ));
    if let Some(t) = &goal.achieved_when {
        s.push_str(&format!("<achieved_when>{}</achieved_when>\n", escape_xml_text(t)));
    }
    if let Some(t) = &goal.healthy_state {
        s.push_str(&format!("<healthy_state>{}</healthy_state>\n", escape_xml_text(t)));
    }
    if !goal.distance_evaluation.is_empty() {
        s.push_str(&format!(
            "<distance_evaluation>{}</distance_evaluation>\n",
            escape_xml_text(&goal.distance_evaluation)
        ));
    }
    xml_list(&mut s, "constraints", "constraint", &goal.constraints);
    let mut scope: Vec<String> = goal.scope.paths.clone();
    scope.extend(goal.scope.objects.iter().cloned());
    if let Some(w) = &goal.scope.workspace {
        scope.push(format!("workspace {w}"));
    }
    xml_list(&mut s, "scope", "item", &scope);
    s.push_str("</definition>\n");
    // Skills: a stable place, never lost with history.
    if !st.current_skills.is_empty() {
        s.push_str("<current_skills>\n");
        for k in &st.current_skills {
            s.push_str(&format!(
                "<skill name=\"{}\"{}{}>{}</skill>\n",
                escape_xml_attr(&k.name),
                k.reference
                    .as_ref()
                    .map(|r| format!(" ref=\"{}\"", escape_xml_attr(r)))
                    .unwrap_or_default(),
                k.version
                    .as_ref()
                    .map(|v| format!(" version=\"{}\"", escape_xml_attr(v)))
                    .unwrap_or_default(),
                escape_xml_text(k.note.as_deref().unwrap_or_default())
            ));
        }
        s.push_str("</current_skills>\n");
    }
    // The state the last Activations left.
    s.push_str("<state>\n");
    match &st.last_activation {
        Some(l) => s.push_str(&format!(
            "<last_activation epoch=\"{}\" activation=\"{}\" outcome=\"{}\" decision=\"{}\" at=\"{}\">{}</last_activation>\n",
            l.epoch,
            l.seq,
            l.outcome.as_str(),
            l.decision.as_str(),
            crate::runner::input_view::utc_rfc3339(l.at_ms),
            escape_xml_text(&l.summary)
        )),
        None => s.push_str("<last_activation>none: this is the first activation</last_activation>\n"),
    }
    if let Some(d) = &st.distance {
        s.push_str(&format!(
            "<distance observed_at=\"{}\">{}</distance>\n",
            crate::runner::input_view::utc_rfc3339(d.observed_at_ms),
            escape_xml_text(&d.text)
        ));
        xml_list(&mut s, "unknowns", "unknown", &d.unknowns);
    }
    xml_list(&mut s, "pending", "item", &st.pending);
    let questions: Vec<String> = st.questions.iter().map(|q| q.text.clone()).collect();
    xml_list(&mut s, "open_questions", "question", &questions);
    if let Some(e) = &st.last_error {
        s.push_str(&format!(
            "<last_error>{}</last_error>\n",
            escape_xml_text(e.get("message").and_then(Value::as_str).unwrap_or_default())
        ));
    }
    s.push_str("</state>\n");
    // Work of the Goal not absorbed yet, as the registry has it now.
    if !works.is_empty() {
        s.push_str("<works>\n");
        for w in works {
            s.push_str(&format!(
                "<work session=\"{}\" state=\"{}\"{} goal_revision=\"{}\"{}>{}{}</work>\n",
                escape_xml_attr(&w.session_id),
                w.run_state,
                w.outcome
                    .map(|o| format!(" outcome=\"{}\"", format!("{o:?}").to_lowercase()))
                    .unwrap_or_default(),
                w.goal_revision,
                if w.outdated {
                    " outdated=\"true\""
                } else {
                    ""
                },
                escape_xml_text(&w.objective),
                if w.one_line_status.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", escape_xml_text(&w.one_line_status))
                }
            ));
        }
        s.push_str("</works>\n");
    }
    if !st.recent_works.is_empty() {
        s.push_str("<recent_works>\n");
        for w in st.recent_works.iter().rev().take(8) {
            s.push_str(&format!(
                "<work session=\"{}\" outcome=\"{}\">{}</work>\n",
                escape_xml_attr(&w.sid),
                w.outcome
                    .map(|o| format!("{o:?}").to_lowercase())
                    .unwrap_or_else(|| "unknown".into()),
                escape_xml_text(&w.one_line_status)
            ));
        }
        s.push_str("</recent_works>\n");
    }
    // Budget of this Activation and of the period.
    let mut b = String::new();
    if let Some(n) = open.budget.max_total_tokens {
        b.push_str(&format!(" activation_max_tokens=\"{n}\""));
    }
    if let Some(n) = open.budget.max_runs {
        b.push_str(&format!(" activation_max_runs=\"{n}\""));
    }
    b.push_str(&format!(
        " period_secs=\"{}\" period_tokens_used=\"{}\" period_activations_used=\"{}\" period_works_created=\"{}\"",
        crate::goal::window_secs(budget),
        usage.tokens + usage.work_tokens,
        usage.activations,
        usage.works
    ));
    if let Some(n) = budget.period.max_total_tokens {
        b.push_str(&format!(" period_max_tokens=\"{n}\""));
    }
    if let Some(n) = budget.period.max_activations {
        b.push_str(&format!(" period_max_activations=\"{n}\""));
    }
    if let Some(n) = budget.period.max_works {
        b.push_str(&format!(" period_max_works=\"{n}\""));
    }
    s.push_str(&format!("<budget{b}/>\n"));
    s.push_str("<result_format>End this activation with report(is_end=true, result={decision: no_action | acted | created_work | needs_info, distance?: text, pending?: [..], questions?: [..], skills?: {add?: [..], remove?: [..]}}). Not acting is a valid result; not observed is not \"no change\".</result_format>\n");
    s.push_str("</goal_activation>");
    s
}

/// Build the Activation the next batch opens and its Goal header. Every
/// external read happens here, before the batch commit.
pub(super) async fn opening(
    sh: &Arc<Shared>,
    goal: &GoalDefinition,
    picked: &[FetchedInput],
    template: Option<&str>,
) -> Result<Opening> {
    let (st, seq, class) = {
        let s = sh.session.lock().await;
        let g = s
            .state
            .goal
            .as_ref()
            .ok_or_else(|| OpenDanError::Other("goal session without its goal state".into()))?;
        (g.state.clone(), s.state.turn_seq + 1, s.config.session.class.clone())
    };
    let d = crate::goal::defaults(sh.agent_root.as_deref(), &class);
    let budget = crate::goal::effective_budget(goal, &d);
    let usage = crate::goal::current_usage(Some(&st), crate::now_ms(), crate::goal::window_secs(&budget));
    let mut reasons: Vec<ActivationReason> = picked.iter().map(crate::goal::reason_of).collect();
    reasons.dedup_by(|a, b| a.key == b.key);
    let changed: Vec<String> = if st.goal_revision_seen > 0 && st.goal_revision_seen < goal.revision {
        let mut fields: Vec<String> = sh
            .agent()
            .goals()
            .changes(&goal.goal_id, 0)
            .await?
            .into_iter()
            .filter(|c| c.kind == "revised" && c.revision.is_some_and(|r| r > st.goal_revision_seen))
            .flat_map(|c| {
                c.detail
                    .get("fields")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        fields.sort();
        fields.dedup();
        fields
    } else {
        Vec::new()
    };
    let works = works_of(sh, goal, &st).await?;
    let open = OpenActivation {
        seq,
        goal_revision: goal.revision,
        reasons: reasons.clone(),
        started_at_ms: crate::now_ms(),
        usage_offset: usage_len(&sh.dir),
        budget: budget.activation.clone(),
        absorbed: Vec::new(),
    };
    let builtin = render_builtin(goal, &st, &open, &changed, &works, &budget, &usage);
    let text = match template {
        Some(tpl) => {
            let view = GoalView {
                goal_id: goal.goal_id.clone(),
                epoch: st.epoch,
                activation: seq,
                revision: goal.revision,
                revision_seen: st.goal_revision_seen,
                changed: changed.clone(),
                status: goal.status,
                definition: serde_json::to_value(goal).unwrap_or(Value::Null),
                state: serde_json::to_value(&st).unwrap_or(Value::Null),
                works,
                budget: json!({ "activation": open.budget, "period": budget.period, "used": usage }),
                reasons,
                text: builtin.clone(),
            };
            super::assembler::render_template(tpl, json!({ "goal": view })).await?
        }
        None => builtin,
    };
    let brief = format!(
        "goal activation {seq} of goal {} (revision {}): header not repeated in history",
        goal.goal_id, goal.revision
    );
    Ok(Opening { open, text, brief })
}

/// Receipt application (live and recovery): the Activation a batch opened,
/// and the end of a Work it consumed.
pub(super) fn apply_receipt(state: &mut SessionState, r: &InputReceipt) {
    let Some(g) = state.goal.as_mut() else {
        return;
    };
    if r.opens_turn {
        if let Some(v) = r.extra.get(RECEIPT_GOAL_ACTIVATION) {
            match serde_json::from_value::<OpenActivation>(v.clone()) {
                Ok(open) => g.activation = Some(open),
                Err(e) => log::warn!("goal activation of receipt {}: {e}", r.input_seq),
            }
        }
    } else if let (Some(v), Some(open)) = (r.extra.get(RECEIPT_GOAL_REASONS), g.activation.as_mut()) {
        let reasons: Vec<ActivationReason> = serde_json::from_value(v.clone()).unwrap_or_default();
        for reason in reasons {
            if !open.reasons.iter().any(|x| x.key == reason.key) {
                open.reasons.push(reason);
            }
        }
    }
    let mut changed = false;
    for i in &r.inputs {
        if i.src != INTERNAL_CHILD_SRC {
            continue;
        }
        let Some((sid, event)) = super::children::child_of_internal_key(&i.key) else {
            continue;
        };
        if event != "finished" || g.state.is_absorbed(sid) {
            continue;
        }
        g.state.absorbed.push(sid.to_string());
        let extra = g.state.absorbed.len().saturating_sub(GOAL_ABSORBED_KEEP);
        g.state.absorbed.drain(..extra);
        g.state.active_works.retain(|w| w.sid != sid);
        if let Some(open) = g.activation.as_mut() {
            open.absorbed.push(sid.to_string());
        }
        changed = true;
    }
    if changed {
        g.state.rev += 1;
        g.state.updated_at_ms = crate::now_ms();
    }
}

/// How the Turn of an Activation ended.
pub(super) struct TurnEnd<'a> {
    pub(super) status: TurnStatus,
    /// The Turn ended on a question nobody answers now.
    pub(super) needs_input: bool,
    pub(super) answer: Option<&'a str>,
    pub(super) report: Option<&'a ReportSubmission>,
    pub(super) error: Option<&'a Value>,
}

fn cut(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut s: String = t.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// Inside the commit that closes the Turn of an Activation: the record, the
/// Goal State it leaves. Nothing outside the session is written here.
pub(super) async fn close_activation(sh: &Shared, s: &mut Session, end: TurnEnd<'_>) -> Result<()> {
    let Some(binding) = binding(&s.config).cloned() else {
        return Ok(());
    };
    let Some(open) = s.state.goal.as_mut().and_then(|g| g.activation.take()) else {
        return Ok(());
    };
    let now = crate::now_ms();
    let class = s.config.session.class.clone();
    let goal = sh.agent().goals().get(&binding.goal_id).await?;
    let d = crate::goal::defaults(sh.agent_root.as_deref(), &class);
    let budget = goal
        .as_ref()
        .map(|g| crate::goal::effective_budget(g, &d))
        .unwrap_or_else(|| d.budget.clone());
    let window = crate::goal::window_secs(&budget);
    let (tokens, runs) = usage_since(&s.dir, open.usage_offset);
    // The result of a final report, as the format requires.
    let parsed = end
        .report
        .filter(|r| r.is_end)
        .and_then(|r| r.result.as_ref())
        .map(GoalResult::parse);
    let result = match &parsed {
        Some(Ok(r)) => Some(r.clone()),
        Some(Err(e)) => {
            log::warn!("session {}: activation {} result rejected: {e}", s.sid(), open.seq);
            None
        }
        None => None,
    };
    let outcome = match end.status {
        TurnStatus::Completed if end.needs_input => ActivationOutcome::NeedsInput,
        TurnStatus::Completed => ActivationOutcome::Completed,
        TurnStatus::Failed => ActivationOutcome::Failed,
        TurnStatus::BudgetExhausted => ActivationOutcome::BudgetExhausted,
        TurnStatus::Stopped => ActivationOutcome::Stopped,
    };
    let decision = match (&result, outcome) {
        (Some(r), _) => r.decision,
        (None, ActivationOutcome::NeedsInput) => ActivationDecision::NeedsInfo,
        (None, ActivationOutcome::Completed) => {
            log::warn!(
                "session {}: activation {} completed without a goal result; decision unknown",
                s.sid(),
                open.seq
            );
            ActivationDecision::Unknown
        }
        (None, _) => ActivationDecision::Unknown,
    };
    let summary = cut(
        end.report
            .map(|r| r.report.as_str())
            .or(end.answer)
            .unwrap_or_default(),
        2000,
    );
    // Work this Activation created / took in, from the registry.
    let entries = sh
        .agent()
        .sessions()
        .query(&RegistryQuery {
            goal_id: Some(binding.goal_id.clone()),
            ..Default::default()
        })
        .await?;
    let works_created: Vec<String> = entries
        .iter()
        .filter(|e| {
            e.origin
                .as_ref()
                .and_then(|o| o.goal.as_ref())
                .is_some_and(|g| g.epoch == binding.epoch && g.activation_seq == open.seq)
        })
        .map(|e| e.session_id.clone())
        .collect();
    let mut work_tokens = 0;
    let mut recent = Vec::new();
    for sid in &open.absorbed {
        let Some(e) = entries.iter().find(|e| &e.session_id == sid) else {
            continue;
        };
        if let Ok(st) = SessionDir::open(&e.location).and_then(|d| d.statistics()) {
            work_tokens += st.total_tokens;
        }
        recent.push(RecentWork {
            sid: sid.clone(),
            outcome: e.status.outcome,
            one_line_status: cut(&e.status.one_line_status, 300),
            report_ref: Some(format!("{}/{REPORT_FILE}", e.location)),
            absorbed_in: (binding.epoch, open.seq),
        });
    }
    let error = end.error.cloned();
    let record = GoalActivation {
        schema: GOAL_ACTIVATION_SCHEMA.to_string(),
        goal_id: binding.goal_id.clone(),
        epoch: binding.epoch,
        seq: open.seq,
        session_id: s.sid().to_string(),
        goal_revision: open.goal_revision,
        started_at_ms: open.started_at_ms,
        ended_at_ms: now,
        reasons: open.reasons.clone(),
        outcome,
        decision,
        summary: summary.clone(),
        report_ref: Some(format!("worklog:{}:turn:{}", s.sid(), open.seq)),
        works_created: works_created.clone(),
        works_absorbed: open.absorbed.clone(),
        cost: ActivationCost {
            runs: runs.len() as u32,
            total_tokens: tokens,
        },
        end_reason: error
            .as_ref()
            .and_then(|e| e.get("message").and_then(Value::as_str).map(str::to_string))
            .or_else(|| match &parsed {
                Some(Err(e)) => Some(format!("result rejected: {e}")),
                _ => None,
            }),
        result: end.report.and_then(|r| r.result.clone()),
    };
    let sid = s.sid().to_string();
    let g = s.state.goal.as_mut().expect("goal state");
    let st = &mut g.state;
    crate::goal::roll_usage(&mut st.usage, now, window);
    st.usage.tokens += tokens;
    st.usage.activations += 1;
    st.usage.work_tokens += work_tokens;
    st.usage.works = entries
        .iter()
        .filter(|e| {
            e.origin
                .as_ref()
                .and_then(|o| o.goal.as_ref())
                .is_some_and(|g| g.at_ms >= st.usage.period_start_ms)
        })
        .count() as u64;
    st.last_activation = Some(LastActivation {
        epoch: binding.epoch,
        seq: open.seq,
        at_ms: now,
        reasons: open.reasons.clone(),
        outcome,
        decision,
        summary: cut(&summary, 600),
    });
    st.goal_revision_seen = open.goal_revision;
    if let Some(r) = &result {
        if let Some(d) = &r.distance {
            st.distance = Some(match d {
                DistanceInput::Text(text) => GoalDistance {
                    text: text.clone(),
                    observed_at_ms: now,
                    evidence_refs: Vec::new(),
                    unknowns: Vec::new(),
                },
                DistanceInput::Full {
                    text,
                    evidence_refs,
                    unknowns,
                } => GoalDistance {
                    text: text.clone(),
                    observed_at_ms: now,
                    evidence_refs: evidence_refs.clone(),
                    unknowns: unknowns.clone(),
                },
            });
        }
        if let Some(p) = &r.pending {
            st.pending = p.clone();
        }
        if let Some(q) = &r.questions {
            st.questions = q
                .iter()
                .map(|t| GoalQuestion {
                    text: t.clone(),
                    at_ms: now,
                    epoch: binding.epoch,
                    activation: open.seq,
                })
                .collect();
        }
        if let Some(k) = &r.skills {
            apply_skills(&mut st.current_skills, &k.add, &k.remove);
        }
    }
    if outcome == ActivationOutcome::NeedsInput && result.as_ref().is_none_or(|r| r.questions.is_none()) {
        // The question is the Turn's last words; nobody is told (phase 1).
        st.questions.push(GoalQuestion {
            text: cut(end.answer.unwrap_or_default(), 1000),
            at_ms: now,
            epoch: binding.epoch,
            activation: open.seq,
        });
    }
    let extra = st.questions.len().saturating_sub(GOAL_QUESTIONS_KEEP);
    st.questions.drain(..extra);
    // Work not taken in yet, as the registry has it now.
    st.active_works = entries
        .iter()
        .filter(|e| e.kind != SessionKind::Goal && !st.absorbed.contains(&e.session_id))
        .filter_map(|e| {
            let o = e.origin.as_ref()?.goal.as_ref()?;
            Some(GoalWork {
                sid: e.session_id.clone(),
                objective: cut(&e.objective, 300),
                epoch: o.epoch,
                created_in_activation: o.activation_seq,
                goal_revision: o.revision,
            })
        })
        .collect();
    st.recent_works.extend(recent);
    let extra = st.recent_works.len().saturating_sub(GOAL_RECENT_WORKS_KEEP);
    st.recent_works.drain(..extra);
    st.last_error = error;
    st.rev += 1;
    st.updated_at_ms = now;
    g.unwritten.push(record);
    log::info!(
        "session {sid}: goal {} activation {} {} ({}), {tokens} tokens",
        binding.goal_id,
        open.seq,
        outcome.as_str(),
        decision.as_str()
    );
    Ok(())
}

fn apply_skills(skills: &mut Vec<GoalSkill>, add: &[GoalSkill], remove: &[String]) {
    skills.retain(|k| !remove.contains(&k.name));
    for a in add {
        match skills.iter_mut().find(|k| k.name == a.name) {
            Some(k) => *k = a.clone(),
            None => skills.push(a.clone()),
        }
    }
}

/// `control(goal_skills)`.
pub(super) fn apply_skills_control(
    state: &mut SessionState,
    add: &[GoalSkill],
    remove: &[String],
) -> std::result::Result<(), String> {
    let Some(g) = state.goal.as_mut() else {
        return Err("goal_skills only applies to goal sessions".into());
    };
    apply_skills(&mut g.state.current_skills, add, remove);
    g.state.rev += 1;
    g.state.updated_at_ms = crate::now_ms();
    Ok(())
}

/// Publish what the session committed: the Goal State, then the Activation
/// records, then the marks (best effort; the next drive catches up).
pub(super) async fn flush(sh: &Shared) {
    let (gs, pending) = {
        let s = sh.session.lock().await;
        match s.state.goal.as_ref() {
            Some(g) if g.needs_flush() => (g.clone(), true),
            _ => return,
        }
    };
    if !pending {
        return;
    }
    let mut published = gs.published_rev;
    if gs.published_rev < gs.state.rev {
        match sh.agent().goals().write_state(&sh.lease, &gs.state).await {
            Ok(()) => published = gs.state.rev,
            Err(OpenDanError::GoalConflict(e)) => {
                // A later epoch owns the Goal State: this one's is history.
                log::warn!("session {}: goal state not published: {e}", sh.dir.sid());
                published = gs.state.rev;
            }
            Err(e) => {
                log::warn!("session {}: goal state not published, will retry: {e}", sh.dir.sid());
                return;
            }
        }
    }
    crate::fault::point("goal:after_state_publish");
    let written = gs.unwritten.len();
    if written > 0 {
        if let Err(e) = sh
            .agent()
            .goals()
            .append_activations(&sh.lease, &gs.unwritten)
            .await
        {
            log::warn!("session {}: activation records not written, will retry: {e}", sh.dir.sid());
            return;
        }
        crate::fault::point("goal:after_activation_append");
    }
    let mut s = sh.session.lock().await;
    if let Some(g) = s.state.goal.as_mut() {
        g.published_rev = g.published_rev.max(published);
        let done: Vec<(u64, u64)> = gs.unwritten.iter().map(|a| (a.epoch, a.seq)).collect();
        g.unwritten.retain(|a| !done.contains(&(a.epoch, a.seq)));
    }
    if let Err(e) = s.commit_state(&sh.lease) {
        log::warn!("session {}: goal flush marks not committed: {e}", sh.dir.sid());
    }
}
