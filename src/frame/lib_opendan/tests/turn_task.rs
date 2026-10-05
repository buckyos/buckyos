//! Turns as tasks of the host's task service, and the placeholder / final
//! edit pair of a Turn that takes long.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::*;
use libopendan::api::{create_sub_session, SubSessionSpec};
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{
    drive, has_pending_turn_tasks, DriveResult, OutboundSink, RunnerDeps, SendResult, StopWhen,
    TurnTaskEnd, TurnTaskOpen, TurnTaskSink, TurnTaskStatus, AGENT_TASK_META,
};
use libopendan::SessionTemplate;
use ndn_lib::{MsgObject, MsgRelType};
use serde_json::json;

const BOB: &str = "did:bns:bob";

#[derive(Default)]
struct Tasks {
    opened: Mutex<Vec<TurnTaskOpen>>,
    updates: Mutex<Vec<(String, TurnTaskStatus)>>,
    closed: Mutex<Vec<(String, TurnTaskEnd)>>,
    /// Closes that fail before the service starts answering.
    fail_close: Mutex<u32>,
    fail_open: bool,
}

#[async_trait]
impl TurnTaskSink for Tasks {
    async fn open(&self, turn: &TurnTaskOpen) -> Result<String, String> {
        if self.fail_open {
            return Err("task service unreachable".into());
        }
        self.opened.lock().unwrap().push(turn.clone());
        Ok(format!("t-{}-{}", turn.session_id, turn.turn))
    }

    async fn update(&self, task_id: &str, status: &TurnTaskStatus) -> Result<(), String> {
        self.updates
            .lock()
            .unwrap()
            .push((task_id.to_string(), status.clone()));
        Ok(())
    }

    async fn close(&self, task_id: &str, end: &TurnTaskEnd) -> Result<(), String> {
        let mut fail = self.fail_close.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            return Err("task service unreachable".into());
        }
        self.closed
            .lock()
            .unwrap()
            .push((task_id.to_string(), end.clone()));
        Ok(())
    }
}

#[derive(Default)]
struct Sink {
    /// The target can be edited: a placeholder is offered.
    editable: bool,
    reject_placeholder: bool,
    sent: Mutex<Vec<OutboundRecord>>,
}

#[async_trait]
impl OutboundSink for Sink {
    async fn placeholder(&self, _cfg: &SessionConfig, mut base: MsgObject, _turn: u64) -> Option<MsgObject> {
        if !self.editable {
            return None;
        }
        base.content.content = "on it".into();
        Some(base)
    }

    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        self.sent.lock().unwrap().push(record.clone());
        if self.reject_placeholder && record.key.ends_with(":placeholder") {
            return SendResult::Rejected {
                reason: "blocked".into(),
            };
        }
        SendResult::Sent {
            msg_id: Some(msg_key(&record.msg)),
            deliveries: vec!["d-1".into()],
        }
    }
}

fn ui_spec() -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some(format!("{AGENT}/dm%3A{BOB}"));
    spec.outbound = Some(OutboundBinding {
        to: BOB.into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec
}

fn from_bob(t: &str) -> PostedInput {
    let m = text_msg(&parse_did(BOB).unwrap(), &parse_did(AGENT).unwrap(), t);
    PostedInput::msg(APP, m, MsgDelivery::default()).unwrap()
}

fn deps_with(
    env: &Env,
    llm: Arc<ScriptedLlm>,
    sink: &Arc<Sink>,
    tasks: &Arc<Tasks>,
    delay: Duration,
) -> RunnerDeps {
    let mut deps = env.deps(llm);
    deps.outbound = Some(sink.clone());
    deps.turn_tasks = Some(tasks.clone());
    deps.options.placeholder_delay = delay;
    deps
}

fn task_of(msg: &MsgObject) -> Option<String> {
    msg.meta
        .get(AGENT_TASK_META)
        .and_then(|t| t.get("task_id"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
}

#[tokio::test]
async fn a_fast_turn_has_a_task_and_no_placeholder() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("what time is it"))
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    let deps = deps_with(
        &env,
        ScriptedLlm::new(|_, _| text("noon")),
        &sink,
        &tasks,
        Duration::from_secs(60),
    );
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    let task_id = format!("t-{}-1", sd.sid());
    {
        let opened = tasks.opened.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].turn, 1);
        assert_eq!(opened[0].parent_task, None);
        assert_eq!(opened[0].title, "what time is it");
        assert_eq!(opened[0].kind, SessionKind::Ui);
    }
    let closed = tasks.closed.lock().unwrap().clone();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].0, task_id);
    assert_eq!(closed[0].1.status, TurnStatus::Completed);
    assert_eq!(closed[0].1.summary.as_deref(), Some("noon"));
    assert!(tasks.updates.lock().unwrap().iter().all(|(id, _)| id == &task_id));
    // One ordinary message, carrying the task.
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].msg.content.content, "noon");
    assert!(sent[0].msg.relates_to.is_none());
    assert_eq!(task_of(&sent[0].msg), Some(task_id.clone()));
    let st = sd.state().unwrap();
    assert_eq!(st.turn_tasks.len(), 1);
    assert!(st.turn_tasks[0].reported && st.turn_tasks[0].placeholder.is_none());
    assert_eq!(st.outbox[0].purpose, OutboxPurpose::Reply);
    assert!(!has_pending_turn_tasks(&st));
    // Nothing is opened, closed or sent twice.
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(tasks.opened.lock().unwrap().len(), 1);
    assert_eq!(tasks.closed.lock().unwrap().len(), 1);
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_turn_sends_a_placeholder_and_edits_it() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("summarize the repo"))
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    let llm = ScriptedLlm::new(|_, _| {
        std::thread::sleep(Duration::from_millis(600));
        text("**done**")
    });
    let deps = deps_with(&env, llm, &sink, &tasks, Duration::from_millis(100));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    let task_id = format!("t-{}-1", sd.sid());
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "{sent:?}");
    let (placeholder, edit) = (&sent[0], &sent[1]);
    assert_eq!(placeholder.msg.content.content, "on it");
    assert_eq!(task_of(&placeholder.msg), Some(task_id));
    assert!(placeholder.msg.relates_to.is_none());
    assert_ne!(placeholder.key, edit.key, "different idempotency keys");
    let rel = edit.msg.relates_to.as_ref().expect("the reply edits the placeholder");
    assert_eq!(rel.rel, MsgRelType::Edit);
    assert_eq!(rel.target.to_string(), msg_key(&placeholder.msg));
    assert_eq!(edit.msg.content.content, "**done**");
    assert_eq!(task_of(&edit.msg), None, "only the anchor carries the task");
    let st = sd.state().unwrap();
    assert_eq!(st.turn_tasks[0].placeholder, Some(msg_key(&placeholder.msg)));
    assert_eq!(
        st.outbox.iter().map(|e| e.purpose).collect::<Vec<_>>(),
        vec![OutboxPurpose::Placeholder, OutboxPurpose::FinalEdit]
    );
    assert!(st.outbox.iter().all(|e| e.status == OutboxStatus::Sent));
}

#[tokio::test]
async fn the_first_tool_call_sends_the_placeholder() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("list files"))
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call("c1", "shell", json!({ "command": "echo hi" })),
        _ => text("one file"),
    });
    let deps = deps_with(&env, llm, &sink, &tasks, Duration::from_secs(60));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0].msg.content.content, "on it");
    assert_eq!(
        sent[1].msg.relates_to.as_ref().unwrap().target.to_string(),
        msg_key(&sent[0].msg)
    );
    assert!(
        tasks
            .updates
            .lock()
            .unwrap()
            .iter()
            .any(|(_, s)| s.tool.as_deref() == Some("shell")),
        "the task shows the tool being run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_target_that_cannot_be_edited_only_gets_the_reply() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("hello"))
        .await
        .unwrap();
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => {
            std::thread::sleep(Duration::from_millis(300));
            tool_call("c1", "shell", json!({ "command": "echo hi" }))
        }
        _ => text("hi bob"),
    });
    let deps = deps_with(&env, llm, &sink, &tasks, Duration::from_millis(50));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].msg.relates_to.is_none());
    assert_eq!(sent[0].msg.content.content, "hi bob");
    assert!(task_of(&sent[0].msg).is_some());
}

#[tokio::test]
async fn a_rejected_placeholder_lets_the_reply_leave_on_its_own() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("list files"))
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        editable: true,
        reject_placeholder: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call("c1", "shell", json!({ "command": "echo hi" })),
        _ => text("one file"),
    });
    let deps = deps_with(&env, llm, &sink, &tasks, Duration::from_secs(60));
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[1].msg.relates_to.is_none(), "nothing to edit at the other side");
    assert_eq!(sent[1].msg.content.content, "one file");
    assert!(task_of(&sent[1].msg).is_some());
    let st = sd.state().unwrap();
    assert_eq!(st.turn_tasks[0].placeholder, None);
}

#[tokio::test]
async fn a_stopped_turn_ends_its_placeholder_and_its_task() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("long job"))
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    let tasks = Arc::new(Tasks::default());
    let queue_dir = env.queue_dir.clone();
    let queue = queue_of(&sd);
    let llm = ScriptedLlm::new(move |_, n| {
        if n == 1 {
            post_blocking(
                &queue_dir,
                &queue,
                PostedInput::control(APP, "stop-1", ControlCommand::Stop { reason: None }),
            );
        }
        tool_call(&format!("c{n}"), "shell", json!({ "command": "echo hi" }))
    });
    let deps = deps_with(&env, llm, &sink, &tasks, Duration::from_secs(60));
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(
        matches!(r, DriveResult::Finished { outcome: Some(Outcome::Stopped), .. }),
        "{r:?}"
    );
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(
        sent[1].msg.relates_to.as_ref().unwrap().target.to_string(),
        msg_key(&sent[0].msg),
        "the placeholder never stays `working`"
    );
    assert_eq!(sent[1].msg.content.content, "Stopped.");
    let closed = tasks.closed.lock().unwrap().clone();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].1.status, TurnStatus::Stopped);
}

#[tokio::test]
async fn a_task_service_that_is_down_never_holds_the_reply() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("hello"))
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        editable: true,
        ..Default::default()
    });
    // No task: no placeholder, the reply leaves as before.
    let down = Arc::new(Tasks {
        fail_open: true,
        ..Default::default()
    });
    let llm = ScriptedLlm::new(|_, n| match n {
        0 => tool_call("c1", "shell", json!({ "command": "echo hi" })),
        _ => text("hi bob"),
    });
    let deps = deps_with(&env, llm, &sink, &down, Duration::from_secs(60));
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].msg.content.content, "hi bob");
    assert_eq!(task_of(&sent[0].msg), None);
    assert!(sd.state().unwrap().turn_tasks.is_empty());

    // A terminal state that was not taken is reported by a later drive.
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("again"))
        .await
        .unwrap();
    let flaky = Arc::new(Tasks {
        fail_close: Mutex::new(1),
        ..Default::default()
    });
    let deps = deps_with(
        &env,
        ScriptedLlm::new(|_, _| text("sure")),
        &sink,
        &flaky,
        Duration::from_secs(60),
    );
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let st = sd.state().unwrap();
    assert!(has_pending_turn_tasks(&st));
    assert!(flaky.closed.lock().unwrap().is_empty());
    let deps = deps_with(
        &env,
        ScriptedLlm::new(|_, _| panic!("no inference")),
        &sink,
        &flaky,
        Duration::from_secs(60),
    );
    drive(&sd, &deps, StopWhen::Idle).await;
    assert!(!has_pending_turn_tasks(&sd.state().unwrap()));
    assert_eq!(flaky.closed.lock().unwrap().len(), 1);
    assert_eq!(flaky.opened.lock().unwrap().len(), 1, "the task is not created again");
}

#[tokio::test]
async fn a_sub_session_task_hangs_under_the_turn_that_created_it() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("do two things"))
        .await
        .unwrap();
    let sink = Arc::new(Sink::default());
    let tasks = Arc::new(Tasks::default());
    let root = env.root.clone();
    let parent = sd.sid().to_string();
    let child: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let created = child.clone();
    // The sub session is created while the parent's Turn is open.
    let llm = ScriptedLlm::new(move |_, _| {
        let root = root.clone();
        let parent = parent.clone();
        let sid = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let env = Env::at(&root);
                let sub = SubSessionSpec {
                    objective: "first thing".into(),
                    key: Some("k1".into()),
                    ..Default::default()
                };
                create_sub_session(env.agent().as_ref(), APP, &parent, sub, env.channels().as_ref())
                    .await
                    .unwrap()
                    .path()
                    .to_path_buf()
            })
        })
        .join()
        .unwrap();
        *created.lock().unwrap() = Some(sid.display().to_string());
        text("started")
    });
    let deps = deps_with(&env, llm, &sink, &tasks, Duration::from_secs(60));
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let parent_task = format!("t-{}-1", sd.sid());
    let child_dir = libopendan::SessionDir::open(child.lock().unwrap().as_ref().unwrap()).unwrap();
    assert_eq!(
        child_dir.config().unwrap().session.origin.unwrap().parent_task,
        Some(parent_task.clone())
    );
    let deps = deps_with(
        &env,
        ScriptedLlm::new(|_, _| text("first thing done")),
        &sink,
        &tasks,
        Duration::from_secs(60),
    );
    drive(&child_dir, &deps, StopWhen::Finished).await;
    let opened = tasks.opened.lock().unwrap().clone();
    let of_child = opened
        .iter()
        .find(|o| o.session_id == child_dir.sid())
        .expect("the sub session's Turn has a task");
    assert_eq!(of_child.parent_task, Some(parent_task));
    assert_eq!(of_child.kind, SessionKind::Work);
    // Its result goes to the parent session, never out as a message.
    assert!(sink
        .sent
        .lock()
        .unwrap()
        .iter()
        .all(|r| r.msg.content.content != "first thing done"));
}

// --- restarts ---------------------------------------------------------

const ENV_ROOT: &str = "LIBOPENDAN_TEST_ROOT";
const ENV_SESSION: &str = "LIBOPENDAN_TEST_SESSION";

fn append(path: &std::path::Path, line: &serde_json::Value) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(f, "{line}").unwrap();
}

fn lines(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// A message service and a task service that outlive the driving process:
/// what they were asked is on disk, the same key is answered the same way.
struct Outside {
    dir: std::path::PathBuf,
}

#[async_trait]
impl OutboundSink for Outside {
    async fn placeholder(&self, _cfg: &SessionConfig, mut base: MsgObject, _turn: u64) -> Option<MsgObject> {
        base.content.content = "on it".into();
        Some(base)
    }

    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        append(&self.dir.join("sent.jsonl"), &serde_json::to_value(record).unwrap());
        SendResult::Sent {
            msg_id: Some(msg_key(&record.msg)),
            deliveries: Vec::new(),
        }
    }
}

#[async_trait]
impl TurnTaskSink for Outside {
    async fn open(&self, turn: &TurnTaskOpen) -> Result<String, String> {
        let id = format!("t-{}-{}", turn.session_id, turn.turn);
        append(&self.dir.join("tasks.jsonl"), &json!({ "open": id }));
        Ok(id)
    }

    async fn update(&self, _task_id: &str, _status: &TurnTaskStatus) -> Result<(), String> {
        Ok(())
    }

    async fn close(&self, task_id: &str, end: &TurnTaskEnd) -> Result<(), String> {
        append(
            &self.dir.join("tasks.jsonl"),
            &json!({ "close": task_id, "status": end.status }),
        );
        Ok(())
    }
}

fn restart_deps(env: &Env) -> RunnerDeps {
    let outside = Arc::new(Outside {
        dir: env.root.clone(),
    });
    let llm = ScriptedLlm::new(|req, _| {
        if has_tool_result(req, "c1").is_some() {
            text("one file")
        } else {
            tool_call("c1", "shell", json!({ "command": "echo hi" }))
        }
    });
    let mut deps = env.deps(llm);
    deps.outbound = Some(outside.clone());
    deps.turn_tasks = Some(outside);
    deps.options.placeholder_delay = Duration::from_secs(60);
    deps
}

/// Child entry point: a no-op unless spawned by `dies_at`.
#[tokio::test]
async fn child_driver() {
    let (Ok(root), Ok(session)) = (std::env::var(ENV_ROOT), std::env::var(ENV_SESSION)) else {
        return;
    };
    let env = Env::at(std::path::Path::new(&root));
    let sd = libopendan::SessionDir::open(&session).unwrap();
    let r = drive(&sd, &restart_deps(&env), StopWhen::TurnClosed).await;
    eprintln!("child drive result: {r:?}");
}

async fn dies_at(fault: &str) {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &from_bob("list files"))
        .await
        .unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_driver", "--nocapture", "--test-threads=1"])
        .env(ENV_ROOT, &env.root)
        .env(ENV_SESSION, sd.path())
        .env("LIBOPENDAN_FAULT", fault)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "the child must die at {fault}");
    let deps = restart_deps(&env);
    let mut r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    if !matches!(r, DriveResult::TurnClosed { .. }) {
        // The Turn was closed before the crash: only the hand-overs are left.
        r = drive(&sd, &deps, StopWhen::Idle).await;
    }
    assert!(
        matches!(r, DriveResult::TurnClosed { .. } | DriveResult::Idle { .. }),
        "{fault}: {r:?}"
    );
    let task_id = format!("t-{}-1", sd.sid());
    let st = sd.state().unwrap();
    assert_eq!(st.turn_tasks.len(), 1, "{fault}");
    assert_eq!(st.turn_tasks[0].task_id, task_id, "{fault}: the same task after the restart");
    assert!(st.turn_tasks[0].reported, "{fault}");
    assert!(st.outbox.iter().all(|e| e.status == OutboxStatus::Sent), "{fault}");
    // Whatever was repeated was repeated under the same key with the same
    // message: one placeholder, one final edit of it, one task.
    let sent: Vec<OutboundRecord> = lines(&env.root.join("sent.jsonl"))
        .into_iter()
        .map(|v| serde_json::from_value(v).unwrap())
        .collect();
    let mut distinct: Vec<&OutboundRecord> = Vec::new();
    for r in &sent {
        if !distinct.contains(&r) {
            assert!(distinct.iter().all(|d| d.key != r.key), "{fault}: a key carried two messages");
            distinct.push(r);
        }
    }
    assert_eq!(distinct.len(), 2, "{fault}: {sent:?}");
    assert_eq!(distinct[0].msg.content.content, "on it", "{fault}");
    assert_eq!(task_of(&distinct[0].msg), Some(task_id.clone()), "{fault}");
    assert_eq!(distinct[1].msg.content.content, "one file", "{fault}");
    assert_eq!(
        distinct[1].msg.relates_to.as_ref().unwrap().target.to_string(),
        msg_key(&distinct[0].msg),
        "{fault}"
    );
    let tasks = lines(&env.root.join("tasks.jsonl"));
    assert!(
        tasks.iter().all(|t| t["open"] == json!(task_id) || t["close"] == json!(task_id)),
        "{fault}: {tasks:?}"
    );
    assert!(
        tasks.iter().any(|t| t["close"] == json!(task_id) && t["status"] == json!("completed")),
        "{fault}: {tasks:?}"
    );
}

#[tokio::test]
async fn a_restart_at_any_stage_keeps_one_task_one_placeholder_one_edit() {
    for fault in [
        // The task exists, its binding was not committed.
        "turn_task:after_open",
        // The placeholder is committed, not handed over.
        "outbound:after_placeholder_commit",
        // The placeholder was taken, the answer was lost.
        "outbound:after_send",
        // The Turn is closed: the edit and the task's end are still to do.
        "finish_run:after_commit",
        // The edit was taken, the answer was lost.
        "outbound:after_send#2",
        // The task ended at the service, the session did not record it.
        "turn_task:after_close",
    ] {
        dies_at(fault).await;
    }
}
