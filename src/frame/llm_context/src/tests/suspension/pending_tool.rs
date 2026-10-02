//! `PendingTool`: deferred calls, fills by call id, cancelled fills.

use buckyos_api::{AiMessage, AiResponse, AiRole};
use serde_json::json;

use crate::deps::{Injection};
use crate::error::{LLMComputeError};
use crate::observation::{Observation, ToolExecStatus};
use crate::outcome::{LLMContextOutcome, ResumeFill};
use crate::state::{LLMContextSnapshot, Suspension};
use crate::{LLMContext};

use super::mocks::*;

#[tokio::test]
async fn a_suspended_context_refuses_to_run_and_to_mismatched_fills() {
    let mut req = request();
    req.budget.context_yield_threshold = threshold(1);
    let llm = Llm::new(vec![]);
    let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
    let LLMContextOutcome::ContextLimitReached { snapshot, .. } = ctx.run().await else {
        panic!("expected ContextLimitReached");
    };
    // run() again without resuming: refused, state untouched.
    let outcome = ctx.run().await;
    assert!(
        matches!(outcome, LLMContextOutcome::Error { error: LLMComputeError::Internal(ref m), .. } if m.contains("suspended")),
        "{outcome:?}"
    );
    assert!(ctx.suspension().is_some());
    assert_eq!(llm.calls(), 0);
    // Nothing can be injected into a suspended context either.
    assert_eq!(
        ctx.inject(Injection {
            messages: vec![AiMessage::text(AiRole::User, "late input")],
            host: None,
        }),
        crate::InjectionPosition::None
    );
    assert_eq!(ctx.snapshot().state.accumulated.len(), 1);

    let d = || deps(llm.clone(), Tools::new());
    corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::ResumeFromMidRun,
        d(),
    ));
    corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::ToolResults {
            results: vec![success("x", "y")],
        },
        d(),
    ));
    // A function call context cannot take a behavior rewrite and vice versa.
    corrupted(LLMContext::resume(
        snapshot,
        ResumeFill::RewrittenSteps {
            input: vec![AiMessage::text(AiRole::User, "s")],
            history_summaries: vec![],
            steps: vec![],
            last_step: None,
        },
        d(),
    ));
}

// ===================================================================
// PendingTool
// ===================================================================

#[tokio::test]
async fn pending_without_allow_deferred_is_a_contract_violation() {
    let llm = Llm::new(vec![tools_response(vec![
        call("defer", "p"),
        call("a", "c"),
    ])]);
    let tools = Tools::new();
    let mut req = request();
    req.tool_policy.allow_deferred = false;
    let mut ctx = LLMContext::new(req, deps(llm, tools.clone()));
    let outcome = ctx.run().await;
    let LLMContextOutcome::Error { error, trace, .. } = outcome else {
        panic!("expected Error, got {outcome:?}");
    };
    assert!(
        matches!(error, LLMComputeError::Internal(ref m) if m.contains("allow_deferred=false"))
    );
    assert_eq!(tools.calls(), vec!["p"]);
    let statuses: Vec<_> = trace.tool_trace.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        vec![ToolExecStatus::Unknown, ToolExecStatus::NotExecuted]
    );
    assert!(ctx.suspension().is_none());
}

#[tokio::test]
async fn pending_stops_the_batch_and_resume_runs_only_the_rest() {
    let llm = Llm::new(vec![
        with_usage(
            tools_response(vec![
                call("a", "c1"),
                call("defer", "c2"),
                call("fail", "c3"),
            ]),
            10,
        ),
        with_usage(AiResponse::text("done"), 5),
    ]);
    let tools = Tools::new();
    let mut ctx = LLMContext::new(request(), deps(llm.clone(), tools.clone()));

    let outcome = ctx.run().await;
    let LLMContextOutcome::PendingTool {
        pending,
        snapshot,
        trace,
        ..
    } = outcome
    else {
        panic!("expected PendingTool, got {outcome:?}");
    };
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].call.call_id, "c2");
    assert_eq!(tools.calls(), vec!["c1", "c2"], "c3 is not dispatched yet");
    let statuses: Vec<_> = trace.tool_trace.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        vec![ToolExecStatus::Succeeded, ToolExecStatus::Pending]
    );
    let batch = snapshot
        .state
        .tool_batch
        .clone()
        .expect("batch continuation");
    assert_eq!(batch.remaining.len(), 1);
    assert_eq!(batch.remaining[0].call_id, "c3");
    assert_eq!(
        snapshot.state.tool_iterations_left, 6,
        "the tool iteration is charged when the batch completes"
    );

    // Another run before the fill is refused and dispatches nothing.
    assert!(matches!(ctx.run().await, LLMContextOutcome::Error { .. }));
    assert_eq!(tools.calls().len(), 2);

    let snapshot = round_trip(&snapshot);
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ToolResults {
            results: vec![success("c2", "late result")],
        },
        deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    let outcome = ctx.run().await;
    let LLMContextOutcome::Done { usage, trace, .. } = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    assert_eq!(usage.total_tokens, Some(15));
    assert_eq!(tools.calls(), vec!["c1", "c2", "c3"], "nothing re-runs");
    let statuses: Vec<_> = trace
        .tool_trace
        .iter()
        .map(|r| (r.call_id.as_str(), r.status))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("c2", ToolExecStatus::Succeeded),
            ("c3", ToolExecStatus::Failed)
        ]
    );
    assert_eq!(
        tool_result_ids(&llm.seen()[1]),
        vec!["c1", "c2", "c3"],
        "results in call order, each once"
    );
    let state = ctx.snapshot().state;
    assert_eq!(state.tool_iterations_left, 5);
    assert_eq!(
        state.consecutive_errors, 1,
        "c3's failure counts once for the batch"
    );
    assert!(state.tool_batch.is_none() && state.suspended.is_none());
}

#[tokio::test]
async fn two_consecutive_suspensions_keep_state_across_json() {
    let llm = Llm::new(vec![
        with_usage(
            tools_response(vec![call("defer", "p1"), call("defer", "p2")]),
            3,
        ),
        with_usage(AiResponse::text("done"), 4),
    ]);
    let tools = Tools::new();
    let mut req = request();
    req.budget.max_wallclock_ms = Some(60_000);
    let mut ctx = LLMContext::new(req, deps(llm.clone(), tools.clone()));
    let mut host = json!({"libopendan": {"input_receipts": [1]}});
    ctx.set_host_meta(Some(host.clone()));

    let LLMContextOutcome::PendingTool { snapshot, .. } = ctx.run().await else {
        panic!("expected PendingTool");
    };
    let mut snapshot = round_trip(&snapshot);
    assert_eq!(snapshot.state.host.as_ref(), Some(&host));
    // A long wait is not charged to the wallclock budget.
    let now = snapshot.state.suspended.as_ref().unwrap().at_ms();
    snapshot.state.started_at_ms -= 1_000_000;
    if let Some(Suspension::PendingTool { at_ms, .. }) = snapshot.state.suspended.as_mut() {
        *at_ms = now - 999_000;
    }

    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ToolResults {
            results: vec![success("p1", "r1")],
        },
        deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    let LLMContextOutcome::PendingTool {
        pending, snapshot, ..
    } = ctx.run().await
    else {
        panic!("expected the second PendingTool");
    };
    assert_eq!(pending[0].call.call_id, "p2");
    let snapshot = round_trip(&snapshot);
    host["libopendan"]["input_receipts"] = json!([1, 2]);
    let mut snapshot = snapshot;
    snapshot.state.host = Some(host.clone());

    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ToolResults {
            results: vec![success("p2", "r2")],
        },
        deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    let outcome = ctx.run().await;
    let LLMContextOutcome::Done { usage, .. } = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    assert_eq!(usage.total_tokens, Some(7));
    assert_eq!(tools.calls(), vec!["p1", "p2"]);
    assert_eq!(tool_result_ids(&llm.seen()[1]), vec!["p1", "p2"]);
    let state = ctx.snapshot().state;
    assert_eq!(state.host, Some(host));
    assert_eq!(state.tool_iterations_left, 5);
}

#[tokio::test]
async fn tool_results_are_matched_by_call_id() {
    let llm = Llm::new(vec![tools_response(vec![call("defer", "p")])]);
    let mut ctx = LLMContext::new(request(), deps(llm.clone(), Tools::new()));
    let LLMContextOutcome::PendingTool { snapshot, .. } = ctx.run().await else {
        panic!("expected PendingTool");
    };
    let d = || deps(llm.clone(), Tools::new());
    let fill = |results: Vec<(String, Observation)>| ResumeFill::ToolResults { results };

    let m = corrupted(LLMContext::resume(snapshot.clone(), fill(vec![]), d()));
    assert!(m.contains("missing result"), "{m}");
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        fill(vec![success("p", "a"), success("p", "b")]),
        d(),
    ));
    assert!(m.contains("duplicate"), "{m}");
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        fill(vec![success("p", "a"), success("q", "b")]),
        d(),
    ));
    assert!(m.contains("not pending"), "{m}");
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        fill(vec![(
            "p".into(),
            Observation::Pending {
                call_id: "p".into(),
                tool_result: None,
            },
        )]),
        d(),
    ));
    assert!(m.contains("still pending"), "{m}");
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        fill(vec![("p".into(), success("q", "a").1)]),
        d(),
    ));
    assert!(m.contains("carries call id"), "{m}");
    // A PendingTool snapshot cannot be bypassed.
    corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::ResumeFromMidRun,
        d(),
    ));
    corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::RewrittenHistory {
            history: vec![AiMessage::text(AiRole::User, "x")],
        },
        d(),
    ));
    // Stripping the suspension by hand leaves the call unanswered.
    let mut stripped = snapshot.clone();
    stripped.state.suspended = None;
    stripped.state.tool_batch = None;
    let m = corrupted(LLMContext::resume(
        stripped,
        ResumeFill::ResumeFromMidRun,
        d(),
    ));
    assert!(m.contains("unanswered"), "{m}");
    // Every accepted terminal result.
    for obs in [
        success("p", "ok").1,
        Observation::Error {
            call_id: "p".into(),
            message: "e".into(),
            tool_result: None,
        },
        Observation::Cancelled {
            call_id: "p".into(),
            reason: "user".into(),
        },
        Observation::Unresolved {
            call_id: "p".into(),
            reason: "task lost".into(),
            effect_unknown: true,
        },
    ] {
        LLMContext::resume(snapshot.clone(), fill(vec![("p".into(), obs)]), d()).unwrap();
    }
}

#[tokio::test]
async fn out_of_order_results_are_written_in_call_order() {
    // A snapshot waiting for two calls at once (valid shape of the type).
    let mut req = request();
    let mut state = crate::state::LLMContextState::from_request(&req, 1);
    let calls = vec![call("defer", "x"), call("defer", "y")];
    state
        .accumulated
        .push(tools_response(calls.clone()).message);
    state.tool_batch = Some(Default::default());
    state.suspended = Some(Suspension::PendingTool {
        pending: calls
            .into_iter()
            .map(|c| crate::observation::PendingToolCall {
                call: c,
                eta_ms: None,
                tool_result: None,
            })
            .collect(),
        at_ms: 1,
    });
    req.tool_policy.max_tool_iterations = 2;
    let llm = Llm::new(vec![AiResponse::text("done")]);
    let mut ctx = LLMContext::resume(
        LLMContextSnapshot {
            request: req,
            state,
        },
        ResumeFill::ToolResults {
            results: vec![success("y", "second"), success("x", "first")],
        },
        deps(llm.clone(), Tools::new()),
    )
    .unwrap();
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(tool_result_ids(&llm.seen()[0]), vec!["x", "y"]);
}

#[tokio::test]
async fn a_cancelled_fill_winds_the_rest_of_the_batch_down() {
    let llm = Llm::new(vec![
        tools_response(vec![call("defer", "p"), call("a", "c")]),
        AiResponse::text("stopped"),
    ]);
    let tools = Tools::new();
    let mut ctx = LLMContext::new(request(), deps(llm.clone(), tools.clone()));
    let LLMContextOutcome::PendingTool { snapshot, .. } = ctx.run().await else {
        panic!("expected PendingTool");
    };
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ToolResults {
            results: vec![(
                "p".into(),
                Observation::Cancelled {
                    call_id: "p".into(),
                    reason: "user interrupt".into(),
                },
            )],
        },
        deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    assert!(ctx
        .snapshot()
        .state
        .tool_batch
        .unwrap()
        .remaining
        .is_empty());
    let LLMContextOutcome::Done { trace, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(tools.calls(), vec!["p"], "c never starts");
    let statuses: Vec<_> = trace.tool_trace.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        vec![ToolExecStatus::Cancelled, ToolExecStatus::NotExecuted]
    );
    assert_eq!(tool_result_ids(&llm.seen()[1]), vec!["p", "c"]);
}
