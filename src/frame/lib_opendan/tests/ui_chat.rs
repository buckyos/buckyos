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

/// The id of the first TaskMgr task (`t-<32 hex>`) named in `text`.
fn zone_task_id(text: &str) -> Option<String> {
    let i = text.find("t-")?;
    let id: String = text[i..].chars().take(34).collect();
    (id.len() == 34 && id[2..].chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

/// A zone host: `shell` commands that outlive their wait are tasks of the
/// task service (`tool.shell/v1`), not of the runner process.
fn zone_host_deps(
    env: &Env,
    llm: std::sync::Arc<ScriptedLlm>,
) -> (
    libopendan::runner::RunnerDeps,
    std::sync::Arc<agent_tool::MemoryShellTaskBackend>,
) {
    use agent_tool::ShellTaskManager;
    let backend = agent_tool::MemoryShellTaskBackend::new();
    let mut deps = env
        .deps(llm)
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    let shell_tasks = agent_tool::ZoneShellTasks::new(
        backend.clone(),
        AGENT,
        Some("bob".into()),
        deps.xllm.local_tasks.clone(),
    );
    deps.xllm.buckyos_tasks = shell_tasks.resolver(None);
    deps.xllm.shell_tasks = Some(shell_tasks);
    (deps, backend)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zone_host_hands_long_commands_to_the_task_service() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("run the slow job"));
    let seen: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let seen_in_llm = seen.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => shell("c1", "sleep 1; echo slow-done"),
        1 => {
            let r = has_tool_result(req, "c1").expect("c1 result");
            let id = zone_task_id(&r).unwrap_or_else(|| panic!("no task id in {r}"));
            assert!(r.contains(&format!("agent-session task wait {id}")), "{r}");
            *seen_in_llm.lock().unwrap() = Some(id.clone());
            tool_call(
                "c2",
                "shell",
                json!({ "command": format!("agent-session task wait {id} --wait-ms 10000"), "wait_ms": 20000 }),
            )
        }
        2 => {
            let r = has_tool_result(req, "c2").expect("c2 result");
            assert!(r.contains("slow-done") && r.contains("exited with code 0"), "{r}");
            text(r#"{"reply": "the job printed slow-done", "topic": null, "artifacts": []}"#)
        }
        _ => panic!("unexpected call {n}: {}", last_user_text(req)),
    });
    let (deps, backend) = zone_host_deps(&env, llm.clone());
    drive(&sd, &deps, StopWhen::Idle).await;
    let id = seen.lock().unwrap().clone().expect("task id");
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(st.watched_tasks.is_empty(), "{:?}", st.watched_tasks);
    // The task belongs to this session, under the task of its Turn.
    let rec = backend.record(&id).expect("task in the task service");
    assert_eq!(rec.input["session_id"], sd.sid());
    assert_eq!(rec.input["call_id"], "c1");
    assert_eq!(rec.input["agent"], AGENT);
    for _ in 0..50 {
        if backend.record(&id).unwrap().terminal {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let rec = backend.record(&id).unwrap();
    assert_eq!(rec.outcome.as_deref(), Some("succeeded"), "{rec:?}");
    assert!(rec.result.unwrap()["output"].as_str().unwrap().contains("slow-done"));
    // Read already: nothing comes back for it.
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_still_running_after_the_turn_is_followed_by_the_next_drive() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("start the slow job"));
    let seen: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let seen_in_llm = seen.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => shell("c1", "sleep 3; echo later-done"),
        1 => {
            let r = has_tool_result(req, "c1").expect("c1 result");
            *seen_in_llm.lock().unwrap() = Some(zone_task_id(&r).expect("task id"));
            text(r#"{"reply": "started; I will report when it ends", "topic": null, "artifacts": []}"#)
        }
        2 => {
            let input = last_user_text(req);
            let id = seen_in_llm.lock().unwrap().clone().unwrap();
            assert!(input.contains(&format!("task:{id}:terminal")), "{input}");
            assert!(input.contains("later-done") && input.contains("exited with code 0"), "{input}");
            assert!(!input.contains("state unknown"), "{input}");
            text(r#"{"reply": "the job printed later-done", "topic": null, "artifacts": []}"#)
        }
        _ => panic!("unexpected call {n}: {}", last_user_text(req)),
    });
    let (deps, backend) = zone_host_deps(&env, llm.clone());
    // The Turn ends while the command runs: the session keeps following it.
    drive(&sd, &deps, StopWhen::Idle).await;
    let id = seen.lock().unwrap().clone().expect("task id");
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert_eq!(st.watched_tasks, [id.clone()]);
    assert_eq!(llm.count(), 2, "its end is not delivered by the Turn's drive");
    // Later drives (new runner state each) answer for the task through the
    // task service and deliver its end once.
    let started = std::time::Instant::now();
    while llm.count() < 3 && started.elapsed() < std::time::Duration::from_secs(15) {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drive(&sd, &deps, StopWhen::Idle).await;
    }
    assert_eq!(llm.count(), 3);
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 2);
    assert!(st.watched_tasks.is_empty(), "{:?}", st.watched_tasks);
    assert_eq!(backend.record(&id).unwrap().outcome.as_deref(), Some("succeeded"));
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 3, "delivered once");
}
