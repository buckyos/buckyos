//! UI Sessions after the prompt rework (notepads/jarvis-chat-prompt-todo.md):
//! the chat input format, the structured reply, and a long command followed
//! through the task CLI bridge (`agent-session task wait`).

mod common;

use std::path::PathBuf;

use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use libopendan::SessionTemplate;
use serde_json::json;

fn ui_spec() -> libopendan::api::SessionSpec {
    let template = SessionTemplate::load("ui", None).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Abob"));
    spec.owner = Some("did:bns:bob".into());
    spec.outbound = Some(OutboundBinding {
        to: "did:bns:bob".into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec.prompt.llm_context = json!({
        "tools": {
            "enabled": true,
            "tools": [{ "groupname": "bash_core" }],
            "shell": { "mode": "auto", "wait_ms": 200 }
        },
        "json_schema": {
            "type": "object", "additionalProperties": false,
            "required": ["reply", "topic", "artifacts"],
            "properties": {
                "reply": { "type": "string" },
                "topic": { "type": ["object", "null"], "additionalProperties": false,
                           "required": ["title", "tags"],
                           "properties": { "title": { "type": "string" },
                                           "tags": { "type": "array", "items": { "type": "string" } } } },
                "artifacts": { "type": "array", "items": { "type": "string" } }
            }
        }
    });
    spec
}

fn shell(call_id: &str, command: &str) -> buckyos_api::AiResponse {
    tool_call(call_id, "shell", json!({ "command": command }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_followed_through_the_cli_is_not_watched_after_the_turn() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("run the slow job"));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            let system = render(&req.messages[..1]);
            assert!(system.contains("## Current Session Constraints"), "{system}");
            assert!(system.contains("one-to-one chat with your owner (bob, DID did:bns:bob)"), "{system}");
            assert!(!system.contains("## runtime_protocol"), "{system}");
            let input = last_user_text(req);
            assert!(input.starts_with("<<background_context>>\n- current_time "), "{input}");
            assert!(input.ends_with("<</background_context>>\nrun the slow job"), "{input}");
            let mut tools: Vec<&str> = req.tool_specs.iter().map(|t| t.name.as_str()).collect();
            tools.sort();
            assert_eq!(tools, ["edit_file", "read", "shell", "write_file"]);
            assert!(req.force_json && req.json_schema.is_some());
            shell("c1", "sleep 1; echo slow-done")
        }
        1 => {
            let r = has_tool_result(req, "c1").expect("c1 result");
            assert!(r.contains("agent-session task wait local:shell:c1"), "{r}");
            tool_call(
                "c2",
                "shell",
                json!({ "command": "agent-session task wait local:shell:c1 --wait-ms 10000", "wait_ms": 20000 }),
            )
        }
        2 => {
            let r = has_tool_result(req, "c2").expect("c2 result");
            assert!(r.contains("slow-done") && r.contains("exited with code 0"), "{r}");
            text(r#"{"reply": "the job printed slow-done", "topic": {"title": "slow job", "tags": ["job"]}, "artifacts": []}"#)
        }
        _ => panic!("unexpected call {n}: {}", last_user_text(req)),
    });
    let deps = env
        .deps(llm.clone())
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(st.watched_tasks.is_empty(), "{:?}", st.watched_tasks);
    assert_eq!(st.topic.title, "slow job");
    assert_eq!(st.topic.tags, ["job"]);
    // Nothing comes back for the task: no further Turn.
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 3);
    let report = read_worklog(&sd)
        .into_iter()
        .find_map(|e| match e.body {
            WorklogBody::Outcome { report, .. } => report,
            _ => None,
        })
        .unwrap();
    assert_eq!(report, "the job printed slow-done");
    // The background line of the runtime is not shown again.
    assert!(st.background_shown.contains_key("runtime"));
}
