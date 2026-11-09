//! Budgets: tool iterations, Rounds, tokens, inner-context inheritance,
//! wrap-up mode.

use std::collections::HashMap;
use std::sync::Arc;

use buckyos_api::{AiMessage, AiResponse, AiRole, AiToolCall, AiUsage};
use serde_json::json;

use crate::deps::{LLMContextDeps, ToolCallCtx, ToolDispatchError, ToolManager, ToolSpecLite};
use crate::interrupt::LLMContextInterruptHandle;
use crate::observation::Observation;
use crate::outcome::{BudgetKind, ContextOutput, LLMContextOutcome, ResumeFill};
use crate::tests::protocol::{XmlBehaviorParser, XmlStepRenderer};
use crate::LLMContext;

use super::mocks::*;

#[tokio::test]
async fn behavior_actions_consume_tool_iterations_even_on_business_failure() {
    for max_tool_iterations in [0, 2] {
        for fail in [false, true] {
            let response =
                text_response("<response><actions><shell>echo action</shell></actions></response>");
            let llm = Arc::new(ScriptedRecordingLlm::new(vec![
                response;
                max_tool_iterations as usize
                    + 1
            ]));
            let tools: Arc<dyn ToolManager> = if fail {
                Arc::new(FailingTools)
            } else {
                Arc::new(EchoTools)
            };
            let mut req = base_request();
            req.tool_policy.max_tool_iterations = max_tool_iterations;
            let deps = LLMContextDeps::new(llm.clone(), tools)
                .with_result_parser(Arc::new(XmlBehaviorParser::new()))
                .with_step_renderer(Arc::new(XmlStepRenderer::new()));
            let mut ctx = LLMContext::new(req, deps);
            let outcome = ctx.run().await;
            assert!(
                matches!(
                    outcome,
                    LLMContextOutcome::BudgetExhausted {
                        which: BudgetKind::ToolIterations,
                        ..
                    }
                ),
                "max_tool_iterations={max_tool_iterations}, fail={fail}: {outcome:?}"
            );
            let state = ctx.snapshot().state;
            assert_eq!(state.tool_iterations_left, 0);
            let steps: Vec<_> = state.steps.iter().chain(state.last_step.iter()).collect();
            assert_eq!(steps.len(), max_tool_iterations as usize);
            for step in steps {
                assert_eq!(step.action_results.len(), 1);
                assert_eq!(
                    matches!(step.action_results[0], Observation::Error { .. }),
                    fail
                );
            }
            assert_eq!(llm.seen().len(), max_tool_iterations as usize + 1);
        }
    }
}

#[tokio::test]
async fn behavior_action_batch_uses_one_tool_iteration_and_allows_final_response() {
    for report_with_actions in [false, true] {
        let terminal = if report_with_actions {
            "<report>working</report>"
        } else {
            ""
        };
        let llm = Arc::new(ScriptedRecordingLlm::new(vec![
            text_response(&format!(
                "<response><actions><shell>echo a</shell><shell>echo b</shell></actions>{terminal}</response>"
            )),
            text_response("<response><report end=\"true\">finished</report></response>"),
        ]));
        let mut req = base_request();
        req.tool_policy.max_tool_iterations = 1;
        let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
            .with_result_parser(Arc::new(XmlBehaviorParser::new()))
            .with_step_renderer(Arc::new(XmlStepRenderer::new()));
        let mut ctx = LLMContext::new(req, deps);
        let outcome = ctx.run().await;
        let LLMContextOutcome::Done { trace, .. } = outcome else {
            panic!("expected Done, got {outcome:?}");
        };
        assert_eq!(trace.tool_trace.len(), 2);
        assert_eq!(ctx.snapshot().state.tool_iterations_left, 0);
        assert_eq!(llm.seen().len(), 2);
    }
}

#[tokio::test]
async fn behavior_native_tools_and_actions_share_tool_iterations_across_steps() {
    let native = tool_response(
        None,
        vec![AiToolCall {
            name: "echo".into(),
            args: HashMap::new(),
            call_id: "native".into(),
        }],
    );
    let action =
        text_response("<response><actions><shell>echo action</shell></actions></response>");
    for max_tool_iterations in [1, 2, 3, 4] {
        let llm = Arc::new(ScriptedRecordingLlm::new(vec![
            native.clone(),
            action.clone(),
            native.clone(),
            action.clone(),
            text_response("<response><report end=\"true\">finished</report></response>"),
        ]));
        let mut req = base_request();
        req.tool_policy.max_tool_iterations = max_tool_iterations;
        let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
            .with_result_parser(Arc::new(XmlBehaviorParser::new()))
            .with_step_renderer(Arc::new(XmlStepRenderer::new()));
        let mut ctx = LLMContext::new(req, deps);
        let outcome = ctx.run().await;
        if max_tool_iterations == 4 {
            let LLMContextOutcome::Done { trace, .. } = outcome else {
                panic!("expected Done, got {outcome:?}");
            };
            assert_eq!(trace.tool_trace.len(), 4);
        } else {
            assert!(
                matches!(
                    outcome,
                    LLMContextOutcome::BudgetExhausted {
                        which: BudgetKind::ToolIterations,
                        ..
                    }
                ),
                "max_tool_iterations={max_tool_iterations}: {outcome:?}"
            );
        }
        assert_eq!(ctx.snapshot().state.tool_iterations_left, 0);
        assert_eq!(llm.seen().len(), max_tool_iterations as usize + 1);
    }
}

#[tokio::test]
async fn function_call_loop_counts_rounds_and_tool_iterations_separately() {
    let call = |id: &str| AiToolCall {
        name: "echo".into(),
        args: HashMap::new(),
        call_id: id.into(),
    };
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("c1")]),
        tool_response(None, vec![call("c2"), call("c3")]),
        text_response("final answer"),
    ]));
    let mut req = base_request();
    req.tool_policy.max_tool_iterations = 5;
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools));
    let mut ctx = LLMContext::new(req, deps);
    let outcome = ctx.run().await;
    let LLMContextOutcome::Done { output, trace, .. } = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    assert_eq!(
        output,
        ContextOutput::Text {
            content: "final answer".into()
        }
    );
    // 3 Rounds (two tool-call responses + the final answer), 2 tool
    // iterations (one per batch, whatever its size), no behavior Step.
    assert_eq!(llm.seen().len(), 3);
    assert_eq!(trace.tool_trace.len(), 3);
    let state = ctx.snapshot().state;
    assert_eq!(state.tool_iterations_left, 3);
    assert!(state.steps.is_empty() && state.last_step.is_none());
}

#[tokio::test]
async fn behavior_step_spans_rounds_and_holds_several_actions() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        // Step 0, Round 1: a native tool call inside the step's inner loop.
        tool_response(
            None,
            vec![AiToolCall {
                name: "echo".into(),
                args: HashMap::new(),
                call_id: "native".into(),
            }],
        ),
        // Step 0, Round 2: the decision with two actions.
        text_response(
            "<response><actions><shell>echo a</shell><shell>echo b</shell></actions></response>",
        ),
        // Step 1, Round 3: terminal decision.
        text_response("<response><report end=\"true\">finished</report></response>"),
    ]));
    let mut req = base_request();
    req.tool_policy.max_tool_iterations = 5;
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    let mut ctx = LLMContext::new(req, deps);
    let outcome = ctx.run().await;
    let LLMContextOutcome::Done { trace, .. } = outcome else {
        panic!("expected Done, got {outcome:?}");
    };
    assert_eq!(llm.seen().len(), 3, "three Rounds");
    assert_eq!(trace.tool_trace.len(), 3, "one native call + two actions");
    let state = ctx.snapshot().state;
    assert_eq!(state.steps.len(), 2, "two Steps");
    assert_eq!(state.steps[0].meta.step_index, 0);
    assert_eq!(state.steps[0].actions.len(), 2, "both actions in one Step");
    assert_eq!(state.steps[0].action_results.len(), 2);
    assert!(state.steps[1].actions.is_empty());
    // One native batch + one action step; the Rounds are not charged.
    assert_eq!(state.tool_iterations_left, 3);
}

#[tokio::test]
async fn behavior_inner_context_inherits_outer_budget() {
    let usage = |total: u64| {
        Some(AiUsage {
            input_tokens: None,
            output_tokens: None,
            total_tokens: Some(total),
            ..Default::default()
        })
    };
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        AiResponse {
            message: AiMessage::text(
                AiRole::Assistant,
                "<response><intent>go</intent><actions><shell>echo a</shell></actions></response>",
            ),
            usage: usage(8),
            ..Default::default()
        },
        AiResponse {
            message: AiMessage::text(
                AiRole::Assistant,
                "<response><intent>done</intent><report end=\"true\">finished</report></response>",
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

// ---------------------------------------------------------------------------
// Wrap-up mode (`ToolPolicy.wrap_up_tools`)
// ---------------------------------------------------------------------------

/// `shell` / `report` / `ask_user`. `report` with `is_end` finishes the run
/// through the interrupt handle, as a Session host accepting a final report
/// does; `ask_user` answers with a running task (not an end).
#[derive(Default)]
struct WrapUpTools {
    handle: std::sync::Mutex<Option<LLMContextInterruptHandle>>,
    calls: std::sync::Mutex<Vec<String>>,
}

impl WrapUpTools {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ToolManager for WrapUpTools {
    async fn call_tool(
        &self,
        call: AiToolCall,
        _ctx: ToolCallCtx,
    ) -> Result<Observation, ToolDispatchError> {
        self.calls.lock().unwrap().push(call.name.clone());
        let content = match call.name.as_str() {
            "report" => {
                let is_end = call.args.get("is_end") == Some(&json!(true));
                if is_end {
                    if let Some(handle) = self.handle.lock().unwrap().as_ref() {
                        handle.finish("accepted final report");
                    }
                }
                json!({"accepted": true, "is_end": is_end})
            }
            "ask_user" => json!({"task_id": "t-1", "state": "running"}),
            _ => json!("ok"),
        };
        Ok(Observation::Success {
            call_id: call.call_id,
            content,
            bytes: 0,
            truncated: false,
            host: None,
            images: Vec::new(),
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        ["shell", "report", "ask_user"]
            .iter()
            .map(|name| ToolSpecLite {
                name: (*name).to_string(),
                description: String::new(),
                args_schema: json!({}),
            })
            .collect()
    }
}

fn named_call(name: &str, id: &str, args: serde_json::Value) -> AiToolCall {
    AiToolCall {
        name: name.into(),
        args: serde_json::from_value(args).unwrap(),
        call_id: id.into(),
    }
}

fn wrap_up_request(wrap_up_tools: &[&str]) -> crate::LLMContextRequest {
    let mut req = base_request();
    req.tool_policy.max_tool_iterations = 1;
    req.tool_policy.wrap_up_tools = wrap_up_tools.iter().map(|s| s.to_string()).collect();
    req
}

fn offer(names: &[&str]) -> Option<Vec<String>> {
    Some(names.iter().map(|s| s.to_string()).collect())
}

/// Run `req` with `WrapUpTools`, the report tool wired to the context's
/// interrupt handle.
async fn run_wrap_up(
    req: crate::LLMContextRequest,
    llm: Arc<ScriptedRecordingLlm>,
    behavior: bool,
) -> (LLMContextOutcome, LLMContext, Arc<WrapUpTools>) {
    let tools = Arc::new(WrapUpTools::default());
    let mut deps = LLMContextDeps::new(llm, tools.clone());
    if behavior {
        deps = deps
            .with_result_parser(Arc::new(XmlBehaviorParser::new()))
            .with_step_renderer(Arc::new(XmlStepRenderer::new()));
    }
    let mut ctx = LLMContext::new(req, deps);
    *tools.handle.lock().unwrap() = Some(ctx.interrupt_handle());
    let outcome = ctx.run().await;
    (outcome, ctx, tools)
}

#[tokio::test]
async fn used_up_iterations_leave_only_the_wrap_up_tools() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("shell", "c1")]),
        tool_response(
            None,
            vec![named_call("report", "c2", json!({"report": "done", "is_end": true}))],
        ),
    ]));
    let (outcome, ctx, tools) = run_wrap_up(wrap_up_request(&["report"]), llm.clone(), false).await;
    let LLMContextOutcome::Settled { reason, .. } = outcome else {
        panic!("expected the accepted report to end the run, got {outcome:?}");
    };
    assert_eq!(reason, "accepted final report");
    assert_eq!(tools.calls(), ["shell", "report"]);
    // Round 1 offers everything; once the iteration is spent, only `report`.
    assert_eq!(
        llm.offers(),
        [offer(&["shell", "report", "ask_user"]), offer(&["report"])]
    );
    let state = ctx.snapshot().state;
    assert_eq!(state.tool_iterations_left, 0);
    assert_eq!(state.wrap_up_rounds_used, 1);
}

#[tokio::test]
async fn other_tools_in_wrap_up_mode_exhaust_the_budget() {
    for batch in [
        vec![call("shell", "c2")],
        vec![
            named_call("report", "c2", json!({"report": "done", "is_end": true})),
            call("shell", "c3"),
        ],
    ] {
        let llm = Arc::new(ScriptedRecordingLlm::new(vec![
            tool_response(None, vec![call("shell", "c1")]),
            tool_response(Some("one more look"), batch.clone()),
        ]));
        let (outcome, ctx, tools) =
            run_wrap_up(wrap_up_request(&["report"]), llm.clone(), false).await;
        let LLMContextOutcome::BudgetExhausted { which, partial, .. } = outcome else {
            panic!("expected BudgetExhausted for {batch:?}, got {outcome:?}");
        };
        assert_eq!(which, BudgetKind::ToolIterations);
        assert_eq!(
            partial,
            Some(ContextOutput::Text {
                content: "one more look".into()
            })
        );
        assert_eq!(tools.calls(), ["shell"], "nothing of the batch ran");
        assert_eq!(ctx.snapshot().state.wrap_up_rounds_used, 0);
    }
}

#[tokio::test]
async fn wrap_up_rounds_bound_the_wrap_up_batches() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("shell", "c1")]),
        tool_response(None, vec![named_call("ask_user", "c2", json!({"question": "which?"}))]),
        tool_response(
            None,
            vec![named_call("report", "c3", json!({"report": "done", "is_end": true}))],
        ),
    ]));
    let mut req = wrap_up_request(&["report", "ask_user"]);
    req.tool_policy.wrap_up_rounds = 1;
    let (outcome, ctx, tools) = run_wrap_up(req, llm.clone(), false).await;
    assert!(
        matches!(
            outcome,
            LLMContextOutcome::BudgetExhausted {
                which: BudgetKind::ToolIterations,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(tools.calls(), ["shell", "ask_user"]);
    assert_eq!(
        llm.offers(),
        [
            offer(&["shell", "report", "ask_user"]),
            offer(&["report", "ask_user"]),
            None,
        ]
    );
    assert_eq!(ctx.snapshot().state.wrap_up_rounds_used, 1);
}

#[tokio::test]
async fn without_wrap_up_tools_used_up_iterations_offer_nothing() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        tool_response(None, vec![call("shell", "c1")]),
        tool_response(
            None,
            vec![named_call("report", "c2", json!({"report": "done", "is_end": true}))],
        ),
    ]));
    let req = wrap_up_request(&[]);
    // The default policy serializes exactly as before the wrap-up fields.
    let policy = serde_json::to_value(&req.tool_policy).unwrap();
    assert!(policy.get("wrap_up_tools").is_none() && policy.get("wrap_up_rounds").is_none());
    let (outcome, ctx, tools) = run_wrap_up(req, llm.clone(), false).await;
    assert!(
        matches!(
            outcome,
            LLMContextOutcome::BudgetExhausted {
                which: BudgetKind::ToolIterations,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(tools.calls(), ["shell"]);
    assert_eq!(llm.offers(), [offer(&["shell", "report", "ask_user"]), None]);
    let state = serde_json::to_value(ctx.snapshot().state).unwrap();
    assert!(state.get("wrap_up_rounds_used").is_none());
}

#[tokio::test]
async fn behavior_wrap_up_steps_run_only_wrap_up_actions() {
    // A wrap-up step (`ask_user`) runs without an iteration; the report
    // step needs none.
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response("<response><actions><shell>echo a</shell></actions></response>"),
        text_response("<response><actions><ask_user>which one?</ask_user></actions></response>"),
        text_response("<response><report end=\"true\">finished</report></response>"),
    ]));
    let (outcome, ctx, tools) =
        run_wrap_up(wrap_up_request(&["ask_user"]), llm.clone(), true).await;
    assert!(matches!(outcome, LLMContextOutcome::Done { .. }), "{outcome:?}");
    assert_eq!(tools.calls(), ["shell", "ask_user"]);
    // The step's inner context shares the outer budget state: native tools
    // are filtered to the wrap-up set as well.
    assert_eq!(
        llm.offers(),
        [
            offer(&["shell", "report", "ask_user"]),
            offer(&["ask_user"]),
            offer(&["ask_user"]),
        ]
    );
    let state = ctx.snapshot().state;
    assert_eq!(state.tool_iterations_left, 0);
    assert_eq!(state.wrap_up_rounds_used, 1);

    // Any other action exhausts the budget, the step's text as partial.
    let other = "<response><actions><ask_user>q</ask_user><shell>echo b</shell></actions></response>";
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response("<response><actions><shell>echo a</shell></actions></response>"),
        text_response(other),
    ]));
    let (outcome, ctx, tools) =
        run_wrap_up(wrap_up_request(&["ask_user"]), llm.clone(), true).await;
    let LLMContextOutcome::BudgetExhausted { which, partial, .. } = outcome else {
        panic!("expected BudgetExhausted, got {outcome:?}");
    };
    assert_eq!(which, BudgetKind::ToolIterations);
    assert_eq!(partial, Some(ContextOutput::Text { content: other.into() }));
    assert_eq!(tools.calls(), ["shell"]);
    assert_eq!(ctx.snapshot().state.wrap_up_rounds_used, 0);
}

#[tokio::test]
async fn wrap_up_state_survives_snapshots() {
    // A snapshot written before the field existed resumes with 0 rounds used.
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![tool_response(
        None,
        vec![call("shell", "c1")],
    )]));
    let (_, ctx, _) = run_wrap_up(wrap_up_request(&["report"]), llm, false).await;
    let mut old = serde_json::to_value(ctx.snapshot()).unwrap();
    assert!(old["state"].get("wrap_up_rounds_used").is_none());
    assert_eq!(old["request"]["tool_policy"]["wrap_up_tools"], json!(["report"]));
    assert!(old["request"]["tool_policy"].get("wrap_up_rounds").is_none());
    old["request"]["tool_policy"]
        .as_object_mut()
        .unwrap()
        .remove("wrap_up_tools");
    let old: crate::LLMContextSnapshot = serde_json::from_value(old).unwrap();
    assert!(old.request.tool_policy.wrap_up_tools.is_empty());
    assert_eq!(old.request.tool_policy.wrap_up_rounds, crate::DEFAULT_WRAP_UP_ROUNDS);
    assert_eq!(old.state.wrap_up_rounds_used, 0);

    // A host that zeroes the iterations of a snapshot (a wrap-up after the
    // wallclock ran out) resumes straight into wrap-up mode.
    let mut snap = ctx.snapshot();
    snap.state.tool_iterations_left = 0;
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response("summary")]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(WrapUpTools::default()));
    let mut resumed = LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, deps).unwrap();
    assert!(matches!(resumed.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(llm.offers(), [offer(&["report"])]);

    // Rounds used are carried: none left, no tools offered.
    let mut snap = ctx.snapshot();
    snap.state.tool_iterations_left = 0;
    snap.state.wrap_up_rounds_used = 2;
    let snap: crate::LLMContextSnapshot =
        serde_json::from_value(serde_json::to_value(&snap).unwrap()).unwrap();
    assert_eq!(snap.state.wrap_up_rounds_used, 2);
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response("summary")]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(WrapUpTools::default()));
    let mut resumed = LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, deps).unwrap();
    assert!(matches!(resumed.run().await, LLMContextOutcome::Done { .. }));
    assert_eq!(llm.offers(), [None]);
}
