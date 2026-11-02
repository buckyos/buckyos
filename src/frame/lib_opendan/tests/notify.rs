//! System Notify on the agent side (notepads/opendan-agent-notify-todo.md
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

