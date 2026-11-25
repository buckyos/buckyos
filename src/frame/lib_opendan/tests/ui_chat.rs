//! UI Sessions after the prompt rework (notepads/done/jarvis-chat-prompt-todo.md):
//! the chat input format, the structured reply, and a long command followed
//! through the task CLI bridge (`agent-session task wait`).

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiRole, ProviderStateCoordinate};
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{
    compose_reply, drive, DriveResult, OutboundSink, SendResult, StopWhen, TurnReply,
};
use libopendan::{AgentStateClient, SessionTemplate};
use ndn_lib::MsgObject;
use serde_json::json;

fn ui_spec() -> libopendan::api::SessionSpec {
    ui_spec_waiting(200)
}

/// A UI session whose calls wait `task_wait_ms` before a command becomes a
/// task (long enough for an `agent-session` command itself to answer).
fn ui_spec_waiting(task_wait_ms: u64) -> libopendan::api::SessionSpec {
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
            "tools": [{ "groupname": "bash_core" }]
        },
        "runtime": { "task_wait_ms": task_wait_ms },
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

#[tokio::test]
async fn uncompressed_chat_keeps_native_messages_across_restarted_turns() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    let long_input = format!("original request {} keep-this-ending", "detail ".repeat(900));
    let provider_state = AiContent::ProviderState {
        source: ProviderStateCoordinate {
            provider_profile_id: "test-provider".into(),
            adapter_type: "openai-responses".into(),
            origin_provider: "openai".into(),
            origin_model: "test-model".into(),
        },
        provider: "openai".into(),
        value: json!({"type": "reasoning", "id": "original-item", "encrypted_content": "opaque"}),
    };
    let retained_state = provider_state.clone();
    let llm = ScriptedLlm::new(move |req, n| {
        assert!(!render(&req.messages).contains("<history_summary>"));
        match n {
            0 => {
                let mut response = shell("original-call", "echo original-tool-result");
                response.message.content.insert(0, retained_state.clone());
                response
            }
            1 => {
                assert!(has_tool_result(req, "original-call")
                    .unwrap()
                    .contains("original-tool-result"));
                text(r#"{"reply":"first answer","topic":null,"artifacts":[]}"#)
            }
            2 => text(r#"{"reply":"second answer","topic":null,"artifacts":[]}"#),
            3 => text(r#"{"reply":"third answer","topic":null,"artifacts":[]}"#),
            _ => panic!("unexpected inference {n}"),
        }
    });
    let mut previous = Vec::new();
    for (turn, input) in [long_input.as_str(), "second request", "third request"]
        .into_iter()
        .enumerate()
    {
        post_blocking(&env.queue_dir, &queue_of(&sd), msg(input));
        let first_request = llm.count();
        let result = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
        assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
        let requests = llm.requests.lock().unwrap();
        let messages = &requests[first_request].messages;
        assert_eq!(messages[0].role, AiRole::System);
        assert_eq!(&messages[1..1 + previous.len()], previous.as_slice());
        assert_eq!(messages.len(), previous.len() + 2);
        drop(requests);
        let state = sd.state().unwrap();
        assert_eq!(state.turns_completed, turn as u64 + 1);
        let run_id = state.last_run.unwrap();
        assert_eq!(sd.runs().list().unwrap(), vec![run_id.clone()]);
        let (_, snapshot) = sd.runs().load_checked(&run_id).unwrap();
        previous = snapshot.unwrap().state.accumulated[1..].to_vec();
        assert!(sd.summary_opt().unwrap().is_none());
    }
    assert_eq!(llm.count(), 4);
    assert_eq!(previous.iter().filter(|m| m.role == AiRole::User).count(), 3);
    assert_eq!(previous.iter().filter(|m| m.role == AiRole::Assistant).count(), 4);
    assert_eq!(previous.iter().filter(|m| m.role == AiRole::Tool).count(), 1);
    assert!(previous[0].text_content().ends_with(&long_input));
    assert_eq!(render(&previous).matches("keep-this-ending").count(), 1);
    assert_eq!(previous.iter().flat_map(|m| &m.content).filter(|c| **c == provider_state).count(), 1);
    let worklog = read_worklog(&sd);
    assert_eq!(worklog.iter().filter(|e| matches!(&e.body, WorklogBody::UserMessage { .. })).count(), 3);
    assert_eq!(worklog.iter().filter(|e| matches!(&e.body, WorklogBody::ActionResult { call_id, .. } if call_id == "original-call")).count(), 1);
}

#[tokio::test]
async fn chat_compacts_when_native_history_exceeds_the_budget() {
    let env = Env::new();
    let mut spec = ui_spec();
    spec.prompt.history_budget_tokens = Some(2_000);
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text(r#"{"reply":"first answer","topic":null,"artifacts":[]}"#),
        1 => {
            assert!(render(&req.messages).contains("history compressor"));
            text("The user supplied a long request and received the first answer.")
        }
        2 => {
            let history = req.messages[1].text_content();
            assert!(history.starts_with("<history_summary>\n"), "{history}");
            assert!(history.contains("The user supplied a long request"), "{history}");
            assert!(last_user_text(req).ends_with("continue"));
            text(r#"{"reply":"second answer","topic":null,"artifacts":[]}"#)
        }
        _ => panic!("unexpected inference {n}"),
    });
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("long request ".repeat(2_000)));
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    assert!(sd.summary_opt().unwrap().is_none());
    assert_eq!(llm.count(), 1);
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("continue"));
    let result = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    assert_eq!(llm.count(), 3);
    let summary = sd.summary_opt().unwrap().expect("native history triggered compaction");
    assert!(summary.start_offset > 0);
    assert!(summary.history_summary.contains("The user supplied a long request"));
    assert_eq!(sd.state().unwrap().turns_completed, 2);
}

/// The messages of a request after its system text.
fn after_system(req: &llm_context::deps::LlmInferenceRequest) -> Vec<buckyos_api::AiMessage> {
    req.messages.iter().skip_while(|m| m.role == AiRole::System).cloned().collect()
}

/// Chat route TODO §4 (D11): the same long conversation in a large and in a
/// small window. `Turns` of about 6k tokens each; returns what each chat
/// inference was sent.
async fn long_chat(window: u32, turns: usize) -> (Env, libopendan::SessionDir, Vec<Vec<buckyos_api::AiMessage>>) {
    let env = Env::new();
    let mut spec = ui_spec();
    spec.prompt.llm_context["context_window"] = json!(window);
    let sd = env.create_work(spec).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        if render(&req.messages).contains("history compressor") {
            return text("What was said so far.");
        }
        s2.lock().unwrap().push(after_system(req));
        text(r#"{"reply":"noted","topic":null,"artifacts":[]}"#)
    });
    let mut deps = env.deps(llm.clone());
    deps.summarizer = Some(llm.clone());
    for turn in 1..=turns {
        post_blocking(&env.queue_dir, &queue_of(&sd), msg(&format!("turn {turn} {}", "words here ".repeat(2_200))));
        let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{turn}: {result:?}");
    }
    let seen = seen.lock().unwrap().clone();
    (env, sd, seen)
}

#[tokio::test]
async fn a_long_chat_keeps_its_prefix_in_a_large_window_and_is_compressed_in_a_small_one() {
    // 6 turns ≈ 36k tokens: twice what used to be compressed whatever the
    // window, far from the 160k line of this one.
    let (_env, sd, seen) = long_chat(200_000, 6).await;
    assert!(sd.summary_opt().unwrap().is_none(), "not compressed");
    for pair in seen.windows(2) {
        assert_eq!(pair[1][..pair[0].len()], pair[0][..], "the next request starts with the last one");
    }
    // The line of a 24k window is crossed on the way; the chat goes on, and
    // after the compression its requests extend each other again.
    let (_env, sd, seen) = long_chat(24_000, 6).await;
    let summary = sd.summary_opt().unwrap().expect("compressed");
    assert!(summary.history_summary.contains("What was said so far."));
    assert_eq!(sd.state().unwrap().turns_completed, 6);
    let extends = seen.windows(2).filter(|p| p[1].len() > p[0].len() && p[1][..p[0].len()] == p[0][..]).count();
    assert!(extends >= 3, "{extends}");
    assert!(seen.iter().all(|m| libopendan::runner::history::estimate(m) <= 24_000 * 8 / 10), "every request stays below the line");
}

/// Chat route TODO §4: what the conversation recorded and the way to its
/// words outlive every compression.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recorded_notes_and_the_original_words_survive_compressions() {
    let env = Env::new();
    let mut spec = ui_spec_waiting(8000);
    spec.prompt.history_budget_tokens = Some(3_000);
    let sd = env.create_work(spec).await;
    let queue = env.queue_dir.clone();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let sid = sd.sid().to_string();
    let llm = ScriptedLlm::new(move |req, _| {
        let all = render(&req.messages);
        if all.contains("history compressor") {
            // A summarizer that keeps nothing of it.
            return text("Some long messages were exchanged.");
        }
        let last = last_user_text(req);
        let noted = has_tool_result(req, "p1").is_some() || has_tool_result(req, "p5").is_some();
        s2.lock().unwrap().push(all);
        if last.contains("not Monday") && !noted {
            return shell(
                "p1",
                &format!("agent-session ctl perceive 'the owner corrected the deadline to Friday' --queue-dir {}", queue.display()),
            );
        }
        if last.contains("read it back") && has_tool_result(req, "r1").is_none() {
            return shell("r1", &format!("agent-session read-session {sid} --turn 1 --queue-dir {}", queue.display()));
        }
        text(r#"{"reply":"noted","topic":null,"artifacts":[]}"#)
    });
    let mut deps = env
        .deps(llm.clone())
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    deps.summarizer = Some(llm.clone());
    let long = "padding words ".repeat(900);
    let turns = [
        "The report is due on Friday, not Monday.".to_string(),
        format!("first {long}"),
        format!("second {long}"),
        format!("third {long}"),
        "read it back".to_string(),
    ];
    let mut compressions = Vec::new();
    for t in &turns {
        post_blocking(&env.queue_dir, &queue_of(&sd), msg(t));
        let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
        compressions.push(
            read_worklog(&sd)
                .iter()
                .filter(|e| matches!(e.body, WorklogBody::Compaction { .. }))
                .count(),
        );
    }
    assert!(*compressions.last().unwrap() >= 2, "{compressions:?}");
    let summary = sd.summary_opt().unwrap().unwrap();
    assert_eq!(summary.perceptions.len(), 1, "{:?}", summary.perceptions);
    let line = summary.perceptions[0].clone();
    assert!(line.contains("(turn 1): the owner corrected the deadline to Friday"), "{line}");
    let after = seen.lock().unwrap().last().cloned().unwrap();
    assert!(after.contains("<history_summary>") && after.contains(&line), "{after}");
    assert!(after.contains(&format!("agent-session read-session {} --turn <n>", sd.sid())), "{after}");
    // The words themselves, through the command the history names.
    assert!(after.contains("The report is due on Friday, not Monday."), "{after}");
    let reference = line.split(' ').next().unwrap();
    let read = env.agent().cognition().read(sd.sid(), reference).await.unwrap();
    assert!(serde_json::to_string(&read).unwrap().contains("deadline to Friday"));
    // The same cursor and Turn name the same records, whatever was added.
    let range = |after: Option<u64>| libopendan::api::RecordRange { turn: Some(1), after, limit: 2, ..Default::default() };
    let agent = env.agent();
    let first = libopendan::api::read_session_records(agent.as_ref(), sd.sid(), &range(None)).await.unwrap();
    assert_eq!(first.records.len(), 2);
    let second = libopendan::api::read_session_records(agent.as_ref(), sd.sid(), &range(first.next_after)).await.unwrap();
    assert!(second.records.iter().all(|e| e.body.turn() == Some(1)));
    assert!(second.records[0].seq > first.records[1].seq);
    post_blocking(&env.queue_dir, &queue_of(&sd), msg(&format!("fourth {long}")));
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let again = libopendan::api::read_session_records(agent.as_ref(), sd.sid(), &range(first.next_after)).await.unwrap();
    assert_eq!(
        serde_json::to_value(&again.records).unwrap(),
        serde_json::to_value(&second.records).unwrap()
    );
}

#[tokio::test]
async fn uncompressed_chat_keeps_fork_results_without_the_child_transcript() {
    let env = Env::new();
    let mut spec = ui_spec();
    spec.prompt.llm_context["tools"]["tools"] = json!([
        {"groupname": "bash_core"}, {"name": "call_behavior"}
    ]);
    spec.extensions.insert(
        "opendan".into(),
        json!({"behaviors": {"research": {"mode": "fork"}}}),
    );
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("fork-call", "call_behavior", json!({"behavior": "research", "task": "find the answer"})),
        // The call is registered; the branch starts once the caller stops acting.
        1 => text(r#"{"reply":"looking into it","topic":null,"artifacts":[]}"#),
        2 => shell("child-call", "echo CHILD-ONLY-TRANSCRIPT"),
        3 => text(r#"{"reply":"child result","topic":null,"artifacts":[]}"#),
        4 | 5 => {
            let all = render(&req.messages);
            assert!(!all.contains("<history_summary>"), "{all}");
            assert!(!all.contains("CHILD-ONLY-TRANSCRIPT"), "{all}");
            assert!(has_tool_result(req, "child-call").is_none());
            assert!(has_tool_result(req, "fork-call").unwrap().contains("registered"));
            // The branch result comes back as the caller's input, naming the call.
            assert!(all.contains("<process_result") && all.contains("call_id=\"fork-call\""), "{all}");
            assert!(all.contains("child result"), "{all}");
            let uses: Vec<_> = req.messages.iter().filter(|m| {
                m.role == AiRole::Assistant
                    && m.tool_calls().iter().any(|call| call.call_id == "fork-call")
            }).collect();
            let results: Vec<_> = req.messages.iter().filter(|m| {
                m.role == AiRole::Tool && m.content.iter().any(|block| {
                    matches!(block, AiContent::ToolResult { call_id, .. } if call_id == "fork-call")
                })
            }).collect();
            assert_eq!((uses.len(), results.len()), (1, 1));
            text(r#"{"reply":"parent answer","topic":null,"artifacts":[]}"#)
        }
        _ => panic!("unexpected inference {n}"),
    });
    for input in ["research this", "continue"] {
        post_blocking(&env.queue_dir, &queue_of(&sd), msg(input));
        let result = drive(&sd, &env.deps(llm.clone()), StopWhen::TurnClosed).await;
        assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{result:?}");
    }
    assert_eq!(llm.count(), 6);
    assert!(sd.summary_opt().unwrap().is_none());
    assert!(read_worklog(&sd).iter().any(|e| {
        matches!(&e.body, WorklogBody::ActionResult { call_id, .. } if call_id == "child-call")
    }));
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

/// A chat with `submit_reply` in its tools, and `agent-session` on PATH.
async fn compat_chat(env: &Env) -> libopendan::SessionDir {
    let mut spec = ui_spec_waiting(1500);
    spec.prompt.llm_context["tools"]["tools"] =
        json!([{ "groupname": "bash_core" }, { "name": "submit_reply" }]);
    env.create_work(spec).await
}

fn set_mode(queue_dir: &std::path::Path, call_id: &str, mode: &str) -> buckyos_api::AiResponse {
    shell(
        call_id,
        &format!("agent-session ctl reply-mode {mode} --queue-dir {}", queue_dir.display()),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_reply_may_switch_to_the_compatible_mode() {
    let env = Env::new();
    let sd = compat_chat(&env).await;
    let broken = r#"{"reply":"visible text","topic":null,"artifacts":[],"extra":1}"#;
    let queue = env.queue_dir.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        // Turn 1: the correction offers the mode; the model takes it and the
        // next text is the reply of the same Turn.
        0 => {
            assert!(req.force_json && req.json_schema.is_some());
            text(broken)
        }
        1 => {
            let m = last_user_text(req);
            assert!(m.contains("Your reply does not meet the required format"), "{m}");
            assert!(m.contains("agent-session ctl reply-mode compat"), "{m}");
            set_mode(&queue, "m1", "compat")
        }
        2 => {
            assert!(has_tool_result(req, "m1").unwrap().contains("posted"));
            assert!(!req.force_json && req.json_schema.is_none());
            text("plain words, {not json}")
        }
        // Turn 2: still in the mode, told so; a plain text again.
        3 => {
            assert!(!req.force_json);
            assert!(last_user_text(req).contains("reply mode: compat"), "{}", last_user_text(req));
            text("second")
        }
        // Turn 3: a file and a topic go through the tool, while a task runs.
        4 => shell("t1", "printf data > out.txt; sleep 8"),
        5 => {
            assert!(has_tool_result(req, "t1").unwrap().contains("task local:shell:t1"));
            tool_call("r0", "submit_reply", json!({"reply": "here", "artifacts": ["missing.txt"]}))
        }
        6 => {
            let r = has_tool_result(req, "r0").unwrap();
            assert!(r.contains("missing.txt"), "{r}");
            tool_call(
                "r1",
                "submit_reply",
                json!({"reply": "the file", "topic": {"title": "files", "tags": ["file"]}, "artifacts": ["out.txt"]}),
            )
        }
        // Turn 4: back to the schema.
        7 => {
            assert!(!last_user_text(req).contains("reply mode: compat"), "said once");
            set_mode(&queue, "m2", "schema")
        }
        8 => {
            assert!(req.force_json);
            text("not json")
        }
        9 => text(r#"{"reply":"json again","topic":null,"artifacts":[]}"#),
        _ => panic!("unexpected inference {n}: {}", last_user_text(req)),
    });
    let sink = Arc::new(ReplySink::default());
    let mut deps = env
        .deps(llm.clone())
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    deps.outbound = Some(sink.clone());
    for (turn, expected) in [(1, "plain words, {not json}"), (2, "second"), (3, "the file"), (4, "json again")] {
        post_blocking(&env.queue_dir, &queue_of(&sd), msg(&format!("message {turn}")));
        let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert!(matches!(result, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }), "{turn}: {result:?}");
        let reply = sink.replies.lock().unwrap().last().cloned().unwrap();
        assert_eq!(reply.answer.as_deref(), Some(expected));
        assert_eq!(reply.artifacts.len(), usize::from(turn == 3), "{turn}");
        let state = sd.state().unwrap();
        assert_eq!(state.reply_mode, if turn < 4 { ReplyMode::Compat } else { ReplyMode::Schema });
        assert_eq!(state.topic.title, if turn < 3 { "" } else { "files" }, "{turn}");
        assert!(state.final_report.is_none() && state.latest_report.is_none(), "a reply is no report");
        assert_eq!(state.watched_tasks.len(), usize::from(turn >= 3), "what runs is followed on");
    }
    assert_eq!(llm.count(), 10);
    assert!(sd.state().unwrap().outbox.iter().all(|o| !o.msg.content.content.contains("visible text")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_reply_outside_the_compatible_mode_says_how_to_switch() {
    let env = Env::new();
    let sd = compat_chat(&env).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("hello"));
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("r0", "submit_reply", json!({"reply": "hi"})),
        1 => {
            let r = has_tool_result(req, "r0").unwrap();
            assert!(r.contains("agent-session ctl reply-mode compat"), "{r}");
            text(r#"{"reply":"hi","topic":null,"artifacts":[]}"#)
        }
        _ => panic!("unexpected inference"),
    });
    let sink = Arc::new(ReplySink::default());
    let mut deps = env.deps(llm.clone());
    deps.outbound = Some(sink.clone());
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert_eq!(sink.replies.lock().unwrap()[0].answer.as_deref(), Some("hi"));
    assert_eq!(sd.state().unwrap().reply_mode, ReplyMode::Schema);
}

/// The reply is on record before the Turn closes: recovery delivers it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_submitted_reply_outlives_the_run() {
    let env = Env::new();
    let sd = compat_chat(&env).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("hello"));
    let queue = env.queue_dir.clone();
    let llm = ScriptedLlm::fallible(move |_, n| match n {
        0 => Ok(set_mode(&queue, "m1", "compat")),
        1 => Err(llm_context::LLMComputeError::Timeout),
        _ => panic!("unexpected inference {n}"),
    });
    let sink = Arc::new(ReplySink::default());
    let mut deps = env
        .deps(llm.clone())
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    deps.outbound = Some(sink.clone());
    drive(&sd, &deps, StopWhen::MaxOutcomes { n: 1 }).await;
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let runs = sd.runs();
    let mut record = runs.record(&run_id).unwrap();
    let source = ReportSource::Reply { call_id: "r1".into() };
    let reply = ReportSubmission {
        id: "reply-1".into(),
        run_id: run_id.clone(),
        source,
        report: "kept".into(),
        artifacts: vec![],
        result: Some(json!({"topic": {"title": "kept topic", "tags": []}})),
        is_end: true,
        outcome: None,
        accepted_at_ms: 1,
    };
    record.host.as_mut().unwrap().extra["reports"] = json!([reply]);
    record.status = agent_tool::xllm::RunStatus::Completed;
    runs.store().write_record(&record).unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(sink.replies.lock().unwrap()[0].answer.as_deref(), Some("kept"));
    assert_eq!(sd.state().unwrap().topic.title, "kept topic");
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
async fn a_task_checked_through_the_cli_comes_back_once_as_input() {
    let env = Env::new();
    let sd = env.create_work(ui_spec_waiting(1500)).await;
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
            shell("c1", "sleep 4; echo slow-done")
        }
        1 => {
            let r = has_tool_result(req, "c1").expect("c1 result");
            // The session follows it; the model may still look at it itself.
            assert!(r.contains("comes back to you as input") && r.contains("agent-session task state <task_id>"), "{r}");
            tool_call("c2", "shell", json!({ "command": "agent-session task state local:shell:c1" }))
        }
        2 => {
            let r = has_tool_result(req, "c2").expect("c2 result");
            assert!(r.contains("task local:shell:c1 is still running"), "{r}");
            text(r#"{"reply": "the job is running", "topic": null, "artifacts": []}"#)
        }
        3 => {
            let input = last_user_text(req);
            assert!(input.contains("status=\"finished\"") && input.contains("slow-done"), "{input}");
            text(r#"{"reply": "the job printed slow-done", "topic": {"title": "slow job", "tags": ["job"]}, "artifacts": []}"#)
        }
        _ => panic!("unexpected call {n}: {}", last_user_text(req)),
    });
    let deps = env
        .deps(llm.clone())
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    let started = std::time::Instant::now();
    drive(&sd, &deps, StopWhen::Idle).await;
    while llm.count() < 4 && started.elapsed() < std::time::Duration::from_secs(15) {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drive(&sd, &deps, StopWhen::Idle).await;
    }
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(st.watched_tasks.is_empty(), "{:?}", st.watched_tasks);
    assert_eq!(st.topic.title, "slow job");
    assert_eq!(st.topic.tags, ["job"]);
    // Delivered once: no further Turn.
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 4);
    let report = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Outcome { report, .. } => report,
            _ => None,
        })
        .last()
        .unwrap();
    assert_eq!(report, "the job printed slow-done");
    // The background line of the runtime is not shown again.
    assert!(st.background_shown.contains_key("runtime"));
}

/// A session host: `shell` commands that outlive their wait are tasks of
/// the runner process (`local:shell:<call_id>`), followed by the session.
fn session_host_deps(env: &Env, llm: std::sync::Arc<ScriptedLlm>) -> libopendan::runner::RunnerDeps {
    env.deps(llm)
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")))
}

/// The session follows its tasks itself: there is nothing to wait on, the
/// task's end comes back as input (simple Work Session TODO §3.10).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_host_refuses_task_wait() {
    let env = Env::new();
    let sd = env.create_work(ui_spec_waiting(1500)).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("run the slow job"));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => shell("c1", "sleep 3; echo slow-done"),
        1 => {
            let r = has_tool_result(req, "c1").expect("c1 result");
            assert!(r.contains("it continues as task local:shell:c1"), "{r}");
            assert!(r.contains("<runtime_hints>") && r.contains("agent-session task state <task_id>"), "{r}");
            assert!(!r.contains("task wait") && !r.contains("wait_ms"), "{r}");
            tool_call(
                "c2",
                "shell",
                json!({ "command": "agent-session task wait local:shell:c1" }),
            )
        }
        2 => {
            let r = has_tool_result(req, "c2").expect("c2 result");
            assert!(r.contains("there is no `task wait`") && r.contains("comes back to you as input"), "{r}");
            text(r#"{"reply": "started; its result comes later", "topic": null, "artifacts": []}"#)
        }
        3 => {
            let input = last_user_text(req);
            assert!(input.contains("<<task_result task=\"local:shell:c1\" call_id=\"c1\" status=\"finished\">>"), "{input}");
            assert!(input.contains("slow-done"), "{input}");
            text(r#"{"reply": "the job printed slow-done", "topic": null, "artifacts": []}"#)
        }
        _ => panic!("unexpected call {n}: {}", last_user_text(req)),
    });
    let deps = session_host_deps(&env, llm.clone());
    let started = std::time::Instant::now();
    drive(&sd, &deps, StopWhen::Idle).await;
    while llm.count() < 4 && started.elapsed() < std::time::Duration::from_secs(15) {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drive(&sd, &deps, StopWhen::Idle).await;
    }
    assert_eq!(llm.count(), 4);
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(st.watched_tasks.is_empty(), "{:?}", st.watched_tasks);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_while_a_task_runs_keeps_the_turn_open_until_its_result() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    post_blocking(&env.queue_dir, &queue_of(&sd), msg("start the slow job"));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => shell("c1", "sleep 3; echo later-done"),
        1 => {
            let r = has_tool_result(req, "c1").expect("c1 result");
            assert!(r.contains("local:shell:c1"), "{r}");
            text(r#"{"reply": "started; I will report when it ends", "topic": null, "artifacts": []}"#)
        }
        2 => {
            let input = last_user_text(req);
            assert!(input.contains("<<task_result task=\"local:shell:c1\" call_id=\"c1\" status=\"finished\">>"), "{input}");
            assert!(input.contains("Call c1: shell `sleep 3; echo later-done`"), "{input}");
            assert!(input.contains("later-done"), "{input}");
            assert!(!input.contains("state unknown"), "{input}");
            text(r#"{"reply": "the job printed later-done", "topic": null, "artifacts": []}"#)
        }
        _ => panic!("unexpected call {n}: {}", last_user_text(req)),
    });
    let deps = session_host_deps(&env, llm.clone());
    // The reply came while the command runs: the run and its Turn stay open
    // and follow the task.
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 0);
    assert_eq!(st.watched_tasks, ["local:shell:c1".to_string()]);
    assert_eq!(llm.count(), 2, "its end is not delivered by the first drive");
    // Later drives (new runner state each) answer for the task from the
    // process's task manager and deliver its end once.
    let started = std::time::Instant::now();
    while llm.count() < 3 && started.elapsed() < std::time::Duration::from_secs(15) {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drive(&sd, &deps, StopWhen::Idle).await;
    }
    assert_eq!(llm.count(), 3);
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(st.watched_tasks.is_empty(), "{:?}", st.watched_tasks);
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 3, "delivered once");
}
