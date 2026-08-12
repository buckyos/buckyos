//! Snapshot overrides — the data shape and helpers that schedulers use to
//! rebuild the next [`LLMContext`] from a previous run's snapshot while
//! changing only the inputs they care about (system prompt, tool policy,
//! ...). Inherited state (`accumulated` history, step records, usage,
//! pending tool calls — though see the pre-condition below) carries over
//! unchanged.
//!
//! Three scheduler modes are expressed via the same two functions:
//!
//! - **switch** — same session, swap system messages + tool policy, persist
//!   the rebuilt snapshot back to the session's state file.
//! - **fork** — sub-context inherits parent snapshot, runs to completion,
//!   parent resumes from its own on-disk snapshot. The sub-context's
//!   rebuilt snapshot is intentionally never persisted.
//! - **independent** — each named behavior has its own snapshot file; the
//!   active one is loaded, optionally rebuilt with new policies, and run.
//!
//! Originally lived in `opendan::llm_context_helper`. Moved here so the
//! waist owns its own data-rebuilding surface; opendan now imports these
//! types directly rather than re-defining them.

use buckyos_api::{AiMessage, AiRole};

use crate::context_loop::LLMContext;
use crate::deps::LLMContextDeps;
use crate::error::LLMComputeError;
use crate::outcome::ResumeFill;
use crate::request::{
    BudgetSpec, ErrorPolicy, HumanPolicy, LLMContextRequest, ModelPolicy, OutputSpec, ToolPolicy,
};
use crate::state::LLMContextSnapshot;

/// Overlay applied to a base [`LLMContextSnapshot`] before rebuilding the next
/// [`LLMContext`]. Every field is `Option`/`bool` so callers only specify what
/// they want to change; unset fields are inherited verbatim from the base.
#[derive(Debug, Clone, Default)]
pub struct RequestOverrides {
    pub system_messages: Option<Vec<AiMessage>>,
    /// Replace the non-system input/history segment. Used by fork callers
    /// that need a clean child context instead of inheriting the parent's
    /// full accumulated transcript.
    pub user_messages: Option<Vec<AiMessage>>,
    pub tool_policy: Option<ToolPolicy>,
    pub objective: Option<String>,
    pub behavior_name: Option<String>,
    /// Outer Option = override or not; inner Option = override target value
    /// (`Some(Some(_))` set, `Some(None)` clear, `None` keep).
    pub trace: Option<Option<String>>,
    pub model_policy: Option<ModelPolicy>,
    pub budget: Option<BudgetSpec>,
    pub human_policy: Option<HumanPolicy>,
    pub error_policy: Option<ErrorPolicy>,
    pub output: Option<OutputSpec>,

    /// Reset `state.rounds_left` to the new `tool_policy.max_rounds`. Caller
    /// sets `true` for fork / independent, leaves `false` for switch (which
    /// continues the parent budget).
    pub reset_rounds: bool,
    /// Reset `state.consecutive_errors` to 0. Caller sets `true` for fork /
    /// independent — switch keeps the counter so a behavior swap cannot
    /// silently bypass the error cap.
    pub reset_errors: bool,

    pub reset_behavior_hot_tail: bool,

    /// Fork-only hard constraint. When `true`, the rebuilt request will
    /// carry `forbid_next_behavior = true` and the Behavior Loop will scrub
    /// any `<next_behavior>` the LLM emits. See
    /// [`LLMContextRequest::forbid_next_behavior`].
    pub forbid_next_behavior: bool,
}

/// Apply [`RequestOverrides`] to a snapshot, returning a new snapshot ready to
/// feed into [`LLMContext::resume`] with [`ResumeFill::ResumeFromMidRun`].
///
/// Maintains the invariant that `request.input` and `state.accumulated` share
/// the same leading System segment.
pub fn apply_overrides_to_snapshot(
    mut snap: LLMContextSnapshot,
    ov: RequestOverrides,
) -> LLMContextSnapshot {
    if let Some(new_system) = ov.system_messages {
        replace_leading_system(&mut snap.request.input, &new_system);
        replace_leading_system(&mut snap.state.accumulated, &new_system);
    }
    if let Some(new_user) = ov.user_messages {
        replace_after_leading_system(&mut snap.request.input, &new_user);
        replace_after_leading_system(&mut snap.state.accumulated, &new_user);
        snap.state.pending_tool_calls.clear();
        snap.state.steps.clear();
        snap.state.history_summaries.clear();
        snap.state.history_inputs.clear();
        snap.state.last_step = None;
        snap.state.last_report = None;
    }

    if let Some(tp) = ov.tool_policy {
        if ov.reset_rounds {
            snap.state.rounds_left = tp.max_rounds;
        }
        snap.request.tool_policy = tp;
    } else if ov.reset_rounds {
        // No new policy supplied but caller asked for reset — reset to the
        // existing policy's max_rounds.
        snap.state.rounds_left = snap.request.tool_policy.max_rounds;
    }

    if ov.reset_errors {
        snap.state.consecutive_errors = 0;
    }

    if let Some(obj) = ov.objective {
        snap.request.objective = obj;
    }
    if let Some(behavior_name) = ov.behavior_name {
        snap.request.behavior_name = behavior_name;
    }
    if let Some(trace) = ov.trace {
        snap.request.trace = trace;
    }
    if let Some(mp) = ov.model_policy {
        snap.request.model_policy = mp;
    }
    if let Some(b) = ov.budget {
        snap.request.budget = b;
    }
    if let Some(hp) = ov.human_policy {
        snap.request.human_policy = hp;
    }
    if let Some(ep) = ov.error_policy {
        snap.request.error_policy = ep;
    }
    if let Some(o) = ov.output {
        snap.request.output = o;
    }

    // Fork hard constraint. The flag is sticky on the rebuilt request — a
    // child fork sub-ctx inherits it from its parent automatically; an
    // intentional clear has to be expressed by setting `forbid_next_behavior
    // = false` on a fresh `RequestOverrides`, which (Default) is exactly
    // the no-op case for this field — so we only ever flip the bit on.
    if ov.forbid_next_behavior {
        snap.request.forbid_next_behavior = true;
    }

    if ov.reset_behavior_hot_tail {
        if let Some(last) = snap.state.last_step.take() {
            snap.state.steps.push(last);
        }
    }

    snap
}

/// Replace the leading run of `System`-role messages in `msgs` with `new_system`.
/// Non-System messages following the leading block are left untouched.
fn replace_leading_system(msgs: &mut Vec<AiMessage>, new_system: &[AiMessage]) {
    let leading = msgs
        .iter()
        .position(|m| m.role != AiRole::System)
        .unwrap_or(msgs.len());
    let tail = msgs.split_off(leading);
    msgs.clear();
    msgs.extend(new_system.iter().cloned());
    msgs.extend(tail);
}

fn replace_after_leading_system(msgs: &mut Vec<AiMessage>, new_tail: &[AiMessage]) {
    let leading = msgs
        .iter()
        .position(|m| m.role != AiRole::System)
        .unwrap_or(msgs.len());
    msgs.truncate(leading);
    msgs.extend(new_tail.iter().cloned());
}

/// Build the next [`LLMContext`] from a base snapshot, inheriting `state` (and
/// therefore `accumulated` / `steps` / `usage`) while applying `overrides` to
/// the request side. Used by **switch** (caller writes snapshot back to the
/// session) and **fork** (caller throws sub snapshot away after sub run ends).
///
/// Returns `Err(SnapshotCorrupted)` if the base snapshot is in a suspended
/// state (has `pending_tool_calls`) — caller must resume that one first via
/// the normal [`LLMContext::resume`] flow before inheriting from it.
pub fn rebuild_with_inherit(
    base_snap: LLMContextSnapshot,
    overrides: RequestOverrides,
    deps: LLMContextDeps,
) -> Result<LLMContext, LLMComputeError> {
    if !base_snap.state.pending_tool_calls.is_empty() {
        return Err(LLMComputeError::SnapshotCorrupted(
            "rebuild_with_inherit: base snapshot has pending tool calls — \
             resume the pending tools before forking/switching"
                .to_string(),
        ));
    }
    let rebuilt = apply_overrides_to_snapshot(base_snap, overrides);
    LLMContext::resume(rebuilt, ResumeFill::ResumeFromMidRun, deps)
}

/// Build a fresh [`LLMContext`] with no inherited state. Thin wrapper over
/// [`LLMContext::new`] — exposed alongside [`rebuild_with_inherit`] so all
/// scheduler-side ctx construction can route through this module.
pub fn build_fresh(request: LLMContextRequest, deps: LLMContextDeps) -> LLMContext {
    LLMContext::new(request, deps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{ContextOwnerRef, ToolMode};
    use crate::state::LLMContextState;
    use buckyos_api::AiContent;

    fn msg(role: AiRole, text: &str) -> AiMessage {
        AiMessage {
            role,
            content: vec![AiContent::Text {
                text: text.to_string(),
            }],
        }
    }

    fn snap_with(input: Vec<AiMessage>, accumulated: Vec<AiMessage>) -> LLMContextSnapshot {
        let request = LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: "s".into(),
            },
            trace: None,
            objective: String::new(),
            behavior_name: String::new(),
            input,
            model_policy: ModelPolicy::default(),
            tool_policy: ToolPolicy::default(),
            output: OutputSpec::default(),
            budget: BudgetSpec::default(),
            human_policy: HumanPolicy::default(),
            error_policy: ErrorPolicy::default(),
            forbid_next_behavior: false,
        };
        let mut state = LLMContextState::from_request(&request, 0);
        state.accumulated = accumulated;
        state.rounds_left = request.tool_policy.max_rounds;
        LLMContextSnapshot { request, state }
    }

    #[test]
    fn replace_leading_system_replaces_block_and_keeps_tail() {
        let mut msgs = vec![
            msg(AiRole::System, "old-1"),
            msg(AiRole::System, "old-2"),
            msg(AiRole::User, "u-1"),
            msg(AiRole::Assistant, "a-1"),
            msg(AiRole::User, "u-2"),
        ];
        let new_sys = vec![msg(AiRole::System, "new")];
        replace_leading_system(&mut msgs, &new_sys);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, AiRole::System);
        assert_eq!(msgs[0].text_content(), "new");
        assert_eq!(msgs[1].role, AiRole::User);
        assert_eq!(msgs[1].text_content(), "u-1");
        assert_eq!(msgs[3].text_content(), "u-2");
    }

    #[test]
    fn replace_leading_system_with_no_existing_system_prepends() {
        let mut msgs = vec![msg(AiRole::User, "u")];
        let new_sys = vec![msg(AiRole::System, "s")];
        replace_leading_system(&mut msgs, &new_sys);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, AiRole::System);
        assert_eq!(msgs[1].role, AiRole::User);
    }

    #[test]
    fn replace_leading_system_with_empty_new_strips_existing() {
        let mut msgs = vec![msg(AiRole::System, "old"), msg(AiRole::User, "u")];
        replace_leading_system(&mut msgs, &[]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, AiRole::User);
    }

    #[test]
    fn replace_after_leading_system_keeps_system_and_replaces_tail() {
        let mut msgs = vec![
            msg(AiRole::System, "sys"),
            msg(AiRole::User, "old-u"),
            msg(AiRole::Assistant, "old-a"),
        ];
        replace_after_leading_system(&mut msgs, &[msg(AiRole::User, "new-u")]);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text_content(), "sys");
        assert_eq!(msgs[1].text_content(), "new-u");
    }

    #[test]
    fn apply_overrides_syncs_system_in_request_and_accumulated() {
        let snap = snap_with(
            vec![msg(AiRole::System, "sys-old"), msg(AiRole::User, "u")],
            vec![
                msg(AiRole::System, "sys-old"),
                msg(AiRole::User, "u"),
                msg(AiRole::Assistant, "a-runtime"),
            ],
        );
        let ov = RequestOverrides {
            system_messages: Some(vec![msg(AiRole::System, "sys-new")]),
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);

        assert_eq!(out.request.input[0].text_content(), "sys-new");
        assert_eq!(out.request.input[1].text_content(), "u");
        assert_eq!(out.state.accumulated[0].text_content(), "sys-new");
        assert_eq!(out.state.accumulated[1].text_content(), "u");
        // Accumulated tail (assistant from earlier rounds) survives.
        assert_eq!(out.state.accumulated[2].text_content(), "a-runtime");
    }

    #[test]
    fn apply_overrides_user_messages_replaces_parent_history() {
        let snap = snap_with(
            vec![msg(AiRole::System, "sys-old"), msg(AiRole::User, "u")],
            vec![
                msg(AiRole::System, "sys-old"),
                msg(AiRole::User, "u"),
                msg(AiRole::Assistant, "a-runtime"),
            ],
        );
        let ov = RequestOverrides {
            system_messages: Some(vec![msg(AiRole::System, "sys-new")]),
            user_messages: Some(vec![msg(AiRole::User, "parent history")]),
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);

        assert_eq!(out.request.input.len(), 2);
        assert_eq!(out.request.input[0].text_content(), "sys-new");
        assert_eq!(out.request.input[1].text_content(), "parent history");
        assert_eq!(out.state.accumulated, out.request.input);
    }

    #[test]
    fn apply_overrides_reset_rounds_uses_new_policy() {
        let snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        let mut tp = ToolPolicy::default();
        tp.mode = ToolMode::Whitelist;
        tp.max_rounds = 99;
        let ov = RequestOverrides {
            tool_policy: Some(tp),
            reset_rounds: true,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert_eq!(out.state.rounds_left, 99);
        assert_eq!(out.request.tool_policy.max_rounds, 99);
    }

    #[test]
    fn apply_overrides_no_reset_keeps_rounds_left() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        snap.state.rounds_left = 3; // mid-run state
        let mut tp = ToolPolicy::default();
        tp.max_rounds = 99;
        let ov = RequestOverrides {
            tool_policy: Some(tp),
            reset_rounds: false,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert_eq!(out.state.rounds_left, 3, "switch must not reset budget");
        assert_eq!(out.request.tool_policy.max_rounds, 99);
    }

    #[test]
    fn apply_overrides_reset_errors_clears_counter() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        snap.state.consecutive_errors = 2;
        let ov = RequestOverrides {
            reset_errors: true,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert_eq!(out.state.consecutive_errors, 0);
    }

    #[test]
    fn apply_overrides_trace_can_set_or_clear() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        snap.request.trace = Some("parent::1".into());

        let out = apply_overrides_to_snapshot(
            snap.clone(),
            RequestOverrides {
                trace: Some(Some("parent::1::fork-0".into())),
                ..Default::default()
            },
        );
        assert_eq!(out.request.trace.as_deref(), Some("parent::1::fork-0"));

        let out_cleared = apply_overrides_to_snapshot(
            snap.clone(),
            RequestOverrides {
                trace: Some(None),
                ..Default::default()
            },
        );
        assert!(out_cleared.request.trace.is_none());

        let out_unchanged = apply_overrides_to_snapshot(
            snap,
            RequestOverrides {
                trace: None,
                ..Default::default()
            },
        );
        assert_eq!(out_unchanged.request.trace.as_deref(), Some("parent::1"));
    }

    #[test]
    fn apply_overrides_forbid_next_behavior_sets_request_flag() {
        let snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        assert!(!snap.request.forbid_next_behavior);
        let ov = RequestOverrides {
            forbid_next_behavior: true,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert!(out.request.forbid_next_behavior);
    }

    #[test]
    fn apply_overrides_behavior_switch_resets_hot_tail() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        let mut last = crate::behavior_loop::StepRecord::default();
        last.meta.behavior_name = "plan".into();
        last.meta.step_index = 7;
        last.assistant_text = "old hot".into();
        snap.state.last_step = Some(last);
        snap.request.behavior_name = "plan".into();

        let out = apply_overrides_to_snapshot(
            snap,
            RequestOverrides {
                behavior_name: Some("execute".into()),
                reset_behavior_hot_tail: true,
                ..Default::default()
            },
        );

        assert_eq!(out.request.behavior_name, "execute");
        assert!(out.state.last_step.is_none());
        assert_eq!(out.state.steps.len(), 1);
        assert_eq!(out.state.steps[0].meta.behavior_name, "plan");
        assert_eq!(out.state.steps[0].assistant_text, "old hot");
    }
}
