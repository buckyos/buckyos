//! `ContextLimitReached`: thresholds, the rewrite, provider refusals.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use serde_json::json;

use crate::deps::{CheckpointHook, Injection};
use crate::error::{CheckpointStage, LLMComputeError, ProviderFailure};
use crate::outcome::{ContextLimitKind, LLMContextOutcome, ResumeFill};
use crate::request::{ContextThreshold};
use crate::state::{LLMContextSnapshot, Suspension};
use crate::{LLMContext};

use super::mocks::*;

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
    let tool_iterations_left = snapshot.state.tool_iterations_left;
    assert_eq!(tool_iterations_left, 5);

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
    assert_eq!(ctx.snapshot().state.tool_iterations_left, tool_iterations_left);
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
