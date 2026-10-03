//! Budgets: tool iterations, Rounds, tokens, inner-context inheritance.

use std::collections::HashMap;
use std::sync::{Arc};

use buckyos_api::{AiMessage, AiResponse, AiRole, AiToolCall, AiUsage};

use crate::deps::{LLMContextDeps, ToolManager};
use crate::observation::{Observation};
use crate::outcome::{BudgetKind, ContextOutput, LLMContextOutcome};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

#[tokio::test]
async fn behavior_actions_consume_tool_iterations_even_on_business_failure() {
    for max_tool_iterations in [0, 2] {
        for fail in [false, true] {
            let response = text_response(
                "<response><actions><shell>echo action</shell></actions></response>",
            );
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
    for terminal_with_actions in [false, true] {
        let terminal = if terminal_with_actions {
            "<next_behavior>END</next_behavior>"
        } else {
            ""
        };
        let llm = Arc::new(ScriptedRecordingLlm::new(vec![
            text_response(&format!(
                "<response><actions><shell>echo a</shell><shell>echo b</shell></actions>{terminal}</response>"
            )),
            text_response("<response><next_behavior>END</next_behavior></response>"),
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
        assert_eq!(llm.seen().len(), if terminal_with_actions { 1 } else { 2 });
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
            text_response("<response><next_behavior>END</next_behavior></response>"),
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
        text_response("<response><next_behavior>END</next_behavior></response>"),
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
                "<response><thinking>go</thinking><actions><shell>echo a</shell></actions></response>",
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
