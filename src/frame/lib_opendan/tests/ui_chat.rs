//! UI Sessions after the prompt rework (notepads/jarvis-chat-prompt-todo.md):
//! the chat input format, the structured reply, and a long command followed
//! through the task CLI bridge (`agent-session task wait`).

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{
    compose_reply, drive, DriveResult, OutboundSink, SendResult, StopWhen, TurnReply,
};
use libopendan::SessionTemplate;
use ndn_lib::MsgObject;
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

#[derive(Default)]
struct ReplySink {
    replies: Mutex<Vec<TurnReply>>,
}

#[async_trait]
impl OutboundSink for ReplySink {
    async fn compose(
        &self,
        _cfg: &SessionConfig,
        base: MsgObject,
        reply: &TurnReply,
        objects: Option<&dyn libopendan::runner::ObjectStore>,
    ) -> libopendan::Result<Option<MsgObject>> {
        self.replies.lock().unwrap().push(reply.clone());
        if reply.status != TurnStatus::Completed {
            return Ok(None);
        }
        compose_reply(base, reply, objects).await
    }

    async fn send(&self, _sid: &str, _record: &OutboundRecord) -> SendResult {
        SendResult::Sent {
            msg_id: None,
            deliveries: vec![],
        }
    }
}

#[tokio::test]
async fn structured_reply_repair_delivers_only_the_answer() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("hello"));
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("plain answer"),
        1 => {
            assert!(last_user_text(req).contains("Your reply does not meet the required format"));
            text(r#"{"reply":"你好","topic":{"title":"问候","tags":["chat"]},"artifacts":[]}"#)
        }
        _ => panic!("unexpected inference"),
    });
    let sink = Arc::new(ReplySink::default());
    let mut deps = env.deps(llm.clone());
    deps.outbound = Some(sink.clone());
    let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    assert_eq!(llm.count(), 2);
    let state = sd.state().unwrap();
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.outbox[0].msg.content.content, "你好");
    assert_eq!(state.topic.title, "问候");
    assert_eq!(state.topic.tags, ["chat"]);
    assert_eq!(sink.replies.lock().unwrap()[0].answer.as_deref(), Some("你好"));
}

#[tokio::test]
async fn structured_reply_failure_never_delivers_raw_output() {
    for raw in [
        "```json\n{broken json\n```",
        r#"{"reply":"visible text","topic":null,"artifacts":[],"extra":"invalid"}"#,
        r#"{"reply":"attached","topic":null,"artifacts":["missing.txt"]}"#,
    ] {
        let env = Env::new();
        let sd = env.create_work(ui_spec()).await;
        post_blocking(&env.queue_dir, &queue_of(&sd), msg("hello"));
        let llm = ScriptedLlm::new(move |_, n| {
            if n < 2 {
                text(raw)
            } else {
                text(r#"{"reply":"recovered","topic":null,"artifacts":[]}"#)
            }
        });
        let sink = Arc::new(ReplySink::default());
        let mut deps = env.deps(llm.clone());
        deps.outbound = Some(sink.clone());
        let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Failed, answer: None, .. }), "{result:?}");
        assert_eq!(llm.count(), 2);
        let state = sd.state().unwrap();
        assert_eq!(state.last_error.as_ref().unwrap()["kind"], "output_format");
        assert_eq!(state.turns_completed, 0);
        assert!(state.outbox.is_empty());
        assert!(state.topic.title.is_empty());
        let replies = sink.replies.lock().unwrap();
        assert_eq!(replies[0].status, TurnStatus::Failed);
        assert!(replies[0].answer.is_none());
        assert!(replies[0].artifacts.is_empty());
        drop(replies);
        assert!(read_worklog(&sd).iter().any(|e| matches!(&e.body, WorklogBody::AssistantMessage { assistant, .. } if assistant == raw)));
        post_blocking(&env.queue_dir, &queue_of(&sd), msg("try again"));
        drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert_eq!(sd.state().unwrap().outbox[0].msg.content.content, "recovered");
        assert_eq!(llm.count(), 3);
    }
}

#[tokio::test]
async fn structured_reply_empty_answer_does_not_fall_back_to_json() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("hello"));
    let llm = ScriptedLlm::new(|_, _| text(r#"{"reply":"","topic":null,"artifacts":[]}"#));
    let sink = Arc::new(ReplySink::default());
    let mut deps = env.deps(llm.clone());
    deps.outbound = Some(sink.clone());
    let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    assert_eq!(llm.count(), 1);
    assert_eq!(sink.replies.lock().unwrap()[0].answer.as_deref(), Some(""));
    assert!(sd.state().unwrap().outbox.is_empty());
}

#[tokio::test]
async fn structured_reply_recovery_uses_the_same_delivery_rules() {
    for (raw, expected) in [
        (r#"{"reply":"recovered","topic":{"title":"topic","tags":["tag"]},"artifacts":[]}"#, Some("recovered")),
        (r#"{"reply":"","topic":null,"artifacts":[]}"#, Some("")),
        (r#"{"reply":"invalid","topic":null,"artifacts":[],"extra":1}"#, None),
        (r#"{"reply":"attached","topic":null,"artifacts":["missing.txt"]}"#, None),
    ] {
        let env = Env::new();
        let sd = env.create_work(ui_spec()).await;
        post_blocking(&env.queue_dir, &queue_of(&sd), msg("hello"));
        let llm = ScriptedLlm::fallible(|_, _| Err(llm_context::LLMComputeError::Timeout));
        let sink = Arc::new(ReplySink::default());
        let mut deps = env.deps(llm.clone());
        deps.outbound = Some(sink.clone());
        drive(&sd, &deps, StopWhen::MaxOutcomes { n: 1 }).await;
        let run_id = sd.state().unwrap().live_run.unwrap().run_id;
        let runs = sd.runs();
        let mut record = runs.record(&run_id).unwrap();
        let mut snapshot = runs.store().get_snapshot(&run_id, record.latest_snapshot_idx.unwrap()).unwrap();
        snapshot.state.accumulated.push(buckyos_api::AiMessage::text(buckyos_api::AiRole::Assistant, raw));
        record.latest_snapshot_idx = Some(runs.store().put_snapshot(&run_id, &snapshot).unwrap());
        record.status = agent_tool::xllm::RunStatus::Completed;
        runs.store().write_record(&record).unwrap();
        drive(&sd, &deps, StopWhen::Idle).await;
        assert_eq!(llm.count(), 1);
        let replies = sink.replies.lock().unwrap();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].answer.as_deref(), expected);
        assert_eq!(replies[0].status, if expected.is_some() { TurnStatus::Completed } else { TurnStatus::Failed });
        if expected == Some("recovered") {
            let state = sd.state().unwrap();
            assert_eq!(state.outbox[0].msg.content.content, "recovered");
            assert_eq!(state.topic.title, "topic");
            assert_eq!(state.topic.tags, ["tag"]);
        } else {
            assert!(sd.state().unwrap().outbox.is_empty());
        }
    }
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
