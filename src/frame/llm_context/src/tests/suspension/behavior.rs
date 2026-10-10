//! Suspensions in behavior mode: pending actions, inner native pending,
//! context limit on the materialized prompt, interrupted steps.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiResponse, AiRole};
use serde_json::json;

use crate::behavior_loop::StepRecord;
use crate::context_window::estimate_messages;
use crate::deps::{LLMContextDeps, LlmClient, LlmInferenceRequest};
use crate::error::{LLMComputeError};
use crate::observation::{Observation};
use crate::outcome::{ContextLimitKind, LLMContextOutcome, ResumeFill};
use crate::state::{Suspension};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

fn xml(body: &str) -> AiResponse {
    AiResponse::text(format!("<response>{body}</response>"))
}

#[tokio::test]
async fn behavior_action_pending_keeps_the_step_and_resumes_after_it() {
    let llm = Llm::new(vec![
        xml("<thinking>go</thinking><actions><shell>echo a</shell><shell>defer b</shell><shell>echo c</shell></actions>"),
        xml("<thinking>saw all</thinking><actions><shell>echo d</shell></actions>"),
        xml("<report end=\"true\">finished</report>"),
    ]);
    let tools = Tools::new();
    let mut ctx = LLMContext::new(
        behavior_request(),
        behavior_deps(llm.clone(), tools.clone()),
    );

    let outcome = ctx.run().await;
    let LLMContextOutcome::PendingTool {
        pending, snapshot, ..
    } = outcome
    else {
        panic!("expected PendingTool, got {outcome:?}");
    };
    assert_eq!(pending[0].call.call_id, "2");
    assert_eq!(tools.calls(), vec!["1", "2"]);
    assert!(snapshot.state.steps.is_empty() && snapshot.state.last_step.is_none());
    let step = &snapshot
        .state
        .action_step
        .as_ref()
        .expect("step in progress")
        .step;
    assert_eq!(step.actions.len(), 3);
    assert_eq!(step.action_results.len(), 1);
    assert_eq!(
        snapshot.state.tool_iterations_left, 5,
        "the step's tool iteration is already charged"
    );

    let snapshot = round_trip(&snapshot);
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ToolResults {
            results: vec![success("2", "b done")],
        },
        behavior_deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    let outcome = ctx.run().await;
    assert!(
        matches!(outcome, LLMContextOutcome::Done { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        tools.calls(),
        vec!["1", "2", "3", "4"],
        "no action runs twice"
    );
    let state = ctx.snapshot().state;
    assert_eq!(state.steps.len(), 3);
    let indices: Vec<u32> = state.steps.iter().map(|s| s.meta.step_index).collect();
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(state.steps[0].action_results.len(), 3);
    assert!(matches!(
        &state.steps[0].action_results[1],
        Observation::Success { content, .. } if content == &json!("b done")
    ));
    assert_eq!(state.next_action_id, 4);
    assert_eq!(llm.calls(), 3, "the pending step is not re-inferred");
}

#[tokio::test]
async fn behavior_inner_native_pending_resumes_the_inner_loop_without_rerunning_tools() {
    let llm = Llm::new(vec![
        xml("<thinking>step 0</thinking><actions><shell>echo a</shell></actions>"),
        tools_response(vec![call("echo", "n1"), call("defer", "n2")]),
        xml("<report end=\"true\">finished</report>"),
    ]);
    let tools = Tools::new();
    let mut ctx = LLMContext::new(
        behavior_request(),
        behavior_deps(llm.clone(), tools.clone()),
    );

    let outcome = ctx.run().await;
    let LLMContextOutcome::PendingTool { snapshot, .. } = outcome else {
        panic!("expected PendingTool, got {outcome:?}");
    };
    assert_eq!(tools.calls(), vec!["1", "n1", "n2"]);
    assert_eq!(
        snapshot.state.last_step.as_ref().map(|s| s.meta.step_index),
        Some(0)
    );
    let tail = &snapshot.state.accumulated[snapshot.request.input.len()..];
    assert_eq!(
        tool_result_ids(tail),
        vec!["n1"],
        "the inner transcript is kept"
    );

    let mut ctx = LLMContext::resume(
        round_trip(&snapshot),
        ResumeFill::ToolResults {
            results: vec![success("n2", "deferred")],
        },
        behavior_deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    let outcome = ctx.run().await;
    assert!(
        matches!(outcome, LLMContextOutcome::Done { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        tools.calls(),
        vec!["1", "n1", "n2"],
        "native tools do not re-run"
    );
    let seen = llm.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(tool_result_ids(&seen[2]), vec!["n1", "n2"]);
    let state = ctx.snapshot().state;
    assert_eq!(
        state.accumulated.len(),
        ctx.snapshot().request.input.len(),
        "tail cleared"
    );
    assert_eq!(state.steps.len(), 2);
}

#[tokio::test]
async fn behavior_context_limit_measures_the_prompt_and_rewrites_steps() {
    let step = |n: u32, pad: usize| {
        xml(&format!(
            "<thinking>step {n} {}</thinking><actions><shell>echo {n}</shell></actions>",
            "z".repeat(pad)
        ))
    };
    let script = || {
        vec![
            step(0, 400),
            step(1, 0),
            xml("<thinking>after rewrite</thinking><actions><shell>echo 2</shell></actions>"),
            xml("<report end=\"true\">finished</report>"),
        ]
    };
    // Probe: the prompts of an unlimited run.
    let probe = Llm::new(script());
    let mut ctx = LLMContext::new(
        behavior_request(),
        behavior_deps(probe.clone(), Tools::new()),
    );
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    let probe_seen = probe.seen();
    let second = estimate_messages(&CharTokenizer, &probe_seen[1]);
    let third = estimate_messages(&CharTokenizer, &probe_seen[2]);
    assert!(third > second + 1);

    let llm = Llm::new(script());
    let tools = Tools::new();
    let mut req = behavior_request();
    req.budget.context_yield_threshold = threshold(second as u32 + 1);
    let mut ctx = LLMContext::new(req, behavior_deps(llm.clone(), tools.clone()));

    let outcome = ctx.run().await;
    let LLMContextOutcome::ContextLimitReached {
        which,
        accumulated,
        snapshot,
        ..
    } = outcome
    else {
        panic!("expected ContextLimitReached, got {outcome:?}");
    };
    assert_eq!(which, ContextLimitKind::ApproachingWindow);
    assert_eq!(llm.calls(), 2);
    assert_eq!(
        accumulated, probe_seen[2],
        "the materialized prompt that was not sent"
    );
    assert_eq!(
        LLMContext::rewritable_history(&snapshot, &behavior_deps(llm.clone(), tools.clone())),
        accumulated,
        "rebuildable from the snapshot alone"
    );
    assert!(matches!(
        snapshot.state.suspended,
        Some(Suspension::ContextLimit { estimated_tokens: Some(t), .. }) if t == third
    ));
    assert_eq!(snapshot.state.steps.len(), 1);
    assert!(snapshot.state.last_step.is_some());

    // Mismatched behavior rewrites are rejected.
    let d = || behavior_deps(llm.clone(), tools.clone());
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::RewrittenHistory {
            history: vec![AiMessage::text(AiRole::User, "x")],
        },
        d(),
    ));
    assert!(m.contains("behavior context_limit"), "{m}");
    let mut foreign = snapshot.state.steps[0].clone();
    foreign.meta.step_index = 7;
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::RewrittenSteps {
            input: vec![AiMessage::text(AiRole::User, "x")],
            history_summaries: vec![],
            steps: vec![foreign],
            last_step: None,
        },
        d(),
    ));
    assert!(m.contains("not in the snapshot"), "{m}");
    let m = corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::RewrittenSteps {
            input: vec![AiMessage::text(AiRole::User, "x")],
            history_summaries: vec![],
            steps: vec![],
            last_step: Some(snapshot.state.steps[0].clone()),
        },
        d(),
    ));
    assert!(m.contains("hot step"), "{m}");

    // Fold step 0 into the input, keep the hot step.
    let hot: StepRecord = snapshot.state.last_step.clone().unwrap();
    let input = vec![AiMessage::text(AiRole::User, "hello; summary of step 0")];
    let mut ctx = LLMContext::resume(
        round_trip(&snapshot),
        ResumeFill::RewrittenSteps {
            input: input.clone(),
            history_summaries: vec![],
            steps: vec![],
            last_step: Some(hot),
        },
        d(),
    )
    .unwrap();
    let outcome = ctx.run().await;
    assert!(
        matches!(outcome, LLMContextOutcome::Done { .. }),
        "{outcome:?}"
    );
    let seen = llm.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(
        seen[2][0], input[0],
        "the rewritten input is the new prefix"
    );
    assert!(
        estimate_messages(&CharTokenizer, &seen[2]) < second,
        "the prompt shrank"
    );
    assert_eq!(
        seen[3][..seen[2].len()],
        seen[2][..],
        "append-only again after the rewrite"
    );
    let state = ctx.snapshot().state;
    let indices: Vec<u32> = state.steps.iter().map(|s| s.meta.step_index).collect();
    assert_eq!(indices, vec![1, 2, 3], "numbering continues");
    assert_eq!(tools.calls(), vec!["1", "2", "3"]);
}

/// Returns `first`, then interrupts its own context on the next call.
struct InterruptSecond {
    first: Mutex<Option<AiResponse>>,
    handle: Mutex<Option<crate::LLMContextInterruptHandle>>,
    calls: Mutex<usize>,
}

#[async_trait]
impl LlmClient for InterruptSecond {
    async fn infer(&self, _req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        *self.calls.lock().unwrap() += 1;
        if let Some(r) = self.first.lock().unwrap().take() {
            return Ok(r);
        }
        if let Some(h) = self.handle.lock().unwrap().as_ref() {
            h.interrupt("preempted");
        }
        Err(LLMComputeError::Cancelled)
    }
}

#[tokio::test]
async fn an_interrupted_behavior_step_keeps_the_native_tools_it_ran() {
    let llm = Arc::new(InterruptSecond {
        first: Mutex::new(Some(tools_response(vec![call("echo", "n1")]))),
        handle: Mutex::new(None),
        calls: Mutex::new(0),
    });
    let tools = Tools::new();
    let d = LLMContextDeps::new(llm.clone(), tools.clone())
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new().without_timestamps()));
    let mut ctx = LLMContext::new(behavior_request(), d);
    *llm.handle.lock().unwrap() = Some(ctx.interrupt_handle());
    let outcome = ctx.run().await;
    let LLMContextOutcome::Interrupted { snapshot, .. } = outcome else {
        panic!("expected Interrupted, got {outcome:?}");
    };
    assert_eq!(tools.calls(), vec!["n1"]);
    let tail = &snapshot.state.accumulated[snapshot.request.input.len()..];
    assert_eq!(tool_result_ids(tail), vec!["n1"], "the inner transcript so far is kept");

    let resumed_llm = Llm::new(vec![xml("<report end=\"true\">finished</report>")]);
    let mut ctx = LLMContext::resume(
        round_trip(&snapshot),
        ResumeFill::ResumeFromMidRun,
        behavior_deps(resumed_llm.clone(), tools.clone()),
    )
    .unwrap();
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(tools.calls(), vec!["n1"], "the native tool is not replayed");
    assert_eq!(tool_result_ids(&resumed_llm.seen()[0]), vec!["n1"]);
}
