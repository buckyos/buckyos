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
/// matching): the user when one of its calls asks.
pub(super) fn waiting_for(snapshot: &LLMContextSnapshot) -> WaitingFor {
    let pending = snapshot.state.pending_calls();
    WaitingFor {
        kind: if waits_for_user(snapshot) {
            WaitingKind::Input
        } else {
            WaitingKind::Tool
        },
        refs: pending.iter().map(|p| p.task_id.clone()).collect(),
        deadline_ms: pending.iter().filter_map(|p| p.until_ms).min(),
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
    if !allowed(&sh.session.lock().await.config) {
        return Some(refused(
            call_id,
            "this session cannot ask the user (no user is reading it); decide with what you have and state your assumptions in the report".into(),
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
    None
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
        if waited.contains(&task_id) {
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
