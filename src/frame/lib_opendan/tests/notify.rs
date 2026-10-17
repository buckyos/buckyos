//! System Notify on the agent side (notepads/opendan-agent-notify-todo.md
//! §3.5–§3.10): a Work asks through a notification and waits for it, the
//! `_notify` observation of UI sessions, and `agent-session notify`.
//! Acceptance ids (V07…) refer to §5 of the TODO.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::*;
use libopendan::notify::{
    notify_event, LocalNotify, NotifyAnswer, NotifyHost, NotifyKind, NotifyLevel,
    NotifyListReq, NotifyStatus, NotifyView,
};
use libopendan::protocol::*;
use libopendan::runner::{
    drive, DriveResult, RunnerDeps, StopWhen, TurnTaskEnd, TurnTaskOpen, TurnTaskSink,
    TurnTaskStatus,
};
use libopendan::SessionTemplate;
use serde_json::{json, Value};

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

fn ask(call_id: &str, args: &str) -> buckyos_api::AiResponse {
    tool_call(
        call_id,
        "shell",
        json!({ "command": format!("agent-session ask-user {args}") }),
    )
}

fn shell(call_id: &str, command: &str) -> buckyos_api::AiResponse {
    tool_call(call_id, "shell", json!({ "command": command }))
}

fn work(objective: &str, timeout: Option<u64>) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("work", None).unwrap().spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    if let Some(t) = timeout {
        spec.policy.ask_user_timeout_secs = t;
    }
    spec
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

async fn only_notify(local: &LocalNotify) -> NotifyView {
    let all = local.list(NotifyListReq::default()).await.unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
    all[0].clone()
}

fn result_of(req: &llm_context::deps::LlmInferenceRequest, call_id: &str) -> Value {
    let r = has_tool_result(req, call_id).unwrap_or_else(|| panic!("no result for {call_id}"));
    serde_json::from_str(&r).unwrap_or_else(|e| panic!("{e}: {r}"))
}

/// V07 / V08: a Work's `ask-user` becomes an `input` notification (source
/// session / turn / call, level timely, deadline 3h by default); the Work
/// waits for it (HumanInput on its Turn task) and the answer given on the
/// card is the call's result.
#[tokio::test]
async fn a_work_asks_through_a_notification_and_continues_with_the_answer() {
    let env = Env::new();
    let sd = env.create_work(work("migrate the notes app", None)).await;
    assert!(sd.config().unwrap().channels.kmsg().is_none());
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => ask("q1", "'Which database?' --option pg=Postgres --option sqlite=SQLite"),
        1 => {
            let r = result_of(req, "q1");
            assert_eq!(r["option"], "pg", "{r}");
            assert_eq!(r["text"], "Postgres", "{r}");
            assert_eq!(r["via"], "notification", "{r}");
            assert_eq!(r["in_reply_to_question"], true, "{r}");
            text("migrated to Postgres")
        }
        _ => panic!("unexpected call {n}"),
    });
    let tasks = Arc::new(Tasks::default());
    let (deps, local) = notify_deps(&env, llm.clone(), &tasks);
    let before = libopendan::now_ms();
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    let v = only_notify(&local).await;
    assert_eq!(v.kind, NotifyKind::Input);
    assert_eq!(v.level, NotifyLevel::Timely);
    assert_eq!(v.status, NotifyStatus::Open);
    assert_eq!(v.prompt.title.as_deref(), Some("Which database?"));
    assert_eq!(libopendan::notify::prompt_options(&v.prompt).len(), 2);
    assert_eq!(v.source.session_id.as_deref(), Some(sd.sid()));
    assert_eq!(v.source.call_id.as_deref(), Some("q1"));
    assert_eq!(v.source.turn, Some(1));
    assert_eq!(v.source.operation.as_deref(), Some("migrate the notes app"));
    let st = sd.state().unwrap();
    let w = st.waiting_for.clone().unwrap();
    assert_eq!(w.kind, WaitingKind::Input);
    assert_eq!(w.refs, vec!["userinput:q1".to_string()]);
    let deadline = w.deadline_ms.unwrap();
    assert!(deadline >= before + 10_800_000 && deadline <= libopendan::now_ms() + 10_800_000);
    assert_eq!(v.expires_at_ms, Some(deadline));
    assert_eq!(w.human_tasks, vec![v.task_id.clone().unwrap()]);
    let last = tasks.updates.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.message, "waiting for the user's answer (notification)");
    assert_eq!(last.waiting_for.unwrap().human_tasks, vec![v.task_id.clone().unwrap()]);
    // Driving again changes nothing: the same notification, still waiting.
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(only_notify(&local).await.notify_id, v.notify_id);
    assert_eq!(llm.count(), 1);
    // The user answers on the card.
    local
        .answer(&v.notify_id, NotifyAnswer::option("pg"), None)
        .await
        .unwrap();
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    assert!(sd.report().unwrap().contains("migrated to Postgres"));
}

/// V09 / V10: no answer before the deadline, a dismissed and a canceled
/// notification: the call's result says the question was not answered
/// (a result, not an error) and the Work goes on.
#[tokio::test]
async fn unanswered_questions_are_results() {
    for case in ["timeout", "dismissed", "canceled"] {
        let env = Env::new();
        let timeout = (case == "timeout").then_some(1);
        let sd = env.create_work(work("decide the layout", timeout)).await;
        let expect = case.to_string();
        let llm = ScriptedLlm::new(move |req, n| match n {
            0 => ask("q1", "'Grid or list?'"),
            1 => {
                let r = result_of(req, "q1");
                assert_eq!(r["answered"], false, "{r}");
                assert_eq!(r["reason"], expect.as_str(), "{r}");
                text("went with a grid")
            }
            _ => panic!("unexpected call {n}"),
        });
        let tasks = Arc::new(Tasks::default());
        let (deps, local) = notify_deps(&env, llm.clone(), &tasks);
        drive(&sd, &deps, StopWhen::Idle).await;
        let v = only_notify(&local).await;
        match case {
            "timeout" => tokio::time::sleep(Duration::from_millis(1200)).await,
            "dismissed" => {
                local.answer(&v.notify_id, NotifyAnswer::dismiss(), None).await.unwrap();
            }
            _ => {
                local.cancel(&v.notify_id, "no_longer_needed").await.unwrap();
            }
        }
        let r = drive(&sd, &deps, StopWhen::Finished).await;
        assert!(r.is_finished(), "{case}: {r:?}");
        assert_eq!(llm.count(), 2, "{case}");
        assert!(local.state(&v.notify_id).await.unwrap().status.is_closed(), "{case}");
    }
}

/// V11: a Work stopped while it waits: its notification is canceled
/// (`turn_stopped`) and the Turn ends stopped.
#[tokio::test]
async fn a_stop_while_waiting_cancels_the_notification() {
    let env = Env::new();
    let sd = env.create_work(work("wait for the user", None)).await;
    let llm = ScriptedLlm::new(move |_, n| match n {
        0 => ask("q1", "'Proceed?'"),
        _ => panic!("unexpected call {n}"),
    });
    let tasks = Arc::new(Tasks::default());
    let (deps, local) = notify_deps(&env, llm.clone(), &tasks);
    drive(&sd, &deps, StopWhen::Idle).await;
    let v = only_notify(&local).await;
    deps.stop.request();
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let after = local.state(&v.notify_id).await.unwrap();
    assert_eq!(after.status, NotifyStatus::Canceled);
    assert_eq!(after.closed_reason.as_deref(), Some("turn_stopped"));
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
}

/// V12 (in-process part): answered while nothing drove the session — a new
/// driver fills the answer once; V15: without a notification service a
/// Work still cannot ask, and a Goal is told to use `questions`.
#[tokio::test]
async fn answered_while_unloaded_and_without_a_service() {
    let env = Env::new();
    let sd = env.create_work(work("pick a name", None)).await;
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => ask("q1", "'Name?'"),
        1 => {
            assert_eq!(result_of(req, "q1")["text"], "Notes2");
            text("named Notes2")
        }
        _ => panic!("unexpected call {n}"),
    });
    let tasks = Arc::new(Tasks::default());
    let (deps, local) = notify_deps(&env, llm.clone(), &tasks);
    drive(&sd, &deps, StopWhen::Idle).await;
    drop(deps);
    let v = only_notify(&local).await;
    local.answer(&v.notify_id, NotifyAnswer::text("Notes2"), None).await.unwrap();
    let (deps, _) = notify_deps(&env, llm.clone(), &tasks);
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 2);
    let results: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(results, vec!["q1".to_string()]);

    // No service: today's behaviour.
    let env = Env::new();
    let sd = env.create_work(work("decide alone", None)).await;
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => ask("q1", "'Which one?'"),
        1 => {
            assert!(has_tool_result(req, "q1").unwrap().contains("cannot ask the user"));
            text("decided by myself")
        }
        _ => panic!("unexpected call {n}"),
    });
    let deps = env
        .deps(llm.clone())
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
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
    let sd = env.create_work(work("remind the user", None)).await;
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

/// V13: a waiting Work unloaded by its host is loaded again by the
/// recheck once the user answered (and would be at its deadline).
#[tokio::test]
async fn an_unloaded_waiting_work_is_rechecked() {
    use libopendan::host::{HostDeps, Supervisor};
    let env = Env::new();
    let sd = env.create_work(work("ask then finish", None)).await;
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => ask("q1", "'Go?'"),
        1 => {
            assert_eq!(result_of(req, "q1")["text"], "go");
            text("went")
        }
        _ => panic!("unexpected call {n}"),
    });
    let tasks = Arc::new(Tasks::default());
    let (deps, local) = notify_deps(&env, llm.clone(), &tasks);
    let host = HostDeps {
        who: APP.into(),
        agent: deps.agent.clone(),
        inputs: deps.inputs.clone(),
        waker: deps.waker.clone(),
        xllm: deps.xllm.clone(),
        assembler: Arc::new(libopendan::runner::DefaultAssembler::default()),
        session_cli: deps.session_cli.clone(),
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        options: deps.options.clone(),
        max_child_concurrency: 4,
        bridges: Vec::new(),
        outbound: None,
        objects: None,
        turn_tasks: None,
        notify: Some(local.clone()),
        runtime_id: None,
    };
    let sup = Supervisor::new(host, Some(Duration::from_millis(100)))
        .with_recheck(Duration::from_millis(300));
    let wait_for = |what: &'static str, f: Box<dyn Fn() -> bool>| {
        let sup = sup.clone();
        async move {
            for _ in 0..200 {
                sup.tick().await.unwrap();
                if f() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("timed out waiting for {what}");
        }
    };
    let sid = sd.sid().to_string();
    let s1 = sd.clone();
    wait_for(
        "the question",
        Box::new(move || {
            s1.state()
                .ok()
                .and_then(|s| s.waiting_for)
                .is_some_and(|w| !w.human_tasks.is_empty())
        }),
    )
    .await;
    let s2 = sup.clone();
    let id = sid.clone();
    wait_for("unload", Box::new(move || !s2.is_loaded(&id))).await;
    let v = only_notify(&local).await;
    local.answer(&v.notify_id, NotifyAnswer::text("go"), None).await.unwrap();
    let s3 = sd.clone();
    wait_for("finish", Box::new(move || s3.state().is_ok_and(|s| s.is_finished()))).await;
    assert_eq!(llm.count(), 2);
    sup.shutdown().await;
}
