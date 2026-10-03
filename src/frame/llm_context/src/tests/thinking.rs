//! Thinking blocks are replayed verbatim while the prefix is untouched, and
//! dropped as a whole when an override rewrites it.

use std::sync::Arc;

use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, ProviderStateCoordinate};
use serde_json::json;

use crate::deps::LLMContextDeps;
use crate::outcome::LLMContextOutcome;
use crate::request::{ModelPolicy, ToolMode, ToolPolicy};
use crate::snapshot_overrides::{apply_overrides_to_snapshot, RequestOverrides};
use crate::state::LLMContextSnapshot;
use crate::{is_thinking, LLMContext, StepRenderer, XmlBehaviorParser, XmlStepRenderer};

use super::mocks::*;

fn thinking(text: &str, signature: &str) -> AiContent {
    AiContent::Thinking {
        source: ProviderStateCoordinate {
            provider_profile_id: "anthropic".into(),
            adapter_type: "claude-messages".into(),
            origin_provider: "anthropic".into(),
            origin_model: "claude-opus".into(),
        },
        summary: None,
        text: Some(text.into()),
        provider_metadata: Some(json!({ "signature": signature })),
    }
}

fn redacted() -> AiContent {
    AiContent::ProviderState {
        source: ProviderStateCoordinate::unbound(),
        provider: "claude".into(),
        value: json!({"type": "redacted_thinking", "data": "opaque"}),
    }
}

fn with_thinking(mut response: AiResponse, block: AiContent) -> AiResponse {
    response.message.content.insert(0, block);
    response
}

fn thinking_blocks(messages: &[AiMessage]) -> Vec<AiContent> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|c| is_thinking(c))
        .cloned()
        .collect()
}

fn snapshot_thinking(snapshot: &LLMContextSnapshot) -> usize {
    let state = &snapshot.state;
    thinking_blocks(&snapshot.request.input).len()
        + thinking_blocks(&state.accumulated).len()
        + state
            .steps
            .iter()
            .chain(state.last_step.iter())
            .filter_map(|s| s.assistant_message.as_ref())
            .map(|m| thinking_blocks(std::slice::from_ref(m)).len())
            .sum::<usize>()
}

#[tokio::test]
async fn function_call_loop_replays_earlier_thinking_verbatim() {
    let first = thinking("plan a", "sig-1");
    let second = thinking("plan c", "sig-2");
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        with_thinking(tool_response(None, vec![call("a", "c1")]), first.clone()),
        with_thinking(tool_response(None, vec![call("c", "c2")]), second.clone()),
        text_response("done"),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(ScriptedTools::new()));
    let mut ctx = LLMContext::new(base_request(), deps);
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));

    let seen = llm.seen();
    assert_eq!(seen.len(), 3);
    assert_eq!(thinking_blocks(&seen[1]), vec![first.clone()]);
    assert_eq!(thinking_blocks(&seen[2]), vec![first.clone(), second.clone()]);
    assert_eq!(
        serde_json::to_vec(&thinking_blocks(&seen[2])).unwrap(),
        serde_json::to_vec(&vec![first.clone(), second.clone()]).unwrap()
    );
    // The earlier request is a strict prefix of the later one.
    assert_eq!(seen[2][..seen[1].len()], seen[1][..]);

    let snapshot = ctx.snapshot();
    let restored: LLMContextSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    assert_eq!(
        thinking_blocks(&restored.state.accumulated),
        vec![first, second]
    );
    assert_eq!(restored.state.accumulated, snapshot.state.accumulated);
}

#[tokio::test]
async fn behavior_step_keeps_thinking_across_sediment() {
    let block = thinking("why shell", "sig-step");
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        with_thinking(
            text_response(
                r#"<response>
<thinking>run it</thinking>
<actions><shell>echo done</shell></actions>
</response>"#,
            ),
            block.clone(),
        ),
        text_response(
            "<response><thinking>next</thinking><actions><shell>echo again</shell></actions></response>",
        ),
        text_response("<response><thinking>done</thinking><next_behavior>END</next_behavior></response>"),
    ]));
    let mut req = base_request();
    req.behavior_name = "plan".into();
    let renderer = Arc::new(XmlStepRenderer::new().without_timestamps());
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(renderer.clone());
    let mut ctx = LLMContext::new(req, deps);
    assert!(matches!(ctx.run().await, LLMContextOutcome::Done { .. }));

    let seen = llm.seen();
    assert_eq!(seen.len(), 3);
    // Second request: the step is the hot `last_step`; third: sedimented.
    let hot: Vec<&AiMessage> = seen[1]
        .iter()
        .filter(|m| m.content.contains(&block))
        .collect();
    let sedimented: Vec<&AiMessage> = seen[2]
        .iter()
        .filter(|m| m.content.contains(&block))
        .collect();
    assert_eq!(hot.len(), 1);
    assert_eq!(hot, sedimented);

    let snapshot = ctx.snapshot();
    let restored: LLMContextSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    let step = &restored.state.steps[0];
    assert_eq!(&renderer.render(step).0, hot[0]);
    assert_eq!(
        step.assistant_message.as_ref().unwrap().content[0],
        block
    );
}

fn snapshot_with_thinking() -> LLMContextSnapshot {
    let assistant = AiMessage::new(
        AiRole::Assistant,
        vec![
            thinking("hmm", "sig"),
            redacted(),
            AiContent::Text {
                text: "answer".into(),
            },
        ],
    );
    let mut req = base_request();
    req.input = vec![
        AiMessage::text(AiRole::System, "sys"),
        AiMessage::text(AiRole::User, "hello"),
    ];
    let mut ctx = LLMContext::new(
        req,
        LLMContextDeps::new(
            Arc::new(ScriptedLlm::new(vec![])),
            Arc::new(ScriptedTools::new()),
        ),
    )
    .snapshot();
    ctx.state.accumulated.push(assistant.clone());
    let mut step = crate::behavior_loop::StepRecord::default();
    step.assistant_message = Some(assistant.clone());
    ctx.state.steps.push(step.clone());
    step.meta.step_index = 1;
    ctx.state.last_step = Some(step);
    ctx
}

#[test]
fn overrides_that_change_the_prefix_drop_all_thinking() {
    let base = snapshot_with_thinking();
    assert_eq!(snapshot_thinking(&base), 6);

    let changing = vec![
        RequestOverrides {
            system_messages: Some(vec![AiMessage::text(AiRole::System, "other sys")]),
            ..Default::default()
        },
        RequestOverrides {
            user_messages: Some(vec![
                AiMessage::text(AiRole::User, "parent"),
                AiMessage::new(AiRole::Assistant, vec![thinking("p", "s"), AiContent::text("a")]),
            ]),
            ..Default::default()
        },
        RequestOverrides {
            tool_policy: Some(ToolPolicy {
                mode: ToolMode::Whitelist,
                whitelist: vec!["a".into()],
                ..base.request.tool_policy.clone()
            }),
            ..Default::default()
        },
        RequestOverrides {
            model_policy: Some(ModelPolicy {
                preferred: "other-model".into(),
                ..base.request.model_policy.clone()
            }),
            ..Default::default()
        },
        RequestOverrides {
            behavior_name: Some("execute".into()),
            ..Default::default()
        },
    ];
    for ov in changing {
        let label = format!("{ov:?}");
        let out = apply_overrides_to_snapshot(base.clone(), ov);
        assert_eq!(snapshot_thinking(&out), 0, "{label}");
        // Text of the stripped messages survives.
        assert!(out
            .state
            .accumulated
            .iter()
            .any(|m| m.role == AiRole::Assistant));
    }
}

#[test]
fn overrides_that_keep_the_prefix_keep_thinking() {
    let base = snapshot_with_thinking();
    let keeping = vec![
        RequestOverrides {
            system_messages: Some(vec![AiMessage::text(AiRole::System, "sys")]),
            ..Default::default()
        },
        RequestOverrides {
            tool_policy: Some(ToolPolicy {
                max_tool_iterations: 99,
                ..base.request.tool_policy.clone()
            }),
            reset_tool_iterations: true,
            ..Default::default()
        },
        RequestOverrides {
            model_policy: Some(ModelPolicy {
                temperature: Some(0.3),
                ..base.request.model_policy.clone()
            }),
            ..Default::default()
        },
        RequestOverrides {
            behavior_name: Some(base.request.behavior_name.clone()),
            ..Default::default()
        },
        RequestOverrides {
            objective: Some("other".into()),
            trace: Some(Some("t::fork".into())),
            budget: Some(Default::default()),
            human_policy: Some(Default::default()),
            error_policy: Some(Default::default()),
            output: Some(Default::default()),
            reset_errors: true,
            reset_behavior_hot_tail: true,
            ..Default::default()
        },
    ];
    for ov in keeping {
        let label = format!("{ov:?}");
        let out = apply_overrides_to_snapshot(base.clone(), ov);
        assert_eq!(snapshot_thinking(&out), 6, "{label}");
    }
}
