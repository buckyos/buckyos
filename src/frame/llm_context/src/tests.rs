use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiResponseSummary, AiRole, AiToolCall, AiUsage};
use serde_json::json;

use crate::deps::{
    LLMContextDeps, LlmClient, LlmInferenceRequest, ToolManager, ToolSpecLite, TurnHook,
};
use crate::error::LLMComputeError;
use crate::observation::Observation;
use crate::outcome::{ContextOutput, LLMContextOutcome, ResumeFill};
use crate::request::{
    ContextOwnerRef, LLMContextRequest, ModelPolicy, OutputSpec, ToolMode, ToolPolicy,
};
use crate::state::LLMContextSnapshot;
use crate::LLMContext;

/// Scripted LLM responses popped off in order.
struct ScriptedLlm {
    script: Mutex<Vec<AiResponseSummary>>,
}

impl ScriptedLlm {
    fn new(script: Vec<AiResponseSummary>) -> Self {
        Self {
            script: Mutex::new(script),
        }
    }
}

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn infer(
        &self,
        _req: LlmInferenceRequest,
    ) -> Result<AiResponseSummary, LLMComputeError> {
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        Ok(guard.remove(0))
    }
}

struct EchoTools;

#[async_trait]
impl ToolManager for EchoTools {
    async fn call_tool(&self, call: AiToolCall) -> Observation {
        let value = serde_json::to_value(&call.args).unwrap_or(serde_json::Value::Null);
        Observation::Success {
            call_id: call.call_id,
            content: json!({ "echo": value }),
            bytes: 0,
            truncated: false,
        }
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        vec![ToolSpecLite {
            name: "echo".into(),
            description: "echo the args".into(),
            args_schema: json!({}),
        }]
    }
}

fn base_request() -> LLMContextRequest {
    LLMContextRequest {
        owner: ContextOwnerRef::OneShot { id: "t".into() },
        trace: Some("trace-1".into()),
        objective: "test".into(),
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

#[tokio::test]
async fn done_without_tool_calls() {
    let llm = Arc::new(ScriptedLlm::new(vec![AiResponseSummary {
        text: Some("hi there".into()),
        usage: Some(AiUsage {
            input_tokens: Some(5),
            output_tokens: Some(3),
            total_tokens: Some(8),
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
        AiResponseSummary {
            text: Some("calling echo".into()),
            tool_calls: vec![call],
            ..Default::default()
        },
        AiResponseSummary {
            text: Some("done after echo".into()),
            ..Default::default()
        },
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
    assert!(trace.tool_trace[0].ok);
}

struct CountingHook {
    count: Arc<Mutex<u32>>,
}

impl TurnHook for CountingHook {
    fn before_inference(&self, _snapshot: &LLMContextSnapshot) {
        *self.count.lock().unwrap() += 1;
    }
}

#[tokio::test]
async fn turn_hook_fires_before_each_inference() {
    let llm = Arc::new(ScriptedLlm::new(vec![AiResponseSummary {
        text: Some("hello back".into()),
        ..Default::default()
    }]));
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
    let llm = Arc::new(ScriptedLlm::new(vec![AiResponseSummary {
        text: Some("first reply".into()),
        ..Default::default()
    }]));
    let deps = LLMContextDeps::new(llm, Arc::new(EchoTools));
    let mut ctx = LLMContext::new(base_request(), deps);
    let _ = ctx.run().await;
    let snapshot = ctx.snapshot();

    // Resume the mid-run snapshot. A second LLM call must succeed because
    // the snapshot is *not* in a suspended state.
    let llm2 = Arc::new(ScriptedLlm::new(vec![AiResponseSummary {
        text: Some("after resume".into()),
        ..Default::default()
    }]));
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
        AiResponseSummary {
            tool_calls: vec![make_call("c-1")],
            ..Default::default()
        },
        AiResponseSummary {
            tool_calls: vec![make_call("c-2")],
            ..Default::default()
        },
        AiResponseSummary {
            tool_calls: vec![make_call("c-3")],
            ..Default::default()
        },
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
