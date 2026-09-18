use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiToolCall, AiUsage, ResourceRef};
use serde_json::json;

use crate::deps::{
    LLMContextDeps, LlmClient, LlmInferenceRequest, ToolDispatchError, ToolManager, ToolSpecLite,
    TurnHook,
};
use crate::error::{CheckpointStage, LLMComputeError, ProviderFailure};
use crate::observation::{Observation, ToolExecStatus};
use crate::outcome::{BudgetKind, ContextOutput, LLMContextOutcome, ResumeFill};
use crate::request::{
    ContextOwnerRef, LLMContextRequest, ModelPolicy, OutputSpec, ToolMode, ToolPolicy,
};
use crate::state::{LLMContextSnapshot, LLMContextState};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};
use crate::{StepResultHook, StepResultHookOutput};

/// Scripted LLM responses popped off in order.
struct ScriptedLlm {
    script: Mutex<Vec<AiResponse>>,
}

impl ScriptedLlm {
    fn new(script: Vec<AiResponse>) -> Self {
        Self {
            script: Mutex::new(script),
        }
    }
}

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn infer(&self, _req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        Ok(guard.remove(0))
    }
}

struct RecordingLlm {
    response: AiResponse,
    seen: Mutex<Vec<Vec<AiMessage>>>,
}

impl RecordingLlm {
    fn new(response: AiResponse) -> Self {
        Self {
            response,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for RecordingLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        Ok(self.response.clone())
    }
}

struct RecoverOnceLlm {
    response: AiResponse,
    seen: Mutex<Vec<Vec<AiMessage>>>,
    failed: Mutex<bool>,
}

impl RecoverOnceLlm {
    fn new(response: AiResponse) -> Self {
        Self {
            response,
            seen: Mutex::new(Vec::new()),
            failed: Mutex::new(false),
        }
    }

    fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for RecoverOnceLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        let mut failed = self.failed.lock().unwrap();
        if !*failed {
            *failed = true;
            return Err(LLMComputeError::provider(
                ProviderFailure::Transient,
                "temporary aicc failure",
            ));
        }
        Ok(self.response.clone())
    }
}

struct EchoTools;

#[async_trait]
impl ToolManager for EchoTools {
    async fn call_tool(&self, call: AiToolCall) -> Result<Observation, ToolDispatchError> {
        let value = serde_json::to_value(&call.args).unwrap_or(serde_json::Value::Null);
        Ok(Observation::Success {
            call_id: call.call_id,
            content: json!({ "echo": value }),
            bytes: 0,
            truncated: false,
            tool_result: None,
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        vec![ToolSpecLite {
            name: "echo".into(),
            description: "echo the args".into(),
            args_schema: json!({}),
        }]
    }
}

struct CustomStepResultHook;

#[async_trait]
impl StepResultHook for CustomStepResultHook {
    async fn on_behavior_step_ob(
        &self,
        _snapshot: &LLMContextSnapshot,
        step: &crate::behavior_loop::StepRecord,
    ) -> Result<StepResultHookOutput, String> {
        Ok(StepResultHookOutput {
            user_message: Some(AiMessage::text(
                AiRole::User,
                format!("custom step result {}", step.meta.step_index),
            )),
            history_inputs: Vec::new(),
            skip_next_inference: false,
        })
    }
}

struct SkipStepResultHook;

#[async_trait]
impl StepResultHook for SkipStepResultHook {
    async fn on_behavior_step_ob(
        &self,
        _snapshot: &LLMContextSnapshot,
        _step: &crate::behavior_loop::StepRecord,
    ) -> Result<StepResultHookOutput, String> {
        Ok(StepResultHookOutput {
            skip_next_inference: true,
            ..Default::default()
        })
    }
}

fn base_request() -> LLMContextRequest {
    LLMContextRequest {
        owner: ContextOwnerRef::OneShot { id: "t".into() },
        trace: Some("trace-1".into()),
        objective: "test".into(),
        behavior_name: String::new(),
        input: vec![AiMessage::text(AiRole::User, "hello")],
        model_policy: ModelPolicy {
            preferred: "test-model".into(),
            ..ModelPolicy::default()
        },
        tool_policy: ToolPolicy {
            mode: ToolMode::All,
            max_rounds: 4,
            max_calls_per_round: 4,
            ..ToolPolicy::default()
        },
        output: OutputSpec::Text,
        budget: Default::default(),
        human_policy: Default::default(),
        error_policy: Default::default(),
        forbid_next_behavior: false,
    }
}

fn text_response(text: &str) -> AiResponse {
    AiResponse::text(text)
}

fn tool_response(text: Option<&str>, calls: Vec<AiToolCall>) -> AiResponse {
    AiResponse::from_parts(text.map(str::to_string), calls, vec![])
}

#[tokio::test]
async fn done_without_tool_calls() {
    let llm = Arc::new(ScriptedLlm::new(vec![AiResponse {
        message: AiMessage::text(AiRole::Assistant, "hi there"),
        usage: Some(AiUsage {
            input_tokens: Some(5),
            output_tokens: Some(3),
            total_tokens: Some(8),
            request_units: None,
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
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[tokio::test]
async fn one_tool_round_then_done() {
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

struct CountingHook {
    count: Arc<Mutex<u32>>,
}

impl TurnHook for CountingHook {
    fn before_inference(&self, _snapshot: &LLMContextSnapshot) -> Result<(), String> {
        *self.count.lock().unwrap() += 1;
        Ok(())
    }
}

#[tokio::test]
async fn turn_hook_fires_before_each_inference() {
    let llm = Arc::new(ScriptedLlm::new(vec![text_response("hello back")]));
    let count = Arc::new(Mutex::new(0));
    let hook: Arc<dyn TurnHook> = Arc::new(CountingHook {
        count: count.clone(),
    });
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools)).with_turn_hook(hook);
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
    assert!(snapshot.state.pending_tool_calls.is_empty());
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
async fn tool_rounds_budget_exhausted() {
    let mut args: HashMap<String, serde_json::Value> = HashMap::new();
    args.insert("msg".into(), json!("ping"));
    let make_call = |id: &str| AiToolCall {
        name: "echo".into(),
        args: args.clone(),
        call_id: id.into(),
    };

    // 3 inferences, each demands another tool call. max_rounds = 2 ⇒
    // after 2 rounds the loop bails out with BudgetExhausted.
    let llm = Arc::new(ScriptedLlm::new(vec![
        tool_response(None, vec![make_call("c-1")]),
        tool_response(None, vec![make_call("c-2")]),
        tool_response(None, vec![make_call("c-3")]),
    ]));
    let mut req = base_request();
    req.tool_policy.max_rounds = 2;
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(req, deps);

    match ctx.run().await {
        LLMContextOutcome::BudgetExhausted { which, .. } => {
            assert!(matches!(which, crate::outcome::BudgetKind::ToolRounds));
        }
        other => panic!("expected BudgetExhausted, got {other:?}"),
    }
}

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
async fn behavior_turn_tail_renders_after_inherited_steps_and_clears_after_inference() {
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
        .expect("resume should accept tail-only behavior turn input");

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
        "inherited plan step must render before the turn trigger"
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
        "turn tail should be consumed after the first behavior inference"
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
async fn behavior_loop_ignores_next_behavior_when_actions_exist() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<response>
<thinking>run before switching</thinking>
<actions><exec_bash>echo done</exec_bash></actions>
<next_behavior>END</next_behavior>
</response>"#,
        ),
        text_response(
            r#"<response>
<observation>action result observed</observation>
<thinking>now switch</thinking>
<next_behavior>END</next_behavior>
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
        Some("END")
    );

    let snapshot = ctx.snapshot();
    assert_eq!(snapshot.state.steps.len(), 2);
    assert_eq!(snapshot.state.steps[0].actions.len(), 1);
    assert_eq!(snapshot.state.steps[0].actions[0].call_id, "1");
    assert_eq!(snapshot.state.steps[0].next_behavior, None);
    assert_eq!(snapshot.state.steps[0].action_results.len(), 1);
    assert_eq!(snapshot.state.next_action_id, 1);
    assert_eq!(
        snapshot.state.steps[1].next_behavior.as_deref(),
        Some("END")
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
async fn behavior_loop_ignores_next_behavior_when_sendmsg_exists() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        text_response(
            r#"<response>
<thinking>notify before switching</thinking>
<actions><sendmsg target="user">working</sendmsg></actions>
<next_behavior>END</next_behavior>
</response>"#,
        ),
        text_response(
            r#"<response>
<thinking>now switch</thinking>
<next_behavior>END</next_behavior>
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
    assert_eq!(snapshot.state.steps.len(), 2);
    assert_eq!(snapshot.state.steps[0].messages_sent.len(), 1);
    assert_eq!(snapshot.state.steps[0].next_behavior, None);
    assert_eq!(
        snapshot.state.steps[1].next_behavior.as_deref(),
        Some("END")
    );
}

// =====================================================================
// Error-handling scenarios (notepads/llm-context-error-handling-todo.md §10)
// =====================================================================

/// Scripted responses / errors, recording every request like `RecordingLlm`.
struct ScriptedRecordingLlm {
    script: Mutex<Vec<Result<AiResponse, LLMComputeError>>>,
    seen: Mutex<Vec<Vec<AiMessage>>>,
}

impl ScriptedRecordingLlm {
    fn new(script: Vec<AiResponse>) -> Self {
        Self::with_results(script.into_iter().map(Ok).collect())
    }

    fn with_results(script: Vec<Result<AiResponse, LLMComputeError>>) -> Self {
        Self {
            script: Mutex::new(script),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for ScriptedRecordingLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        guard.remove(0)
    }
}

/// Tool manager scripted by tool name: `fail:*` is a business error,
/// `dispatch:*` is an infrastructure failure with unknown effect, anything
/// else succeeds. Counts every dispatch attempt.
struct ScriptedTools {
    calls: Mutex<Vec<String>>,
}

impl ScriptedTools {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolManager for ScriptedTools {
    async fn call_tool(&self, call: AiToolCall) -> Result<Observation, ToolDispatchError> {
        self.calls.lock().unwrap().push(call.name.clone());
        if call.name.starts_with("dispatch:") {
            return Err(ToolDispatchError::effect_unknown("sandbox connection lost"));
        }
        if call.name.starts_with("fail:") {
            return Ok(Observation::Error {
                call_id: call.call_id,
                message: "bad args".to_string(),
                tool_result: None,
            });
        }
        Ok(Observation::Success {
            call_id: call.call_id,
            content: json!("ok"),
            bytes: 2,
            truncated: false,
            tool_result: None,
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        ["a", "fail:b", "dispatch:b", "c", "exec_bash"]
            .iter()
            .map(|name| ToolSpecLite {
                name: (*name).to_string(),
                description: String::new(),
                args_schema: json!({}),
            })
            .collect()
    }
}

fn call(name: &str, id: &str) -> AiToolCall {
    AiToolCall {
        name: name.into(),
        args: HashMap::new(),
        call_id: id.into(),
    }
}

fn tool_result_text(message: &AiMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AiContent::ToolResult { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|part| match part {
                        buckyos_api::AiToolResultContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn statuses(trace: &crate::outcome::ContextRunTrace) -> Vec<(String, ToolExecStatus)> {
    trace
        .tool_trace
        .iter()
        .map(|r| (r.tool_name.clone(), r.status))
        .collect()
}

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
                request_units: None,
            }),
            ..Default::default()
        },
        AiResponse {
            message: AiMessage::text(AiRole::Assistant, r#"{"answer": 42}"#),
            usage: Some(AiUsage {
                input_tokens: Some(1),
                output_tokens: Some(1),
                total_tokens: Some(3),
                request_units: None,
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
async fn multiple_tool_errors_in_one_round_count_as_one_failure() {
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
        panic!("expected Done: three failed calls are one failed round");
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
        panic!("expected Error after the 4th consecutive failed round");
    };
    assert!(matches!(error, LLMComputeError::ToolFailed { .. }));
    assert_eq!(llm.seen().len(), 4);
    assert_eq!(trace.tool_trace.len(), 4);
}

#[tokio::test]
async fn clean_tool_round_resets_consecutive_errors() {
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
<actions><exec_bash>echo a</exec_bash><exec_bash>echo b</exec_bash><exec_bash>echo c</exec_bash></actions>
</response>"#,
        ),
        text_response("<response><thinking>saw it</thinking><next_behavior>END</next_behavior></response>"),
    ]));
    struct FailSecond {
        tools: Arc<ScriptedTools>,
    }
    #[async_trait]
    impl ToolManager for FailSecond {
        async fn call_tool(&self, mut c: AiToolCall) -> Result<Observation, ToolDispatchError> {
            if c.call_id == "2" {
                c.name = "fail:exec_bash".into();
            }
            self.tools.call_tool(c).await
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
<actions><exec_bash>echo a</exec_bash><exec_bash>echo b</exec_bash></actions>
</response>"#,
        ),
        text_response("never reached"),
    ]));
    struct DispatchFailFirst {
        tools: Arc<ScriptedTools>,
    }
    #[async_trait]
    impl ToolManager for DispatchFailFirst {
        async fn call_tool(&self, mut c: AiToolCall) -> Result<Observation, ToolDispatchError> {
            if c.call_id == "1" {
                c.name = "dispatch:exec_bash".into();
            }
            self.tools.call_tool(c).await
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
async fn turn_hook_failure_blocks_inference_and_keeps_snapshot_resumable() {
    struct FailingHook;
    impl TurnHook for FailingHook {
        fn before_inference(&self, _snapshot: &LLMContextSnapshot) -> Result<(), String> {
            Err("disk full".to_string())
        }
    }
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response("hello")]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_turn_hook(Arc::new(FailingHook));
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
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)).with_turn_hook(Arc::new(
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
async fn turn_hook_failure_in_behavior_mode_surfaces_as_checkpoint_error() {
    struct FailingHook;
    impl TurnHook for FailingHook {
        fn before_inference(&self, _snapshot: &LLMContextSnapshot) -> Result<(), String> {
            Err("disk full".to_string())
        }
    }
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response("hello")]));
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_turn_hook(Arc::new(FailingHook))
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

#[tokio::test]
async fn behavior_inner_context_inherits_outer_budget() {
    let usage = |total: u64| {
        Some(AiUsage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: Some(total),
            request_units: None,
        })
    };
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        AiResponse {
            message: AiMessage::text(
                AiRole::Assistant,
                "<response><thinking>go</thinking><actions><exec_bash>echo a</exec_bash></actions></response>",
            ),
            usage: usage(8),
            ..Default::default()
        },
        AiResponse {
            message: AiMessage::text(
                AiRole::Assistant,
                "<response><thinking>done</thinking><next_behavior>END</next_behavior></response>",
            ),
            usage: usage(8),
            ..Default::default()
        },
    ]));
    let mut req = base_request();
    req.behavior_name = "do".into();
    req.budget.max_total_tokens = Some(10);
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);

    let outcome = ctx.run().await;
    let LLMContextOutcome::BudgetExhausted { which, usage, .. } = outcome else {
        panic!("expected BudgetExhausted, got {outcome:?}");
    };
    assert_eq!(which, BudgetKind::Tokens);
    assert_eq!(usage.total_tokens, Some(16));
}
