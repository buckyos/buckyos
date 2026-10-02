//! The behavior loop: step metadata, inner transcript, `<next_behavior>`, step result hook.

use std::sync::{Arc};

use buckyos_api::{AiMessage, AiRole};

use crate::deps::{LLMContextDeps};
use crate::error::{LLMComputeError, ProviderFailure};
use crate::observation::{Observation};
use crate::outcome::{LLMContextOutcome, ResumeFill};
use crate::state::{LLMContextSnapshot, LLMContextState};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

#[tokio::test]
async fn behavior_loop_assigns_step_metadata() {
    let llm = Arc::new(ScriptedLlm::new(vec![text_response(
        "<response><thinking>done</thinking><next_behavior>END</next_behavior></response>",
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
    assert_eq!(
        behavior_result.and_then(|r| r.next_behavior).as_deref(),
        Some("END")
    );

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
        "<response><thinking>start do</thinking><next_behavior>END</next_behavior></response>",
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
        "<response><thinking>recovered</thinking><next_behavior>END</next_behavior></response>",
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
    assert_eq!(llm.seen().len(), 1, "waist must not re-infer after a provider failure");
    assert_eq!(ctx.snapshot().state.consecutive_errors, 0);
}

#[tokio::test]
async fn behavior_loop_parse_error_feeds_back_as_user_message_and_recovers() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(""),
        text_response(
            "<response><thinking>recovered</thinking><next_behavior>END</next_behavior></response>",
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
async fn behavior_loop_honours_terminal_end_declared_with_actions() {
    let llm = Arc::new(ScriptedLlm::new(vec![text_response(
        r#"<response>
<thinking>done, closing the loop</thinking>
<actions><exec_bash>echo done</exec_bash></actions>
<next_behavior>END</next_behavior>
</response>"#,
    )]));
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
    // The action still ran...
    assert_eq!(trace.tool_trace.len(), 1);
    // ...and END was honoured, so this one step ended the behavior. Anything
    // else makes a model that always closes with `actions + END` loop forever.
    assert_eq!(
        behavior_result.and_then(|r| r.next_behavior).as_deref(),
        Some("END")
    );

    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 1);
    assert_eq!(snapshot.state.steps[0].actions.len(), 1);
    assert_eq!(snapshot.state.steps[0].actions[0].call_id, "1");
    assert_eq!(snapshot.state.steps[0].action_results.len(), 1);
    assert_eq!(
        snapshot.state.steps[0].next_behavior.as_deref(),
        Some("END")
    );
    assert_eq!(snapshot.state.next_action_id, 1);
}

#[tokio::test]
async fn behavior_loop_still_defers_jump_target_declared_with_actions() {
    // A jump target keeps the old contract: its target behavior must observe
    // this step's results first, so the directive is suppressed for one step.
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<response>
<thinking>run before switching</thinking>
<actions><exec_bash>echo done</exec_bash></actions>
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
<next_behavior>END</next_behavior>
</response>"#,
    )));
    let scripted = Arc::new(ScriptedLlm::new(vec![text_response(
        r#"<response>
<thinking>run action</thinking>
<actions><exec_bash>echo done</exec_bash></actions>
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
<actions><exec_bash>echo done</exec_bash></actions>
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
async fn behavior_loop_honours_terminal_end_declared_with_sendmsg() {
    let llm = Arc::new(ScriptedLlm::new(vec![text_response(
        r#"<response>
<thinking>notify and finish</thinking>
<actions><sendmsg target="user">working</sendmsg></actions>
<next_behavior>END</next_behavior>
</response>"#,
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
    assert_eq!(
        behavior_result.and_then(|r| r.next_behavior).as_deref(),
        Some("END")
    );

    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 1);
    assert_eq!(snapshot.state.steps[0].messages_sent.len(), 1);
    assert_eq!(
        snapshot.state.steps[0].next_behavior.as_deref(),
        Some("END")
    );
}

#[tokio::test]
async fn behavior_loop_releases_terminal_end_when_a_dispatched_action_failed() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<response>
<thinking>last step</thinking>
<actions><exec_bash>boom</exec_bash></actions>
<next_behavior>END</next_behavior>
</response>"#,
        ),
        text_response(
            r#"<response>
<observation>it failed</observation>
<next_behavior>END</next_behavior>
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
    assert_eq!(
        behavior_result.and_then(|r| r.next_behavior).as_deref(),
        Some("END")
    );

    // The failure had to be observed first, so END did not end the step it was
    // declared on: the loop ran one more step and the model re-declared it.
    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 2);
    assert!(matches!(
        snapshot.state.steps[0].action_results[0],
        Observation::Error { .. }
    ));
    assert_eq!(snapshot.state.steps[0].next_behavior, None);
    assert_eq!(
        snapshot.state.steps[1].next_behavior.as_deref(),
        Some("END")
    );
}
