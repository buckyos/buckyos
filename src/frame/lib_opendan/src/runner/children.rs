//! Sub sessions of the session being driven (xAgent §4.10–§4.16).
//!
//! A parent's attention to its sub sessions is implicit: they are found in
//! the registry by `origin.parent_session`, nothing is subscribed and the
//! parent needs no input queue. The registry state is the truth; what this
//! module synthesizes only accelerates:
//!
//! - progress (`origin.report = progress`): merged into `pending_events`
//!   like any Observe event;
//! - needs attention / finished: candidates of a controlled input batch
//!   (source [`INTERNAL_CHILD_SRC`]). A candidate is synthesized again on
//!   every pass until the receipt of the batch that consumed it is
//!   committed; the receipt records the delivered state per child
//!   (`subscription_cursors["_child:<sid>"].attention`).
//!
//! A suspended call waiting for `session:<sid>` is answered by
//! [`SessionTaskResolver`] from the same registry state; that child's state
//! is then the call's tool result and is not delivered as an event too.

use std::sync::Arc;

use async_trait::async_trait;
use llm_context::tasks::{
    CancelUnsupported, RunningTaskResolver, TaskBrief, TaskResult, TaskState,
};
use serde_json::{json, Value};

use crate::error::Result;
use crate::protocol::*;
use crate::state::AgentStateClient;

use super::shared::{commit_and_report, Shared};

/// Task id prefix of a suspended call waiting for a sub session.
pub const SESSION_TASK_PREFIX: &str = "session:";

const ATTENTION_FINISHED: &str = "finished";
const ATTENTION_INPUT: &str = "needs_input";
const ATTENTION_DECISION: &str = "needs_decision";

fn cursor_key(sid: &str) -> String {
    format!("_child:{sid}")
}

/// `child:<sid>:<event>@<rev>` → `(sid, event)`.
pub(super) fn child_of_internal_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("child:")?;
    let (rest, _rev) = rest.rsplit_once('@')?;
    rest.rsplit_once(':')
}

fn attention_of(state: &SessionState, sid: &str) -> Option<String> {
    state
        .subscription_cursors
        .get(&cursor_key(sid))
        .and_then(|c| c.get("attention"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(super) fn set_attention(state: &mut SessionState, sid: &str, attention: Option<&str>) {
    let c = state
        .subscription_cursors
        .entry(cursor_key(sid))
        .or_insert_with(|| json!({}));
    c["attention"] = match attention {
        Some(a) => json!(a),
        None => Value::Null,
    };
}

/// What a sub session's registry state asks of its parent right now.
fn attention_now(e: &RegistryEntry) -> Option<&'static str> {
    if e.status.run_state == RunState::Finished {
        Some(ATTENTION_FINISHED)
    } else if e.status.pending_decision.is_some() {
        Some(ATTENTION_DECISION)
    } else if e.status.run_state == RunState::Waiting
        && e.status.waiting_for == Some(WaitingKind::Input)
    {
        Some(ATTENTION_INPUT)
    } else {
        None
    }
}

fn reports(e: &RegistryEntry) -> Option<ReportMode> {
    e.origin
        .as_ref()
        .and_then(|o| o.report)
        .filter(|r| *r != ReportMode::None)
}

fn cut(mut text: String) -> String {
    if text.len() > MAX_EVENT_SUMMARY_BYTES {
        let mut end = MAX_EVENT_SUMMARY_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

fn summary_of(e: &RegistryEntry, attention: Option<&str>) -> String {
    let st = &e.status;
    let mut text = match attention {
        Some(ATTENTION_FINISHED) => format!(
            "sub session {} finished ({})",
            e.session_id,
            st.outcome
                .map(|o| format!("{o:?}").to_lowercase())
                .unwrap_or_else(|| "no outcome".into())
        ),
        Some(ATTENTION_DECISION) => {
            format!("sub session {} waits for a decision", e.session_id)
        }
        Some(_) => format!(
            "sub session {} waits for input; answer it with `agent-session post {} --msg <text>`",
            e.session_id, e.session_id
        ),
        None => format!("sub session {} is {}", e.session_id, st.run_state.as_str()),
    };
    if !st.one_line_status.is_empty() {
        text.push_str(&format!(": {}", st.one_line_status));
    }
    if attention == Some(ATTENTION_FINISHED) && !st.report_brief.is_empty() {
        text.push_str(&format!("\nreport: {}", st.report_brief));
    }
    cut(text)
}

/// Sub sessions the session still has to hear from before it may finish:
/// reporting children that are not finished, or whose end was not delivered.
pub(super) async fn unsettled_children(sh: &Shared) -> Result<Vec<String>> {
    let me = sh.dir.sid().to_string();
    let children = sh.agent().sessions().children_of(&[me]).await?;
    let s = sh.session.lock().await;
    Ok(children
        .iter()
        .filter(|e| reports(e).is_some())
        .filter(|e| attention_of(&s.state, &e.session_id).as_deref() != Some(ATTENTION_FINISHED))
        .map(|e| e.session_id.clone())
        .collect())
}

/// One look at the session's sub sessions. Progress is saved as
/// semi-subscription state (and committed here); what needs the parent's
/// attention is returned as input candidates. `pending_tasks`: task ids the
/// suspended calls of the live run wait for.
pub(super) async fn poll_children(
    sh: &Arc<Shared>,
    pending_tasks: &[String],
) -> Result<Vec<FetchedInput>> {
    let me = sh.dir.sid().to_string();
    let children = sh.agent().sessions().children_of(&[me]).await?;
    if children.is_empty() {
        return Ok(Vec::new());
    }
    let mut inputs = Vec::new();
    let mut s = sh.session.lock().await;
    if s.state.is_finished() {
        return Ok(inputs);
    }
    let mut changed = false;
    for e in &children {
        let Some(report) = reports(e) else {
            continue;
        };
        let sid = e.session_id.as_str();
        if pending_tasks.contains(&format!("{SESSION_TASK_PREFIX}{sid}")) {
            continue;
        }
        let delivered = attention_of(&s.state, sid);
        let now = attention_now(e);
        match now {
            Some(a) if delivered.as_deref() != Some(a) => {
                let ev = AgentEvent {
                    subscription_id: None,
                    source: EventSource::new("session", sid),
                    event: a.to_string(),
                    seq: Some(e.status.rev),
                    summary: summary_of(e, Some(a)),
                    data_ref: None,
                    terminal: a == ATTENTION_FINISHED,
                };
                inputs.push(FetchedInput {
                    src: INTERNAL_CHILD_SRC.to_string(),
                    index: 0,
                    kind: INPUT_TYPE_EVENT.to_string(),
                    key: format!("child:{sid}:{a}@{}", e.status.rev),
                    from: "runner".to_string(),
                    at_ms: crate::now_ms(),
                    input: Ok(SessionInput::Event(ev)),
                });
            }
            None if delivered.is_some() => {
                // It left the state the parent was told about: the next time
                // it needs attention is a new event.
                set_attention(&mut s.state, sid, None);
                changed = true;
            }
            _ => {}
        }
        if report == ReportMode::Progress && now != Some(ATTENTION_FINISHED) {
            let key = cursor_key(sid);
            let cur = s.state.subscription_cursors.get(&key).cloned();
            let seen = cur
                .as_ref()
                .and_then(|c| c.get("rev"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let view = json!({
                "run_state": e.status.run_state,
                "one_line_status": e.status.one_line_status,
                "activity": e.status.activity.summary,
            });
            if e.status.rev > seen && cur.as_ref().and_then(|c| c.get("view")) != Some(&view) {
                let ev = AgentEvent {
                    subscription_id: Some(key.clone()),
                    source: EventSource::new("session", sid),
                    event: "progress".to_string(),
                    seq: Some(e.status.rev),
                    summary: summary_of(e, None),
                    data_ref: None,
                    terminal: false,
                };
                merge_pending_event(
                    &mut s.state.pending_events,
                    Some(&key),
                    &ev,
                    &format!("child:{sid}:progress@{}", e.status.rev),
                    None,
                    crate::now_ms(),
                );
                let c = s
                    .state
                    .subscription_cursors
                    .entry(key)
                    .or_insert_with(|| json!({}));
                c["rev"] = json!(e.status.rev);
                c["view"] = view;
                changed = true;
            }
        }
    }
    if changed {
        commit_and_report(sh, &mut s).await?;
    }
    Ok(inputs)
}

/// Answers for `session:<sid>` tasks from the registry (read-only); any
/// other task id goes to `inner`. Depends on the Agent State only, never on
/// runner memory.
pub struct SessionTaskResolver {
    agent: Arc<dyn AgentStateClient>,
    inner: Option<Arc<dyn RunningTaskResolver>>,
}

impl SessionTaskResolver {
    pub fn new(
        agent: Arc<dyn AgentStateClient>,
        inner: Option<Arc<dyn RunningTaskResolver>>,
    ) -> Self {
        Self { agent, inner }
    }

    async fn session_state(&self, sid: &str) -> TaskState {
        let e = match self.agent.sessions().lookup(sid).await {
            Ok(Some(e)) => e,
            Ok(None) => {
                return TaskState::Unknown {
                    reason: format!("session {sid} is not registered"),
                }
            }
            Err(e) => {
                return TaskState::Unknown {
                    reason: format!("the registry cannot be read: {e}"),
                }
            }
        };
        let st = &e.status;
        match attention_now(&e) {
            Some(ATTENTION_FINISHED) => TaskState::Finished(TaskResult {
                success: st.outcome == Some(Outcome::Succeeded),
                output: json!({
                    "session_id": sid,
                    "status": "finished",
                    "outcome": st.outcome,
                    "acceptance": st.acceptance,
                    "one_line_status": st.one_line_status,
                    "report_brief": st.report_brief,
                    "answer_ref": format!("{}/{REPORT_FILE}", e.location),
                    "artifact_id": e.artifact_id,
                })
                .to_string(),
                tool_result: None,
            }),
            Some(a) => TaskState::Finished(TaskResult {
                success: true,
                output: json!({
                    "session_id": sid,
                    "status": a,
                    "question": st.one_line_status,
                    "pending_decision": st.pending_decision,
                    "note": "answer with `agent-session post <sid> --msg <text>` (or `ctl <sid> decide`), then wait again",
                })
                .to_string(),
                tool_result: None,
            }),
            None => TaskState::Running {
                brief: format!(
                    "sub session {sid} is {}: {}",
                    st.run_state.as_str(),
                    st.one_line_status
                ),
                elapsed_ms: crate::now_ms().saturating_sub(st.updated_at_ms),
                output_tail: String::new(),
                cancellable: false,
            },
        }
    }
}

#[async_trait]
impl RunningTaskResolver for SessionTaskResolver {
    async fn state(&self, task_id: &str) -> TaskState {
        match (task_id.strip_prefix(SESSION_TASK_PREFIX), &self.inner) {
            (Some(sid), _) => self.session_state(sid).await,
            (None, Some(inner)) => inner.state(task_id).await,
            (None, None) => TaskState::Unknown {
                reason: "no task manager of this runner knows the task".into(),
            },
        }
    }

    async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState {
        let Some(sid) = task_id.strip_prefix(SESSION_TASK_PREFIX) else {
            return match &self.inner {
                Some(inner) => inner.wait(task_id, until_ms).await,
                None => self.state(task_id).await,
            };
        };
        loop {
            let state = self.session_state(sid).await;
            if !matches!(state, TaskState::Running { .. })
                || until_ms.is_some_and(|t| crate::now_ms() >= t)
            {
                return state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    async fn cancel(&self, task_id: &str) -> std::result::Result<TaskState, CancelUnsupported> {
        match (task_id.strip_prefix(SESSION_TASK_PREFIX), &self.inner) {
            // Waiting for a session is not owning it: stopping it is an
            // explicit `ctl <sid> stop`.
            (Some(_), _) | (None, None) => Err(CancelUnsupported {
                task_id: task_id.to_string(),
            }),
            (None, Some(inner)) => inner.cancel(task_id).await,
        }
    }

    fn can_resolve(&self, task_id: &str) -> bool {
        task_id.starts_with(SESSION_TASK_PREFIX)
            || self.inner.as_ref().is_some_and(|i| i.can_resolve(task_id))
    }

    fn watch(&self, task_id: &str) {
        if let Some(inner) = &self.inner {
            inner.watch(task_id);
        }
    }

    async fn active(&self) -> Vec<TaskBrief> {
        match &self.inner {
            Some(inner) => inner.active().await,
            None => Vec::new(),
        }
    }
}

/// After the suspended calls of a run were answered: a sub session whose
/// state was the answer is not reported as an event too.
pub(super) async fn mark_waited(sh: &Shared, task_ids: &[String]) -> Result<bool> {
    let mut marks = Vec::new();
    for t in task_ids {
        let Some(sid) = t.strip_prefix(SESSION_TASK_PREFIX) else {
            continue;
        };
        if let Some(e) = sh.agent().sessions().lookup(sid).await? {
            if let Some(a) = attention_now(&e) {
                marks.push((sid.to_string(), a));
            }
        }
    }
    if marks.is_empty() {
        return Ok(false);
    }
    let mut s = sh.session.lock().await;
    for (sid, a) in marks {
        set_attention(&mut s.state, &sid, Some(a));
    }
    Ok(true)
}
