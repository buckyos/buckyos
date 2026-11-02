//! Simple Work Session, all asynchronous (notepads/
//! opendan-simple-worksession-async-todo.md §5 B10): a `work_simple`
//! session of the Jarvis package has many Turns — a reply closes one, the
//! end of a task it started or a timer tick opens the next — and only
//! `report(is_end=true)` ends it. Long `shell` commands become tasks of
//! the runner process after the in-call wait; the session follows them
//! itself (no `task wait`), shows their new output before every inference
//! and gives a reply that ends nothing one correction.
//!
//! The Jarvis package of this repository is the AgentRoot (the real
//! `work_simple` behavior and `[session.work]` class); the model is
//! scripted. Commands are real `sleep`s (never `std::thread::sleep` in a
//! script: it holds a runtime worker).

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole};
use common::*;
use libopendan::protocol::*;
use libopendan::runner::{
    drive, DriveResult, RunnerDeps, RunnerOptions, StopWhen, TurnTaskEnd, TurnTaskOpen,
    TurnTaskSink, TurnTaskStatus,
};
use libopendan::{SessionDir, SessionTemplate};
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

const PACKAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/jarvis_agent/agent");

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// The Jarvis package as the AgentRoot.
fn install_jarvis(env: &Env) {
    let pkg = Path::new(PACKAGE);
    for f in ["agent.toml", "role.md", "self.md"] {
        std::fs::copy(pkg.join(f), env.agent_root.join(f)).unwrap();
    }
    for d in ["behaviors", "cmd_skills"] {
        copy_dir(&pkg.join(d), &env.agent_root.join(d));
    }
}

/// Turn tasks as a host's task service sees them.
#[derive(Default)]
struct Tasks {
    opened: Mutex<Vec<TurnTaskOpen>>,
    updates: Mutex<Vec<(String, TurnTaskStatus)>>,
    closed: Mutex<Vec<(String, TurnTaskEnd)>>,
}

#[async_trait]
impl TurnTaskSink for Tasks {
    async fn open(&self, turn: &TurnTaskOpen) -> Result<String, String> {
        let mut o = self.opened.lock().unwrap();
        if let Some(i) = o.iter().position(|t| t.session_id == turn.session_id && t.turn == turn.turn) {
            return Ok(format!("t-turn-{i}"));
        }
        o.push(turn.clone());
        Ok(format!("t-turn-{}", o.len() - 1))
    }
    async fn update(&self, task_id: &str, status: &TurnTaskStatus) -> Result<(), String> {
        self.updates
            .lock()
            .unwrap()
            .push((task_id.to_string(), status.clone()));
        Ok(())
    }
    async fn close(&self, task_id: &str, end: &TurnTaskEnd) -> Result<(), String> {
        let mut c = self.closed.lock().unwrap();
        if !c.iter().any(|(id, _)| id == task_id) {
            c.push((task_id.to_string(), end.clone()));
        }
        Ok(())
    }
}

struct Setup {
    env: Env,
    tasks: Arc<Tasks>,
}

fn setup() -> Setup {
    let env = Env::new();
    install_jarvis(&env);
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    std::env::remove_var("LIBOPENDAN_FAULT");
    Setup {
        env,
        tasks: Arc::new(Tasks::default()),
    }
}

/// A `work_simple` session of the `work` class as `create-worksession`
/// makes it; commands become tasks after 300 ms; `tick_secs` as given.
async fn create(s: &Setup, objective: &str, tick_secs: u64) -> SessionDir {
    let mut spec = SessionTemplate::load("work", Some(&s.env.agent_root))
        .unwrap()
        .spec(objective);
    spec.prompt.behavior = Some("work_simple".into());
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true },
        "runtime": { "task_wait_ms": 300 }
    });
    spec.policy.tick_secs = tick_secs;
    spec.freeze = true;
    s.env.create_work(spec).await
}

fn deps(s: &Setup, llm: Arc<ScriptedLlm>) -> RunnerDeps {
    let mut d = s
        .env
        .deps(llm)
        .with_session_cli(PathBuf::from(env!("CARGO_BIN_EXE_xagent")))
        .with_options(RunnerOptions {
            poll_interval: Duration::from_millis(50),
            max_wait: Duration::from_secs(30),
            load_hints: false,
            ..Default::default()
        });
    d.turn_tasks = Some(s.tasks.clone());
    d
}

fn shell(call_id: &str, command: &str, intent: Option<&str>) -> AiResponse {
    let mut args = json!({ "command": command });
    if let Some(i) = intent {
        args["intent"] = json!(i);
    }
    tool_call(call_id, "shell", args)
}

/// Several tool calls in one response.
fn calls(list: &[(&str, &str, Value)]) -> AiResponse {
    AiResponse::new(AiMessage::new(
        AiRole::Assistant,
        list.iter()
            .map(|(id, name, args)| {
                AiContent::tool_use(
                    *id,
                    *name,
                    args.as_object().unwrap().clone().into_iter().collect(),
                )
            })
            .collect(),
    ))
}

fn report_end(text_: &str) -> AiResponse {
    tool_call("rep", "report", json!({ "report": text_, "is_end": true }))
}

/// User messages after the last assistant message: what this inference got.
fn new_inputs(req: &LlmInferenceRequest) -> String {
    let last_assistant = req
        .messages
        .iter()
        .rposition(|m| m.role == AiRole::Assistant)
        .map(|i| i + 1)
        .unwrap_or(0);
    req.messages[last_assistant..]
        .iter()
        .filter(|m| m.role == AiRole::User)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

fn all_requests(llm: &ScriptedLlm) -> Vec<String> {
    llm.requests
        .lock()
        .unwrap()
        .iter()
        .map(|r| render(&r.messages))
        .collect()
}

async fn drive_to_end(s: &Setup, llm: &Arc<ScriptedLlm>, sd: &SessionDir) -> DriveResult {
    drive(sd, &deps(s, llm.clone()), StopWhen::Finished).await
}

// 1. A command that ends within the wait is a synchronous result: no task,
//    one Turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_short_command_is_synchronous() {
    let s = setup();
    let sd = create(&s, "print hello", 30).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "echo hello", None),
        1 => {
            let r = has_tool_result(req, "c1").unwrap();
            assert!(r.contains("hello") && !r.contains("task"), "{r}");
            report_end("printed hello")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let r = drive_to_end(&s, &llm, &sd).await;
    assert!(r.is_finished(), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    assert_eq!(st.turns_completed, 1);
    assert!(st.watched_tasks.is_empty());
    assert_eq!(llm.count(), 2);
}

// 2 + 12b + 9. A long command: the reply closes Turn 1 (the session waits),
//    the task's end opens Turn 2 with its whole result in the same run, and
//    `report` ends the session. Nobody waits for the task; the intent shows
//    as progress and in the worklog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_command_closes_the_turn_and_its_end_opens_the_next() {
    let s = setup();
    let sd = create(&s, "build it", 0).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => {
            let system = system_of(req);
            assert!(system.contains("Only `report` with `is_end: true` ends this session"), "{system}");
            let shell_spec = req.tool_specs.iter().find(|t| t.name == "shell").expect("shell");
            assert!(shell_spec.args_schema["properties"].get("wait_ms").is_none());
            assert!(shell_spec.args_schema["properties"]["intent"].is_object());
            assert!(req.tool_specs.iter().all(|t| t.name != "wait_task"));
            shell("c1", "sleep 1; echo built", Some("build the crate"))
        }
        1 => {
            let r = has_tool_result(req, "c1").unwrap();
            assert!(r.contains("it continues as task local:shell:c1"), "{r}");
            assert!(r.contains("comes back to you as input"), "{r}");
            assert!(!r.contains("wait_task") && !r.contains("task wait"), "{r}");
            text("The build is still running.")
        }
        2 => {
            // Turn 2 in the same run: Turn 1's call is in its history.
            assert!(has_tool_result(req, "c1").is_some());
            let input = new_inputs(req);
            assert!(
                input.contains("<<task_result task=\"local:shell:c1\" call_id=\"c1\" status=\"finished\">>"),
                "{input}"
            );
            assert!(input.contains("Call c1: shell `sleep 1; echo built` — intent: build the crate"), "{input}");
            assert!(input.contains("built"), "{input}");
            assert!(input.contains("This is the end of task local:shell:c1"), "{input}");
            report_end("built")
        }
        _ => panic!("unexpected inference {n}: {}", last_user_text(req)),
    });
    let r = drive_to_end(&s, &llm, &sd).await;
    assert!(r.is_finished(), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    assert_eq!(st.turns_completed, 2);
    // Two inferences for Turn 1 (dispatch, the reply that closes it), one
    // for Turn 2: nothing in between waits for the task.
    assert_eq!(llm.count(), 3);
    assert!(all_requests(&llm).iter().all(|r| !r.contains("<tick")));
    // One run for both Turns.
    assert_eq!(sd.runs().list().unwrap().len(), 1);
    // Each Turn has its task, the first with the reply as its result.
    let opened = s.tasks.opened.lock().unwrap().clone();
    assert_eq!(opened.iter().map(|t| t.turn).collect::<Vec<_>>(), [1, 2]);
    let closed = s.tasks.closed.lock().unwrap().clone();
    assert_eq!(closed.len(), 2, "{closed:?}");
    assert_eq!(closed[0].1.status, TurnStatus::Completed);
    assert!(closed[0].1.summary.as_deref().unwrap_or("").contains("still running"), "{closed:?}");
    let updates = s.tasks.updates.lock().unwrap().clone();
    assert!(
        updates.iter().any(|(_, u)| u.message == "running shell: build the crate"),
        "{:?}",
        updates.iter().map(|(_, u)| u.message.clone()).collect::<Vec<_>>()
    );
    // The worklog: the reply that closed Turn 1, both Turn ends, the intent.
    let log = read_worklog(&sd);
    let turn_ends: Vec<u64> = log
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::TurnEnded { turn, .. } => Some(*turn),
            _ => None,
        })
        .collect();
    assert_eq!(turn_ends, [1, 2]);
    assert!(log.iter().any(|e| matches!(&e.body,
        WorklogBody::Outcome { kind, report: Some(r), turn: 1, .. } if kind == "wait" && r.contains("still running"))));
    assert!(log.iter().any(|e| matches!(&e.body,
        WorklogBody::AssistantMessage { tool_calls, .. }
            if tool_calls.iter().any(|c| c.intent.as_deref() == Some("build the crate")))));
}

// 3 + 4 + 11. Two commands side by side: ticks open Turns while they run,
//    the new output of a task is shown before the inference (only what is
//    new), each end opens a Turn in the order the tasks ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_tasks_tick_and_show_only_new_output() {
    let s = setup();
    let sd = create(&s, "build both", 1).await;
    let ends: Arc<Mutex<Vec<String>>> = Default::default();
    let seen = ends.clone();
    let llm = ScriptedLlm::new(move |req, n| {
        if n == 0 {
            return calls(&[
                (
                    "c1",
                    "shell",
                    json!({ "command": "echo a1; for i in 2 3 4 5; do sleep 0.7; echo a$i; done" }),
                ),
                ("c2", "shell", json!({ "command": "sleep 4.5; echo b-done" })),
            ]);
        }
        let input = new_inputs(req);
        for id in ["c1", "c2"] {
            if input.contains(&format!("<<task_result task=\"local:shell:{id}\"")) {
                seen.lock().unwrap().push(id.to_string());
            }
        }
        if seen.lock().unwrap().len() == 2 {
            return report_end("both built");
        }
        text("Still running.")
    });
    let r = drive_to_end(&s, &llm, &sd).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(*ends.lock().unwrap(), ["c1", "c2"], "ends in the order the tasks ended");
    let reqs = all_requests(&llm);
    assert!(reqs.iter().any(|r| r.contains("<tick timer=\"task_watch\">")), "a tick opened a Turn");
    let backgrounds: Vec<String> = llm
        .requests
        .lock()
        .unwrap()
        .iter()
        .flat_map(|r| r.messages.iter().map(|m| m.text_content()))
        .filter(|t| t.starts_with("<<background_context>>"))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert!(
        backgrounds.iter().any(|b| b.contains("task local:shell:c1") && b.contains("new output")),
        "{backgrounds:#?}"
    );
    // Only the new output: the call's own result showed `a1`, and every
    // later line is shown once at most.
    let shown: Vec<&str> = backgrounds
        .iter()
        .flat_map(|b| b.split("new output:\n").skip(1))
        .flat_map(|o| o.lines().take_while(|l| !l.starts_with("- task") && !l.starts_with("<</")))
        .collect();
    assert!(!shown.contains(&"a1"), "{backgrounds:#?}");
    assert!(shown.contains(&"a2"), "{backgrounds:#?}");
    for line in ["a2", "a3", "a4", "a5"] {
        assert!(shown.iter().filter(|l| **l == line).count() <= 1, "{line}: {backgrounds:#?}");
    }
    let st = sd.state().unwrap();
    assert!(st.turns_completed >= 3, "{}", st.turns_completed);
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
}

// 5. A failed task comes back as `status="failed"` with its exit code and
//    output.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_task_is_reported_as_failed() {
    let s = setup();
    let sd = create(&s, "run the check", 0).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "sleep 0.8; echo oops >&2; exit 3", None),
        1 => text("The check runs."),
        2 => {
            let input = new_inputs(req);
            assert!(input.contains("call_id=\"c1\" status=\"failed\">>"), "{input}");
            assert!(input.contains("exit=3") && input.contains("oops"), "{input}");
            report_end("the check failed with exit 3")
        }
        _ => panic!("unexpected inference {n}"),
    });
    assert!(drive_to_end(&s, &llm, &sd).await.is_finished());
    assert_eq!(llm.count(), 3);
}

// 6 + 10. A reply that ends nothing and waits for nothing gets one
//    correction (its own input) and then fails the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_reply_without_work_is_nudged_once_then_fails() {
    let s = setup();
    let sd = create(&s, "say done", 0).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => text("I think it is done."),
        1 => {
            let input = new_inputs(req);
            assert!(input.contains("<runner_notice>Nothing you started is running"), "{input}");
            text("Still done.")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let r = drive_to_end(&s, &llm, &sd).await;
    assert!(r.is_finished(), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Failed));
    assert_eq!(
        st.last_error.as_ref().and_then(|e| e.get("kind")).and_then(Value::as_str),
        Some("missing_end_report"),
        "{:?}",
        st.last_error
    );
    assert_eq!(llm.count(), 2);
}

// 7. `report(is_end=true)` is refused while a task runs; cancelled, the
//    session can end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_final_report_waits_for_running_tasks_or_their_cancel() {
    let s = setup();
    let sd = create(&s, "start and stop", 0).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "sleep 30", None),
        1 => report_end("done"),
        2 => {
            let r = has_tool_result(req, "rep").unwrap();
            assert!(r.contains("cannot finish while tasks you started are still active (local:shell:c1)"), "{r}");
            tool_call("cx", "cancel_task", json!({ "task_id": "local:shell:c1" }))
        }
        3 => {
            let r = has_tool_result(req, "cx").unwrap();
            assert!(r.contains("cancelled"), "{r}");
            tool_call("rep2", "report", json!({ "report": "stopped it", "is_end": true }))
        }
        _ => panic!("unexpected inference {n}: {}", new_inputs(req)),
    });
    let r = drive_to_end(&s, &llm, &sd).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Succeeded));
    assert_eq!(llm.count(), 4);
}

// 8. A new drive goes on with the kept run: in the same process the task's
//    end opens the next Turn there; after a restart the end is read from
//    the command's execution directory (`exit` recorded), or is unknown.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_session_resumes_in_a_new_drive() {
    let s = setup();
    let sd = create(&s, "build it", 0).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "sleep 1.5; echo built", None),
        1 => text("Building."),
        2 => {
            let input = new_inputs(req);
            assert!(input.contains("status=\"finished\"") && input.contains("built"), "{input}");
            assert!(has_tool_result(req, "c1").is_some(), "same run, same history");
            report_end("built")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let first = deps(&s, llm.clone());
    let r = drive(&sd, &first, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(st.open_turn.is_none());
    assert!(st.live_run.is_some(), "the run is kept between Turns");
    assert_eq!(st.watched_tasks, ["local:shell:c1".to_string()]);
    let r = drive(&sd, &first, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(sd.state().unwrap().turns_completed, 2);
    assert_eq!(llm.count(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_a_restart_a_task_end_is_read_from_its_execution_directory() {
    let s = setup();
    let sd = create(&s, "build it", 0).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "sleep 1; echo built", None),
        1 => text("Building."),
        2 => {
            let input = new_inputs(req);
            assert!(input.contains("status=\"finished\""), "{input}");
            assert!(input.contains("exited with code 0") && input.contains("built"), "{input}");
            report_end("built")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let first = deps(&s, llm.clone());
    let r = drive(&sd, &first, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }), "{r:?}");
    // The host ends as the Loader does: its commands are handed to the OS.
    first.xllm.local_tasks.release_running().await;
    drop(first);
    // The command ends while no host runs; its execution directory records it.
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let exit = sd.runs().dir().join(&run_id).join("exec").join("c1").join("exit");
    let started = std::time::Instant::now();
    while !exit.exists() && started.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(exit.exists());
    // A new host process: its task manager does not know the task.
    let r = drive(&sd, &deps(&s, llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(sd.state().unwrap().turns_completed, 2);
    assert_eq!(llm.count(), 3);
}

// 12. The followup of a task is said once per session; a file read again
//     unchanged is one line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hints_are_shown_once_and_unchanged_reads_deduplicated() {
    let s = setup();
    let sd = create(&s, "two builds and a read", 0).await;
    let workdir = libopendan::runtime::resolve_workdir(&sd).unwrap();
    std::fs::write(workdir.join("notes.txt"), "alpha\nbeta\n").unwrap();
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "sleep 2; echo one", None),
        1 => {
            let r = has_tool_result(req, "c1").unwrap();
            assert!(r.contains("<runtime_hints>"), "{r}");
            shell("c2", "sleep 2.5; echo two", None)
        }
        2 => {
            let r = has_tool_result(req, "c2").unwrap();
            assert!(r.contains("local:shell:c2") && !r.contains("<runtime_hints>"), "said once: {r}");
            tool_call("r1", "read_file", json!({ "path": "notes.txt" }))
        }
        3 => {
            assert!(has_tool_result(req, "r1").unwrap().contains("beta"));
            tool_call("r2", "read_file", json!({ "path": "notes.txt" }))
        }
        4 => {
            let r = has_tool_result(req, "r2").unwrap();
            assert!(r.contains("unchanged since you read it") && !r.contains("beta"), "{r}");
            text("Both are running.")
        }
        _ => {
            let input = new_inputs(req);
            if input.contains("local:shell:c2") || sd_both_done(req) {
                report_end("done")
            } else {
                text("Waiting for the other one.")
            }
        }
    });
    let r = drive_to_end(&s, &llm, &sd).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(sd.state().unwrap().hints_once.contains("shell.task_followup"));
}

/// Both task results are in the request's history.
fn sd_both_done(req: &LlmInferenceRequest) -> bool {
    let all = render(&req.messages);
    all.contains("<<task_result task=\"local:shell:c1\"") && all.contains("<<task_result task=\"local:shell:c2\"")
}

// 13. `ask_user` (a TaskMgr task of a krpc tool, `t-`): the reply closes the
//     Turn, the answer opens the next one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_to_a_question_opens_the_next_turn() {
    let s = setup();
    let sd = create(&s, "ask and go on", 0).await;
    let backend = agent_tool::MemoryTaskBackend::new();
    backend.seed(agent_tool::ZoneTaskRecord {
        task_id: "t-0000000000000000000000000000ask1".into(),
        schema_id: Some("human.input/v1".into()),
        name: "which color?".into(),
        message: Some("which color?".into()),
        ..Default::default()
    });
    // `ask_user` stands in as a command that prints its pending envelope.
    let envelope = json!({
        "agent_tool_protocol": "1",
        "status": "pending",
        "task_id": "t-0000000000000000000000000000ask1",
        "summary": "asked the user",
        "detail": {}
    });
    let workdir = libopendan::runtime::resolve_workdir(&sd).unwrap();
    std::fs::write(workdir.join("ask.json"), envelope.to_string()).unwrap();
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("c1", "cat ask.json", None),
        1 => {
            let r = has_tool_result(req, "c1").unwrap();
            assert!(r.contains("t-0000000000000000000000000000ask1"), "{r}");
            text("I asked which color to use.")
        }
        2 => {
            let input = new_inputs(req);
            assert!(input.contains("<<task_result task=\"t-0000000000000000000000000000ask1\""), "{input}");
            assert!(input.contains("blue"), "{input}");
            report_end("used blue")
        }
        _ => panic!("unexpected inference {n}"),
    });
    let mut d = deps(&s, llm.clone());
    d.xllm.buckyos_tasks = Some(agent_tool::ZoneTaskResolver::new(backend.clone()));
    let r = drive(&sd, &d, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(sd.state().unwrap().turns_completed, 1);
    backend
        .complete("t-0000000000000000000000000000ask1", json!({"action": "submit", "values": {"color": "blue"}}))
        .unwrap();
    let r = drive(&sd, &d, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(sd.state().unwrap().turns_completed, 2);
    assert_eq!(llm.count(), 3);
}
