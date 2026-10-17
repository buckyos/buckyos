//! Reply contracts (`OutputSpec::Json { max_repairs: Some(_) }`): schema
//! and host validation of the final reply, bounded repairs, the raw reply
//! handed to the host when they run out, and a fork's own contract.

use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};
use serde_json::{json, Value};

use crate::deps::{LLMContextDeps, OutputValidator};
use crate::outcome::{ContextOutput, LLMContextOutcome};
use crate::request::{LLMContextRequest, OutputSpec};
use crate::{fork_snapshot, ForkOptions, LLMContext};

use super::mocks::*;

fn schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["reply"],
        "properties": {"reply": {"type": "string"}, "artifacts": {"type": "array", "items": {"type": "string"}}}
    })
}

fn contract_request() -> LLMContextRequest {
    let mut req = base_request();
    req.output = OutputSpec::Json {
        schema: Some(schema()),
        strict: true,
        max_repairs: Some(1),
    };
    req
}

fn last_text(messages: &[AiMessage]) -> String {
    messages.last().map(|m| m.text_content()).unwrap_or_default()
}

#[tokio::test]
async fn a_valid_reply_passes_at_once() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response(
        "```json\n{\"reply\": \"hi\"}\n```",
    )]));
    let mut ctx = LLMContext::new(
        contract_request(),
        LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)),
    );
    let LLMContextOutcome::Done { output, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(output, ContextOutput::Json { content: json!({"reply": "hi"}) });
    assert_eq!(llm.seen().len(), 1);
    assert_eq!(ctx.snapshot().state.output_repairs, 0);
}

#[tokio::test]
async fn a_mismatch_is_repaired_once() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(r#"{"answer": "hi"}"#),
        text_response(r#"{"reply": "hi"}"#),
    ]));
    let mut ctx = LLMContext::new(
        contract_request(),
        LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)),
    );
    let LLMContextOutcome::Done { output, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(output, ContextOutput::Json { content: json!({"reply": "hi"}) });
    let seen = llm.seen();
    assert_eq!(seen.len(), 2);
    // The failed reply stays in the transcript, followed by the reason.
    let repair = last_text(&seen[1]);
    assert!(repair.contains("missing required field `reply`"), "{repair}");
    assert!(repair.contains("JSON Schema"), "{repair}");
    assert_eq!(seen[1][seen[1].len() - 2].role, AiRole::Assistant);
    let state = ctx.snapshot().state;
    assert_eq!(state.output_repairs, 1);
    assert_eq!(state.consecutive_errors, 0, "repairs are not errors");
}

#[tokio::test]
async fn a_second_failure_returns_the_raw_reply() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response("plain words"),
        text_response("still plain words"),
        text_response(r#"{"reply": "never reached"}"#),
    ]));
    let mut ctx = LLMContext::new(
        contract_request(),
        LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)),
    );
    let LLMContextOutcome::Done { output, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    let ContextOutput::Invalid { content, error } = output else {
        panic!("expected Invalid, got {output:?}");
    };
    assert_eq!(content, "still plain words");
    assert!(error.contains("not one JSON value"), "{error}");
    assert_eq!(llm.seen().len(), 2, "at most one repair");
}

struct NoMissingFiles;

#[async_trait]
impl OutputValidator for NoMissingFiles {
    async fn validate(&self, reply: &Value) -> Result<(), String> {
        match reply["artifacts"].as_array() {
            Some(a) if a.iter().any(|f| f == "missing.txt") => {
                Err("artifact missing.txt does not exist".into())
            }
            _ => Ok(()),
        }
    }
}

#[tokio::test]
async fn the_host_validator_takes_part() {
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(r#"{"reply": "here", "artifacts": ["missing.txt"]}"#),
        text_response(r#"{"reply": "here, without the file"}"#),
    ]));
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(EchoTools))
        .with_output_validator(Arc::new(NoMissingFiles));
    let mut ctx = LLMContext::new(contract_request(), deps);
    let LLMContextOutcome::Done { output, .. } = ctx.run().await else {
        panic!("expected Done");
    };
    assert_eq!(
        output,
        ContextOutput::Json { content: json!({"reply": "here, without the file"}) }
    );
    assert!(last_text(&llm.seen()[1]).contains("artifact missing.txt does not exist"));
}

#[tokio::test]
async fn a_fork_replies_in_its_own_contract() {
    // The parent ran once under its contract.
    let llm = Arc::new(ScriptedRecordingLlm::new(vec![text_response(r#"{"reply": "hi"}"#)]));
    let mut parent = LLMContext::new(
        contract_request(),
        LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)),
    );
    assert!(matches!(parent.run().await, LLMContextOutcome::Done { .. }));
    let parent_snap = parent.snapshot();

    let branch_schema = json!({"type": "object", "required": ["outcome"], "properties": {"outcome": {"enum": ["direct"]}}});
    let derived = fork_snapshot(
        &parent_snap,
        ForkOptions {
            output: Some(OutputSpec::Json {
                schema: Some(branch_schema.clone()),
                strict: true,
                max_repairs: Some(1),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(parent_snap.request.output, contract_request().output, "the parent keeps its contract");
    assert_eq!(derived.snapshot.request.input[0], parent_snap.request.input[0]);

    let llm = Arc::new(ScriptedRecordingLlm::new(vec![
        text_response(r#"{"reply": "wrong contract"}"#),
        text_response(r#"{"outcome": "direct"}"#),
    ]));
    let mut branch = LLMContext::resume(
        derived.snapshot,
        crate::outcome::ResumeFill::ResumeFromMidRun,
        LLMContextDeps::new(llm.clone(), Arc::new(EchoTools)),
    )
    .unwrap();
    branch.inject(crate::deps::Injection {
        messages: vec![AiMessage::text(AiRole::User, "analyze")],
        host: None,
    });
    let LLMContextOutcome::Done { output, .. } = branch.run().await else {
        panic!("expected Done");
    };
    assert_eq!(output, ContextOutput::Json { content: json!({"outcome": "direct"}) });
    assert!(last_text(&llm.seen()[1]).contains("missing required field `outcome`"));
}
