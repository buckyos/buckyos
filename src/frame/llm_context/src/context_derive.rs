//! Deriving a child context from a parent snapshot.
//!
//! Two constructions, both pure (the parent snapshot is only read) and both
//! valid whatever the parent is doing — running, suspended on a tool
//! (`PendingTool`), or in the middle of a tool batch / behavior step:
//!
//! - [`derive_child`] — *create-sub-context*: the child has its own request
//!   (system, model, tools, budget) and takes from the parent only what the
//!   caller selects ([`InheritHistory`]).
//! - [`fork_snapshot`] — *fork*: the child keeps the parent's system and the
//!   complete effective history up to a recorded fork point; only the
//!   branch task is appended afterwards.
//!
//! A derived snapshot never carries the parent's suspension, calls waiting
//! for dispatch, usage, error counter or host metadata: the child is a run
//! of its own. What it inherited is reported as an [`InheritBoundary`] so
//! the host records only what the child adds.
//!
//! Nothing here relaxes [`crate::LLMContext::inject`] or
//! [`crate::snapshot_overrides::rebuild_with_inherit`]: the parent still has
//! to be resumed through its own fill.

use buckyos_api::{AiMessage, AiRole};
use serde::{Deserialize, Serialize};

use crate::behavior_loop::StepRecord;
use crate::request::{BudgetSpec, ContextOwnerRef, LLMContextRequest, ToolPolicy};
use crate::state::{LLMContextSnapshot, LLMContextState};
use crate::suspension::{check_paired, strip_step_thinking, strip_thinking, unanswered_tool_calls};

/// Why a derivation was refused. Nothing was built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot derive a child context: {0}")]
pub struct DeriveError(pub String);

fn refused(message: impl Into<String>) -> DeriveError {
    DeriveError(message.into())
}

/// What a create-sub-context child takes from its parent.
#[derive(Debug, Clone, Default)]
pub enum InheritHistory {
    /// Nothing: the child request is the whole input.
    #[default]
    None,
    /// Material selected and rendered by the host (recent dialogue, a
    /// summary, results of calls that already ran), placed after the child's
    /// system block. It is a filtered view chosen by the host, not the
    /// parent's history: the host labels its origin in the text.
    Material(Vec<AiMessage>),
    /// The parent's completed behavior steps and history summaries, as
    /// structured records. Both contexts must run the behavior loop with a
    /// compatible parser / renderer; across loop modes render the steps into
    /// [`InheritHistory::Material`] instead. The step still dispatching
    /// actions (`action_step`) is not a completed record and is left out.
    Steps,
}

/// Where the fork was taken in the parent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkPoint {
    /// Messages of the parent's `accumulated` the fork kept.
    pub messages: usize,
    /// The parent's `next_step_index` at the fork.
    pub next_step_index: u32,
    /// The parent had a tool batch / behavior step in progress: the fork was
    /// taken before it, the batch stays with the parent.
    pub before_open_batch: bool,
}

/// What a derived child inherited: everything at or after these marks is
/// the child's own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritBoundary {
    /// Leading messages of the child's `accumulated` that came with the
    /// derivation (system, inherited history / material).
    pub messages: usize,
    /// Steps with `step_index` below this were inherited.
    pub steps_below: u32,
    /// First action id the child allocates.
    pub next_action_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point: Option<ForkPoint>,
}

/// A derived child: resume `snapshot` with `ResumeFill::ResumeFromMidRun`
/// and inject the task input.
#[derive(Debug, Clone)]
pub struct DerivedContext {
    pub snapshot: LLMContextSnapshot,
    pub boundary: InheritBoundary,
}

fn leading_system(msgs: &[AiMessage]) -> usize {
    msgs.iter()
        .position(|m| m.role != AiRole::System)
        .unwrap_or(msgs.len())
}

fn completed_steps(state: &LLMContextState) -> Vec<StepRecord> {
    state
        .steps
        .iter()
        .chain(state.last_step.iter())
        .cloned()
        .collect()
}

/// create-sub-context: a child with its own request and an explicit
/// selection of the parent's history.
///
/// Budgets, usage and the error counter start from `child_request`; step
/// and action numbering continues the parent's so ids stay unique across
/// both runs.
pub fn derive_child(
    parent: &LLMContextSnapshot,
    mut child_request: LLMContextRequest,
    inherit: InheritHistory,
) -> Result<DerivedContext, DeriveError> {
    let mut steps = Vec::new();
    let mut summaries = Vec::new();
    match inherit {
        InheritHistory::None => {}
        InheritHistory::Material(mut material) => {
            strip_thinking(&mut material);
            material.retain(|m| !m.content.is_empty());
            if material.iter().any(|m| m.role == AiRole::System) {
                return Err(refused(
                    "inherited material must not carry system messages; the child has its own system",
                ));
            }
            check_paired(&material).map_err(|e| refused(e.to_string()))?;
            let at = leading_system(&child_request.input);
            let tail = child_request.input.split_off(at);
            child_request.input.extend(material);
            child_request.input.extend(tail);
        }
        InheritHistory::Steps => {
            if child_request.behavior_name.is_empty() {
                return Err(refused(
                    "structured steps can only be inherited by a behavior loop child; render them as material instead",
                ));
            }
            steps = completed_steps(&parent.state);
            for s in steps.iter_mut() {
                // Thinking is only valid under the prefix it was produced with.
                strip_step_thinking(s);
            }
            summaries = parent.state.history_summaries.clone();
        }
    }
    let mut state = LLMContextState::from_request(&child_request, crate::now_ms());
    state.steps = steps;
    state.history_summaries = summaries;
    state.next_step_index = parent.state.next_step_index;
    state.next_action_id = parent.state.next_action_id;
    let boundary = InheritBoundary {
        messages: state.accumulated.len(),
        steps_below: parent.state.next_step_index,
        next_action_id: parent.state.next_action_id,
        fork_point: None,
    };
    Ok(DerivedContext {
        snapshot: LLMContextSnapshot {
            request: child_request,
            state,
        },
        boundary,
    })
}

/// What a fork may set for the branch. The system block, the model and the
/// loop mode are the parent's and cannot be changed here.
#[derive(Debug, Clone, Default)]
pub struct ForkOptions {
    pub owner: Option<ContextOwnerRef>,
    pub trace: Option<String>,
    pub objective: Option<String>,
    /// Tool set of the branch. A different tool definition is allowed but
    /// may miss the provider's prefix cache.
    pub tool_policy: Option<ToolPolicy>,
    pub budget: Option<BudgetSpec>,
    /// System block the target declares, if any. A fork never replaces the
    /// system: a declaration that differs from the parent's is refused.
    pub expect_system: Option<Vec<AiMessage>>,
}

/// fork: the parent's system and its complete effective history up to the
/// fork point.
///
/// The fork point is the last legal prefix: the whole history when no call
/// is open, otherwise the history before the tool batch (function call) or
/// before the step (behavior) in progress — that batch, its partial results
/// and the calls not dispatched yet stay with the parent and are never run
/// by the branch. Results of calls of that batch the branch needs are given
/// to it as task input by the host; nothing is patched into the prefix.
///
/// The request keeps `behavior_name`, model and output so the inherited
/// history renders exactly as it did for the parent. In function call mode
/// the inherited history becomes the branch's `request.input`; in behavior
/// mode `request.input`, steps, hot step and history inputs are kept as they
/// are.
pub fn fork_snapshot(
    parent: &LLMContextSnapshot,
    options: ForkOptions,
) -> Result<DerivedContext, DeriveError> {
    let mut request = parent.request.clone();
    let pstate = &parent.state;
    if let Some(expected) = &options.expect_system {
        let sys = leading_system(&request.input);
        if request.input[..sys] != expected[..] {
            return Err(refused(
                "fork keeps the parent's system prompt, but the target declares a different one; use create-sub-context",
            ));
        }
    }
    let open_batch = pstate.has_continuation() || !pstate.pending_calls().is_empty();
    let behavior = !request.behavior_name.is_empty();
    let prefix = request.input.len();
    let extends_input =
        pstate.accumulated.len() >= prefix && pstate.accumulated[..prefix] == request.input[..];
    let mut accumulated = pstate.accumulated.clone();
    if open_batch {
        if behavior {
            if !extends_input {
                return Err(refused(
                    "the parent's history does not extend its request input; no legal fork point",
                ));
            }
            // The inner transcript belongs to the step in progress.
            accumulated.truncate(prefix);
        } else {
            let at = accumulated
                .iter()
                .rposition(|m| {
                    m.role == AiRole::Assistant
                        && m.content
                            .iter()
                            .any(|c| matches!(c, buckyos_api::AiContent::ToolUse { .. }))
                })
                .ok_or_else(|| {
                    refused("the parent has an open tool batch but no tool call message")
                })?;
            accumulated.truncate(at);
        }
    }
    let open = unanswered_tool_calls(&accumulated);
    if !open.is_empty() {
        return Err(refused(format!(
            "no legal fork point: tool calls {} are unanswered before the open batch",
            open.join(", ")
        )));
    }
    if accumulated.is_empty() {
        return Err(refused("the parent has no history to fork"));
    }
    if let Some(o) = options.owner {
        request.owner = o;
    }
    if let Some(t) = options.trace {
        request.trace = Some(t);
    }
    if let Some(o) = options.objective {
        request.objective = o;
    }
    if let Some(tp) = options.tool_policy {
        request.tool_policy = tp;
    }
    if let Some(b) = options.budget {
        request.budget = b;
    }
    if !behavior {
        // Function call mode: the inherited history is the branch's stable
        // input; what the branch appends follows it.
        request.input = accumulated.clone();
    }
    let mut state = LLMContextState::from_request(&request, crate::now_ms());
    state.accumulated = accumulated;
    state.steps = pstate.steps.clone();
    state.last_step = pstate.last_step.clone();
    state.history_summaries = pstate.history_summaries.clone();
    state.history_inputs = pstate.history_inputs.clone();
    state.last_report = None;
    state.next_step_index = pstate.next_step_index;
    state.next_action_id = pstate.next_action_id;
    let fork_point = ForkPoint {
        messages: state.accumulated.len(),
        next_step_index: pstate.next_step_index,
        before_open_batch: open_batch,
    };
    let boundary = InheritBoundary {
        messages: fork_point.messages,
        steps_below: pstate.next_step_index,
        next_action_id: pstate.next_action_id,
        fork_point: Some(fork_point),
    };
    Ok(DerivedContext {
        snapshot: LLMContextSnapshot { request, state },
        boundary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::PendingToolCall;
    use crate::request::{ErrorPolicy, HumanPolicy, ModelPolicy, OutputSpec};
    use crate::state::{ActionStep, Suspension, ToolBatch};
    use buckyos_api::{AiContent, AiResponse, AiToolCall};
    use serde_json::json;

    fn msg(role: AiRole, text: &str) -> AiMessage {
        AiMessage::text(role, text.to_string())
    }

    fn call(id: &str) -> AiToolCall {
        AiToolCall {
            name: "t".into(),
            args: Default::default(),
            call_id: id.into(),
        }
    }

    fn tool_use(ids: &[&str]) -> AiMessage {
        AiMessage {
            role: AiRole::Assistant,
            content: ids
                .iter()
                .map(|id| AiContent::ToolUse {
                    call_id: id.to_string(),
                    name: "t".into(),
                    args: Default::default(),
                })
                .collect(),
        }
    }

    fn tool_result(id: &str, text: &str) -> AiMessage {
        AiMessage {
            role: AiRole::Tool,
            content: vec![AiContent::ToolResult {
                call_id: id.into(),
                content: vec![buckyos_api::AiToolResultContent::text(text.to_string())],
                is_error: false,
            }],
        }
    }

    fn request(behavior: &str, input: Vec<AiMessage>) -> LLMContextRequest {
        LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: "s".into(),
            },
            trace: None,
            objective: String::new(),
            behavior_name: behavior.into(),
            input,
            model_policy: ModelPolicy::default(),
            tool_policy: ToolPolicy::default(),
            output: OutputSpec::default(),
            budget: BudgetSpec::default(),
            human_policy: HumanPolicy::default(),
            error_policy: ErrorPolicy::default(),
            forbid_next_behavior: false,
        }
    }

    fn snap(behavior: &str, input: Vec<AiMessage>, tail: Vec<AiMessage>) -> LLMContextSnapshot {
        let request = request(behavior, input);
        let mut state = LLMContextState::from_request(&request, 1);
        state.accumulated.extend(tail);
        LLMContextSnapshot { request, state }
    }

    fn step(behavior: &str, index: u32, text: &str) -> StepRecord {
        let mut s = StepRecord::default();
        s.meta.behavior_name = behavior.into();
        s.meta.step_index = index;
        s.assistant_text = text.into();
        s
    }

    fn fc_parent() -> LLMContextSnapshot {
        let mut p = snap(
            "",
            vec![msg(AiRole::System, "S"), msg(AiRole::User, "task")],
            vec![
                tool_use(&["a"]),
                tool_result("a", "a-result"),
                msg(AiRole::Assistant, "thinking aloud"),
            ],
        );
        p.state.tool_iterations_left = 2;
        p.state.consecutive_errors = 2;
        p.state.usage.total_tokens = Some(99);
        p.state.host = Some(json!({"h": 1}));
        p
    }

    #[test]
    fn fork_keeps_system_and_full_paired_history() {
        let p = fc_parent();
        let before = serde_json::to_value(&p).unwrap();
        let d = fork_snapshot(&p, ForkOptions::default()).unwrap();
        assert_eq!(d.snapshot.state.accumulated, p.state.accumulated);
        assert_eq!(d.snapshot.request.input, p.state.accumulated);
        assert_eq!(d.boundary.messages, 5);
        let fp = d.boundary.fork_point.unwrap();
        assert!(!fp.before_open_batch);
        // The branch is a run of its own.
        assert_eq!(
            d.snapshot.state.tool_iterations_left,
            p.request.tool_policy.max_tool_iterations
        );
        assert_eq!(d.snapshot.state.consecutive_errors, 0);
        assert!(d.snapshot.state.usage.total_tokens.is_none());
        assert!(d.snapshot.state.host.is_none());
        assert_eq!(serde_json::to_value(&p).unwrap(), before, "parent untouched");
    }

    #[test]
    fn fork_before_an_open_batch_leaves_the_batch_with_the_parent() {
        let mut p = fc_parent();
        p.state.accumulated.push(tool_use(&["first", "fork", "other"]));
        p.state.accumulated.push(tool_result("first", "done"));
        p.state.tool_batch = Some(ToolBatch {
            remaining: vec![call("other")],
            batch_error: None,
        });
        p.state.suspended = Some(Suspension::PendingTool {
            pending: vec![PendingToolCall {
                call: call("fork"),
                task_id: "t1".into(),
                until_ms: None,
            }],
            at_ms: 5,
        });
        let d = fork_snapshot(&p, ForkOptions::default()).unwrap();
        // H = history before the triggering batch; nothing of the batch.
        assert_eq!(d.snapshot.state.accumulated.len(), 5);
        assert!(unanswered_tool_calls(&d.snapshot.state.accumulated).is_empty());
        assert!(d.snapshot.state.suspended.is_none());
        assert!(d.snapshot.state.tool_batch.is_none());
        assert!(d.boundary.fork_point.unwrap().before_open_batch);
        // The parent keeps its whole batch.
        assert_eq!(p.state.accumulated.len(), 7);
        assert!(p.state.tool_batch.is_some());
    }

    #[test]
    fn fork_refuses_a_different_system() {
        let p = fc_parent();
        let err = fork_snapshot(
            &p,
            ForkOptions {
                expect_system: Some(vec![msg(AiRole::System, "other")]),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.0.contains("create-sub-context"), "{err}");
        fork_snapshot(
            &p,
            ForkOptions {
                expect_system: Some(vec![msg(AiRole::System, "S")]),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn fork_of_a_behavior_parent_keeps_steps_hot_step_and_inputs() {
        let mut p = snap(
            "plan",
            vec![msg(AiRole::System, "S"), msg(AiRole::User, "task")],
            vec![tool_use(&["inner"])],
        );
        p.state.steps = vec![step("plan", 0, "s0")];
        p.state.last_step = Some(step("plan", 1, "hot"));
        p.state.history_inputs = vec![crate::behavior_loop::HistoryInputRecord {
            source: "x".into(),
            text: "in".into(),
            at_ms: 1,
        }];
        p.state.next_step_index = 3;
        p.state.next_action_id = 7;
        p.state.action_step = Some(ActionStep {
            step: step("plan", 2, "in progress"),
            response: AiResponse::default(),
        });
        let d = fork_snapshot(&p, ForkOptions::default()).unwrap();
        let s = &d.snapshot.state;
        assert_eq!(d.snapshot.request.behavior_name, "plan");
        assert_eq!(s.steps.len(), 1);
        assert_eq!(s.last_step.as_ref().unwrap().assistant_text, "hot");
        assert_eq!(s.history_inputs.len(), 1);
        assert!(s.action_step.is_none(), "the step in progress is not inherited");
        assert_eq!(s.accumulated, p.request.input, "inner transcript dropped");
        assert_eq!((s.next_step_index, s.next_action_id), (3, 7));
        assert_eq!(d.boundary.steps_below, 3);
    }

    #[test]
    fn fork_refuses_when_no_paired_prefix_exists() {
        let mut p = fc_parent();
        // An unanswered call that is not part of an open batch.
        p.state.accumulated.insert(2, tool_use(&["lost"]));
        let err = fork_snapshot(&p, ForkOptions::default()).unwrap_err();
        assert!(err.0.contains("no legal fork point"), "{err}");
    }

    #[test]
    fn derive_child_none_uses_only_the_child_request() {
        let mut p = fc_parent();
        p.state.next_step_index = 4;
        p.state.next_action_id = 9;
        let child = request("", vec![msg(AiRole::System, "child"), msg(AiRole::User, "sub")]);
        let d = derive_child(&p, child, InheritHistory::None).unwrap();
        assert_eq!(d.snapshot.state.accumulated.len(), 2);
        assert_eq!(d.snapshot.request.input[0].text_content(), "child");
        assert_eq!(d.boundary.messages, 2);
        assert_eq!(d.snapshot.state.next_action_id, 9);
        assert!(d.boundary.fork_point.is_none());
    }

    #[test]
    fn derive_child_material_goes_after_the_child_system() {
        let p = fc_parent();
        let child = request("", vec![msg(AiRole::System, "child"), msg(AiRole::User, "sub")]);
        let d = derive_child(
            &p,
            child,
            InheritHistory::Material(vec![msg(AiRole::User, "<parent_dialogue>…</parent_dialogue>")]),
        )
        .unwrap();
        let texts: Vec<String> = d
            .snapshot
            .request
            .input
            .iter()
            .map(|m| m.text_content())
            .collect();
        assert_eq!(texts, vec!["child", "<parent_dialogue>…</parent_dialogue>", "sub"]);
        assert_eq!(d.snapshot.state.accumulated, d.snapshot.request.input);
        assert_eq!(d.boundary.messages, 3);
        // Unpaired tool blocks are not material.
        let child = request("", vec![msg(AiRole::System, "child")]);
        assert!(
            derive_child(&p, child, InheritHistory::Material(vec![tool_use(&["x"])])).is_err()
        );
    }

    #[test]
    fn derive_child_steps_takes_completed_steps_only() {
        let mut p = snap("plan", vec![msg(AiRole::System, "S")], vec![]);
        p.state.steps = vec![step("plan", 0, "s0")];
        p.state.last_step = Some(step("plan", 1, "s1"));
        p.state.next_step_index = 3;
        p.state.action_step = Some(ActionStep {
            step: step("plan", 2, "in progress"),
            response: AiResponse::default(),
        });
        p.state.suspended = Some(Suspension::PendingTool {
            pending: vec![PendingToolCall {
                call: call("c"),
                task_id: "t".into(),
                until_ms: None,
            }],
            at_ms: 1,
        });
        let child = request("do", vec![msg(AiRole::System, "DO")]);
        let d = derive_child(&p, child, InheritHistory::Steps).unwrap();
        let s = &d.snapshot.state;
        assert_eq!(
            s.steps.iter().map(|x| x.meta.step_index).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(s.last_step.is_none() && s.action_step.is_none() && s.suspended.is_none());
        assert_eq!(s.next_step_index, 3);
        assert_eq!(d.boundary.steps_below, 3);
        assert_eq!(d.snapshot.request.input[0].text_content(), "DO");
        // Function call children cannot take structured steps.
        let fc = request("", vec![msg(AiRole::System, "DO")]);
        assert!(derive_child(&p, fc, InheritHistory::Steps).is_err());
    }
}
