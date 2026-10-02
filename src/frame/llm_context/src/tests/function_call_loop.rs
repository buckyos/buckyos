//! The function call loop: done / tool batches / hooks / interrupt / resume.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{
    AiContent,
    AiCost,
    AiMessage,
    AiResponse,
    AiRole,
    AiToolCall,
    AiUsage,
    ResourceRef,
};
use serde_json::json;

use crate::deps::{InferenceHook, LLMContextDeps, LlmClient, LlmInferenceRequest};
use crate::error::{LLMComputeError};
use crate::outcome::{ContextOutput, LLMContextOutcome, ResumeFill};
use crate::state::{LLMContextSnapshot, LLMContextState};
use crate::{LLMContext};

use super::mocks::*;

#[tokio::test]
async fn done_without_tool_calls() {
    let llm = Arc::new(ScriptedLlm::new(vec![AiResponse {
        message: AiMessage::text(AiRole::Assistant, "hi there"),
        usage: Some(AiUsage {
            input_tokens: Some(5),
            output_tokens: Some(3),
            total_tokens: Some(8),
            cache_read_input_tokens: Some(2),
            cache_write_input_tokens: Some(1),
            cache_write_1h_input_tokens: None,
            reasoning_tokens: Some(1),
            image_units: Some(2),
            audio_seconds: Some(1.5),
            video_seconds: Some(2.5),
            request_units: Some(1),
            characters: None,
            audio_input_tokens: None,
            image_input_tokens: None,
            audio_output_tokens: None,
            image_output_tokens: None,
            reported_cost: None,
            cost: Some(AiCost {
                amount: 0.25,
                currency: "USD".to_string(),
            }),
        }),
        ..Default::default()
    }]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);

    match ctx.run().await {
        LLMContextOutcome::Done { output, usage, .. } => {
            match output {
                ContextOutput::Text { content } => assert_eq!(content, "hi there"),
                _ => panic!("expected text output"),
            }
            assert_eq!(usage.total_tokens, Some(8));
            assert_eq!(usage.cache_read_input_tokens, Some(2));
            assert_eq!(usage.cache_write_input_tokens, Some(1));
            assert_eq!(usage.reasoning_tokens, Some(1));
            assert_eq!(usage.image_units, Some(2));
            assert_eq!(usage.audio_seconds, Some(1.5));
            assert_eq!(usage.video_seconds, Some(2.5));
            assert_eq!(usage.request_units, Some(1));
            assert_eq!(
                usage.cost,
                Some(AiCost {
                    amount: 0.25,
                    currency: "USD".to_string(),
                })
            );
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[tokio::test]
async fn one_tool_batch_then_done() {
    let mut args: HashMap<String, serde_json::Value> = HashMap::new();
    args.insert("msg".into(), json!("ping"));
    let call = AiToolCall {
        name: "echo".into(),
        args,
        call_id: "c-1".into(),
    };
    let llm = Arc::new(ScriptedLlm::new(vec![
        tool_response(Some("calling echo"), vec![call]),
        text_response("done after echo"),
    ]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::Done { output, trace, .. } = outcome else {
        panic!("expected Done");
    };
    match output {
        ContextOutput::Text { content } => assert_eq!(content, "done after echo"),
        _ => panic!("expected text"),
    }
    assert_eq!(trace.tool_trace.len(), 1);
    assert_eq!(trace.tool_trace[0].tool_name, "echo");
    assert!(trace.tool_trace[0].ok());
}

#[tokio::test]
async fn done_accumulates_full_assistant_message_with_non_text_blocks() {
    let image = ResourceRef::base64("image/png".to_string(), "AA==".to_string());
    let response = AiResponse::new(AiMessage::new(
        AiRole::Assistant,
        vec![
            AiContent::text("before"),
            AiContent::Image {
                source: image.clone(),
            },
            AiContent::text("after"),
        ],
    ));
    let llm = Arc::new(ScriptedLlm::new(vec![response.clone()]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);

    let LLMContextOutcome::Done { output, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(
        output,
        ContextOutput::Text {
            content: "before\nafter".to_string()
        }
    );
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.accumulated.last(), Some(&response.message));
}

#[test]
fn ai_response_preserves_multimodal_block_order() {
    let mut args = HashMap::new();
    args.insert("q".to_string(), json!("value"));
    let message = AiMessage::new(
        AiRole::Assistant,
        vec![
            AiContent::text("first"),
            AiContent::Image {
                source: ResourceRef::url("https://example.test/image.png".to_string(), None),
            },
            AiContent::text("second"),
            AiContent::ToolUse {
                call_id: "call-1".to_string(),
                name: "lookup".to_string(),
                args,
            },
        ],
    );
    let response = AiResponse::new(message);
    assert!(matches!(
        response.message.content[0],
        AiContent::Text { .. }
    ));
    assert!(matches!(
        response.message.content[1],
        AiContent::Image { .. }
    ));
    assert!(matches!(
        response.message.content[2],
        AiContent::Text { .. }
    ));
    assert!(matches!(
        response.message.content[3],
        AiContent::ToolUse { .. }
    ));
    assert_eq!(response.message.text_content(), "first\nsecond");
    assert_eq!(response.message.tool_calls().len(), 1);
}

#[tokio::test]
async fn invalid_response_role_is_rejected_before_accumulation() {
    let response = AiResponse::new(AiMessage::text(AiRole::User, "not an assistant response"));
    let llm = Arc::new(ScriptedLlm::new(vec![response]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::Error { error, .. } = outcome else {
        panic!("expected Error");
    };
    assert!(matches!(error, LLMComputeError::Internal(_)));
    assert_eq!(ctx.snapshot().state.accumulated, base_request().input);
}

#[tokio::test]
async fn inference_hook_fires_before_each_inference() {
    let llm = Arc::new(ScriptedLlm::new(vec![text_response("hello back")]));
    let count = Arc::new(Mutex::new(0));
    let hook: Arc<dyn InferenceHook> = Arc::new(CountingHook {
        count: count.clone(),
    });
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools)).with_inference_hook(hook);
    let mut ctx = LLMContext::new(base_request(), deps);

    let _ = ctx.run().await;
    // exactly one inference happened ⇒ hook fired exactly once.
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test]
async fn resume_from_mid_run_continues_loop() {
    // Run once to get a snapshot at the outcome boundary.
    let llm = Arc::new(ScriptedLlm::new(vec![text_response("first reply")]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);
    let _ = ctx.run().await;
    let snapshot = ctx.snapshot();

    // Resume the mid-run snapshot. A second LLM call must succeed because
    // the snapshot is *not* in a suspended state.
    let llm2 = Arc::new(ScriptedLlm::new(vec![text_response("after resume")]));
    let deps2 = LLMContextDeps::new(llm2, Arc::new(EchoTools));
    let mut ctx2 = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps2)
        .expect("resume should succeed for non-suspended snapshot");
    match ctx2.run().await {
        LLMContextOutcome::Done { output, .. } => match output {
            ContextOutput::Text { content } => assert_eq!(content, "after resume"),
            _ => panic!("expected text"),
        },
        other => panic!("unexpected outcome: {other:?}"),
    }
}

/// A LLM client that parks on the abort token forever (or until aborted).
/// Lets us race the interrupt handle against an in-flight inference.
struct BlockingLlm;

#[async_trait]
impl LlmClient for BlockingLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        // Wait until aborted, then surface as `Cancelled`. Mirrors what a
        // well-behaved provider adapter would do when it observes the token.
        req.abort.cancelled().await;
        Err(LLMComputeError::Cancelled)
    }
}

#[tokio::test]
async fn interrupt_yields_interrupted_outcome_with_pre_inference_snapshot() {
    let deps = LLMContextDeps::new(Arc::new(BlockingLlm), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);
    let handle = ctx.interrupt_handle();

    // Capture the snapshot we *expect* to be returned in `Interrupted.snapshot`
    // (the accumulated history before the first inference fires).
    let snap_before = ctx.snapshot();

    let runner = tokio::spawn(async move {
        let outcome = ctx.run().await;
        outcome
    });

    // Give the runner a chance to enter `infer()` so we exercise the
    // mid-inference preempt path rather than the early short-circuit.
    tokio::task::yield_now().await;
    assert!(handle.interrupt("user_cancel"));
    // Second interrupt is a no-op.
    assert!(!handle.interrupt("ignored"));

    let outcome = runner.await.expect("runner join");
    // `Interrupted` is a suspended outcome — must not be classified terminal.
    assert!(!outcome.is_terminal());
    let LLMContextOutcome::Interrupted {
        reason,
        snapshot,
        abort,
        ..
    } = outcome
    else {
        panic!("expected Interrupted");
    };
    assert_eq!(reason, "user_cancel");
    assert_eq!(abort.reason, "user_cancel");
    // Snapshot must match pre-inference state — no half assistant messages.
    assert_eq!(
        snapshot.state.accumulated.len(),
        snap_before.state.accumulated.len()
    );
    assert!(snapshot.state.suspended.is_none());
    assert_eq!(snapshot.state.consecutive_errors, 0);
}

#[tokio::test]
async fn interrupt_before_run_short_circuits_without_inference() {
    let deps = LLMContextDeps::new(Arc::new(BlockingLlm), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);
    let handle = ctx.interrupt_handle();
    handle.interrupt("preempted_before_start");

    let outcome = ctx.run().await;
    let LLMContextOutcome::Interrupted { reason, .. } = outcome else {
        panic!("expected Interrupted, got {outcome:?}");
    };
    assert_eq!(reason, "preempted_before_start");
}

#[tokio::test]
async fn resume_from_mid_run_after_interrupt_replays_inference() {
    let blocking = Arc::new(BlockingLlm);
    let deps = LLMContextDeps::new(blocking.clone(), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);
    let handle = ctx.interrupt_handle();
    handle.interrupt("scheduler_preempt");
    let outcome = ctx.run().await;
    let LLMContextOutcome::Interrupted { snapshot, .. } = outcome else {
        panic!("expected Interrupted");
    };

    // Resume with a real LLM that returns a regular response — the run
    // should make forward progress because `Interrupted.snapshot` carries
    // pre-inference state (empty pending_tool_calls / no half output).
    let llm = Arc::new(ScriptedLlm::new(vec![text_response("post-resume reply")]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps)
        .expect("ResumeFromMidRun after Interrupted is the documented path");
    match ctx.run().await {
        LLMContextOutcome::Done { output, .. } => match output {
            ContextOutput::Text { content } => assert_eq!(content, "post-resume reply"),
            _ => panic!("expected text"),
        },
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[tokio::test]
async fn tool_iterations_budget_exhausted() {
    let mut args: HashMap<String, serde_json::Value> = HashMap::new();
    args.insert("msg".into(), json!("ping"));
    let make_call = |id: &str| AiToolCall {
        name: "echo".into(),
        args: args.clone(),
        call_id: id.into(),
    };

    // 3 inferences, each demands another tool call. max_tool_iterations = 2 ⇒
    // after 2 tool iterations the loop bails out with BudgetExhausted.
    let llm = Arc::new(ScriptedLlm::new(vec![
        tool_response(None, vec![make_call("c-1")]),
        tool_response(None, vec![make_call("c-2")]),
        tool_response(None, vec![make_call("c-3")]),
    ]));
    let mut req = base_request();
    req.tool_policy.max_tool_iterations = 2;
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(req, deps);

    match ctx.run().await {
        LLMContextOutcome::BudgetExhausted { which, .. } => {
            assert!(matches!(which, crate::outcome::BudgetKind::ToolIterations));
        }
        other => panic!("expected BudgetExhausted, got {other:?}"),
    }
}

#[test]
fn resume_rejects_other_snapshot_versions() {
    for version in [
        0,
        crate::state::SNAPSHOT_FORMAT_VERSION - 1,
        crate::state::SNAPSHOT_FORMAT_VERSION + 1,
    ] {
        let req = base_request();
        let mut state = LLMContextState::from_request(&req, 0);
        state.snapshot_version = version;
        let snapshot = LLMContextSnapshot {
            request: req,
            state,
        };
        let deps = LLMContextDeps::new(Arc::new(ScriptedLlm::new(vec![])), Arc::new(EchoTools));
        let err = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps)
            .err()
            .expect("an unsupported version is refused");
        assert!(
            matches!(err, LLMComputeError::SnapshotCorrupted(_)),
            "version {version}: {err:?}"
        );
    }
}
