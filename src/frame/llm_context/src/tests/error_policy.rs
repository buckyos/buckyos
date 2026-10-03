//! Error handling: provider failures, self-correction, consecutive errors,
//! policy rejections, dispatch failures, checkpoint failures, corrupted snapshots.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiToolCall, AiUsage};
use serde_json::json;

use crate::deps::{
    ToolCallCtx,InferenceHook, LLMContextDeps, ToolDispatchError, ToolManager};
use crate::error::{CheckpointStage, LLMComputeError, ProviderFailure};
use crate::observation::{Observation, ToolExecStatus};
use crate::outcome::{ContextOutput, LLMContextOutcome, ResumeFill};
use crate::request::{LLMContextRequest, OutputSpec};
use crate::state::{LLMContextSnapshot, LLMContextState};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

#[tokio::test]
async fn provider_failure_ends_run_without_feedback_or_second_inference() {
    let llm = Arc::new(RecoverOnceLlm::new(text_response("never reached")));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);

    let LLMContextOutcome::Error { error, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert_eq!(
        error,
        LLMComputeError::provider(ProviderFailure::Transient, "temporary aicc failure")
    );
    assert_eq!(llm.seen().len(), 1);
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.accumulated, base_request().input);
    assert_eq!(snapshot.state.consecutive_errors, 0);
}

#[tokio::test]
async fn permanent_provider_error_and_adapter_internal_are_terminal() {
    for err in [
        LLMComputeError::provider(ProviderFailure::Permanent, "invalid api key"),
        LLMComputeError::Internal("adapter bug".into()),
        LLMComputeError::Cancelled,
        LLMComputeError::Timeout,
    ] {
        let llm = Arc::new(ScriptedRecordingLlm::with_results(vec![
            Err(err.clone()),
            Ok(text_response("never reached")),
        ]));
        let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools));
        let mut ctx = LLMContext::new(base_request(), deps);
        let outcome = ctx.run().await;
        let LLMContextOutcome::Error { error, .. } = outcome else {
            panic!("expected Error for {err:?}, got {outcome:?}");
        };
        assert_eq!(error, err);
        assert_eq!(llm.seen().len(), 1);
        assert_eq!(ctx.snapshot().state.consecutive_errors, 0);
    }
}

#[tokio::test]
async fn strict_json_parse_error_is_fed_back_then_corrected() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        AiResponse {
            message: AiMessage::text(AiRole::Assistant, "not json"),
            usage: Some(AiUsage {
                input_tokens: Some(1),
                output_tokens: Some(1),
                total_tokens: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        },
        AiResponse {
            message: AiMessage::text(AiRole::Assistant, r#"{"answer": 42}"#),
            usage: Some(AiUsage {
                input_tokens: Some(1),
                output_tokens: Some(1),
                total_tokens: Some(3),
                ..Default::default()
            }),
            ..Default::default()
        },
    ]));
    let mut req = base_request();
    req.output = OutputSpec::Json {
        schema: None,
        strict: true,
    };
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(req, deps);

    let LLMContextOutcome::Done { output, usage, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(
        output,
        ContextOutput::Json {
            content: json!({"answer": 42})
        }
    );
    assert_eq!(usage.total_tokens, Some(5));
    let seen = llm.seen();
    assert_eq!(seen.len(), 2);
    let retry = &seen[1];
    assert_eq!(retry[retry.len() - 2].role, AiRole::Assistant);
    assert_eq!(retry[retry.len() - 2].text_content(), "not json");
    assert_eq!(retry[retry.len() - 1].role, AiRole::User);
    assert!(retry[retry.len() - 1]
        .text_content()
        .contains("output parse failed"));
}

#[tokio::test]
async fn persistent_strict_json_errors_terminate_on_fourth_failure() {
    let llm = Arc::new(ScriptedRecordingLlm::new(
        (0..6).map(|_| text_response("still not json")).collect(),
    ));
    let mut req = base_request();
    req.output = OutputSpec::Json {
        schema: None,
        strict: true,
    };
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(req, deps);

    let LLMContextOutcome::Error { error, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert!(matches!(error, LLMComputeError::OutputParse(_)));
    assert_eq!(llm.seen().len(), 4, "3 feedbacks, the 4th failure terminates");
    assert_eq!(ctx.snapshot().state.consecutive_errors, 4);
}

#[tokio::test]
async fn behavior_parse_errors_terminate_on_fourth_failure() {
    let llm = Arc::new(ScriptedRecordingLlm::new(
        (0..6).map(|_| text_response("")).collect(),
    ));
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let LLMContextOutcome::Error { error, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert!(matches!(error, LLMComputeError::OutputParse(_)));
    assert_eq!(llm.seen().len(), 4);
}

#[tokio::test]
async fn multiple_tool_errors_in_one_batch_count_as_one_failure() {
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(
            None,
            vec![call("fail:b", "c-1"), call("fail:b", "c-2"), call("fail:b", "c-3")],
        ),
        text_response("gave up on tools"),
    ]));
    let mut req = base_request();
    req.error_policy.max_consecutive_errors = 2;
    let deps = LLMContextDeps::new(llm.clone(), tools.clone());
    let mut ctx = LLMContext::new(req, deps);

    let LLMContextOutcome::Done { trace, .. } = ctx.run().await else {
        panic!("expected Done: three failed calls are one failed iteration");
    };
    assert_eq!(tools.calls().len(), 3, "traditional loop runs the whole batch");
    assert_eq!(trace.tool_trace.len(), 3);
    assert!(trace
        .tool_trace
        .iter()
        .all(|r| r.status == ToolExecStatus::Failed));
    assert_eq!(ctx.snapshot().state.consecutive_errors, 1);
}

#[tokio::test]
async fn successful_inference_does_not_reset_consecutive_errors() {
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("fail:b", "c-1")]),
        tool_response(None, vec![call("fail:b", "c-2")]),
        tool_response(None, vec![call("fail:b", "c-3")]),
        tool_response(None, vec![call("fail:b", "c-4")]),
        text_response("never reached"),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), tools);
    let mut ctx = LLMContext::new(base_request(), deps);

    let LLMContextOutcome::Error { error, trace, .. } = ctx.run().await else {
        panic!("expected Error after the 4th consecutive failed iteration");
    };
    assert!(matches!(error, LLMComputeError::ToolFailed { .. }));
    assert_eq!(llm.seen().len(), 4);
    assert_eq!(trace.tool_trace.len(), 4);
}

#[tokio::test]
async fn clean_tool_batch_resets_consecutive_errors() {
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("fail:b", "c-1")]),
        tool_response(None, vec![call("a", "c-2")]),
        text_response("done"),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), tools);
    let mut ctx = LLMContext::new(base_request(), deps);
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(ctx.snapshot().state.consecutive_errors, 0);
}

#[tokio::test]
async fn policy_rejection_keeps_transcript_paired_and_is_correctable() {
    struct RejectOnce {
        rejected: Mutex<bool>,
    }
    #[async_trait]
    impl crate::deps::PolicyEngine for RejectOnce {
        async fn gate_tool_calls(
            &self,
            _request: &LLMContextRequest,
            calls: Vec<AiToolCall>,
        ) -> Result<Vec<AiToolCall>, String> {
            let mut rejected = self.rejected.lock().unwrap();
            if !*rejected {
                *rejected = true;
                return Err("tool `a` needs approval".to_string());
            }
            Ok(calls)
        }
    }
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("a", "c-1"), call("c", "c-2")]),
        tool_response(None, vec![call("c", "c-3")]),
        text_response("done"),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), tools.clone()).with_policy(Arc::new(RejectOnce {
        rejected: Mutex::new(false),
    }));
    let mut ctx = LLMContext::new(base_request(), deps);

    let LLMContextOutcome::Done { trace, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(tools.calls(), vec!["c".to_string()]);
    let seen = llm.seen();
    assert_eq!(seen.len(), 3);
    let second = &seen[1];
    let tool_results: Vec<&AiMessage> = second.iter().filter(|m| m.role == AiRole::Tool).collect();
    assert_eq!(tool_results.len(), 2, "every rejected call is answered");
    assert!(tool_results
        .iter()
        .all(|m| tool_result_text(m).contains("policy rejected")));
    assert_eq!(
        statuses(&trace),
        vec![
            ("a".to_string(), ToolExecStatus::NotExecuted),
            ("c".to_string(), ToolExecStatus::NotExecuted),
            ("c".to_string(), ToolExecStatus::Succeeded),
        ]
    );
}

#[tokio::test]
async fn tool_dispatch_failure_stops_batch_and_keeps_partial_results() {
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(
            None,
            vec![call("a", "c-1"), call("dispatch:b", "c-2"), call("c", "c-3")],
        ),
        text_response("never reached in this run"),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), tools.clone());
    let mut ctx = LLMContext::new(base_request(), deps);

    let LLMContextOutcome::Error { error, trace, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert_eq!(
        error,
        LLMComputeError::ToolRuntime {
            tool: "dispatch:b".into(),
            call_id: "c-2".into(),
            message: "sandbox connection lost".into(),
            effect_unknown: true,
        }
    );
    assert!(!error.llm_correctable());
    assert_eq!(tools.calls(), vec!["a".to_string(), "dispatch:b".to_string()]);
    assert_eq!(
        statuses(&trace),
        vec![
            ("a".to_string(), ToolExecStatus::Succeeded),
            ("dispatch:b".to_string(), ToolExecStatus::Unknown),
            ("c".to_string(), ToolExecStatus::NotExecuted),
        ]
    );
    assert_eq!(llm.seen().len(), 1);

    // The transcript stays paired: A's result, B unresolved, C not executed.
    let snapshot = ctx.snapshot();
    let tail: Vec<&AiMessage> = snapshot
        .state
        .accumulated
        .iter()
        .filter(|m| m.role == AiRole::Tool)
        .collect();
    assert_eq!(tail.len(), 3);
    assert!(tool_result_text(tail[1]).contains("result unknown"));
    assert!(tool_result_text(tail[2]).contains("not executed"));
    assert_eq!(snapshot.state.consecutive_errors, 0);

    // The runtime may resume; nothing already executed is replayed.
    let llm2 = Arc::new(ScriptedRecordingLlm::new(vec![text_response("recovered")]));
    let deps2 = LLMContextDeps::new(llm2.clone(), tools.clone());
    let mut resumed = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps2)
        .expect("paired transcript is resumable");
    assert!(matches!(resumed.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(tools.calls().len(), 2, "A and B are not re-executed");
}

#[tokio::test]
async fn behavior_actions_stop_after_first_business_error_and_record_skipped() {
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(
            r#"<response>
<thinking>three actions</thinking>
<actions><shell>echo a</shell><shell>echo b</shell><shell>echo c</shell></actions>
</response>"#,
        ),
        text_response("<response><thinking>saw it</thinking><next_behavior>END</next_behavior></response>"),
    ]));
    struct FailSecond {
        tools: Arc<ScriptedTools>,
    }
    #[async_trait]
    impl ToolManager for FailSecond {
        async fn call_tool(&self, mut c: AiToolCall, ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError> {
            if c.call_id == "2" {
                c.name = "fail:shell".into();
            }
            self.tools.call_tool(c, ctx).await
        }
    }
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(
        llm.clone(),
        Arc::new(FailSecond {
            tools: tools.clone(),
        }),
    )
    .with_result_parser(Arc::new(XmlBehaviorParser::new()))
    .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let LLMContextOutcome::Done { trace, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(tools.calls().len(), 2, "action C is not dispatched");
    assert_eq!(
        statuses(&trace)
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>(),
        vec![
            ToolExecStatus::Succeeded,
            ToolExecStatus::Failed,
            ToolExecStatus::NotExecuted
        ]
    );
    let snapshot = ctx.snapshot();
    let step = &snapshot.state.steps[0];
    assert_eq!(step.actions.len(), 3);
    assert!(matches!(step.action_results[0], Observation::Success { .. }));
    assert!(matches!(step.action_results[1], Observation::Error { .. }));
    assert!(matches!(
        step.action_results[2],
        Observation::Unresolved {
            effect_unknown: false,
            ..
        }
    ));
    let seen = llm.seen();
    assert!(seen[1]
        .iter()
        .any(|m| m.role == AiRole::User && m.text_content().contains("Not executed")));
}

#[tokio::test]
async fn behavior_action_dispatch_failure_ends_run_with_sedimented_partial_step() {
    let tools = Arc::new(ScriptedTools::new());
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(
            r#"<response>
<thinking>two actions</thinking>
<actions><shell>echo a</shell><shell>echo b</shell></actions>
</response>"#,
        ),
        text_response("never reached"),
    ]));
    struct DispatchFailFirst {
        tools: Arc<ScriptedTools>,
    }
    #[async_trait]
    impl ToolManager for DispatchFailFirst {
        async fn call_tool(&self, mut c: AiToolCall, ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError> {
            if c.call_id == "1" {
                c.name = "dispatch:shell".into();
            }
            self.tools.call_tool(c, ctx).await
        }
    }
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(
        llm.clone(),
        Arc::new(DispatchFailFirst {
            tools: tools.clone(),
        }),
    )
    .with_result_parser(Arc::new(XmlBehaviorParser::new()))
    .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let LLMContextOutcome::Error { error, trace, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert!(matches!(
        error,
        LLMComputeError::ToolRuntime {
            effect_unknown: true,
            ..
        }
    ));
    assert_eq!(tools.calls().len(), 1);
    assert_eq!(llm.seen().len(), 1);
    assert_eq!(
        statuses(&trace)
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>(),
        vec![ToolExecStatus::Unknown, ToolExecStatus::NotExecuted]
    );
    let snapshot = ctx.snapshot();
    let step = snapshot.state.last_step.as_ref().expect("partial step kept");
    assert_eq!(step.action_results.len(), 2);
    assert!(matches!(
        step.action_results[0],
        Observation::Unresolved {
            effect_unknown: true,
            ..
        }
    ));
    assert_eq!(snapshot.state.consecutive_errors, 0);
}

#[tokio::test]
async fn inference_hook_failure_blocks_inference_and_keeps_snapshot_resumable() {
    struct FailingHook;
    impl InferenceHook for FailingHook {
        fn before_inference(&self, _snapshot: &LLMContextSnapshot) -> Result<(), String> {
            Err("disk full".to_string())
        }
    }
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response("hello")]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_inference_hook(Arc::new(FailingHook));
    let mut ctx = LLMContext::new(base_request(), deps);
    let before = ctx.snapshot();

    let LLMContextOutcome::Error { error, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert_eq!(
        error,
        LLMComputeError::Checkpoint {
            stage: CheckpointStage::BeforeInference,
            message: "disk full".into(),
        }
    );
    assert!(error.infra_retry_safe());
    assert_eq!(llm.seen().len(), 0, "no inference is paid for");
    let after = ctx.snapshot();
    assert_eq!(after.state.accumulated, before.state.accumulated);

    // Once the store is healthy again, only the checkpoint is redone.
    let count = Arc::new(Mutex::new(0));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)).with_inference_hook(Arc::new(
        CountingHook {
            count: count.clone(),
        },
    ));
    let mut resumed = LLMContext::resume(after, ResumeFill::ResumeFromMidRun, deps)
        .expect("checkpoint failure leaves a resumable snapshot");
    assert!(matches!(resumed.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(*count.lock().unwrap(), 1);
    assert_eq!(llm.seen().len(), 1);
}

#[tokio::test]
async fn inference_hook_failure_in_behavior_mode_surfaces_as_checkpoint_error() {
    struct FailingHook;
    impl InferenceHook for FailingHook {
        fn before_inference(&self, _snapshot: &LLMContextSnapshot) -> Result<(), String> {
            Err("disk full".to_string())
        }
    }
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response("hello")]));
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_inference_hook(Arc::new(FailingHook))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);
    let LLMContextOutcome::Error { error, .. } = ctx.run().await else {
        panic!("expected Error");
    };
    assert!(matches!(error, LLMComputeError::Checkpoint { .. }));
    assert_eq!(llm.seen().len(), 0);
}

#[tokio::test]
async fn resume_rejects_history_with_unanswered_tool_calls() {
    let req = base_request();
    let mut state = LLMContextState::from_request(&req, 1);
    state.accumulated.push(AiMessage::new(
        AiRole::Assistant,
        vec![AiContent::tool_use("c-1", "echo", HashMap::new())],
    ));
    let snapshot = LLMContextSnapshot {
        request: req,
        state,
    };
    let deps = LLMContextDeps::new(
        Arc::new(ScriptedLlm::new(vec![text_response("x")])),
        Arc::new(EchoTools),
    );
    let err = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps)
        .err()
        .expect("unanswered tool call must be rejected");
    assert!(matches!(err, LLMComputeError::SnapshotCorrupted(_)));
}
