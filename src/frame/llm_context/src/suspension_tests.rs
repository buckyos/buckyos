//! X7: `PendingTool` / `ContextLimitReached` produced by a real `run()`,
//! resumed through (de)serialized snapshots.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiToolCall, AiUsage};
use serde_json::json;

use crate::behavior_loop::StepRecord;
use crate::context_window::estimate_messages;
use crate::deps::{
    CheckpointHook, Injection, LLMContextDeps, LlmClient, LlmInferenceRequest, Tokenizer,
    ToolDispatchError, ToolManager,
};
use crate::error::{CheckpointStage, LLMComputeError, ProviderFailure};
use crate::observation::{Observation, ToolExecStatus};
use crate::outcome::{ContextLimitKind, LLMContextOutcome, ResumeFill};
use crate::request::{
    ContextOwnerRef, ContextThreshold, LLMContextRequest, ModelPolicy, OutputSpec, ToolMode,
    ToolPolicy,
};
use crate::state::{LLMContextSnapshot, Suspension};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

/// One token per char: estimates are exact in tests.
struct CharTokenizer;

impl Tokenizer for CharTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
}

/// Scripted responses / errors; records every request's messages.
struct Llm {
    script: Mutex<Vec<Result<AiResponse, LLMComputeError>>>,
    seen: Mutex<Vec<Vec<AiMessage>>>,
}

impl Llm {
    fn new(script: Vec<AiResponse>) -> Arc<Self> {
        Self::with_results(script.into_iter().map(Ok).collect())
    }

    fn with_results(script: Vec<Result<AiResponse, LLMComputeError>>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }

    fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait]
impl LlmClient for Llm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        guard.remove(0)
    }
}

/// A call whose name or arguments mention `defer` is deferred, `fail` is a
/// business error, `big` returns a large result; everything else succeeds.
/// Records every dispatched call id.
struct Tools {
    calls: Mutex<Vec<String>>,
}

impl Tools {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolManager for Tools {
    async fn call_tool(&self, call: AiToolCall) -> Result<Observation, ToolDispatchError> {
        self.calls.lock().unwrap().push(call.call_id.clone());
        let text = format!(
            "{} {}",
            call.name,
            serde_json::to_string(&call.args).unwrap_or_default()
        );
        if text.contains("defer") {
            return Ok(Observation::Pending {
                call_id: call.call_id,
                tool_result: None,
            });
        }
        if text.contains("fail") {
            return Ok(Observation::Error {
                call_id: call.call_id,
                message: "bad args".into(),
                tool_result: None,
            });
        }
        let content = if text.contains("big") {
            json!("x".repeat(400))
        } else {
            json!(format!("ok {}", call.call_id))
        };
        Ok(Observation::Success {
            call_id: call.call_id,
            content,
            bytes: 0,
            truncated: false,
            tool_result: None,
        })
    }
}

fn request() -> LLMContextRequest {
    LLMContextRequest {
        owner: ContextOwnerRef::OneShot { id: "x7".into() },
        trace: Some("x7".into()),
        objective: "x7".into(),
        behavior_name: String::new(),
        input: vec![AiMessage::text(AiRole::User, "hello")],
        model_policy: ModelPolicy {
            preferred: "m".into(),
            ..ModelPolicy::default()
        },
        tool_policy: ToolPolicy {
            mode: ToolMode::All,
            max_rounds: 6,
            max_calls_per_round: 6,
            allow_deferred: true,
            ..ToolPolicy::default()
        },
        output: OutputSpec::Text,
        budget: Default::default(),
        human_policy: Default::default(),
        error_policy: Default::default(),
        forbid_next_behavior: false,
    }
}

fn behavior_request() -> LLMContextRequest {
    let mut r = request();
    r.behavior_name = "do".into();
    r
}

fn deps(llm: Arc<Llm>, tools: Arc<Tools>) -> LLMContextDeps {
    LLMContextDeps::new(llm, tools).with_tokenizer(Arc::new(CharTokenizer))
}

fn behavior_deps(llm: Arc<Llm>, tools: Arc<Tools>) -> LLMContextDeps {
    deps(llm, tools)
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new().without_timestamps()))
}

fn call(name: &str, id: &str) -> AiToolCall {
    AiToolCall {
        name: name.into(),
        args: HashMap::new(),
        call_id: id.into(),
    }
}

fn tools_response(calls: Vec<AiToolCall>) -> AiResponse {
    AiResponse::from_parts(None, calls, vec![])
}

fn with_usage(mut r: AiResponse, total: u64) -> AiResponse {
    r.usage = Some(AiUsage {
        total_tokens: Some(total),
        ..Default::default()
    });
    r
}

/// Snapshots cross processes as JSON.
fn round_trip(s: &LLMContextSnapshot) -> LLMContextSnapshot {
    serde_json::from_str(&serde_json::to_string(s).unwrap()).unwrap()
}

fn success(id: &str, text: &str) -> (String, Observation) {
    (
        id.to_string(),
        Observation::Success {
            call_id: id.into(),
            content: json!(text),
            bytes: 0,
            truncated: false,
            tool_result: None,
        },
    )
}

fn tool_result_ids(messages: &[AiMessage]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            AiContent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect()
}

fn corrupted(r: Result<LLMContext, LLMComputeError>) -> String {
    match r {
        Err(LLMComputeError::SnapshotCorrupted(m)) => m,
        Err(e) => panic!("expected SnapshotCorrupted, got {e:?}"),
        Ok(_) => panic!("expected SnapshotCorrupted, resume succeeded"),
    }
}

fn threshold(value: u32) -> Option<ContextThreshold> {
    Some(ContextThreshold::AbsoluteTokens { value })
}

// "hello" as the only message: 4 (overhead) + 5 = 9 tokens.
const HELLO_TOKENS: u32 = 9;

// ===================================================================
// ContextLimitReached
// ===================================================================

#[tokio::test]
async fn threshold_below_at_and_above_the_first_request() {
    for (value, expect_yield) in [
        (HELLO_TOKENS + 1, false),
        (HELLO_TOKENS, true),
        (HELLO_TOKENS - 1, true),
    ] {
        let llm = Llm::new(vec![AiResponse::text("done")]);
        let mut req = request();
        req.budget.context_yield_threshold = threshold(value);
        let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
        let outcome = ctx.run().await;
        if expect_yield {
            let LLMContextOutcome::ContextLimitReached {
                which,
                accumulated,
                snapshot,
                ..
            } = outcome
            else {
                panic!("threshold {value}: expected ContextLimitReached, got {outcome:?}");
            };
            assert_eq!(which, ContextLimitKind::ApproachingWindow);
            assert_eq!(accumulated.len(), 1);
            assert_eq!(
                llm.calls(),
                0,
                "a first request over the threshold is never sent"
            );
            assert!(matches!(
                snapshot.state.suspended,
                Some(Suspension::ContextLimit {
                    estimated_tokens: Some(9),
                    ..
                })
            ));
        } else {
            assert!(
                matches!(outcome, LLMContextOutcome::Done { .. }),
                "{outcome:?}"
            );
            assert_eq!(llm.calls(), 1);
        }
    }
}

#[tokio::test]
async fn hard_limit_counts_the_completion_reserve() {
    for (reserve, expect_yield) in [(11, false), (12, true)] {
        let llm = Llm::new(vec![AiResponse::text("done")]);
        let mut req = request();
        req.budget.context_window_tokens = Some(20);
        req.model_policy.max_completion_tokens = Some(reserve);
        let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
        let outcome = ctx.run().await;
        if expect_yield {
            assert!(
                matches!(
                    outcome,
                    LLMContextOutcome::ContextLimitReached {
                        which: ContextLimitKind::HardLimit,
                        ..
                    }
                ),
                "{outcome:?}"
            );
            assert_eq!(llm.calls(), 0);
        } else {
            assert!(
                matches!(outcome, LLMContextOutcome::Done { .. }),
                "{outcome:?}"
            );
        }
    }
}

#[tokio::test]
async fn invalid_threshold_ends_the_run_before_any_inference() {
    for (t, window) in [
        (Some(ContextThreshold::Ratio { value: 0.5 }), None),
        (Some(ContextThreshold::Ratio { value: 1.5 }), Some(100)),
        (Some(ContextThreshold::AbsoluteTokens { value: 0 }), None),
        (None, Some(0)),
    ] {
        let llm = Llm::new(vec![AiResponse::text("done")]);
        let mut req = request();
        req.budget.context_yield_threshold = t;
        req.budget.context_window_tokens = window;
        let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
        let outcome = ctx.run().await;
        let LLMContextOutcome::Error { error, .. } = outcome else {
            panic!("expected Error, got {outcome:?}");
        };
        assert!(
            matches!(error, LLMComputeError::Internal(ref m) if m.contains("invalid context limits"))
        );
        assert_eq!(llm.calls(), 0);
    }
    // Ratio with a window works.
    let llm = Llm::new(vec![AiResponse::text("done")]);
    let mut req = request();
    req.budget.context_window_tokens = Some(100);
    req.budget.context_yield_threshold = Some(ContextThreshold::Ratio { value: 0.09 });
    let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
    assert!(matches!(
        ctx.run().await,
        LLMContextOutcome::ContextLimitReached {
            which: ContextLimitKind::ApproachingWindow,
            ..
        }
    ));
}

#[tokio::test]
async fn tool_results_over_the_threshold_yield_before_the_next_inference_and_rewrite_continues() {
    let llm = Llm::new(vec![
        with_usage(tools_response(vec![call("big", "c1")]), 10),
        with_usage(AiResponse::text("done"), 5),
    ]);
    let tools = Tools::new();
    let mut req = request();
    req.budget.context_yield_threshold = threshold(200);
    let mut ctx = LLMContext::new(req, deps(llm.clone(), tools.clone()));

    let outcome = ctx.run().await;
    let LLMContextOutcome::ContextLimitReached {
        which,
        usage,
        accumulated,
        snapshot,
        ..
    } = outcome
    else {
        panic!("expected ContextLimitReached, got {outcome:?}");
    };
    assert_eq!(which, ContextLimitKind::ApproachingWindow);
    assert_eq!(usage.total_tokens, Some(10));
    assert_eq!(llm.calls(), 1, "the over-limit request is not sent");
    assert_eq!(
        tool_result_ids(&accumulated),
        vec!["c1"],
        "the tool result is in the snapshot"
    );
    let rounds_left = snapshot.state.rounds_left;
    assert_eq!(rounds_left, 5);

    // The scheduler rewrites; the run continues without resetting budgets.
    let snapshot = round_trip(&snapshot);
    let rewritten = vec![
        AiMessage::text(AiRole::User, "hello"),
        AiMessage::text(AiRole::User, "summary: a big result was read"),
    ];
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::RewrittenHistory {
            history: rewritten.clone(),
        },
        deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    assert_eq!(
        ctx.snapshot().request.input,
        rewritten,
        "the rewrite is the new base"
    );
    let outcome = ctx.run().await;
    let LLMContextOutcome::Done { usage, .. } = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    assert_eq!(
        usage.total_tokens,
        Some(15),
        "usage is not reset by the rewrite"
    );
    assert_eq!(llm.seen()[1], rewritten);
    assert_eq!(ctx.snapshot().state.rounds_left, rounds_left);
    assert_eq!(tools.calls(), vec!["c1"]);
}

#[tokio::test]
async fn a_rewrite_that_still_does_not_fit_yields_again_without_inference() {
    let llm = Llm::new(vec![AiResponse::text("never")]);
    let mut req = request();
    req.budget.context_yield_threshold = threshold(HELLO_TOKENS);
    let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
    let LLMContextOutcome::ContextLimitReached { snapshot, .. } = ctx.run().await else {
        panic!("expected ContextLimitReached");
    };
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::RewrittenHistory {
            history: vec![AiMessage::text(AiRole::User, "hello, still too long")],
        },
        deps(llm.clone(), Tools::new()),
    )
    .unwrap();
    assert!(matches!(
        ctx.run().await,
        LLMContextOutcome::ContextLimitReached { .. }
    ));
    assert_eq!(llm.calls(), 0);
}

struct InjectOnce {
    done: Mutex<bool>,
    fail: bool,
}

#[async_trait]
impl CheckpointHook for InjectOnce {
    async fn before_inference(
        &self,
        _snapshot: &LLMContextSnapshot,
    ) -> Result<Option<Injection>, String> {
        if self.fail {
            return Err("disk full".into());
        }
        let mut done = self.done.lock().unwrap();
        if *done {
            return Ok(None);
        }
        *done = true;
        Ok(Some(Injection {
            messages: vec![AiMessage::text(AiRole::User, "y".repeat(100))],
            host: None,
        }))
    }
}

#[tokio::test]
async fn an_injection_over_the_threshold_is_in_the_snapshot_and_nothing_is_sent() {
    let llm = Llm::new(vec![AiResponse::text("never")]);
    let mut req = request();
    req.budget.context_yield_threshold = threshold(50);
    let hook = Arc::new(InjectOnce {
        done: Mutex::new(false),
        fail: false,
    });
    let mut ctx = LLMContext::new(
        req,
        deps(llm.clone(), Tools::new()).with_checkpoint_hook(hook),
    );
    let LLMContextOutcome::ContextLimitReached { snapshot, .. } = ctx.run().await else {
        panic!("expected ContextLimitReached");
    };
    assert_eq!(snapshot.state.accumulated.len(), 2, "the injection is kept");
    assert_eq!(llm.calls(), 0);
}

#[tokio::test]
async fn checkpoint_failure_and_interrupt_are_not_masked_by_the_limit() {
    let mut req = request();
    req.budget.context_yield_threshold = threshold(1);
    let failing = Arc::new(InjectOnce {
        done: Mutex::new(true),
        fail: true,
    });
    let llm = Llm::new(vec![]);
    let mut ctx = LLMContext::new(
        req.clone(),
        deps(llm.clone(), Tools::new()).with_checkpoint_hook(failing),
    );
    let outcome = ctx.run().await;
    assert!(
        matches!(
            outcome,
            LLMContextOutcome::Error {
                error: LLMComputeError::Checkpoint {
                    stage: CheckpointStage::BeforeInference,
                    ..
                },
                ..
            }
        ),
        "{outcome:?}"
    );
    assert!(ctx.suspension().is_none());

    let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
    ctx.interrupt_handle().interrupt("stop");
    let outcome = ctx.run().await;
    assert!(
        matches!(outcome, LLMContextOutcome::Interrupted { .. }),
        "{outcome:?}"
    );
    assert_eq!(llm.calls(), 0);
}

#[tokio::test]
async fn provider_refusal_is_a_yield_other_provider_errors_are_not() {
    let llm = Llm::with_results(vec![Err(LLMComputeError::provider(
        ProviderFailure::ContextLimit,
        "context_length_exceeded",
    ))]);
    let mut ctx = LLMContext::new(request(), deps(llm.clone(), Tools::new()));
    let outcome = ctx.run().await;
    let LLMContextOutcome::ContextLimitReached {
        which, snapshot, ..
    } = outcome
    else {
        panic!("expected ContextLimitReached, got {outcome:?}");
    };
    assert_eq!(which, ContextLimitKind::ProviderRefused);
    assert_eq!(snapshot.state.accumulated.len(), 1, "pre-inference state");

    let llm = Llm::with_results(vec![Err(LLMComputeError::provider(
        ProviderFailure::Permanent,
        "400 bad request: context too long",
    ))]);
    let mut ctx = LLMContext::new(request(), deps(llm, Tools::new()));
    assert!(matches!(
        ctx.run().await,
        LLMContextOutcome::Error {
            error: LLMComputeError::Provider {
                failure: ProviderFailure::Permanent,
                ..
            },
            ..
        }
    ));
}

#[tokio::test]
async fn rewrite_strips_thinking_and_rejects_broken_pairing() {
    let mut req = request();
    req.budget.context_yield_threshold = threshold(1);
    let llm = Llm::new(vec![]);
    let mut ctx = LLMContext::new(req, deps(llm.clone(), Tools::new()));
    let LLMContextOutcome::ContextLimitReached { snapshot, .. } = ctx.run().await else {
        panic!("expected ContextLimitReached");
    };

    let orphan = vec![
        AiMessage::text(AiRole::User, "hello"),
        AiMessage::new(
            AiRole::Tool,
            vec![AiContent::ToolResult {
                call_id: "gone".into(),
                content: vec![],
                is_error: false,
            }],
        ),
    ];
    let msg = corrupted(LLMContext::resume(
        snapshot.clone(),
        ResumeFill::RewrittenHistory { history: orphan },
        deps(llm.clone(), Tools::new()),
    ));
    assert!(msg.contains("without its tool call"), "{msg}");

    let with_thinking = vec![
        AiMessage::text(AiRole::User, "hi"),
        AiMessage::new(
            AiRole::Assistant,
            vec![
                AiContent::Thinking {
                    summary: None,
                    text: Some("hmm".into()),
                    provider_metadata: Some(json!({"signature": "sig"})),
                },
                AiContent::Text {
                    text: "answer".into(),
                },
            ],
        ),
        AiMessage::new(
            AiRole::Assistant,
            vec![AiContent::Thinking {
                summary: None,
                text: Some("only thinking".into()),
                provider_metadata: None,
            }],
        ),
    ];
    let ctx = LLMContext::resume(
        snapshot,
        ResumeFill::RewrittenHistory {
            history: with_thinking,
        },
        deps(llm, Tools::new()),
    )
    .unwrap();
    let acc = ctx.snapshot().state.accumulated;
    assert_eq!(acc.len(), 2, "a message holding only thinking is dropped");
    assert!(acc
        .iter()
        .flat_map(|m| m.content.iter())
        .all(|c| !matches!(c, AiContent::Thinking { .. })));
}

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
        snapshot.state.rounds_left, 6,
        "the round is counted when the batch completes"
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
    assert_eq!(state.rounds_left, 5);
    assert_eq!(
        state.consecutive_errors, 1,
        "c3's failure counts once for the round"
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
    assert_eq!(state.rounds_left, 5);
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
    req.tool_policy.max_rounds = 2;
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

// ===================================================================
// Behavior mode
// ===================================================================

fn xml(body: &str) -> AiResponse {
    AiResponse::text(format!("<response>{body}</response>"))
}

#[tokio::test]
async fn behavior_action_pending_keeps_the_step_and_resumes_after_it() {
    let llm = Llm::new(vec![
        xml("<thinking>go</thinking><actions><exec_bash>echo a</exec_bash><exec_bash>defer b</exec_bash><exec_bash>echo c</exec_bash></actions>"),
        xml("<thinking>saw all</thinking><actions><exec_bash>echo d</exec_bash></actions>"),
        xml("<next_behavior>END</next_behavior>"),
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
        snapshot.state.rounds_left, 5,
        "the step's round is already counted"
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
async fn behavior_inner_native_pending_resumes_the_turn_without_rerunning_tools() {
    let llm = Llm::new(vec![
        xml("<thinking>step 0</thinking><actions><exec_bash>echo a</exec_bash></actions>"),
        tools_response(vec![call("echo", "n1"), call("defer", "n2")]),
        xml("<next_behavior>END</next_behavior>"),
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
        "the inner turn is kept as the tail"
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
            "<thinking>step {n} {}</thinking><actions><exec_bash>echo {n}</exec_bash></actions>",
            "z".repeat(pad)
        ))
    };
    let script = || {
        vec![
            step(0, 400),
            step(1, 0),
            xml("<thinking>after rewrite</thinking><actions><exec_bash>echo 2</exec_bash></actions>"),
            xml("<next_behavior>END</next_behavior>"),
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
async fn an_interrupted_behavior_turn_keeps_the_native_tools_it_ran() {
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
    assert_eq!(tool_result_ids(tail), vec!["n1"], "the turn so far is kept");

    let resumed_llm = Llm::new(vec![xml("<next_behavior>END</next_behavior>")]);
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
