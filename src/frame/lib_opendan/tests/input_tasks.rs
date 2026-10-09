//! Input protocol 3 and long task scenarios: waiting for a task outside the
//! context (串行等待), following a background task after its run (并行等待),
//! stop while a tool runs, the pending input limit, consumption policy,
//! media blocks, dedup / reply path, read-only old sessions.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use buckyos_api::{AiContent, AiRole};
use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, RunnerDeps, StopWhen};
use libopendan::{OpenDanError, SessionDir};
use llm_context::deps::LlmClient;
use llm_context::tasks::{
    CancelUnsupported, RunningTaskResolver, TaskBrief, TaskResult, TaskState,
};
use serde_json::{json, Value};

/// `start_task({id, mode})`: `wait` suspends the call on task `bucky:<id>`,
/// `background` returns at once with the task id in its result.
struct StartTask;

#[async_trait]
impl AgentTool for StartTask {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "start_task".into(),
            description: "start a task".into(),
            args_schema: json!({ "type": "object", "properties": {
                "id": { "type": "string" }, "mode": { "type": "string" } } }),
            output_schema: json!({ "type": "object" }),
            usage: None,
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let id = args.get("id").and_then(Value::as_str).unwrap_or("1");
        let task_id = format!("bucky:{id}");
        let wait = args.get("mode").and_then(Value::as_str) != Some("background");
        let mut r = AgentToolResult::from_details(json!({ "task": task_id }))
            .with_tool("start_task")
            .with_task_id(task_id.clone())
            .with_status(if wait {
                AgentToolStatus::Pending
            } else {
                AgentToolStatus::Success
            });
        r.summary = format!("task {task_id} started");
        Ok(r)
    }
}

/// A task manager outside the runner process.
#[derive(Default)]
struct FakeTasks {
    states: Mutex<HashMap<String, TaskState>>,
    cancelled: Mutex<Vec<String>>,
}

impl FakeTasks {
    fn running() -> TaskState {
        TaskState::Running {
            brief: "build".into(),
            elapsed_ms: 1000,
            output_tail: String::new(),
            cancellable: true,
        }
    }

    fn finish(&self, task_id: &str, output: &str) {
        self.states.lock().unwrap().insert(
            task_id.into(),
            TaskState::Finished(TaskResult {
                success: true,
                output: output.into(),
                tool_result: None,
            }),
        );
    }
}

#[async_trait]
impl RunningTaskResolver for FakeTasks {
    async fn state(&self, task_id: &str) -> TaskState {
        self.states
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .unwrap_or_else(Self::running)
    }

    async fn wait(&self, task_id: &str, _: Option<u64>) -> TaskState {
        self.state(task_id).await
    }

    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported> {
        self.cancelled.lock().unwrap().push(task_id.to_string());
        Ok(self.state(task_id).await)
    }

    async fn active(&self) -> Vec<TaskBrief> {
        Vec::new()
    }
}

fn task_spec(objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = work_spec(objective);
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [ { "groupname": "bash" }, { "name": "start_task" } ] }
    });
    spec
}

fn task_deps(env: &Env, llm: Arc<dyn LlmClient>, tasks: Option<Arc<FakeTasks>>) -> RunnerDeps {
    let mut deps = env.deps(llm);
    deps.xllm.host_tools.insert("start_task".into(), Arc::new(StartTask));
    deps.xllm.buckyos_tasks = tasks.map(|t| t as Arc<dyn RunningTaskResolver>);
    deps
}

fn results_of(sd: &SessionDir, call: &str) -> Vec<(String, String)> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult {
                call_id,
                status,
                result,
                ..
            } if call_id == call => Some((status, result)),
            _ => None,
        })
        .collect()
}

fn waiting_script() -> Arc<ScriptedLlm> {
    ScriptedLlm::new(|req, n| match n {
        0 => tool_call("t1", "start_task", json!({ "id": "1", "mode": "wait" })),
        _ => {
            let r = has_tool_result(req, "t1").expect("the suspended call was answered");
            text(&format!("task said: {r}"))
        }
    })
}

/// 串行等待: the run is suspended on the call, the session waits for the
/// task outside the context (no inference while polling), messages are held,
/// and the result resumes the same run and Turn.
#[tokio::test]
async fn a_suspended_call_is_filled_when_its_task_ends() {
    let env = Env::new();
    let sd = env.create_work(task_spec("build")).await;
    let tasks = Arc::new(FakeTasks::default());
    let llm = waiting_script();
    let deps = task_deps(&env, llm.clone(), Some(tasks.clone()));
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(
        matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }),
        "{r:?}"
    );
    assert_eq!(llm.count(), 1);
    let st = sd.state().unwrap();
    let run_id = st.live_run.clone().unwrap().run_id;
    let wf = st.waiting_for.clone().unwrap();
    assert_eq!((wf.kind, wf.refs.clone()), (WaitingKind::Tool, vec!["bucky:1".to_string()]));
    // A message and a notification for the task arrive while it runs: the
    // message stays queued (it is no tool result), the notification is
    // consumed and only triggers the query.
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("are you there?"))
        .await
        .unwrap();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &event("task:bucky:1:revision:1", None, "task", "bucky:1", Some(1), "progress"),
    )
    .await
    .unwrap();
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }), "{r:?}");
    assert_eq!(llm.count(), 1, "polling a running task needs no inference");
    let st = sd.state().unwrap();
    assert!(st.source("q").consumed_above.contains(&2), "the notification was consumed");
    assert_eq!(st.source("q").acked_index, 0, "the message is still queued");
    assert!(read_worklog(&sd).iter().any(
        |e| matches!(&e.body, WorklogBody::EventDropped { reason, .. } if reason == "pending_call")
    ));
    // The task ends while nobody drives: no notification is needed, the
    // empty-or-not inbox does not matter.
    tasks.finish("bucky:1", "built ok");
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    assert!(sd.report().unwrap().contains("built ok"));
    let st = sd.state().unwrap();
    assert_eq!(st.last_run.as_deref(), Some(run_id.as_str()), "the same run continued");
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let res = results_of(&sd, "t1");
    assert_eq!(res.len(), 1, "one result for the call: {res:?}");
    assert_eq!(res[0].0, "ok");
}

/// A runner that cannot reach the task manager does not take the run over;
/// one that can but gets "unknown" fills that and lets the LLM decide.
#[tokio::test]
async fn a_waiting_run_needs_a_resolver_and_unknown_is_filled() {
    let env = Env::new();
    let sd = env.create_work(task_spec("build")).await;
    let tasks = Arc::new(FakeTasks::default());
    let llm = waiting_script();
    drive(&sd, &task_deps(&env, llm.clone(), Some(tasks.clone())), StopWhen::Idle).await;
    let before = sd.state().unwrap();
    let r = drive(&sd, &task_deps(&env, llm.clone(), None), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::RecoveryBlocked(_)), "{r:?}");
    let after = sd.state().unwrap();
    assert_eq!(after.live_run, before.live_run, "the scene is kept");
    assert_eq!(llm.count(), 1);
    // The task manager answers "unknown" (e.g. it restarted).
    tasks.states.lock().unwrap().insert(
        "bucky:1".into(),
        TaskState::Unknown {
            reason: "restarted".into(),
        },
    );
    let r = drive(&sd, &task_deps(&env, llm.clone(), Some(tasks)), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    let res = results_of(&sd, "t1");
    assert_eq!(res.len(), 1);
    assert!(res[0].1.contains("state unknown"), "{res:?}");
}

/// Stop while the run waits: the wait is cancelled, the call is answered,
/// the run ends `Stopped` with every call paired.
#[tokio::test]
async fn stop_while_waiting_for_a_task() {
    let env = Env::new();
    let sd = env.create_work(task_spec("build")).await;
    let tasks = Arc::new(FakeTasks::default());
    let llm = waiting_script();
    let deps = task_deps(&env, llm.clone(), Some(tasks.clone()));
    drive(&sd, &deps, StopWhen::Idle).await;
    libopendan::post_input(
        env.agent().as_ref(),
        sd.sid(),
        &PostedInput::control(APP, "stop-1", ControlCommand::Stop { reason: None }),
    )
    .await
    .unwrap();
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(
        matches!(r, DriveResult::Finished { outcome: Some(Outcome::Stopped), .. }),
        "{r:?}"
    );
    assert_eq!(llm.count(), 1);
    assert_eq!(*tasks.cancelled.lock().unwrap(), vec!["bucky:1".to_string()]);
    assert_eq!(results_of(&sd, "t1").len(), 1);
    let st = sd.state().unwrap();
    assert!(st.live_run.is_none() && st.open_turn.is_none());
}

/// 并行等待: the call returned, the task keeps running after the run. Its
/// completion is found by asking with the saved task id — no queue, no
/// notification — and handled like a subscribed event.
#[tokio::test]
async fn a_background_task_completion_is_an_input_event() {
    let env = Env::new();
    let mut spec = task_spec("start the build and wait for news");
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    let sd = env.create_work(spec).await;
    let tasks = Arc::new(FakeTasks::default());
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("b1", "start_task", json!({ "id": "2", "mode": "background" })),
        1 => text("started; I will report when it ends"),
        _ => {
            let u = last_user_text(req);
            assert!(
                u.contains("<event key=\"task:bucky:2:terminal\" source=\"task:bucky:2\" event=\"finished\" terminal=\"true\">task bucky:2 finished: all green</event>"),
                "{u}"
            );
            text("the build is green")
        }
    });
    let deps = task_deps(&env, llm.clone(), Some(tasks.clone()));
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(llm.count(), 2);
    assert_eq!(sd.state().unwrap().watched_tasks, vec!["bucky:2".to_string()]);
    // Still running: nothing to infer on.
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 2);
    tasks.finish("bucky:2", "all green");
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 3);
    let st = sd.state().unwrap();
    assert!(st.watched_tasks.is_empty());
    assert_eq!((st.turn_seq, st.turns_completed), (2, 2));
    assert!(st.inputs.get("_task").is_none(), "no consumption progress for synthesized inputs");
}

/// A queued stop interrupts a tool that is still running: the monitor only
/// looks at the control, the driver consumes it.
#[tokio::test]
async fn stop_interrupts_a_running_tool() {
    let env = Env::new();
    let sd = env.create_work(work_spec("long command")).await;
    let (qd, q) = (env.queue_dir.clone(), queue_of(&sd));
    let llm = ScriptedLlm::new(move |_, n| {
        if n == 0 {
            let (qd, q) = (qd.clone(), q.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(400));
                post_blocking(
                    &qd,
                    &q,
                    PostedInput::control(APP, "stop-1", ControlCommand::Stop { reason: None }),
                );
            });
            tool_call("c1", "shell", json!({ "command": "sleep 60" }))
        } else {
            text("never")
        }
    });
    let started = Instant::now();
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(
        matches!(r, DriveResult::Finished { outcome: Some(Outcome::Stopped), .. }),
        "{r:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
    assert_eq!(llm.count(), 1, "nothing is inferred after the stop");
    let res = results_of(&sd, "c1");
    assert_eq!(res.len(), 1, "the call is paired with a result: {res:?}");
    assert_ne!(res[0].0, "ok");
    // A later drive does nothing: the stop was applied once.
    assert!(drive(&sd, &env.deps(llm), StopWhen::Idle).await.is_finished());
}

/// 64 records may wait on a session's bus; the 65th is refused (retryable),
/// nothing is overwritten, and consuming frees the capacity.
#[tokio::test]
async fn the_bus_holds_at_most_64_pending_inputs() {
    let env = Env::new();
    let mut spec = work_spec("many messages");
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 100 }),
    };
    let sd = env.create_work(spec).await;
    let agent = env.agent();
    for i in 0..MAX_PENDING_INPUTS {
        libopendan::post_input(agent.as_ref(), sd.sid(), &msg(format!("m{i}")))
            .await
            .unwrap();
    }
    let err = libopendan::post_input(agent.as_ref(), sd.sid(), &msg("one too many"))
        .await
        .unwrap_err();
    assert!(matches!(err, OpenDanError::InputFull { pending: 64, .. }), "{err}");
    // Controls share the limit.
    let stop = PostedInput::control(APP, "stop-1", ControlCommand::Stop { reason: None });
    assert!(matches!(
        libopendan::post_input(agent.as_ref(), sd.sid(), &stop).await,
        Err(OpenDanError::InputFull { .. })
    ));
    let llm = ScriptedLlm::new(|_, _| text("ok"));
    drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!(st.source("q").acked_index, 64, "every accepted record was consumed");
    assert!(llm.transcript(llm.count() - 1).contains("m63"));
    assert!(!llm.transcript(llm.count() - 1).contains("one too many"));
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("room again"))
        .await
        .unwrap();
}

/// `input.mode`: Single takes one selected input per batch, Batch several;
/// a bootstrap that finds inputs merges them into the `on_init` batch.
#[tokio::test]
async fn single_and_batch_consumption() {
    let env = Env::new();
    for mode in [InputMode::Single, InputMode::Batch] {
        let mut spec = work_spec("chat");
        spec.end_condition = EndCondition {
            kind: EndConditionType::MaxTurns,
            detail: json!({ "n": 2 }),
        };
        spec.prompt.input.mode = mode;
        let sd = env.create_work(spec).await;
        let agent = env.agent();
        for t in ["first question", "second question"] {
            libopendan::post_input(agent.as_ref(), sd.sid(), &msg(t)).await.unwrap();
        }
        let llm = ScriptedLlm::new(|_, _| text("answer"));
        drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;
        let first = llm.requests.lock().unwrap()[0].clone();
        let u = last_user_text(&first);
        assert!(u.starts_with("<session_input hook=\"on_init\""), "{u}");
        assert!(u.contains("first question"), "{u}");
        match mode {
            InputMode::Single => {
                assert!(!u.contains("second question"), "{u}");
                assert_eq!(llm.count(), 2, "the second input opened its own Turn");
                let second = llm.requests.lock().unwrap()[1].clone();
                let u2 = last_user_text(&second);
                assert!(u2.starts_with("<session_input hook=\"on_input\""), "{u2}");
                assert!(u2.contains("second question"));
            }
            InputMode::Batch => {
                assert!(u.contains("second question"), "{u}");
                assert_eq!(llm.count(), 1, "no second on_input batch");
            }
        }
        assert_eq!(sd.state().unwrap().source("q").acked_index, 2);
    }
}

/// `input.media = inline`: the same text plus image / document blocks; the
/// history keeps the text, whose attachment lines locate the objects.
#[tokio::test]
async fn inline_media_blocks_follow_the_text() {
    let env = Env::new();
    let mut spec = work_spec("look at the picture");
    spec.prompt.input.media = InputMedia::Inline;
    let sd = env.create_work(spec).await;
    let image = ndn_lib::ObjId::new(&format!("cyfile:{}", "a1".repeat(32))).unwrap();
    let m = attach(
        text_msg(
            &parse_did(USER).unwrap(),
            &parse_did(AGENT).unwrap(),
            "what is this <thing>?",
        ),
        image.clone(),
        Some("photo.png".into()),
    );
    let posted = PostedInput::msg(APP, m, MsgDelivery::default()).unwrap();
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &posted).await.unwrap();
    let llm = ScriptedLlm::new(|req, _| {
        let user = req.messages.iter().rev().find(|m| m.role == AiRole::User).unwrap();
        assert_eq!(user.content.len(), 2, "{:?}", user.content);
        assert!(matches!(user.content[1], AiContent::Image { .. }));
        text("a cat")
    });
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await.is_finished());
    let users: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::UserMessage { content, .. } => Some(content),
            _ => None,
        })
        .collect();
    assert_eq!(users.len(), 1);
    assert!(users[0].contains(&format!(
        "<attachment index=\"0\" media=\"image\" name=\"photo.png\" mime=\"image/png\" obj_id=\"{}\"/>",
        image.to_string()
    )));
    assert!(users[0].contains("what is this &lt;thing&gt;?"));
    assert!(users[0].contains(&format!("from=\"alice.bns.did\" time=\"")), "{}", users[0]);
}

/// A re-posted record (same key) enters the context once; the default reply
/// path follows the last message of the batch.
#[tokio::test]
async fn reposts_are_deduplicated_and_reply_follows_the_last_message() {
    let env = Env::new();
    let sd = env.create_work(work_spec("chat")).await;
    let ch = env.channels();
    let q = queue_of(&sd);
    let (m1, m2) = (msg("hello once"), msg("and the last one"));
    for p in [&m1, &m1, &m2, &m1] {
        libopendan::channel::kmsg::post_to_queue(&ch.client(), &q, p).await.unwrap();
    }
    let llm = ScriptedLlm::new(|_, _| text("hi"));
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await.is_finished());
    let t = llm.transcript(0);
    assert_eq!(t.matches("hello once").count(), 1, "{t}");
    let st = sd.state().unwrap();
    assert_eq!(st.source("q").acked_index, 4);
    assert!(matches!(
        &st.reply,
        Some(ReplyRoute::Message { to, reply_to: Some(r), .. }) if to == USER && r == &m2.key
    ));
}

/// After a protocol upgrade an old session is read-only until migrated: no
/// append, no drive; reading it still works.
#[tokio::test]
async fn an_old_session_is_read_only() {
    let env = Env::new();
    let sd = env.create_work(work_spec("old")).await;
    let path = sd.state_dir().join("state.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v["schema"] = json!("opendan.session_state/4");
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let err = libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg("hi"))
        .await
        .unwrap_err();
    assert!(matches!(err, OpenDanError::SessionReadonly { .. }), "{err}");
    let llm = ScriptedLlm::new(|_, _| text("never"));
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(matches!(r, DriveResult::RecoveryBlocked(_)), "{r:?}");
    assert_eq!(llm.count(), 0);
    assert_eq!(sd.state().unwrap().rev, 1, "nothing was written");
}

/// A template that fails (unknown format) keeps the scene: the selected
/// input is neither consumed nor confirmed, nothing is inferred.
#[tokio::test]
async fn a_failing_template_consumes_nothing() {
    let env = Env::new();
    let mut spec = work_spec("chat");
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    spec.prompt.templates.on_input =
        Some("{{ input | render_format: \"no.such.format\" }}".into());
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, _| text("ok"));
    let deps = env.deps(llm.clone());
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1);
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg("hello")).await.unwrap();
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(
        matches!(&r, DriveResult::Error { error, .. } if error["message"].as_str().unwrap().contains("unknown format")),
        "{r:?}"
    );
    assert_eq!(llm.count(), 1);
    assert_eq!(sd.state().unwrap().source("q").acked_index, 0);
}

/// The user's time zone is bound to the session and shown through its
/// default semi subscription; rendered times are UTC whatever the runner's
/// zone is. A later change waits for the next controlled input.
#[tokio::test]
async fn user_timezone_is_a_default_semi_subscription() {
    let env = Env::new();
    let mut spec = work_spec("chat");
    spec.timezone = Some("Asia/Shanghai".into());
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, n| {
        let users = user_texts(req);
        let k = users.len();
        if n == 0 {
            assert!(users[0].contains("Asia/Shanghai"), "{users:?}");
            assert!(users[1].contains("Z\">"), "UTC with a Z suffix: {users:?}");
        } else {
            assert!(users[k - 2].contains("America/Los_Angeles"), "{users:?}");
        }
        text("ok")
    });
    let deps = env.deps(llm.clone());
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1);
    let agent = env.agent();
    let tz = PostedInput::event(
        APP,
        "timezone:America/Los_Angeles",
        AgentEvent {
            subscription_id: None,
            source: EventSource::new("system", USER_TIMEZONE_SOURCE_ID),
            event: "timezone".into(),
            seq: None,
            summary: "The user's time zone is America/Los_Angeles.".into(),
            data_ref: None,
            terminal: false,
        },
    );
    libopendan::post_input(agent.as_ref(), sd.sid(), &tz).await.unwrap();
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 1, "the change does not wake the session");
    libopendan::post_input(agent.as_ref(), sd.sid(), &msg("what time is it?")).await.unwrap();
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 2);
}

/// A run filled once may suspend again on another task: the new wait is
/// kept, the old result is not filled a second time.
#[tokio::test]
async fn a_filled_run_can_wait_for_another_task() {
    let env = Env::new();
    let sd = env.create_work(task_spec("two builds")).await;
    let tasks = Arc::new(FakeTasks::default());
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => tool_call("t1", "start_task", json!({ "id": "1", "mode": "wait" })),
        1 => {
            assert_eq!(has_tool_result(req, "t1").as_deref(), Some("first ok"));
            tool_call("t2", "start_task", json!({ "id": "2", "mode": "wait" }))
        }
        _ => {
            assert_eq!(has_tool_result(req, "t2").as_deref(), Some("second ok"));
            text("both done")
        }
    });
    let deps = task_deps(&env, llm.clone(), Some(tasks.clone()));
    drive(&sd, &deps, StopWhen::Idle).await;
    tasks.finish("bucky:1", "first ok");
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }), "{r:?}");
    assert_eq!(llm.count(), 2);
    assert_eq!(
        sd.state().unwrap().waiting_for.unwrap().refs,
        vec!["bucky:2".to_string()]
    );
    tasks.finish("bucky:2", "second ok");
    assert!(drive(&sd, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(llm.count(), 3);
    assert_eq!(results_of(&sd, "t1").len(), 1);
    assert_eq!(results_of(&sd, "t2").len(), 1);
    assert_eq!(sd.state().unwrap().turns_completed, 1);
}

/// `inline` media refused by the provider: the session removes the blocks,
/// says so in the text and retries once; the attachment stays locatable.
#[tokio::test]
async fn refused_inline_media_degrades_to_references_once() {
    let env = Env::new();
    let mut spec = work_spec("look at the picture");
    spec.prompt.input.media = InputMedia::Inline;
    let sd = env.create_work(spec).await;
    let image = ndn_lib::ObjId::new(&format!("cyfile:{}", "a1".repeat(32))).unwrap();
    let m = attach(
        text_msg(&parse_did(USER).unwrap(), &parse_did(AGENT).unwrap(), "what is this?"),
        image.clone(),
        Some("photo.png".into()),
    );
    let posted = PostedInput::msg(APP, m, MsgDelivery::default()).unwrap();
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &posted).await.unwrap();
    let llm = ScriptedLlm::fallible(move |req, _| {
        let user = req.messages.iter().rev().find(|m| m.role == AiRole::User).unwrap();
        if user.content.iter().any(|c| matches!(c, AiContent::Image { .. })) {
            return Err(llm_context::error::LLMComputeError::Provider {
                failure: llm_context::error::ProviderFailure::Permanent,
                message: "image input is not supported by this model".into(),
            });
        }
        let t = user.text_content();
        assert!(t.contains("blocks of this message were removed"), "{t}");
        assert!(t.contains(&image.to_string()), "still locatable: {t}");
        Ok(text("cannot see it, reading the file instead"))
    });
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2, "one refused request, one retry");
    // A text-only request that is refused is not retried this way.
    let sd2 = env.create_work(work_spec("plain")).await;
    let always = ScriptedLlm::fallible(|_, _| {
        Err(llm_context::error::LLMComputeError::Provider {
            failure: llm_context::error::ProviderFailure::Permanent,
            message: "no".into(),
        })
    });
    let r = drive(&sd2, &env.deps(always.clone()), StopWhen::Finished).await;
    assert!(matches!(r, DriveResult::Error { .. }), "{r:?}");
    assert_eq!(always.count(), 1);
}

const ENV_C16_ROOT: &str = "LIBOPENDAN_TEST_C16_ROOT";
const ENV_C16_SESSION: &str = "LIBOPENDAN_TEST_C16_SESSION";

fn c16_script() -> Arc<ScriptedLlm> {
    ScriptedLlm::new(|req, _| {
        let u = last_user_text(req);
        if u.contains("source=\"task:bucky:2\"") {
            assert!(u.contains("task bucky:2 finished: all green"), "{u}");
            text("the build is green")
        } else if has_tool_result(req, "b1").is_some() {
            text("started; I will report when it ends")
        } else {
            tool_call("b1", "start_task", json!({ "id": "2", "mode": "background" }))
        }
    })
}

/// Child entry point of the C16 test: a no-op unless spawned by it.
#[tokio::test]
async fn c16_child_driver() {
    let (Ok(root), Ok(session)) = (std::env::var(ENV_C16_ROOT), std::env::var(ENV_C16_SESSION)) else {
        return;
    };
    let env = Env::at(std::path::Path::new(&root));
    let sd = SessionDir::open(&session).unwrap();
    let deps = task_deps(&env, c16_script(), Some(Arc::new(FakeTasks::default())));
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    eprintln!("child drive result: {r:?}");
}

/// C16 / E27: the run that started a background task ends, its terminal
/// record is on disk, and the process dies before the session state took
/// the task over. The redone finish finds the task in the run's own record
/// (no resolver of this process ever saw the run) and its completion is
/// delivered once.
#[tokio::test]
async fn a_background_task_survives_a_crash_before_the_run_end_commit() {
    let env = Env::new();
    let mut spec = task_spec("start the build and wait for news");
    spec.end_condition = EndCondition {
        kind: EndConditionType::MaxTurns,
        detail: json!({ "n": 2 }),
    };
    let sd = env.create_work(spec).await;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "c16_child_driver", "--nocapture", "--test-threads=1"])
        .env(ENV_C16_ROOT, &env.root)
        .env(ENV_C16_SESSION, sd.path())
        .env("LIBOPENDAN_FAULT", "outcome:after_checkpoint")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "child did not exit");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success(), "the child must die after the run's terminal checkpoint");
    let before = sd.state().unwrap();
    assert!(before.live_run.is_some() && before.watched_tasks.is_empty(), "state never saw the run end");

    let tasks = Arc::new(FakeTasks::default());
    let llm = c16_script();
    let deps = task_deps(&env, llm.clone(), Some(tasks.clone()));
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(llm.count(), 0, "the finish is redone, nothing is inferred again");
    let st = sd.state().unwrap();
    assert_eq!(st.watched_tasks, vec!["bucky:2".to_string()], "found in the run's record");
    assert_eq!(st.turns_completed, 1);
    // Still running: nothing to infer on.
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(llm.count(), 0);
    tasks.finish("bucky:2", "all green");
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 1, "delivered exactly once");
    let st = sd.state().unwrap();
    assert!(st.watched_tasks.is_empty());
    assert_eq!((st.turn_seq, st.turns_completed), (2, 2));
    assert_eq!(drive(&sd, &deps, StopWhen::Idle).await.is_finished(), true);
    assert_eq!(llm.count(), 1);
}
