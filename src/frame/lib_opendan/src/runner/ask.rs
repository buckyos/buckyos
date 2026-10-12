//! Asking the user from inside a run (`agent-session ask-user`).
//!
//! The command answers `Pending { task_id: "userinput:<call_id>" }` and the
//! run suspends on that call like on any task. Only the session resolves it:
//! the question leaves along the open Turn's reply path (a `thread` message
//! on the Turn's placeholder), the session waits for input
//! (`waiting_for.kind = input`) with the Turn still open, and the next
//! message of its queue becomes the call's tool result — consumed with an
//! input receipt that carries no message of its own. Events, sub session
//! states and observations stay with whoever waits for them; a sub context
//! keeps never reading its caller's batches.

use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;
use ndn_lib::{MsgRelType, RefTarget};
use serde_json::Value;

use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::session::Session;

use super::shared::Shared;

/// Whether a session may ask its user: it has an input queue, waiting for
/// the user is allowed, and it is not a sub session (whose "user" is the
/// session above it).
pub(super) fn allowed(cfg: &SessionConfig) -> bool {
    cfg.channels.kmsg().is_some()
        && cfg.session.policy.wait_user_msg == WaitPolicy::Allowed
        && cfg
            .session
            .origin
            .as_ref()
            .map_or(true, |o| o.parent_session.is_none())
}

/// Whether a session nobody reads asks through a notification instead: a
/// Work without an input queue, when the host has a notification service
/// and the template lets it wait (`ask_user_timeout_secs > 0`). Which way a
/// question goes is the session's, never the LLM's choice.
pub(super) fn via_notify(cfg: &SessionConfig, has_notify: bool) -> bool {
    has_notify
        && cfg.session.kind == SessionKind::Work
        && cfg.channels.kmsg().is_none()
        && cfg.session.policy.ask_user_timeout_secs > 0
}

pub(super) fn is_user_input(task_id: &str) -> bool {
    task_id.starts_with(USER_INPUT_TASK_PREFIX)
}

/// Calls of `snapshot` suspended on the user's answer.
pub(super) fn user_calls(snapshot: &LLMContextSnapshot) -> Vec<String> {
    snapshot
        .state
        .pending_calls()
        .iter()
        .filter(|p| is_user_input(&p.task_id))
        .map(|p| p.call.call_id.clone())
        .collect()
}

pub(super) fn waits_for_user(snapshot: &LLMContextSnapshot) -> bool {
    !user_calls(snapshot).is_empty()
}

/// What a run suspended on tool calls waits for (status display and event
/// matching): the user when one of its calls asks. A question asked
/// through a notification adds its deadline and the task the answer is
/// committed to.
pub(super) fn waiting_for(snapshot: &LLMContextSnapshot, run: &RunHandle) -> WaitingFor {
    let pending = snapshot.state.pending_calls();
    let asked: Vec<AskNotify> = user_calls(snapshot)
        .iter()
        .filter_map(|c| run.ask_notify(c))
        .collect();
    WaitingFor {
        kind: if waits_for_user(snapshot) {
            WaitingKind::Input
        } else {
            WaitingKind::Tool
        },
        refs: pending.iter().map(|p| p.task_id.clone()).collect(),
        deadline_ms: pending
            .iter()
            .filter_map(|p| p.until_ms)
            .chain(asked.iter().map(|a| a.deadline_ms))
            .min(),
        human_tasks: asked.iter().filter_map(|a| a.task_id.clone()).collect(),
    }
}

fn refused(call_id: &str, message: String) -> Observation {
    Observation::Error {
        call_id: call_id.to_string(),
        message,
        tool_result: None,
    }
}

/// Tool manager: a call answering `Pending` on `userinput:`. The question
/// is kept with the run before the snapshot suspends on it; a session that
/// cannot ask, or a malformed request, gets an error result instead (the
/// run does not suspend).
pub(super) async fn on_pending(
    sh: &Shared,
    run: &RunHandle,
    call_id: &str,
    task_id: &str,
    detail: Option<&Value>,
) -> Option<Observation> {
    let cfg = sh.session.lock().await.config.clone();
    let notify = !allowed(&cfg) && via_notify(&cfg, sh.deps.notify.is_some());
    if !allowed(&cfg) && !notify {
        return Some(refused(
            call_id,
            if cfg.session.kind == SessionKind::Goal {
                "in a goal session put the question in `questions` of your report; it reaches the user and the answer comes back as a later activation".into()
            } else {
                "this session cannot ask the user (no user is reading it); decide with what you have and state your assumptions in the report".into()
            },
        ));
    }
    if task_id != format!("{USER_INPUT_TASK_PREFIX}{call_id}") {
        return Some(refused(
            call_id,
            "ask-user must be the command of this call itself (not inside a pipeline, a script or another call)".into(),
        ));
    }
    let ask = match detail.map(|d| serde_json::from_value::<UserInputRequest>(d.clone())) {
        Some(Ok(a)) => a,
        Some(Err(e)) => return Some(refused(call_id, format!("ask-user: {e}"))),
        None => return Some(refused(call_id, "ask-user: the question is missing".into())),
    };
    if let Err(e) = ask.validate() {
        return Some(refused(call_id, format!("ask-user: {e}")));
    }
    if let Err(e) = run.save_ask(call_id, &ask) {
        return Some(refused(call_id, format!("ask-user: cannot keep the question: {e}")));
    }
    if notify {
        let deadline_ms = crate::now_ms()
            .saturating_add(cfg.session.policy.ask_user_timeout_secs.saturating_mul(1000));
        let record = AskNotify {
            deadline_ms,
            notify_id: None,
            task_id: None,
        };
        if run.ask_notify(call_id).is_none() {
            if let Err(e) = run.save_ask_notify(call_id, &record) {
                return Some(refused(call_id, format!("ask-user: cannot keep the question: {e}")));
            }
        }
    }
    None
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.trim().lines().next().unwrap_or_default().trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Before the state that waits for them is committed: the questions of
/// `snapshot`'s suspended calls asked through a notification and not sent
/// yet go out (the key makes a resend after a crash the same
/// notification). A failed send is tried again by the next commit of the
/// wait; the deadline holds either way.
pub(super) async fn send_notify_questions(
    sh: &Shared,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
) {
    let Some(notify) = sh.deps.notify.clone() else {
        return;
    };
    let (sid, turn, objective) = {
        let s = sh.session.lock().await;
        (
            s.sid().to_string(),
            s.state.current_turn(),
            s.config.session.objective.clone(),
        )
    };
    for call_id in user_calls(snapshot) {
        let Some(mut record) = run.ask_notify(&call_id) else {
            continue;
        };
        if record.notify_id.is_some() {
            continue;
        }
        let Some(ask) = run.ask(&call_id) else {
            continue;
        };
        let mut source = crate::notify::NotifySource {
            session_id: Some(sid.clone()),
            turn: Some(turn),
            call_id: Some(call_id.clone()),
            operation: Some(first_line(&objective, 200)).filter(|o| !o.is_empty()),
            ..Default::default()
        };
        crate::notify::complete_notify_source(sh.agent(), &mut source).await;
        let title = first_line(&ask.question, 120);
        let message = if ask.question.trim() == title {
            String::new()
        } else {
            ask.question.trim().to_string()
        };
        let options = ask
            .input
            .options
            .iter()
            .map(|o| crate::notify::HumanPromptOption::new(o.id.clone(), o.label.clone()))
            .collect::<Vec<_>>();
        let allow_text = ask.input.kind == UserInputKind::Text || ask.input.allow_text;
        let req = crate::notify::NotifySendReq {
            to_user: String::new(),
            kind: crate::notify::NotifyKind::Input,
            level: Some(crate::notify::NotifyLevel::Timely),
            category: None,
            prompt: crate::notify::input_prompt(title, message, options, allow_text),
            source: Some(source),
            expires_at_ms: Some(record.deadline_ms),
            idempotency_key: format!("{sid}:{turn}:ask:{call_id}"),
        };
        match notify.send(req).await {
            Ok(view) => {
                record.notify_id = Some(view.notify_id);
                record.task_id = view.task_id;
                if let Err(e) = run.save_ask_notify(&call_id, &record) {
                    log::warn!("session {sid}: notification of call {call_id} not recorded: {e}");
                }
            }
            Err(e) => log::warn!("session {sid}: question of call {call_id} not sent yet: {e}"),
        }
    }
}

fn not_answered(call_id: &str, reason: &str) -> Observation {
    let content = serde_json::json!({
        "answered": false,
        "reason": reason,
        "note": "decide with what you have, or end with a failure report that says what is missing",
    })
    .to_string();
    Observation::Success {
        call_id: call_id.to_string(),
        bytes: content.len(),
        content: Value::String(content),
        truncated: false,
        tool_result: None,
    }
}

/// Results of the calls that asked through a notification and whose wait
/// is over: answered, dismissed, expired, canceled, or past the deadline
/// (the notification is then canceled as `ask_timeout`). `force` (stop):
/// every such call is answered now and its notification canceled.
pub(super) async fn notify_answers(
    sh: &Shared,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
    force: bool,
) -> Vec<(String, Observation)> {
    let mut out = Vec::new();
    let notify = sh.deps.notify.clone();
    let now = crate::now_ms();
    for call_id in user_calls(snapshot) {
        let Some(record) = run.ask_notify(&call_id) else {
            continue;
        };
        let (Some(notify), Some(id)) = (notify.clone(), record.notify_id.clone()) else {
            if force {
                out.push((call_id.clone(), unanswered(&call_id, "the user did not answer: the turn was stopped")));
            } else if now >= record.deadline_ms {
                out.push((call_id.clone(), not_answered(&call_id, "timeout")));
            }
            continue;
        };
        if force {
            if let Err(e) = notify.cancel(&id, "turn_stopped").await {
                log::warn!("notification {id} not canceled: {e}");
            }
            out.push((call_id.clone(), unanswered(&call_id, "the user did not answer: the turn was stopped")));
            continue;
        }
        let view = match notify.state(&id).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("notification {id}: state unknown: {e}");
                if now >= record.deadline_ms {
                    let _ = notify.cancel(&id, "ask_timeout").await;
                    out.push((call_id.clone(), not_answered(&call_id, "timeout")));
                }
                continue;
            }
        };
        use crate::notify::NotifyStatus as S;
        match view.status {
            S::Answered => {
                let r = view.result.clone().unwrap_or_default();
                let option = r.option().map(str::to_string);
                let label = option.as_ref().and_then(|o| {
                    crate::notify::prompt_options(&view.prompt)
                        .into_iter()
                        .find(|x| &x.value == o)
                        .map(|x| x.label)
                });
                let answer = UserInputAnswer {
                    text: r
                        .text()
                        .map(str::to_string)
                        .or(label)
                        .or(option.clone())
                        .unwrap_or_default(),
                    attachments: Vec::new(),
                    option,
                    in_reply_to_question: true,
                    msg_id: String::new(),
                    from: r.submitted_by.clone().unwrap_or_default(),
                    at_ms: r.submitted_at.unwrap_or_default().max(0) as u64,
                    via: Some("notification".into()),
                    notify_id: Some(id.clone()),
                };
                out.push((call_id.clone(), answer_observation(&call_id, &answer)));
            }
            S::Dismissed => out.push((call_id.clone(), not_answered(&call_id, "dismissed"))),
            S::Expired => out.push((call_id.clone(), not_answered(&call_id, "timeout"))),
            S::Canceled => {
                let reason = if view.closed_reason.as_deref() == Some("ask_timeout") {
                    "timeout"
                } else {
                    "canceled"
                };
                out.push((call_id.clone(), not_answered(&call_id, reason)));
            }
            S::Open | S::Read if now >= record.deadline_ms => {
                if let Err(e) = notify.cancel(&id, "ask_timeout").await {
                    log::warn!("notification {id} not canceled: {e}");
                }
                out.push((call_id.clone(), not_answered(&call_id, "timeout")));
            }
            S::Open | S::Read => {}
        }
    }
    out
}

/// Inside the commit that makes state wait for `snapshot`'s suspended
/// calls: the questions of calls state did not wait for yet are queued, so
/// a question leaves exactly once (also when this is redone after a crash).
/// `true`: something was queued.
pub(super) async fn queue_questions(
    sh: &Shared,
    s: &mut Session,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
) -> bool {
    let waited: Vec<String> = s
        .state
        .waiting_for
        .as_ref()
        .map(|w| w.refs.clone())
        .unwrap_or_default();
    let mut queued = false;
    for call_id in user_calls(snapshot) {
        let task_id = format!("{USER_INPUT_TASK_PREFIX}{call_id}");
        if waited.contains(&task_id) || run.ask_notify(&call_id).is_some() {
            continue;
        }
        match run.ask(&call_id) {
            Some(ask) => {
                super::outbound::queue_question(sh, s, &call_id, &ask).await;
                queued = true;
            }
            None => log::warn!(
                "session {}: run {} waits for the answer to call {call_id} but has no question recorded",
                s.sid(),
                run.run_id()
            ),
        }
    }
    queued
}

/// The message that answers: the first msg the session would take next
/// (context messages are material, not an answer). Removed from
/// `candidates`.
pub(super) fn take_answer(candidates: &mut Vec<FetchedInput>) -> Option<FetchedInput> {
    let at = candidates
        .iter()
        .position(|m| m.msg().is_some_and(|s| !s.delivery.context))?;
    Some(candidates.remove(at))
}

/// The tool result of `call_id` from the message `m`.
pub(super) fn answer_of(state: &SessionState, call_id: &str, m: &FetchedInput) -> UserInputAnswer {
    let Some(sm) = m.msg() else {
        return UserInputAnswer::default();
    };
    let msg = &sm.msg;
    let targets = super::outbound::question_targets(state, state.current_turn());
    let in_reply_to_question = msg
        .relates_to
        .as_ref()
        .is_some_and(|r| r.rel == MsgRelType::Thread && targets.contains(&r.target.to_string()));
    let option = msg
        .content
        .machine
        .as_ref()
        .filter(|mc| mc.intent.as_deref() == Some(AGENT_ASK_ANSWER_INTENT))
        .and_then(|mc| serde_json::to_value(&mc.data).ok())
        .filter(|d| d.get("call_id").and_then(Value::as_str) == Some(call_id))
        .and_then(|d| d.get("option").and_then(Value::as_str).map(str::to_string));
    let attachments = msg
        .content
        .refs
        .iter()
        .filter_map(|r| match &r.target {
            RefTarget::DataObj { obj_id, .. } => Some(UserInputAttachment {
                obj_id: obj_id.to_string(),
                name: r.label.clone().filter(|l| !l.trim().is_empty()),
            }),
            _ => None,
        })
        .collect();
    UserInputAnswer {
        text: msg.content.content.trim().to_string(),
        attachments,
        option,
        in_reply_to_question,
        msg_id: m.key.clone(),
        from: msg.from.to_string(),
        at_ms: m.at_ms,
        via: None,
        notify_id: None,
    }
}

pub(super) fn answer_observation(call_id: &str, answer: &UserInputAnswer) -> Observation {
    let content = serde_json::to_string(answer).unwrap_or_default();
    Observation::Success {
        call_id: call_id.to_string(),
        bytes: content.len(),
        content: Value::String(content),
        truncated: false,
        tool_result: None,
    }
}

/// A question the user was not asked to the end (the Turn stopped, or the
/// batch had another question already answered).
pub(super) fn unanswered(call_id: &str, why: &str) -> Observation {
    Observation::Cancelled {
        call_id: call_id.to_string(),
        reason: why.to_string(),
        effect_unknown: false,
    }
}
