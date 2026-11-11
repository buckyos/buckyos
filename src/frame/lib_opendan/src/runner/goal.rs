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

use agent_tool::{escape_xml_attr, escape_xml_text};
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::notify::NotifyStatus;
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

/// The clock of Goal decisions (the host's Goal clock).
fn now(sh: &Shared) -> u64 {
    sh.deps.options.goal_clock.now_ms()
}

/// How often a budget hold reads the budget again.
const BUDGET_HOLD_RECHECK_MS: u64 = 60_000;

/// Whether the inputs picked for a new Activation must wait (TODO H2, steps
/// 2–4: the hard interval, the hard guarantee, the budgets; a dormant Goal
/// holds its inputs before this). Explicit acts and run opportunities the
/// host already let through (`activate`, scheduled checks, the hard
/// guarantee, an epoch's start) open at once; a Work's end, a message and
/// an answer wait in the queue and open one Activation together when the
/// gate lets them through. A hold is recorded once in the Goal State.
pub(super) async fn hold_events(sh: &Arc<Shared>, goal: &GoalDefinition, picked: &[FetchedInput]) -> Result<bool> {
    let bypass = picked.iter().any(|p| crate::goal::reason_of(p).kind.bypasses_gate());
    let t = now(sh);
    let (st, class) = {
        let s = sh.session.lock().await;
        let Some(g) = s.state.goal.as_ref() else {
            return Ok(false);
        };
        (g.state.clone(), s.config.session.class.clone())
    };
    if bypass {
        release_hold(sh).await?;
        return Ok(false);
    }
    // A budget hold read less than a minute ago stands without a new scan.
    if let Some(h) = &st.holding {
        if h.reason != HoldReason::MinInterval
            && t.saturating_sub(h.checked_at_ms) < BUDGET_HOLD_RECHECK_MS
            && h.until_ms.is_none_or(|u| u > t)
        {
            return Ok(true);
        }
    }
    let d = crate::goal::defaults(sh.agent_root.as_deref(), &class);
    let verdict = match crate::goal::last_inference_ms(Some(&st)) {
        Some(last) if t.saturating_sub(last) < crate::goal::min_interval_secs(goal, &d) * 1000 => {
            crate::goal::GateVerdict::Held {
                reason: HoldReason::MinInterval,
                until_ms: Some(last + crate::goal::min_interval_secs(goal, &d) * 1000),
                why: String::new(),
            }
        }
        _ => crate::goal::gate_now(sh.agent(), goal, Some(&st), &d, t).await?.0,
    };
    match verdict {
        crate::goal::GateVerdict::Pass { .. } => {
            release_hold(sh).await?;
            Ok(false)
        }
        crate::goal::GateVerdict::Dormant => Ok(true),
        crate::goal::GateVerdict::Held { reason, until_ms, .. } => {
            let window = crate::goal::window_secs(&crate::goal::effective_budget(goal, &d));
            note_hold(sh, reason, until_ms, window, t).await?;
            Ok(true)
        }
    }
}

async fn note_hold(sh: &Shared, reason: HoldReason, until_ms: Option<u64>, window_secs: u64, t: u64) -> Result<()> {
    let mut s = sh.session.lock().await;
    let Some(g) = s.state.goal.as_mut() else {
        return Ok(());
    };
    let st = &mut g.state;
    match st.holding.as_mut() {
        Some(h) if h.reason == reason => {
            if reason == HoldReason::MinInterval {
                return Ok(());
            }
            h.until_ms = until_ms;
            h.checked_at_ms = t;
        }
        _ => {
            let start = crate::goal::period_start(t, window_secs);
            if st.holds.period_start_ms != start {
                st.holds = GoalHolds {
                    period_start_ms: start,
                    ..Default::default()
                };
            }
            match reason {
                HoldReason::MinInterval => st.holds.min_interval += 1,
                HoldReason::PeriodBudget | HoldReason::AgentBudget => st.holds.budget += 1,
            }
            st.holding = Some(GoalHold {
                reason,
                since_ms: t,
                until_ms,
                checked_at_ms: t,
            });
            log::info!(
                "session {}: goal {} holds its events ({})",
                sh.dir.sid(),
                g.goal_id,
                reason.as_str()
            );
        }
    }
    let st = &mut g.state;
    st.rev += 1;
    st.updated_at_ms = t;
    commit_and_report(sh, &mut s).await
}

async fn release_hold(sh: &Shared) -> Result<()> {
    let mut s = sh.session.lock().await;
    let Some(g) = s.state.goal.as_mut() else {
        return Ok(());
    };
    if g.state.holding.take().is_none() {
        return Ok(());
    }
    g.state.rev += 1;
    g.state.updated_at_ms = now(sh);
    commit_and_report(sh, &mut s).await
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
    if let Some(max) = open.budget.max_wall_secs {
        let ran = now(sh).saturating_sub(open.started_at_ms) / 1000;
        if ran >= max {
            return Some(format!(
                "the activation ran {ran}s of its {max}s: whatever it started that still runs is left to run, its end is not followed (start the next step in a work session)"
            ));
        }
    }
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
        // Work that ended comes as its event and then in `<recent_works>`:
        // `<works>` lists what still runs (TODO B5).
        if e.kind == SessionKind::Goal || st.is_absorbed(&e.session_id) || e.status.run_state == RunState::Finished {
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

/// What the gate and the check did lately (the `<gate>` of a Goal header).
#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct GateView {
    hard_interval_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    hard_guarantee_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timetable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    check_script: Option<GoalCheckScript>,
    period_resets_at_ms: u64,
    agent: crate::goal::AgentUsage,
    stats: GateStats,
    holds: GoalHolds,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_check: Option<CheckRecord>,
    check_errors: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_check_error: Option<String>,
    no_action_in_a_row: u32,
}

fn render_gate(v: &GateView) -> String {
    let mut s = format!("<gate hard_interval_secs=\"{}\"", v.hard_interval_secs);
    match v.hard_guarantee_secs {
        Some(g) => s.push_str(&format!(" hard_guarantee_secs=\"{g}\"")),
        None => s.push_str(" hard_guarantee=\"off\""),
    }
    s.push_str(&format!(
        " check_timetable=\"{}\"",
        escape_xml_attr(v.timetable.as_deref().unwrap_or("none"))
    ));
    match &v.check_script {
        Some(c) => s.push_str(&format!(
            " check_script=\"version {} by {}\"",
            c.version,
            escape_xml_attr(&c.source)
        )),
        None => s.push_str(" check_script=\"none\""),
    }
    s.push_str(&format!(" no_action_in_a_row=\"{}\">\n", v.no_action_in_a_row));
    s.push_str(&format!(
        "<this_period checks_true=\"{}\" checks_false=\"{}\" checks_error=\"{}\" notifications_by_checks=\"{}\" skipped_by_interval=\"{}\" skipped_by_budget=\"{}\" events_held_by_interval=\"{}\" events_held_by_budget=\"{}\" hard_guarantees=\"{}\"/>\n",
        v.stats.checks_true,
        v.stats.checks_false,
        v.stats.checks_error,
        v.stats.notified,
        v.stats.skipped_min_interval,
        v.stats.skipped_budget,
        v.holds.min_interval,
        v.holds.budget,
        v.stats.guarantees
    ));
    if let Some(c) = &v.last_check {
        let mut attrs = format!(
            "at=\"{}\" outcome=\"{}\"",
            crate::runner::input_view::utc_rfc3339(c.at_ms),
            c.outcome.as_str()
        );
        if let Some(h) = &c.held {
            attrs.push_str(&format!(" held=\"{}\"", escape_xml_attr(h)));
        }
        if let Some(n) = &c.notified {
            attrs.push_str(&format!(" notified=\"{}\"", escape_xml_attr(n)));
        }
        s.push_str(&format!(
            "<last_check {attrs}>{}</last_check>\n",
            escape_xml_text(c.reason.as_deref().unwrap_or_default())
        ));
    }
    if v.check_errors > 0 {
        s.push_str(&format!(
            "<check_errors in_a_row=\"{}\">{}</check_errors>\n",
            v.check_errors,
            escape_xml_text(v.last_check_error.as_deref().unwrap_or_default())
        ));
    }
    s.push_str("</gate>\n");
    s
}

#[allow(clippy::too_many_arguments)]
fn render_builtin(
    goal: &GoalDefinition,
    st: &GoalState,
    open: &OpenActivation,
    changed: &[String],
    works: &[WorkView],
    budget: &GoalBudget,
    usage: &GoalUsage,
    gate: &GateView,
    agent_root: Option<&std::path::Path>,
    extra: &str,
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
    // Skills: a stable place, never lost with history. A goal skill's text
    // is read when needed (`path`), never copied here (TODO F1, N2).
    if !st.current_skills.is_empty() {
        s.push_str("<current_skills read_with=\"cat <path>\">\n");
        for k in &st.current_skills {
            s.push_str(&format!(
                "<skill name=\"{}\"{}{}{}>{}</skill>\n",
                escape_xml_attr(&k.name),
                k.reference
                    .as_ref()
                    .map(|r| format!(" ref=\"{}\"", escape_xml_attr(r)))
                    .unwrap_or_default(),
                crate::goal::skill_path(agent_root, k)
                    .map(|p| format!(" path=\"{}\"", escape_xml_attr(&p.display().to_string())))
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
    if !st.questions.is_empty() {
        s.push_str("<open_questions>\n");
        for q in &st.questions {
            s.push_str("<question");
            if let Some(status) = q.status {
                s.push_str(&format!(" status=\"{}\"", status.as_str()));
            }
            s.push('>');
            s.push_str(&escape_xml_text(&q.text));
            if let Some(a) = &q.answer {
                s.push_str("<answer>");
                s.push_str(&escape_xml_text(a));
                s.push_str("</answer>");
            }
            s.push_str("</question>\n");
        }
        s.push_str("</open_questions>\n");
    }
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
    // Budget of this Activation and of the period: everything the user gives
    // this goal to work in the background, its Work included.
    let mut b = String::new();
    if let Some(n) = open.budget.max_total_tokens {
        b.push_str(&format!(" activation_max_tokens=\"{n}\""));
    }
    if let Some(n) = open.budget.max_runs {
        b.push_str(&format!(" activation_max_runs=\"{n}\""));
    }
    if let Some(n) = open.budget.max_wall_secs {
        b.push_str(&format!(" activation_max_wall_secs=\"{n}\""));
    }
    b.push_str(&format!(
        " period_secs=\"{}\" period_resets_at=\"{}\" period_tokens_used=\"{}\" tokens_used_by_goal=\"{}\" tokens_used_by_works=\"{}\" period_activations_used=\"{}\" period_works_created=\"{}\"",
        crate::goal::window_secs(budget),
        crate::runner::input_view::utc_rfc3339(gate.period_resets_at_ms),
        usage.total_tokens(),
        usage.tokens,
        usage.work_tokens,
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
    if let Some(n) = gate.agent.max {
        b.push_str(&format!(
            " agent_background_tokens_used=\"{}\" agent_background_max_tokens=\"{n}\"",
            gate.agent.used
        ));
    }
    s.push_str(&format!("<budget{b}/>\n"));
    s.push_str(&render_gate(gate));
    s.push_str(extra);
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
    let t = now(sh);
    let window = crate::goal::window_secs(&budget);
    let usage = crate::goal::period_usage(sh.agent(), &goal.goal_id, Some(&st), t, window).await?;
    let schedule = sh.agent().goals().schedule(&goal.goal_id).await?.unwrap_or_else(|| GoalSchedule::new(&goal.goal_id, "local"));
    let mut stats = schedule.stats.clone();
    if stats.period_start_ms != usage.period_start_ms {
        stats = GateStats::default();
    }
    let mut holds = st.holds.clone();
    if holds.period_start_ms != usage.period_start_ms {
        holds = GoalHolds::default();
    }
    let gate = GateView {
        hard_interval_secs: crate::goal::min_interval_secs(goal, &d),
        hard_guarantee_secs: crate::goal::guarantee_secs(goal, &d),
        timetable: crate::goal_time::Timetable::of(goal, &d).map(|t| t.describe()),
        check_script: goal.schedule.check.clone(),
        period_resets_at_ms: usage.period_start_ms + window * 1000,
        agent: crate::goal::agent_usage(sh.agent(), &d, t).await?,
        stats,
        holds,
        last_check: schedule.last_check.clone(),
        check_errors: schedule.check_errors,
        last_check_error: schedule.last_check_error.clone(),
        no_action_in_a_row: st.consecutive_no_action,
    };
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
    let mut activation_budget = budget.activation.clone();
    let caps = super::consolidation::caps(sh);
    if caps.consolidation {
        // Every model entry of the consolidation Goal (scheduled, created,
        // manual, answer) stays inside what the period has left.
        if let (Some(max), Some(period)) = (activation_budget.max_total_tokens, budget.period.max_total_tokens) {
            let used = usage.total_tokens();
            activation_budget.max_total_tokens = Some(max.min(period.saturating_sub(used)).max(1));
        }
    }
    let extra = if caps.consolidation {
        super::consolidation::header_block(sh).await
    } else {
        String::new()
    };
    let open = OpenActivation {
        seq,
        goal_revision: goal.revision,
        reasons: reasons.clone(),
        started_at_ms: t,
        usage_offset: usage_len(&sh.dir),
        budget: activation_budget,
        absorbed: Vec::new(),
    };
    let builtin = render_builtin(goal, &st, &open, &changed, &works, &budget, &usage, &gate, sh.agent_root.as_deref(), &extra);
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
                budget: json!({ "activation": open.budget, "period": budget.period, "used": usage, "gate": gate }),
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
    // What the Activation started and still runs is left to run; its end is
    // not followed (TODO H0).
    s.state.watched_tasks.clear();
    let now = now(sh);
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
    let mut recent = Vec::new();
    for sid in &open.absorbed {
        let Some(e) = entries.iter().find(|e| &e.session_id == sid) else {
            continue;
        };
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
    let period_start = crate::goal::period_start(now, window);
    let (own_tokens, work_tokens) = crate::goal::goal_tokens_since(sh.agent(), &binding.goal_id, period_start)
        .await
        .unwrap_or((0, 0));
    let g = s.state.goal.as_mut().expect("goal state");
    let st = &mut g.state;
    let mut dropped: Vec<GoalQuestion> = Vec::new();
    crate::goal::roll_usage(&mut st.usage, now, window);
    st.usage.tokens = own_tokens;
    st.usage.activations += 1;
    st.usage.work_tokens = work_tokens;
    st.consecutive_no_action = if decision == ActivationDecision::NoAction {
        st.consecutive_no_action.saturating_add(1)
    } else {
        0
    };
    st.holding = None;
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
            // The same question again is the same notification.
            let before = std::mem::take(&mut st.questions);
            st.questions = q
                .iter()
                .map(|t| match before.iter().find(|b| &b.text == t) {
                    Some(b) => b.clone(),
                    None => GoalQuestion {
                        text: t.clone(),
                        at_ms: now,
                        epoch: binding.epoch,
                        activation: open.seq,
                        notify_id: None,
                        status: None,
                        answer: None,
                        candidate_id: None,
                    },
                })
                .collect();
            dropped = before
                .into_iter()
                .filter(|b| !st.questions.iter().any(|q| q.text == b.text))
                .collect();
        }
        if let Some(k) = &r.skills {
            apply_skills(&mut st.current_skills, &k.add, &k.remove);
        }
    }
    if outcome == ActivationOutcome::NeedsInput && result.as_ref().is_none_or(|r| r.questions.is_none()) {
        // The question is the Turn's last words; nobody is told (phase 1).
        let text = cut(end.answer.unwrap_or_default(), 1000);
        if !st.questions.iter().any(|q| q.text == text) {
            st.questions.push(GoalQuestion {
                text,
                at_ms: now,
                epoch: binding.epoch,
                activation: open.seq,
                notify_id: None,
                status: None,
                answer: None,
                candidate_id: None,
            });
        }
    }
    let extra = st.questions.len().saturating_sub(GOAL_QUESTIONS_KEEP);
    dropped.extend(st.questions.drain(..extra));
    // The questions reach the user (and leave) before this commit; their
    // answers taken in by this Activation are recorded.
    if let Some(notify) = sh.deps.notify.clone() {
        let title = goal.as_ref().map(|g| if g.title.trim().is_empty() { g.objective.clone() } else { g.title.clone() });
        questions_to_user(
            sh,
            notify.as_ref(),
            &binding.goal_id,
            &sid,
            title.as_deref(),
            st,
            &open.reasons,
            &dropped,
        )
        .await;
    }
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

/// The Goal Session failed for good (its workspace binding is invalid): an
/// open Activation is closed as failed; without one the Goal State alone
/// says why. The Goal stays as it is until it is restarted.
pub(super) async fn fail_session(sh: &Shared, s: &mut Session, error: &Value) -> Result<()> {
    let Some(g) = s.state.goal.as_mut() else {
        return Ok(());
    };
    let mut error = error.clone();
    error["recovery"] = json!("resolve the workspace, then restart the goal for a new goal session");
    if g.activation.is_some() {
        let end = TurnEnd {
            status: TurnStatus::Failed,
            needs_input: false,
            answer: None,
            report: None,
            error: Some(&error),
        };
        return close_activation(sh, s, end).await;
    }
    g.state.last_error = Some(error);
    g.state.rev += 1;
    g.state.updated_at_ms = crate::now_ms();
    Ok(())
}

fn question_key(goal_id: &str, text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(text.as_bytes()));
    format!("goal:{goal_id}:q:{}", &digest[..16])
}

fn question_status(status: NotifyStatus) -> GoalQuestionStatus {
    match status {
        NotifyStatus::Open | NotifyStatus::Read => GoalQuestionStatus::Open,
        NotifyStatus::Answered => GoalQuestionStatus::Answered,
        NotifyStatus::Dismissed => GoalQuestionStatus::Dismissed,
        NotifyStatus::Expired => GoalQuestionStatus::Expired,
        NotifyStatus::Canceled => GoalQuestionStatus::Canceled,
    }
}

/// Inside the commit that closes an Activation: questions the Goal no
/// longer asks are canceled, new ones are sent (one notification per
/// question text, whatever how often it is asked), answers this Activation
/// took in are recorded. A failure leaves the question unsent: the next
/// Activation that keeps it tries again.
#[allow(clippy::too_many_arguments)]
async fn questions_to_user(
    sh: &Shared,
    notify: &dyn crate::notify::NotifyHost,
    goal_id: &str,
    sid: &str,
    title: Option<&str>,
    st: &mut GoalState,
    reasons: &[ActivationReason],
    dropped: &[GoalQuestion],
) {
    for q in dropped {
        if let (Some(id), None | Some(GoalQuestionStatus::Open)) = (&q.notify_id, q.status) {
            if let Err(e) = notify.cancel(id, "goal_dropped").await {
                log::warn!("goal {goal_id}: notification {id} not canceled: {e}");
            }
        }
    }
    for r in reasons.iter().filter(|r| r.kind == ActivationReasonKind::Answer) {
        let Some(id) = r.reference.as_deref() else {
            continue;
        };
        let Some(q) = st.questions.iter_mut().find(|q| q.notify_id.as_deref() == Some(id)) else {
            continue;
        };
        match notify.state(id).await {
            Ok(v) => {
                q.status = Some(question_status(v.status));
                q.answer = v.result.as_ref().and_then(|r| {
                    r.text().map(str::to_string).or_else(|| {
                        let o = r.option()?;
                        Some(
                            crate::notify::prompt_options(&v.prompt)
                                .into_iter()
                                .find(|x| x.value == o)
                                .map(|x| x.label)
                                .unwrap_or_else(|| o.to_string()),
                        )
                    })
                });
            }
            Err(e) => log::warn!("goal {goal_id}: answer {id} not readable: {e}"),
        }
    }
    for q in st.questions.iter_mut().filter(|q| q.notify_id.is_none()) {
        let first = q.text.trim().lines().next().unwrap_or_default().trim().to_string();
        let head = cut(&first, 120);
        let message = if q.text.trim() == head { String::new() } else { q.text.trim().to_string() };
        let mut source = crate::notify::NotifySource {
            session_id: Some(sid.to_string()),
            goal_id: Some(goal_id.to_string()),
            operation: title.map(|t| cut(t, 200)),
            ..Default::default()
        };
        crate::notify::complete_notify_source(sh.agent(), &mut source).await;
        let req = crate::notify::NotifySendReq {
            to_user: String::new(),
            kind: crate::notify::NotifyKind::Input,
            level: Some(crate::notify::NotifyLevel::Normal),
            category: None,
            prompt: crate::notify::input_prompt(head, message, Vec::new(), true),
            source: Some(source),
            expires_at_ms: None,
            idempotency_key: question_key(goal_id, &q.text),
        };
        match notify.send(req).await {
            Ok(v) => {
                q.notify_id = Some(v.notify_id);
                q.status = Some(question_status(v.status));
            }
            Err(e) => log::warn!("goal {goal_id}: question not sent yet: {e}"),
        }
    }
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

/// A stop of the Goal Session itself: its Goal goes dormant (the session
/// and its state stay, no Activation opens until the Goal is woken).
pub(super) async fn sleep(sh: &Shared, s: &Session, from: &str, reason: Option<&str>) -> Result<()> {
    let b = binding(&s.config)
        .ok_or_else(|| OpenDanError::InvalidArgument("not a goal session".into()))?;
    let req = crate::state::goals::GoalStatusChange {
        status: GoalStatus::Paused,
        reason: reason.map(str::to_string),
        evidence: None,
    };
    sh.agent()
        .goals()
        .set_status(&b.goal_id, &req, &crate::state::goals::GoalCaller::owner(from))
        .await
        .map(|_| ())
}

/// A Goal that is dormant (`ended`: or ended) infers no more: the
/// Activation in progress is stopped at this boundary. `true`: it is being
/// stopped.
pub(super) async fn stop_if_dormant(sh: &Shared, ended: bool) -> Result<bool> {
    let why = match gate(sh).await? {
        Some(Gate::Paused(_)) => "the goal is dormant",
        Some(Gate::End(_)) if ended => "the goal ended",
        _ => return Ok(false),
    };
    let mut s = sh.session.lock().await;
    if s.state.open_turn.is_none() || s.state.turn_stop_requested {
        return Ok(s.state.turn_stop_requested);
    }
    s.state.turn_stop_requested = true;
    super::inputs::clear_interruption(&mut s);
    let rev = s.state.rev;
    s.append_worklog(
        &sh.lease,
        vec![WorklogBody::ControlApplied {
            input: InputRef {
                src: "_runner".into(),
                index: 0,
                key: format!("stop@{rev}"),
                kind: INPUT_TYPE_CONTROL.into(),
            },
            command: "stop".into(),
            detail: json!({ "reason": why, "from": sh.deps.who, "scope": "turn", "applied": true }),
        }],
    )?;
    commit_and_report(sh, &mut s).await?;
    Ok(true)
}
