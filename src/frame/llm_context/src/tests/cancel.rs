//! Interrupt, graceful finish and deadline while a tool runs (long-tool TODO
//! §3.1), the inline `Cancelled` / `Pending` contracts and the background
//! env slot.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{AiContent, AiRole, AiToolCall};
use serde_json::json;

use crate::deps::{LLMContextDeps, ToolCallCtx, ToolDispatchError, ToolManager, ToolSpecLite};
use crate::error::LLMComputeError;
use crate::observation::{Observation, ToolExecStatus};
use crate::outcome::{BudgetKind, LLMContextOutcome, ResumeFill};
use crate::tasks::{CancelUnsupported, RunningTaskResolver, TaskBrief, TaskState};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

/// Tools scripted by name:
/// - `slow`: cancellable — waits for the ctx to fire, then `Cancelled {
///   effect_unknown: false }`;
/// - `stubborn`: cannot cancel — ignores a graceful finish, gives up only
///   on interrupt / deadline with `Cancelled { effect_unknown: true }`;
/// - `brief`: cannot cancel, finishes on its own after 300 ms;
/// - `bad_cancel`: returns `Cancelled` although nothing fired;
/// - `no_task`: returns `Pending` without a task id;
/// - anything else: success at once.
struct CancelTools {
    calls: Mutex<Vec<String>>,
}

impl CancelTools {
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
impl ToolManager for CancelTools {
    async fn call_tool(
        &self,
        call: AiToolCall,
        ctx: ToolCallCtx,
    ) -> Result<Observation, ToolDispatchError> {
        self.calls.lock().unwrap().push(call.call_id.clone());
        match call.name.as_str() {
            "slow" => {
                let cause = ctx.cancelled().await;
                Ok(Observation::Cancelled {
                    call_id: call.call_id,
                    reason: format!("command killed ({cause:?}); partial side effects possible"),
                    effect_unknown: false,
                })
            }
            "stubborn" => {
                let cause = ctx.cancelled_hard().await;
                Ok(Observation::Cancelled {
                    call_id: call.call_id,
                    reason: format!("abandoned while running ({cause:?})"),
                    effect_unknown: true,
                })
            }
            "brief" => {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(Observation::Success {
                    call_id: call.call_id,
                    content: json!("brief done"),
                    bytes: 10,
                    truncated: false,
                    tool_result: None,
                })
            }
            "bad_cancel" => Ok(Observation::Cancelled {
                call_id: call.call_id,
                reason: "nope".into(),
                effect_unknown: false,
            }),
            "no_task" => Ok(Observation::Pending {
                call_id: call.call_id,
                task_id: String::new(),
                until_ms: None,
                tool_result: None,
            }),
            _ => Ok(Observation::Success {
                call_id: call.call_id,
                content: json!("ok"),
                bytes: 2,
                truncated: false,
                tool_result: None,
            }),
        }
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        [
            "slow",
            "stubborn",
            "brief",
            "bad_cancel",
            "no_task",
            "a",
            "shell",
        ]
        .iter()
        .map(|n| ToolSpecLite {
            name: (*n).to_string(),
            description: String::new(),
            args_schema: json!({}),
        })
        .collect()
    }
}

fn fc_deps(llm: Arc<ScriptedRecordingLlm>, tools: Arc<CancelTools>) -> LLMContextDeps {
    LLMContextDeps::new(llm, tools)
}

fn behavior_deps(llm: Arc<ScriptedRecordingLlm>, tools: Arc<CancelTools>) -> LLMContextDeps {
    LLMContextDeps::new(llm, tools)
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()))
}

fn tool_results_of(messages: &[buckyos_api::AiMessage]) -> Vec<(String, String, bool)> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            AiContent::ToolResult {
                call_id,
                content,
                is_error,
            } => Some((
                call_id.clone(),
                content
                    .iter()
                    .filter_map(|p| match p {
                        buckyos_api::AiToolResultContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
                *is_error,
            )),
            _ => None,
        })
        .collect()
}

// ===================================================================
// Function call loop
// ===================================================================

#[tokio::test]
async fn interrupt_during_a_cancellable_tool_pairs_it_and_does_not_rerun_it() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("slow", "c1"), call("a", "c2")]),
        text_response("after"),
    ]));
    let tools = CancelTools::new();
    let mut ctx = LLMContext::new(base_request(), fc_deps(llm.clone(), tools.clone()));
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.interrupt("ctrl-c");
    });
    let outcome = ctx.run().await;
    let LLMContextOutcome::Interrupted {
        reason, snapshot, ..
    } = outcome
    else {
        panic!("expected Interrupted, got {outcome:?}");
    };
    assert_eq!(reason, "ctrl-c");
    assert_eq!(tools.calls(), vec!["c1"], "c2 never started");
    let results = tool_results_of(&snapshot.state.accumulated);
    assert_eq!(results.len(), 2, "both calls are paired");
    assert!(results[0].1.starts_with("[cancelled]"), "{results:?}");
    assert!(!results[0].2);
    assert!(results[1].1.starts_with("[not executed]"), "{results:?}");
    assert!(snapshot.state.tool_batch.is_none() && snapshot.state.suspended.is_none());

    // Resume: the cancelled tool does not run again, the LLM sees it.
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ResumeFromMidRun,
        fc_deps(llm.clone(), tools.clone()),
    )
    .unwrap();
    let LLMContextOutcome::Done { .. } = ctx.run().await else {
        panic!("expected Done")
    };
    assert_eq!(tools.calls(), vec!["c1"]);
    let seen = llm.seen();
    assert!(tool_result_text(&seen[1][2]).contains("[cancelled]"));
}

#[tokio::test]
async fn interrupt_during_a_non_cancellable_tool_abandons_it_with_unknown_effect() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("stubborn", "c1")],
    )]));
    let tools = CancelTools::new();
    let mut ctx = LLMContext::new(base_request(), fc_deps(llm, tools));
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.interrupt("ctrl-c");
    });
    let LLMContextOutcome::Interrupted { snapshot, .. } = ctx.run().await else {
        panic!("expected Interrupted")
    };
    let results = tool_results_of(&snapshot.state.accumulated);
    assert!(
        results[0].1.starts_with("[cancelled, result unknown]"),
        "{results:?}"
    );
    assert!(
        results[0].2,
        "an abandoned call is flagged like an unresolved one"
    );
}

#[tokio::test]
async fn wallclock_deadline_during_a_tool_exhausts_the_budget() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("slow", "c1")],
    )]));
    let tools = CancelTools::new();
    let mut req = base_request();
    req.budget.max_wallclock_ms = Some(200);
    let mut ctx = LLMContext::new(req, fc_deps(llm, tools));
    let started = std::time::Instant::now();
    let outcome = ctx.run().await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        matches!(
            outcome,
            LLMContextOutcome::BudgetExhausted {
                which: BudgetKind::Wallclock,
                ..
            }
        ),
        "{outcome:?}"
    );
    let results = tool_results_of(&ctx.snapshot().state.accumulated);
    assert!(results[0].1.contains("Deadline"), "{results:?}");
}

#[tokio::test]
async fn inline_cancelled_without_a_cause_is_a_contract_violation() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("bad_cancel", "c1"), call("a", "c2")],
    )]));
    let tools = CancelTools::new();
    let mut ctx = LLMContext::new(base_request(), fc_deps(llm, tools));
    let LLMContextOutcome::Error { error, trace, .. } = ctx.run().await else {
        panic!("expected Error")
    };
    assert!(matches!(error, LLMComputeError::Internal(ref m) if m.contains("Cancelled inline")));
    assert_eq!(
        statuses(&trace)
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>(),
        vec![ToolExecStatus::Unknown, ToolExecStatus::NotExecuted]
    );
}

#[tokio::test]
async fn pending_without_task_id_is_a_contract_violation() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("no_task", "c1")],
    )]));
    let tools = CancelTools::new();
    let mut req = base_request();
    req.tool_policy.allow_deferred = true;
    let mut ctx = LLMContext::new(req, fc_deps(llm, tools));
    let LLMContextOutcome::Error { error, .. } = ctx.run().await else {
        panic!("expected Error")
    };
    assert!(matches!(error, LLMComputeError::Internal(ref m) if m.contains("without a task_id")));
}

#[tokio::test]
async fn finish_during_inference_pairs_the_tool_calls_as_cancelled_and_settles() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("a", "c1"), call("a", "c2")]),
        text_response("after"),
    ]));
    let tools = CancelTools::new();
    let mut ctx = LLMContext::new(base_request(), fc_deps(llm.clone(), tools.clone()));
    // Finish before the first inference is paid for: the response still
    // arrives (scripted), its tool calls are not dispatched.
    ctx.interrupt_handle().finish("session stop");
    let outcome = ctx.run().await;
    let LLMContextOutcome::Settled {
        reason,
        snapshot,
        trace,
        ..
    } = outcome
    else {
        panic!("expected Settled, got {outcome:?}");
    };
    assert_eq!(reason, "session stop");
    assert!(tools.calls().is_empty(), "nothing dispatched");
    // A finish requested before any inference settles at once.
    assert_eq!(llm.seen().len(), 0);
    assert!(trace.tool_trace.is_empty());
    assert!(snapshot.state.suspended.is_none() && snapshot.state.tool_batch.is_none());

    // Finish requested while the inference runs (here: right after it).
    let llm2 = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("slow", "c1"), call("a", "c2")]),
        text_response("after"),
    ]));
    let tools2 = CancelTools::new();
    let mut ctx = LLMContext::new(base_request(), fc_deps(llm2.clone(), tools2.clone()));
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.finish("stop");
    });
    let LLMContextOutcome::Settled {
        snapshot, trace, ..
    } = ctx.run().await
    else {
        panic!("expected Settled")
    };
    assert_eq!(tools2.calls(), vec!["c1"]);
    let results = tool_results_of(&snapshot.state.accumulated);
    assert!(results[0].1.contains("Finishing"), "{results:?}");
    assert!(
        results[1]
            .1
            .contains("[cancelled] not executed: the run is finishing"),
        "{results:?}"
    );
    assert_eq!(
        statuses(&trace)
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>(),
        vec![ToolExecStatus::Cancelled, ToolExecStatus::NotExecuted]
    );
    // Resumable: nothing re-runs, the LLM continues.
    let mut ctx = LLMContext::resume(
        snapshot,
        ResumeFill::ResumeFromMidRun,
        fc_deps(llm2.clone(), tools2.clone()),
    )
    .unwrap();
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(tools2.calls(), vec!["c1"]);
}

#[tokio::test]
async fn finish_waits_for_a_non_cancellable_tool_within_the_grace_period() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("brief", "c1"), call("a", "c2")],
    )]));
    let tools = CancelTools::new();
    let mut req = base_request();
    req.tool_policy.finish_grace_ms = 5_000;
    let mut ctx = LLMContext::new(req, fc_deps(llm, tools.clone()));
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.finish("stop");
    });
    let LLMContextOutcome::Settled {
        snapshot, trace, ..
    } = ctx.run().await
    else {
        panic!("expected Settled")
    };
    let results = tool_results_of(&snapshot.state.accumulated);
    assert_eq!(results[0].1, "brief done", "the tool completed on its own");
    assert!(results[1].1.starts_with("[cancelled]"));
    assert_eq!(
        statuses(&trace)
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>(),
        vec![ToolExecStatus::Succeeded, ToolExecStatus::NotExecuted]
    );
}

#[tokio::test]
async fn finish_escalates_to_an_interrupt_after_the_grace_period() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("stubborn", "c1")],
    )]));
    let tools = CancelTools::new();
    let mut req = base_request();
    req.tool_policy.finish_grace_ms = 200;
    let mut ctx = LLMContext::new(req, fc_deps(llm, tools));
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.finish("stop");
    });
    let started = std::time::Instant::now();
    let outcome = ctx.run().await;
    assert!(started.elapsed() < Duration::from_secs(5));
    let LLMContextOutcome::Interrupted { snapshot, .. } = outcome else {
        panic!("expected Interrupted after escalation, got {outcome:?}")
    };
    let results = tool_results_of(&snapshot.state.accumulated);
    assert!(
        results[0].1.starts_with("[cancelled, result unknown]"),
        "{results:?}"
    );
}

// ===================================================================
// Behavior loop
// ===================================================================

#[tokio::test]
async fn behavior_interrupt_during_an_action_sediments_the_step_and_does_not_rerun_it() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(
            r#"<response><thinking>t</thinking><actions><shell>sleep 100</shell><shell>echo b</shell></actions></response>"#,
        ),
        text_response(
            "<response><thinking>saw</thinking><next_behavior>END</next_behavior></response>",
        ),
    ]));
    struct Route(Arc<CancelTools>);
    #[async_trait]
    impl ToolManager for Route {
        async fn call_tool(
            &self,
            mut c: AiToolCall,
            ctx: ToolCallCtx,
        ) -> Result<Observation, ToolDispatchError> {
            if c.call_id == "1" {
                c.name = "slow".into();
            }
            self.0.call_tool(c, ctx).await
        }
    }
    let tools = CancelTools::new();
    let mut req = base_request();
    req.behavior_name = "do".into();
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(Route(tools.clone())))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.interrupt("ctrl-c");
    });
    let LLMContextOutcome::Interrupted { snapshot, .. } = ctx.run().await else {
        panic!("expected Interrupted")
    };
    assert_eq!(tools.calls(), vec!["1"]);
    let step = snapshot.state.last_step.as_ref().expect("step sedimented");
    assert_eq!(step.action_results.len(), 2);
    assert!(matches!(
        step.action_results[0],
        Observation::Cancelled {
            effect_unknown: false,
            ..
        }
    ));
    assert!(matches!(
        step.action_results[1],
        Observation::Unresolved {
            effect_unknown: false,
            ..
        }
    ));
    assert!(snapshot.state.action_step.is_none());
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(Route(tools.clone())))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps).unwrap();
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(
        tools.calls(),
        vec!["1"],
        "the cancelled action is not re-run"
    );
    let seen = llm.seen();
    assert!(seen[1]
        .iter()
        .any(|m| m.role == AiRole::User && m.text_content().contains("Cancelled")));
}

#[tokio::test]
async fn behavior_finish_during_a_step_pairs_the_rest_and_settles() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(
            r#"<response><thinking>t</thinking><actions><shell>a</shell><shell>b</shell><shell>c</shell></actions></response>"#,
        ),
        text_response(
            "<response><thinking>saw</thinking><next_behavior>END</next_behavior></response>",
        ),
    ]));
    struct Route(Arc<CancelTools>);
    #[async_trait]
    impl ToolManager for Route {
        async fn call_tool(
            &self,
            mut c: AiToolCall,
            ctx: ToolCallCtx,
        ) -> Result<Observation, ToolDispatchError> {
            if c.call_id == "2" {
                c.name = "slow".into();
            }
            self.0.call_tool(c, ctx).await
        }
    }
    let tools = CancelTools::new();
    let mut req = base_request();
    req.behavior_name = "do".into();
    let mut ctx = LLMContext::new(req, behavior_deps(llm.clone(), tools.clone()));
    let _ = Route(tools.clone());
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(Route(tools.clone())))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    ctx = LLMContext::new(ctx.snapshot().request, deps);
    let handle = ctx.interrupt_handle();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.finish("stop");
    });
    let LLMContextOutcome::Settled { snapshot, .. } = ctx.run().await else {
        panic!("expected Settled")
    };
    assert_eq!(tools.calls(), vec!["1", "2"]);
    let step = snapshot.state.last_step.as_ref().expect("step sedimented");
    assert!(matches!(
        step.action_results[0],
        Observation::Success { .. }
    ));
    assert!(matches!(
        step.action_results[1],
        Observation::Cancelled {
            effect_unknown: false,
            ..
        }
    ));
    assert!(matches!(
        step.action_results[2],
        Observation::Cancelled {
            effect_unknown: false,
            ..
        }
    ));
}

// ===================================================================
// Background env
// ===================================================================

struct FakeTasks {
    watched: Mutex<Vec<String>>,
}

#[async_trait]
impl RunningTaskResolver for FakeTasks {
    async fn state(&self, _task_id: &str) -> TaskState {
        TaskState::Unknown {
            reason: "fake".into(),
        }
    }
    async fn wait(&self, task_id: &str, _until_ms: Option<u64>) -> TaskState {
        self.state(task_id).await
    }
    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported> {
        Err(CancelUnsupported {
            task_id: task_id.into(),
        })
    }
    fn watch(&self, task_id: &str) {
        self.watched.lock().unwrap().push(task_id.to_string());
    }
    async fn active(&self) -> Vec<TaskBrief> {
        self.watched
            .lock()
            .unwrap()
            .iter()
            .map(|id| TaskBrief {
                task_id: id.clone(),
                brief: "sleep 100".into(),
                status: "running".into(),
                elapsed_ms: 1000,
                last_line: String::new(),
                cancellable: true,
            })
            .collect()
    }
}

struct TaskTools;
#[async_trait]
impl ToolManager for TaskTools {
    async fn call_tool(
        &self,
        call: AiToolCall,
        _ctx: ToolCallCtx,
    ) -> Result<Observation, ToolDispatchError> {
        Ok(Observation::Success {
            call_id: call.call_id,
            content: json!("started"),
            bytes: 7,
            truncated: false,
            tool_result: Some(crate::observation::ToolResultView {
                task_id: Some("local:shell:1".into()),
                ..Default::default()
            }),
        })
    }
}

#[tokio::test]
async fn background_env_is_appended_to_the_request_not_to_the_history() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("shell", "c1")]),
        text_response("done"),
    ]));
    let tasks = Arc::new(FakeTasks {
        watched: Mutex::new(Vec::new()),
    });
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(TaskTools)).with_tasks(tasks.clone());
    let mut ctx = LLMContext::new(base_request(), deps);
    let LLMContextOutcome::Done { .. } = ctx.run().await else {
        panic!("expected Done")
    };
    assert_eq!(tasks.watched.lock().unwrap().as_slice(), ["local:shell:1"]);
    let seen = llm.seen();
    assert!(
        !seen[0]
            .iter()
            .any(|m| m.text_content().contains("<background_tasks>")),
        "no tasks before the first call"
    );
    let last = seen[1].last().unwrap();
    assert_eq!(last.role, AiRole::User);
    assert!(last
        .text_content()
        .contains("local:shell:1 [running] sleep 100 (1s)"));
    assert!(
        !ctx.snapshot()
            .state
            .accumulated
            .iter()
            .any(|m| m.text_content().contains("<background_tasks>")),
        "the env is not part of the history"
    );
}
