//! The behavior loop: step metadata, inner transcript, `<next_behavior>`, step result hook.

use std::sync::Arc;

use buckyos_api::{AiMessage, AiRole};

use crate::deps::LLMContextDeps;
use crate::error::{LLMComputeError, ProviderFailure};
use crate::observation::Observation;
use crate::outcome::{LLMContextOutcome, ResumeFill};
use crate::state::{LLMContextSnapshot, LLMContextState};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

#[tokio::test]
async fn behavior_loop_assigns_step_metadata() {
    let llm = Arc::new(ScriptedLlm::new(vec![text_response(
        "<response><thinking>done</thinking><report end=\"true\">finished</report></response>",
    )]));
    let mut req = base_request();
    req.behavior_name = "plan".into();
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::Done {
        behavior_result, ..
    } = outcome
    else {
        panic!("expected behavior Done");
    };
    let result = behavior_result.unwrap();
    assert!(result.report_end);
    assert!(result.next_behavior.is_none());

    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 1);
    let step = &snapshot.state.steps[0];
    assert_eq!(step.meta.behavior_name, "plan");
    assert_eq!(step.meta.step_index, 0);
    assert!(step.meta.started_at_ms > 0);
    assert!(step.meta.ended_at_ms.is_some());
    assert_eq!(snapshot.state.next_step_index, 1);
}

#[tokio::test]
async fn inner_transcript_renders_after_inherited_steps_and_clears_after_inference() {
    let llm = Arc::new(RecordingLlm::new(text_response(
        "<response><thinking>start do</thinking><report end=\"true\">finished</report></response>",
    )));
    let mut req = base_request();
    req.behavior_name = "do".into();
    let mut state = LLMContextState::from_request(&req, 1);
    state.steps.push(crate::behavior_loop::StepRecord {
        meta: crate::behavior_loop::StepMeta {
            behavior_name: "plan".into(),
            step_index: 0,
            started_at_ms: 10,
            ended_at_ms: Some(20),
            compression_level: Default::default(),
        },
        assistant_text: "<response><thinking>create todos</thinking></response>".into(),
        thought: Some("created todos".into()),
        ..Default::default()
    });
    state.steps.push(crate::behavior_loop::StepRecord {
        meta: crate::behavior_loop::StepMeta {
            behavior_name: "plan".into(),
            step_index: 1,
            started_at_ms: 21,
            ended_at_ms: Some(30),
            compression_level: Default::default(),
        },
        assistant_text: "<response><thinking>switch to do</thinking><next_behavior>DO</next_behavior></response>".into(),
        thought: Some("switch to do".into()),
        ..Default::default()
    });
    state
        .accumulated
        .push(AiMessage::text(AiRole::User, "Continue TASK_ANCHOR."));
    let snapshot = LLMContextSnapshot {
        request: req,
        state,
    };
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps)
        .expect("resume should accept an inner-transcript-only behavior input");

    let outcome = ctx.run().await;
    assert!(matches!(outcome, LLMContextOutcome::Done { .. }));
    let seen = llm.seen();
    assert_eq!(seen.len(), 1);
    let messages = &seen[0];
    let continue_idx = messages
        .iter()
        .position(|m| m.text_content().contains("Continue TASK_ANCHOR"))
        .expect("continue message");
    let inherited_idx = messages
        .iter()
        .position(|m| {
            m.role == AiRole::User
                && m.text_content()
                    .contains("<step_record behavior=\"plan\" index=\"1\"")
        })
        .expect("inherited plan step");
    assert!(
        inherited_idx < continue_idx,
        "inherited plan step must render before the hand-over input"
    );
    assert!(
        messages
            .iter()
            .all(|m| { m.role != AiRole::Assistant || !m.text_content().contains("switch to do") }),
        "cross-behavior inherited steps must not render as hot assistant/user pairs"
    );

    let snapshot = ctx.snapshot();
    assert_eq!(
        snapshot.state.accumulated.len(),
        snapshot.request.input.len(),
        "the inner transcript should be consumed after the first behavior inference"
    );
}

#[tokio::test]
async fn behavior_loop_provider_failure_ends_run_without_second_inference() {
    let llm = Arc::new(RecoverOnceLlm::new(text_response(
        "<response><thinking>recovered</thinking><report end=\"true\">finished</report></response>",
    )));
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::Error { error, .. } = outcome else {
        panic!("expected Error, got {outcome:?}");
    };
    assert_eq!(
        error,
        LLMComputeError::provider(ProviderFailure::Transient, "temporary aicc failure")
    );
    assert_eq!(
        llm.seen().len(),
        1,
        "waist must not re-infer after a provider failure"
    );
    assert_eq!(ctx.snapshot().state.consecutive_errors, 0);
}

#[tokio::test]
async fn behavior_loop_parse_error_feeds_back_as_user_message_and_recovers() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(""),
        text_response(
            "<response><thinking>recovered</thinking><report end=\"true\">finished</report></response>",
        ),
    ]));
    let mut req = base_request();
    req.behavior_name = "do".into();
    req.input = vec![
        AiMessage::text(AiRole::System, "static system prompt"),
        AiMessage::text(AiRole::User, "Continue TASK_ANCHOR."),
    ];
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    assert!(matches!(outcome, LLMContextOutcome::Done { .. }));

    let seen = llm.seen();
    assert_eq!(seen.len(), 2);
    let retry_messages = &seen[1];
    let error_messages: Vec<&AiMessage> = retry_messages
        .iter()
        .filter(|m| m.text_content().contains("parse failed"))
        .collect();
    assert_eq!(error_messages.len(), 1);
    assert_eq!(error_messages[0].role, AiRole::User);
    assert_eq!(retry_messages[0].role, AiRole::System);
    assert!(
        retry_messages
            .iter()
            .skip(1)
            .all(|m| m.role != AiRole::System),
        "only the static system prefix may use AiRole::System"
    );
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 2);
    assert!(matches!(
        snapshot.state.steps[0].action_results.as_slice(),
        [Observation::Error { .. }]
    ));
}

#[tokio::test]
async fn behavior_loop_rejects_ending_report_with_actions_before_side_effects() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(
            r#"<actions><shell>must not run</shell></actions><report end="true">invalid</report>"#,
        ),
        text_response(r#"<report end="true">corrected final</report>"#),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(base_request(), deps);
    let LLMContextOutcome::Done {
        behavior_result,
        trace,
        ..
    } = ctx.run().await
    else {
        panic!("expected corrected Done");
    };
    assert!(trace.tool_trace.is_empty());
    let result = behavior_result.unwrap();
    assert!(result.report_end);
    assert_eq!(result.self_report.as_deref(), Some("corrected final"));
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 2);
    assert!(snapshot.state.steps[0].is_correction());
    assert_eq!(snapshot.state.next_action_id, 0);
    assert_eq!(
        snapshot.state.last_report.as_deref(),
        Some("corrected final")
    );
    assert!(llm.seen()[1]
        .iter()
        .any(|msg| msg.text_content().contains("cannot contain actions")));
}

#[tokio::test]
async fn behavior_loop_still_defers_jump_target_declared_with_actions() {
    // A jump target keeps the old contract: its target behavior must observe
    // this step's results first, so the directive is suppressed for one step.
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<response>
<thinking>run before switching</thinking>
<actions><shell>echo done</shell></actions>
<next_behavior>CHECK</next_behavior>
</response>"#,
        ),
        text_response(
            r#"<response>
<observation>action result observed</observation>
<thinking>now switch</thinking>
<next_behavior>CHECK</next_behavior>
</response>"#,
        ),
    ]));
    let mut req = base_request();
    req.behavior_name = "plan".into();
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::Done {
        behavior_result,
        trace,
        ..
    } = outcome
    else {
        panic!("expected behavior Done");
    };
    assert_eq!(trace.tool_trace.len(), 1);
    assert_eq!(
        behavior_result.and_then(|r| r.next_behavior).as_deref(),
        Some("CHECK")
    );

    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 2);
    assert_eq!(snapshot.state.steps[0].actions.len(), 1);
    assert_eq!(snapshot.state.steps[0].next_behavior, None);
    assert_eq!(snapshot.state.steps[0].action_results.len(), 1);
    assert_eq!(
        snapshot.state.steps[1].next_behavior.as_deref(),
        Some("CHECK")
    );
}

#[tokio::test]
async fn behavior_loop_on_behavior_step_ob_overrides_next_user_message() {
    let llm = Arc::new(RecordingLlm::new(text_response(
        r#"<response>
<observation>custom observed</observation>
<thinking>now switch</thinking>
<report end="true">finished</report>
</response>"#,
    )));
    let scripted = Arc::new(ScriptedLlm::new(vec![text_response(
        r#"<response>
<thinking>run action</thinking>
<actions><shell>echo done</shell></actions>
</response>"#,
    )]));
    let mut req = base_request();
    req.behavior_name = "plan".into();
    let deps = LLMContextDeps::new(scripted, Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()))
        .with_step_result_hook(Arc::new(CustomStepResultHook));
    let mut ctx = LLMContext::new(req, deps);

    let first = ctx.run().await;
    assert!(matches!(first, LLMContextOutcome::Error { .. }));

    let snapshot = ctx.snapshot();
    assert_eq!(
        snapshot
            .state
            .last_step
            .as_ref()
            .and_then(|step| step.next_user_message.as_ref())
            .map(|msg| msg.text_content())
            .as_deref(),
        Some("custom step result 0")
    );

    let req = snapshot.request.clone();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut resumed = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps)
        .expect("resume behavior snapshot");
    let outcome = resumed.run().await;
    assert!(matches!(outcome, LLMContextOutcome::Done { .. }));
    assert_eq!(req.behavior_name, "plan");
    let seen = llm.seen();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0]
            .iter()
            .any(|m| m.role == AiRole::User && m.text_content() == "custom step result 0"),
        "custom on_behavior_step_ob user message should be rendered into the next inference"
    );
}

#[tokio::test]
async fn behavior_loop_on_behavior_step_ob_can_skip_next_inference() {
    let llm = Arc::new(RecordingLlm::new(text_response(
        r#"<response>
<thinking>run action</thinking>
<actions><shell>echo done</shell></actions>
</response>"#,
    )));
    let mut req = base_request();
    req.behavior_name = "plan".into();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()))
        .with_step_result_hook(Arc::new(SkipStepResultHook));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    assert!(matches!(outcome, LLMContextOutcome::Done { .. }));
    assert_eq!(llm.seen().len(), 1);
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 1);
    assert_eq!(snapshot.state.steps[0].action_results.len(), 1);
    assert!(snapshot.state.steps[0].next_user_message.is_none());
}

#[tokio::test]
async fn behavior_loop_rejects_ending_report_with_sendmsg_before_emit() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<actions><sendmsg target="user">must not send</sendmsg></actions><report end="true">invalid</report>"#,
        ),
        text_response(r#"<report end="true">corrected</report>"#),
    ]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(base_request(), deps);
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert!(ctx
        .snapshot()
        .state
        .steps
        .iter()
        .all(|step| step.messages_sent.is_empty()));
    assert!(ctx.snapshot().state.steps[0].is_correction());
}

#[tokio::test]
async fn behavior_loop_observes_failed_action_before_final_report() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<response>
<thinking>last step</thinking>
<actions><shell>boom</shell></actions>
</response>"#,
        ),
        text_response(
            r#"<response>
<observation>it failed</observation>
<report end="true">finished</report>
</response>"#,
        ),
    ]));
    let mut req = base_request();
    req.behavior_name = "plan".into();
    let deps = LLMContextDeps::new(llm, Arc::new(FailingTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::Done {
        behavior_result, ..
    } = outcome
    else {
        panic!("expected behavior Done");
    };
    let result = behavior_result.unwrap();
    assert!(result.report_end);
    assert!(result.next_behavior.is_none());

    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 2);
    assert!(matches!(
        snapshot.state.steps[0].action_results[0],
        Observation::Error { .. }
    ));
    assert_eq!(snapshot.state.steps[0].next_behavior, None);
    assert!(snapshot.state.steps[1].report_end);
}

#[tokio::test]
async fn native_ending_report_conflict_pairs_calls_without_dispatch() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(
            Some(r#"<report end="true">invalid</report>"#),
            vec![call("a", "blocked")],
        ),
        text_response(r#"<report end="true">corrected</report>"#),
    ]));
    let tools = Arc::new(ScriptedTools::new());
    let deps = LLMContextDeps::new(llm.clone(), tools.clone())
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(base_request(), deps);
    let LLMContextOutcome::Done {
        behavior_result,
        trace,
        ..
    } = ctx.run().await
    else {
        panic!("expected corrected Done");
    };
    assert!(tools.calls().is_empty());
    assert_eq!(trace.tool_trace.len(), 1);
    assert_eq!(
        trace.tool_trace[0].status,
        crate::observation::ToolExecStatus::NotExecuted
    );
    assert_eq!(
        behavior_result.unwrap().self_report.as_deref(),
        Some("corrected")
    );
    let seen = llm.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen[1]
        .iter()
        .any(|msg| tool_result_text(msg).contains("cannot contain actions")));
}

#[tokio::test]
async fn progress_report_resumes_and_final_snapshot_returns_without_inference() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response("<report>progress one</report>"),
        text_response(r#"<report end="false">progress two</report>"#),
    ]));
    let make_deps = |llm: Arc<dyn crate::deps::LlmClient>| {
        LLMContextDeps::new(llm, Arc::new(EchoTools))
            .with_result_parser(Arc::new(XmlBehaviorParser::new()))
            .with_step_renderer(Arc::new(XmlStepRenderer::new()))
    };
    let mut ctx = LLMContext::new(base_request(), make_deps(llm));
    assert!(matches!(ctx.run().await, LLMContextOutcome::Error { .. }));
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.last_report.as_deref(), Some("progress two"));
    assert!(!snapshot.state.report_end);
    let snapshot: LLMContextSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    let mut resumed = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, make_deps(Arc::new(ScriptedLlm::new(vec![
        text_response(r#"<report end="true">final report</report><artifacts>["result.txt"]</artifacts><result>{"ok":true}</result>"#),
    ])))).unwrap();
    assert!(matches!(
        resumed.run().await,
        LLMContextOutcome::Done { .. }
    ));
    let final_snapshot = resumed.snapshot();
    assert!(final_snapshot.state.report_end);
    let mut recovered = LLMContext::resume(
        final_snapshot,
        ResumeFill::ResumeFromMidRun,
        make_deps(Arc::new(ScriptedLlm::new(vec![]))),
    )
    .unwrap();
    let LLMContextOutcome::Done {
        behavior_result, ..
    } = recovered.run().await
    else {
        panic!("completed report must recover without inference");
    };
    let result = behavior_result.unwrap();
    assert_eq!(result.self_report.as_deref(), Some("final report"));
    assert!(result.report_end);
    assert!(result.next_behavior.is_none());
    assert_eq!(result.report_artifacts, vec!["result.txt"]);
    assert_eq!(result.report_result, Some(serde_json::json!({"ok":true})));
    assert_eq!(recovered.snapshot().state.steps.len(), 3);
}

#[tokio::test]
async fn thought_only_output_is_corrected_without_implicit_completion() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response("<thinking>finished</thinking>"),
        text_response(r#"<report end="true">explicit final</report>"#),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(base_request(), deps);
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(llm.seen().len(), 2);
    assert!(ctx.snapshot().state.steps[0].is_correction());
}

#[derive(Default)]
struct ReportCheckpoint {
    tools: std::sync::Mutex<Vec<LLMContextSnapshot>>,
    reports: std::sync::Mutex<Vec<String>>,
    fail_tool_at: Option<usize>,
}

#[async_trait::async_trait]
impl crate::deps::CheckpointHook for ReportCheckpoint {
    async fn before_inference(
        &self,
        _: &LLMContextSnapshot,
    ) -> Result<Option<crate::deps::Injection>, String> {
        Ok(None)
    }

    async fn before_tool_call(&self, snapshot: &LLMContextSnapshot) -> Result<(), String> {
        let mut tools = self.tools.lock().unwrap();
        tools.push(snapshot.clone());
        if self.fail_tool_at == Some(tools.len()) {
            Err("tool checkpoint unavailable".into())
        } else {
            Ok(())
        }
    }

    async fn validate_report(
        &self,
        _: &LLMContextSnapshot,
        result: &crate::LLMBehaviorResult,
        _: u32,
    ) -> Result<(), String> {
        self.reports
            .lock()
            .unwrap()
            .push(result.self_report.clone().unwrap());
        if result.self_report.as_deref() == Some("invalid") {
            Err("host rejected this report".into())
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn host_report_validation_precedes_all_side_effects() {
    for native in [false, true] {
        let invalid = if native {
            tool_response(Some("<report>invalid</report>"), vec![call("a", "blocked")])
        } else {
            text_response("<actions><shell>blocked</shell></actions><report>invalid</report>")
        };
        let llm = Arc::new(ScriptedLlm::new(vec![
            invalid,
            text_response(r#"<report end="true">valid</report>"#),
        ]));
        let tools = Arc::new(ScriptedTools::new());
        let hook = Arc::new(ReportCheckpoint::default());
        let deps = LLMContextDeps::new(llm, tools.clone())
            .with_result_parser(Arc::new(XmlBehaviorParser::new()))
            .with_step_renderer(Arc::new(XmlStepRenderer::new()))
            .with_checkpoint_hook(hook.clone());
        let mut ctx = LLMContext::new(base_request(), deps);
        assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
        assert!(tools.calls().is_empty());
        assert_eq!(*hook.reports.lock().unwrap(), vec!["invalid", "valid"]);
        assert_eq!(ctx.snapshot().state.last_report.as_deref(), Some("valid"));
    }
}

#[tokio::test]
async fn tool_checkpoints_keep_outer_snapshot_with_prior_receipts_and_native_report() {
    for behavior in [false, true] {
        let report = behavior.then_some("<report>native progress</report>");
        let llm = Arc::new(ScriptedLlm::new(vec![
            tool_response(report, vec![call("a", "first"), call("c", "second")]),
            text_response(if behavior {
                r#"<report end="true">final</report>"#
            } else {
                "final"
            }),
        ]));
        let hook = Arc::new(ReportCheckpoint::default());
        let mut deps = LLMContextDeps::new(llm, Arc::new(ScriptedTools::new()))
            .with_checkpoint_hook(hook.clone());
        if behavior {
            deps = deps
                .with_result_parser(Arc::new(XmlBehaviorParser::new()))
                .with_step_renderer(Arc::new(XmlStepRenderer::new()));
        }
        let mut req = base_request();
        req.behavior_name = "checkpoint-test".into();
        let mut ctx = LLMContext::new(req.clone(), deps);
        assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
        let snapshots = hook.tools.lock().unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].request.behavior_name, "checkpoint-test");
        assert_eq!(snapshots[0].request.input, req.input);
        assert_eq!(
            snapshots[0]
                .state
                .tool_batch
                .as_ref()
                .unwrap()
                .remaining
                .len(),
            2
        );
        assert_eq!(
            snapshots[1]
                .state
                .tool_batch
                .as_ref()
                .unwrap()
                .remaining
                .len(),
            1
        );
        assert!(snapshots[1]
            .state
            .accumulated
            .iter()
            .any(|msg| !tool_result_text(msg).is_empty()));
        if behavior {
            assert_eq!(
                snapshots[0].state.last_report.as_deref(),
                Some("native progress")
            );
            assert_eq!(
                snapshots[0].state.steps[0].self_report.as_deref(),
                Some("native progress")
            );
            assert!(!snapshots[0].state.steps[0].report_end);
            assert_eq!(ctx.snapshot().state.steps.len(), 2);
        }
    }
}

#[tokio::test]
async fn tool_checkpoint_failure_resumes_only_unexecuted_calls_on_all_surfaces() {
    for surface in ["native", "behavior_native", "behavior_action"] {
        let behavior = surface != "native";
        let batch = if surface == "behavior_action" {
            text_response("<actions><shell>first</shell><shell>second</shell></actions>")
        } else {
            tool_response(None, vec![call("a", "first"), call("c", "second")])
        };
        let llm = Arc::new(ScriptedRecordingLlm::new(vec![
            batch,
            text_response(if behavior {
                r#"<report end="true">final</report>"#
            } else {
                "final"
            }),
        ]));
        let tools = Arc::new(ScriptedTools::new());
        let hook = Arc::new(ReportCheckpoint {
            fail_tool_at: Some(2),
            ..Default::default()
        });
        let mut deps =
            LLMContextDeps::new(llm.clone(), tools.clone()).with_checkpoint_hook(hook.clone());
        if behavior {
            deps = deps
                .with_result_parser(Arc::new(XmlBehaviorParser::new()))
                .with_step_renderer(Arc::new(XmlStepRenderer::new()));
        }
        let mut ctx = LLMContext::new(base_request(), deps.clone());
        assert!(
            matches!(
                ctx.run().await,
                LLMContextOutcome::Error {
                    error: LLMComputeError::Checkpoint {
                        stage: crate::error::CheckpointStage::BeforeToolCall,
                        ..
                    },
                    ..
                }
            ),
            "{surface}"
        );
        assert_eq!(tools.calls().len(), 1);
        let snapshot: LLMContextSnapshot =
            serde_json::from_str(&serde_json::to_string(&ctx.snapshot()).unwrap()).unwrap();
        let mut resumed = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps).unwrap();
        assert!(
            matches!(resumed.run().await, LLMContextOutcome::Done { .. }),
            "{surface}"
        );
        assert_eq!(tools.calls().len(), 2);
        assert_eq!(llm.seen().len(), 2);
    }
}

#[tokio::test]
async fn final_report_recovery_rechecks_host_acceptance_and_allows_correction() {
    let deps = LLMContextDeps::new(
        Arc::new(ScriptedLlm::new(vec![
            text_response("<report>accepted progress</report>"),
            text_response(r#"<report end="true">invalid</report>"#),
        ])),
        Arc::new(EchoTools),
    )
    .with_result_parser(Arc::new(XmlBehaviorParser::new()))
    .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(base_request(), deps);
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    let hook = Arc::new(ReportCheckpoint::default());
    let deps = LLMContextDeps::new(
        Arc::new(ScriptedLlm::new(vec![text_response(
            r#"<report end="true">corrected</report>"#,
        )])),
        Arc::new(EchoTools),
    )
    .with_result_parser(Arc::new(XmlBehaviorParser::new()))
    .with_step_renderer(Arc::new(XmlStepRenderer::new()))
    .with_checkpoint_hook(hook.clone());
    let mut recovered =
        LLMContext::resume(ctx.snapshot(), ResumeFill::ResumeFromMidRun, deps).unwrap();
    assert!(matches!(
        recovered.run().await,
        LLMContextOutcome::Done { .. }
    ));
    assert_eq!(*hook.reports.lock().unwrap(), vec!["invalid", "corrected"]);
    assert!(recovered
        .snapshot()
        .state
        .steps
        .iter()
        .all(|step| step.self_report.as_deref() != Some("invalid")));
    assert_eq!(
        recovered.snapshot().state.last_report.as_deref(),
        Some("corrected")
    );
}
