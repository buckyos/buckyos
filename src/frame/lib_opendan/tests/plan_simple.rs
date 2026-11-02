//! plan_simple (notepads/opendan-plan-simple-async-todo.md): the plan
//! session's class and function contract, and a crash around its
//! observation. The scheduling, update, pause and end of a plan are in
//! `plan_simple_async.rs`.
//!
//! The Jarvis package of this repository is installed as the AgentRoot
//! (the real `plan_simple`, `work_simple` and `explore` behaviors and the
//! `[session.plan_simple]` class); the models are scripted, everything else
//! is real: `agent-session` is the `xagent` binary, work sessions are driven
//! by the host's ChildDriver, the list, registry and worklogs are the
//! committed files.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::plan::*;
use common::*;
use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::todos::TodoStatus;
use libopendan::{SessionDir, SessionTemplate};
use serde_json::json;

/// V-config: the real package's classes and behavior. A Work is a plan
/// session by default; the conversation's plan (`plan_simple`) has a queue
/// and a rare tick; the plan reads its workspace only and its inputs place
/// the background; the model receives `intent` as a
/// required argument of every function (the strict report included) and no
/// XML action protocol; there is no behavior loop in the package any more.
#[tokio::test]
async fn the_plan_simple_class_and_its_function_contract() {
    let env = Env::new();
    install_jarvis(&env);
    let t = SessionTemplate::load("plan_simple", Some(&env.agent_root)).unwrap();
    assert_eq!(t.kind, SessionKind::Work);
    assert_eq!(t.completion, CompletionPolicy::ExplicitReport);
    assert_eq!(t.default_behavior.as_deref(), Some("plan_simple"));
    assert_eq!(t.input, libopendan::InputChannel::Queue);
    assert_eq!(t.tick_secs, 600);
    let work = SessionTemplate::load("work", Some(&env.agent_root)).unwrap();
    assert_eq!(work.default_behavior.as_deref(), Some("plan_simple"), "a Work is a plan by default");
    assert_eq!(work.input, libopendan::InputChannel::None);
    for gone in ["plan", "do"] {
        assert!(env.agent().behaviors().get(gone).await.unwrap().is_none(), "no `{gone}` behavior");
    }
    let b = env.agent().behaviors().get("plan_simple").await.unwrap().unwrap();
    assert_eq!(b.capabilities.workspace, Some(WorkspaceAccess::ReadOnly));
    let entry = b.to_entry("plan_simple", &json!({}), true).unwrap();
    assert!(entry.call_intent);
    assert!(entry.prompt.on_input.as_deref().unwrap().contains("{{ background_context }}"));
    for child in ["work_simple", "explore"] {
        let c = env.agent().behaviors().get(child).await.unwrap().unwrap();
        assert!(!c.capabilities.call_intent, "{child} keeps its own contract");
        assert_eq!(c.capabilities.workspace, None, "{child} writes what it is given");
    }
    tool_env(&env);
    let project = env.root.join("project");
    std::fs::create_dir(&project).unwrap();
    let mut spec = plan_spec(&env, "check the contract");
    spec.workspace = Some(env.workspace(&project).await);
    let sd = env.create_work(spec).await;
    let binding = sd.config().unwrap().workspace_binding.unwrap();
    assert_eq!(binding.access, WorkspaceAccess::ReadOnly, "the plan reads its workspace");
    let specs: Arc<Mutex<Vec<llm_context::deps::ToolSpecLite>>> = Arc::default();
    let sys = Arc::new(Mutex::new(String::new()));
    let (s2, y2) = (specs.clone(), sys.clone());
    let ws = project.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        if s2.lock().unwrap().is_empty() {
            *s2.lock().unwrap() = req.tool_specs.clone();
            *y2.lock().unwrap() = system_of(req);
        }
        // Its file tools never write the workspace.
        match has_tool_result(req, "w") {
            None => tool_call(
                "w",
                "write_file",
                json!({ "intent": "try to write the workspace", "path": ws.join("x.txt").display().to_string(), "content": "x" }),
            ),
            Some(r) => {
                assert!(!ws.join("x.txt").exists(), "{r}");
                tool_call("r", "report", json!({ "intent": "nothing to plan: finish", "report": "nothing to do", "is_end": true }))
            }
        }
    });
    let out = drive_to_end(&env, llm, &sd).await;
    assert!(matches!(out.result, libopendan::runner::DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. }), "{:?}", out.result);
    assert!(!project.join("x.txt").exists());
    let specs = specs.lock().unwrap().clone();
    let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"shell") && names.contains(&"report"), "{names:?}");
    for s in &specs {
        let required = s.args_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "intent"), "{}: {}", s.name, s.args_schema);
        assert_eq!(s.args_schema["properties"]["intent"]["type"], "string", "{}", s.name);
    }
    let report = specs.iter().find(|s| s.name == "report").unwrap();
    assert_eq!(report.args_schema["additionalProperties"], false, "the report stays strict");
    let sys = sys.lock().unwrap().clone();
    assert!(sys.contains("carries `intent`"), "the runtime protocol states the contract");
    assert!(!sys.contains("<next_behavior>") && !sys.contains("<actions>"), "no XML protocol: {sys}");
    assert!(!sys.contains("todo dispatch") && !sys.contains("todo review") && !sys.contains("agent-session wait"), "{sys}");
}

const ENV_ROOT: &str = "PLAN_SIMPLE_CRASH_ROOT";
const ENV_SESSION: &str = "PLAN_SIMPLE_CRASH_SESSION";

/// A small plan: one Todo, started by its `todo add`, whose end comes back
/// as input; a look at the list, delivered. The work session appends one
/// line per execution.
fn crash_script(path: PathBuf) -> Arc<ScriptedLlm> {
    ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => panic!("no explorations"),
        Role::Work => {
            if has_tool_result(req, "w1").is_none() {
                return tool_call("w1", "shell", json!({ "command": "sleep 0.3; echo run >> ran.txt" }));
            }
            tool_call("w2", "report", json!({ "report": "T01: ran", "is_end": true }))
        }
        Role::Plan => {
            if has_tool_result(req, "q1").is_none() {
                return shell("q1", "Record the only Todo; it starts by itself.", "agent-session todo add \"run once\" --content \"Append one line to ran.txt.\" --accept \"one line\"");
            }
            let list = list_of(&path);
            let t = list.get("T01").unwrap();
            if !t.latest().is_some_and(|a| a.ended()) {
                return text("T01 runs.");
            }
            if has_tool_result(req, "q2").is_none() {
                return shell("q2", "Look at the list once T01 ended.", "agent-session todo list");
            }
            let texts = user_texts(req);
            let ends: Vec<&String> = texts
                .iter()
                .filter(|t| t.contains("of todo T01 (attempt 1) finished"))
                .collect();
            assert_eq!(ends.len(), 1, "the end of T01's work session comes in once: {texts:?}");
            assert!(
                !texts.iter().any(|t| t.contains("<todo id=")),
                "a Todo has no background line of its own: {texts:?}"
            );
            tool_call("q3", "report", json!({ "intent": "Deliver.", "report": "ran once", "is_end": true }))
        }
    })
}

/// The driving child process of the crash tests (does nothing in a normal
/// test run).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plan_crash_child() {
    let Ok(root) = std::env::var(ENV_ROOT) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    let sid = std::env::var(ENV_SESSION).unwrap();
    let e = env.agent().sessions().lookup(&sid).await.unwrap().unwrap();
    let sd = SessionDir::open(&e.location).unwrap();
    let out = drive_to_end(&env, crash_script(sd.path().to_path_buf()), &sd).await;
    panic!("the crash child was expected to die, it ended with {:?}", out.result);
}

/// V-end recovery: the host dies right after the batch that carries
/// the end of the Todo's work session was published, or right after it was
/// committed to the session state (batches in the host process: the plan's
/// first, the work session's first, then this one). A new host delivers the
/// end exactly once, runs no call again and creates no second work session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_end_of_a_todo_survives_a_crash_around_its_batch() {
    for fault in ["input_batch:after_input_checkpoint#3", "input_batch:after_state_commit#3"] {
        let env = Env::new();
        install_jarvis(&env);
        let sd = env.create_work(plan_spec(&env, "run once")).await;
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "plan_crash_child", "--nocapture", "--test-threads=1"])
            .env(ENV_ROOT, &env.root)
            .env(ENV_SESSION, sd.sid())
            .env("LIBOPENDAN_FAULT", fault)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "{fault}: the host must die");
        tool_env(&env);
        let llm = crash_script(sd.path().to_path_buf());
        let out = drive_to_end(&env, llm.clone(), &sd).await;
        let st = sd.state().unwrap();
        assert!(
            matches!(out.result, libopendan::runner::DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. }),
            "{fault}: {:?} {:?}",
            out.result,
            st.last_error
        );
        assert_eq!(list_of(sd.path()).get("T01").unwrap().status, TodoStatus::Completed, "{fault}");
        assert_eq!(st.turns_completed, st.turn_seq, "{fault}: every Turn completed");
        let children = env.agent().sessions().children_of(&[sd.sid().to_string()]).await.unwrap();
        assert_eq!(children.len(), 1, "{fault}");
        let ran = std::fs::read_to_string(Path::new(&children[0].location).join("ran.txt")).unwrap();
        assert_eq!(ran.lines().count(), 1, "{fault}: the work session ran once");
    }
}
