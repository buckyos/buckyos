//! System Notify on the agent side (notepads/done/opendan-agent-notify-todo.md
//! §3.5–§3.10): the `_notify` observation of UI sessions and `agent-session
//! notify`. Asking the user is a standard asynchronous call (`ask_user`, a
//! `human.input/v1` task): see `input_tasks.rs`.
//! Acceptance ids (V07…) refer to §5 of the TODO.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use libopendan::notify::{
    notify_event, LocalNotify, NotifyHost, NotifyKind, NotifyLevel,
    NotifyListReq, NotifyStatus, NotifyView,
};
use libopendan::protocol::*;
use libopendan::runner::{
    drive, RunnerDeps, StopWhen, TurnTaskEnd, TurnTaskOpen, TurnTaskSink,
    TurnTaskStatus,
};
use libopendan::SessionTemplate;
use serde_json::json;

#[derive(Default)]
struct Tasks {
    updates: Mutex<Vec<TurnTaskStatus>>,
}

#[async_trait]
impl TurnTaskSink for Tasks {
    async fn open(&self, turn: &TurnTaskOpen) -> Result<String, String> {
        Ok(format!("t-{}-{}", turn.session_id, turn.turn))
    }
    async fn update(&self, _task_id: &str, status: &TurnTaskStatus) -> Result<(), String> {
        self.updates.lock().unwrap().push(status.clone());
        Ok(())
    }
    async fn close(&self, _task_id: &str, _end: &TurnTaskEnd) -> Result<(), String> {
        Ok(())
    }
}

fn shell(call_id: &str, command: &str) -> buckyos_api::AiResponse {
    tool_call(call_id, "shell", json!({ "command": command }))
}

fn work(objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("work", None).unwrap().spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec
}

async fn only_notify(local: &LocalNotify) -> NotifyView {
    let all = local.list(NotifyListReq::default()).await.unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
    all[0].clone()
}

fn notify_deps(env: &Env, llm: Arc<ScriptedLlm>, tasks: &Arc<Tasks>) -> (RunnerDeps, Arc<LocalNotify>) {
    let local = Arc::new(LocalNotify::enable(&env.agent_root, AGENT).unwrap());
    let mut deps = env
        .deps(llm)
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    deps.notify = Some(local.clone());
    deps.turn_tasks = Some(tasks.clone());
    (deps, local)
}

/// V16: a notification shown in the conversation of a UI session is a
/// `_notify` semi-subscription event: it opens no Turn and is shown with
/// the next message, one line with the way to check its state.
#[tokio::test]
async fn ui_sessions_observe_notifications_of_their_conversation() {
    let env = Env::new();
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Abob"));
    spec.outbound = Some(OutboundBinding {
        to: "did:bns:bob".into(),
        to_session: None,
        kind: "chat".into(),
    });
    assert!(spec.subscriptions.iter().any(|s| s.id == NOTIFY_SUBSCRIPTION));
    let sd = env.create_work(spec).await;
    let q = queue_of(&sd);
    let data = buckyos_api::NotifyDeliveredData {
        notify_id: "nt-1".into(),
        url: "/messagehub?notify=nt-1".into(),
        kind: NotifyKind::Input,
        category: buckyos_api::NotifyCategory::Agent,
        level: NotifyLevel::Timely,
        title: "Which database?".into(),
        message: Some("The migration needs a target.".into()),
        options: vec![buckyos_api::HumanPromptOption::new("pg", "Postgres")],
        source: buckyos_api::NotifySource {
            session_id: Some("work-7".into()),
            ..Default::default()
        },
        delivered_msg_id: None,
        reply_route: buckyos_api::NotifyConversation {
            to: "did:bns:bob".into(),
            to_session: None,
        },
        at_ms: 1_760_000_000_000,
    };
    let (key, ev) = notify_event(&data, "dm:did:bns:bob");
    post_blocking(&env.queue_dir, &q, PostedInput::event(APP, key.clone(), ev.clone()));
    // A repeat of the same delivery is the same version.
    post_blocking(&env.queue_dir, &q, PostedInput::event(APP, key, ev));
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => {
            let all = render(&req.messages);
            // A UI session shows it as a line of the background context of
            // the message's own batch (chat format), no snapshot message.
            assert!(!all.contains("<semi_subscription_snapshot>"), "{all}");
            assert!(all.contains("<<background_context>>"), "{all}");
            assert!(all.contains("- [notify nt-1] input/timely from session work-7"), "{all}");
            assert!(all.contains("(options: pg=Postgres)"), "{all}");
            assert!(all.contains("agent-session notify show nt-1"), "{all}");
            assert_eq!(all.matches("[notify nt-1]").count(), 1, "{all}");
            assert!(all.contains("postgres please"), "{all}");
            text("ok")
        }
        _ => panic!("unexpected call {n}"),
    });
    let deps = env.deps(llm.clone());
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 0, "an observation opens no Turn, not even the first");
    let st = sd.state().unwrap();
    assert_eq!(st.turn_seq, 0);
    assert_eq!(st.pending_events.len(), 1);
    post_blocking(&env.queue_dir, &q, msg("postgres please"));
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1);
    assert!(sd.state().unwrap().pending_events.iter().all(|p| p.latest.is_none()));
}

/// V17 / V18: `agent-session notify` inside sessions — a Work sends a
/// notice, lists and cancels; a UI session answers an input on the user's
/// behalf (noted), may not answer what it sent itself, and a closed one
/// gives its final state; the owner side acts as the user.
#[tokio::test]
async fn the_notify_cli_sends_lists_answers_and_cancels() {
    let env = Env::new();
    let local = LocalNotify::enable(&env.agent_root, AGENT).unwrap();
    let sd = env.create_work(work("remind the user")).await;
    let outputs: Arc<Mutex<Vec<String>>> = Arc::default();
    let out = outputs.clone();
    let root = env.agent_root.clone();
    let llm = ScriptedLlm::new(move |req, n| {
        if n > 0 {
            let id = format!("c{n}");
            if let Some(r) = has_tool_result(req, &id) {
                out.lock().unwrap().push(r);
            }
        }
        match n {
            0 => shell("c1", "agent-session notify send --title 'Water the plants' --message 'It is Monday' --level quiet --expires-in 2h"),
            1 => shell("c2", "agent-session notify list"),
            2 => shell("c3", &format!("agent-session notify ack {}", first_notify_id(&root))),
            3 => shell(
                "c4",
                &format!("agent-session notify cancel {} --reason done", first_notify_id(&root)),
            ),
            _ => text("reminded"),
        }
    });
    let tasks = Arc::new(Tasks::default());
    let (deps, _) = notify_deps(&env, llm.clone(), &tasks);
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let outs = outputs.lock().unwrap().clone();
    assert_eq!(outs.len(), 4, "{outs:#?}");
    assert!(outs[0].contains("\"status\": \"open\""), "{}", outs[0]);
    assert!(outs[1].contains("Water the plants"), "{}", outs[1]);
    assert!(outs[2].contains("this session sent the notification"), "{}", outs[2]);
    assert!(outs[3].contains("\"canceled\""), "{}", outs[3]);
    let v = only_notify(&local).await;
    assert_eq!(v.kind, NotifyKind::Notice);
    assert_eq!(v.level, NotifyLevel::Quiet);
    assert_eq!(v.source.session_id.as_deref(), Some(sd.sid()));
    assert_eq!(v.status, NotifyStatus::Canceled);
    assert!(v.expires_at_ms.is_some());

    // A UI session answers a Work's question the user answered in chat.
    let question = local
        .send(buckyos_api::NotifySendReq::input(
            "",
            "Which database?",
            "",
            vec![buckyos_api::HumanPromptOption::new("pg", "Postgres")],
            true,
            "work-1:1:ask:q1",
        ))
        .await
        .unwrap();
    let mut ui = SessionTemplate::load("ui", None).unwrap().spec("chat");
    ui.prompt.llm_context = json!({ "tools": { "enabled": true } });
    let ui_sd = env.create_work(ui).await;
    let qid = question.notify_id.clone();
    let answers: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = answers.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => shell("a1", &format!("agent-session notify answer {qid} --option pg")),
        1 => shell("a2", &format!("agent-session notify answer {qid} --text again")),
        _ => {
            for c in ["a1", "a2"] {
                seen.lock().unwrap().push(has_tool_result(req, c).unwrap());
            }
            text("answered for you")
        }
    });
    let (deps, _) = notify_deps(&env, llm.clone(), &tasks);
    post_blocking(&env.queue_dir, &queue_of(&ui_sd), msg("use postgres"));
    drive(&ui_sd, &deps, StopWhen::TurnClosed).await;
    let answers = answers.lock().unwrap().clone();
    assert!(answers[0].contains("\"answered\""), "{}", answers[0]);
    assert!(answers[1].contains("the notification is closed"), "{}", answers[1]);
    let done = local.state(&question.notify_id).await.unwrap();
    assert_eq!(done.status, NotifyStatus::Answered);
    assert_eq!(done.result.unwrap().option(), Some("pg"));
    let notes = local.notes(&question.notify_id).unwrap();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].starts_with(&format!("answered by {AGENT} in session {}", ui_sd.sid())), "{notes:?}");

    // The owner side (`xagent notify`) plays the user.
    let other = local
        .send(buckyos_api::NotifySendReq::notice("", "FYI", "", "fyi-1"))
        .await
        .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"))
        .args(["notify", "dismiss", &other.notify_id, "--agent", AGENT, "--agent-root"])
        .arg(&env.agent_root)
        .env_remove("OPENDAN_SESSION_ID")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(local.state(&other.notify_id).await.unwrap().status, NotifyStatus::Dismissed);
}

/// Id of the only notification kept by `LocalNotify` at `agent_root`.
fn first_notify_id(agent_root: &std::path::Path) -> String {
    std::fs::read_dir(LocalNotify::dir_of(agent_root))
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".json"))
                .map(str::to_string)
        })
        .next()
        .unwrap()
}


fn now_blocking<T: Send + 'static>(f: impl std::future::Future<Output = T> + Send + 'static) -> T {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    })
    .join()
    .unwrap()
}

/// The result of an intent analysis: what it claims is checked, the
/// questions come from the notifications (chat route TODO §2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_analysis_result_is_checked_and_gets_its_questions() {
    let schema = json!({
        "type": "object", "additionalProperties": false,
        "required": ["outcome", "report", "work", "goal", "forwarded"],
        "properties": {
            "outcome": {"type": "string"}, "report": {"type": "string"},
            "work": {"type": ["object", "null"]}, "goal": {"type": ["object", "null"]},
            "forwarded": {"type": ["object", "null"]}
        }
    });
    let result = |outcome: &str, report: &str, work: serde_json::Value, goal: serde_json::Value, to: serde_json::Value| {
        text(&json!({"outcome": outcome, "report": report, "work": work, "goal": goal, "forwarded": to}).to_string())
    };
    let null = serde_json::Value::Null;
    // What the first result says, and the correction it gets.
    let cases = [
        (result("direct", " ", null.clone(), null.clone(), null.clone()), "`report` must not be empty"),
        (result("work_created", "r", null.clone(), null.clone(), null.clone()), "needs `work`"),
        (
            result("work_created", "r", json!({"session_id": "work-elsewhere"}), null.clone(), null.clone()),
            "is not a session this analysis created",
        ),
        (
            result("goal_created", "r", null.clone(), json!({"goal_id": "goal-none"}), null.clone()),
            "does not exist",
        ),
        (
            result("forwarded", "r", null.clone(), null.clone(), json!({"kind": "work", "id": "work-none"})),
            "is not a goal or a work session that takes messages",
        ),
    ];
    for (first, correction) in cases {
        let env = Env::new();
        let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
        spec.prompt.llm_context = json!({
            "tools": { "enabled": true, "tools": [{ "groupname": "bash_core" }, { "name": "call_behavior" }] },
            "runtime": { "task_wait_ms": 8000 }
        });
        spec.outbound = Some(OutboundBinding {
            to: "did:bns:bob".into(),
            to_session: None,
            kind: "chat".into(),
        });
        spec.extensions.insert(
            "opendan".into(),
            json!({ "behaviors": { "analyze": { "mode": "fork", "reply": {
                "schema": schema, "asks_field": "clarifications", "contract": "intent_analysis"
            } } } }),
        );
        let sd = env.create_work(spec).await;
        post_blocking(&env.queue_dir, &queue_of(&sd), msg("please do it"));
        let local = Arc::new(LocalNotify::enable(&env.agent_root, AGENT).unwrap());
        let (asker, sid, queue, root) = (local.clone(), sd.sid().to_string(), env.queue_dir.clone(), env.agent_root.clone());
        let correction = correction.to_string();
        let llm = ScriptedLlm::new(move |req, n| match n {
            0 => tool_call("f1", "call_behavior", json!({ "behavior": "analyze", "task": "do it" })),
            1 => text("analyzing"),
            2 => {
                // The branch asks; the user answers on the card.
                let (asker, sid) = (asker.clone(), sid.clone());
                now_blocking(async move {
                    let mut ask = buckyos_api::NotifySendReq::input(
                        "bob",
                        "Which color?",
                        "",
                        vec![
                            buckyos_api::HumanPromptOption::new("a", "Alpha"),
                            buckyos_api::HumanPromptOption::new("b", "Beta one"),
                        ],
                        true,
                        "ask:1",
                    );
                    ask.source = Some(buckyos_api::NotifySource {
                        session_id: Some(sid.clone()),
                        ..Default::default()
                    });
                    let asked = asker.send(ask).await.unwrap();
                    let mut answer = buckyos_api::NotifyAnswer::option("b");
                    answer.text = Some("but darker".into());
                    asker.answer(&asked.notify_id, answer, None).await.unwrap();
                    // Another session's question is not this branch's.
                    let mut other = buckyos_api::NotifySendReq::input("bob", "Other?", "", vec![], true, "ask:2");
                    other.source = Some(buckyos_api::NotifySource {
                        session_id: Some("work-other".into()),
                        ..Default::default()
                    });
                    asker.send(other).await.unwrap();
                });
                first.clone()
            }
            3 => {
                let m = last_user_text(req);
                assert!(m.contains(&correction), "{m}");
                shell(
                    "w1",
                    &format!(
                        "agent-session create-worksession --title t --objective 'the work' --queue-dir {} --agent-root {}",
                        queue.display(),
                        root.display()
                    ),
                )
            }
            4 => {
                let r = has_tool_result(req, "w1").unwrap();
                let id = r.split("\"session_id\": \"").nth(1).expect(&r).split('"').next().unwrap();
                text(&json!({"outcome": "work_created", "report": "started", "work": {"session_id": id}, "goal": null, "forwarded": null}).to_string())
            }
            5 => {
                let last = last_user_text(req);
                for part in ["Which color?", "Beta one\\\\nbut darker", "started", "answered"] {
                    assert!(last.contains(part) || last.contains(&part.replace("\\\\n", "\\n")), "{part}: {last}");
                }
                assert!(last.contains("clarifications") && !last.contains("Other?"), "{last}");
                text("it is under way")
            }
            _ => panic!("unexpected inference {n}: {}", last_user_text(req)),
        });
        let mut deps = env
            .deps(llm.clone())
            .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
        deps.notify = Some(local.clone());
        drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert_eq!(llm.count(), 6);
    }
}
