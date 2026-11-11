//! The plan session of notepads/opendan-plan-simple-async-todo.md: it only
//! keeps its TodoList, the host starts every Todo whose prerequisites
//! completed (at the plan's commands and whenever a work session ends), the
//! plan steers running work through `todo update` and `todo
//! pause|resume|stop`, takes the user's forwarded messages as input, checks
//! results with verification Todos and ends when it judges the work done.
//!
//! The Jarvis package is the AgentRoot; models are scripted, `agent-session`
//! is the real `xagent`, work sessions are driven by the host's ChildDriver.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buckyos_api::{AiContent, AiRole};
use common::plan::*;
use common::*;
use libopendan::protocol::*;
use libopendan::runner::DriveResult;
use libopendan::state::AgentStateClient;
use libopendan::todos::{TodoMode, TodoStatus};
use libopendan::SessionDir;
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

/// Call ids of the latest assistant message.
fn last_calls(req: &LlmInferenceRequest) -> Vec<String> {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == AiRole::Assistant)
        .map(|m| {
            m.content
                .iter()
                .filter_map(|c| match c {
                    AiContent::ToolUse { call_id, .. } => Some(call_id.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The request answers the result of call `id` (same Turn, right after it).
fn right_after(req: &LlmInferenceRequest, id: &str) -> bool {
    last_calls(req).iter().any(|c| c == id)
}

fn finished_ok(out: &DriveResult) -> bool {
    matches!(out, DriveResult::Finished { outcome: Some(Outcome::Succeeded), .. })
}

/// Every command the plan's shell calls ran.
fn plan_commands(sd: &SessionDir) -> Vec<String> {
    read_worklog(sd)
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::AssistantMessage { tool_calls, .. } => Some(tool_calls.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|c| c.args.get("command").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn todo_summary(sd: &SessionDir) -> Option<String> {
    read_worklog(sd).iter().find_map(|e| match &e.body {
        WorklogBody::ControlApplied { command, detail, .. } if command == "todo_summary" => {
            detail["summary"].as_str().map(str::to_string)
        }
        _ => None,
    })
}

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[derive(Default)]
struct Notes {
    /// T02 had an attempt when the plan first saw T01's end.
    t02_started_first: Option<bool>,
    t01_ended_at_plan: Option<u64>,
}

/// 1 / 2 / 8 / 9: the plan adds its whole list in one command (T01, T02
/// after it by default, T03 `--after none`, T04 after T03); T01 and T03
/// start before the command returns and run side by side; T01's end starts
/// T02 before the plan infers again; the plan adds a verification Todo
/// after T01 that starts at once; it never dispatches, waits or reviews;
/// the work sessions write the workspace the plan only reads; the end
/// records where every Todo stands.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_whole_list_at_once_runs_in_its_order() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let project = env.root.join("project");
    std::fs::create_dir(&project).unwrap();
    let mut spec = plan_spec(&env, "write a, b, c, d and verify a");
    spec.workspace = Some(env.workspace(&project).await);
    let sd = env.create_work(spec).await;
    let path = sd.path().to_path_buf();
    let notes = Arc::new(Mutex::new(Notes::default()));
    let n2 = notes.clone();
    let path_c = path.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Work => {
            let todo = todo_of(req);
            let first = user_texts(req).into_iter().next().unwrap();
            if todo == "T02" {
                assert!(first.contains("Builds on the results of:\n- T01 write a: T01 written"), "{first}");
            }
            if has_tool_result(req, "w1").is_none() {
                return tool_call(
                    "w1",
                    "shell",
                    json!({ "command": format!("sleep 0.5; echo {todo} >> \"$OPENDAN_WORKSPACE_DIR/{todo}.txt\"") }),
                );
            }
            tool_call("w2", "report", json!({ "report": format!("{todo} written"), "is_end": true }))
        }
        Role::Explore => {
            let todo = todo_of(req);
            if has_tool_result(req, "e1").is_none() {
                return tool_call("e1", "shell", json!({ "command": "sleep 0.3; pwd" }));
            }
            tool_call("e2", "report", json!({ "report": format!("{todo}: verified"), "is_end": true }))
        }
        Role::Plan => {
            let list = list_of(&path_c);
            if has_tool_result(req, "p1").is_none() {
                return shell(
                    "p1",
                    "Add the whole plan at once.",
                    "agent-session todo add \"write a\" --content \"Write T01.txt in the workspace.\" \
&& agent-session todo add \"write b\" --content \"Write T02.txt in the workspace.\" \
&& agent-session todo add \"write c\" --after none --content \"Write T03.txt in the workspace.\" \
&& agent-session todo add \"write d\" --content \"Write T04.txt in the workspace.\"",
                );
            }
            if right_after(req, "p1") {
                let outs = jsons_in(&has_tool_result(req, "p1").unwrap());
                assert_eq!(outs.len(), 4, "{outs:?}");
                assert_eq!(outs[0]["scheduled"][0]["result"], "dispatched", "{}", outs[0]);
                assert_eq!(outs[0]["scheduled"][0]["todo_id"], "T01");
                assert_eq!((outs[1]["status"].as_str(), outs[1]["after"].clone()), (Some("pending"), json!(["T01"])), "{}", outs[1]);
                assert_eq!(outs[2]["scheduled"][0]["todo_id"], "T03", "{}", outs[2]);
                assert_eq!((outs[3]["status"].as_str(), outs[3]["after"].clone()), (Some("pending"), json!(["T03"])), "{}", outs[3]);
                return text("Four Todos planned; T01 and T03 run.");
            }
            let t01 = list.get("T01").unwrap();
            if t01.status == TodoStatus::Completed && has_tool_result(req, "p2").is_none() {
                let mut n = n2.lock().unwrap();
                if n.t02_started_first.is_none() {
                    n.t02_started_first = Some(list.get("T02").unwrap().latest().is_some());
                    n.t01_ended_at_plan = t01.latest().unwrap().ended_at_ms;
                }
                drop(n);
                return shell(
                    "p2",
                    "Verify T01's file with a Todo of its own.",
                    "agent-session todo explore \"verify a\" --after T01 --content \"Check that T01.txt exists in the workspace.\"",
                );
            }
            if right_after(req, "p2") {
                let r = plan_result(req, "p2").unwrap();
                assert_eq!((r["todo_id"].as_str(), r["mode"].as_str()), (Some("T05"), Some("explore")), "{r}");
                assert_eq!(r["scheduled"][0]["result"], "dispatched", "T01 completed: it starts at once: {r}");
                return text("Verification of T01 runs.");
            }
            let all_ended = list.todos.len() == 5 && list.todos.iter().all(|t| t.latest().is_some_and(|a| a.ended()));
            if all_ended && has_tool_result(req, "p3").is_none() {
                return tool_call("p3", "report", json!({ "intent": "All Todos completed and T01 verified.", "report": "a, b, c, d written; T05 verified a", "is_end": true }));
            }
            text("Work runs.")
        }
    });
    let out = drive_to_end(&env, llm.clone(), &sd).await;
    let st = sd.state().unwrap();
    assert!(finished_ok(&out.result), "{:?} {:?}", out.result, st.last_error);
    let list = list_of(&path);
    for id in ["T01", "T02", "T03", "T04", "T05"] {
        let t = list.get(id).unwrap();
        assert_eq!((t.status, t.attempts.len()), (TodoStatus::Completed, 1), "{id}: {t:?}");
    }
    let (t01, t02, t03) = (list.get("T01").unwrap(), list.get("T02").unwrap(), list.get("T03").unwrap());
    assert!(t03.attempts[0].started_at_ms < t01.attempts[0].ended_at_ms.unwrap(), "T01 and T03 ran side by side");
    assert!(t02.attempts[0].started_at_ms >= t01.attempts[0].ended_at_ms.unwrap());
    assert_eq!(notes.lock().unwrap().t02_started_first, Some(true), "T02 started before the plan saw T01's end");
    let t05 = list.get("T05").unwrap();
    assert_eq!((t05.mode, t05.after.clone()), (TodoMode::Explore, vec!["T01".to_string()]));
    assert!(t05.attempts[0].workspace_id.is_none());
    for id in ["T01", "T02", "T03", "T04"] {
        assert_eq!(std::fs::read_to_string(project.join(format!("{id}.txt"))).unwrap(), format!("{id}\n"));
        assert!(list.get(id).unwrap().attempts[0].workspace_id.is_some(), "{id} wrote the workspace");
    }
    let cmds = plan_commands(&sd).join("\n");
    assert!(!cmds.contains("dispatch") && !cmds.contains("agent-session wait") && !cmds.contains("review"), "{cmds}");
    assert_eq!(todo_summary(&sd).as_deref(), Some("5 todos: 5 completed"));
    let children = env.agent().sessions().children_of(&[sd.sid().to_string()]).await.unwrap();
    assert_eq!(children.len(), 5);
    // A Todo's work session is titled by the Todo.
    for id in ["T01", "T05"] {
        let t = list.get(id).unwrap();
        let sid = t.attempts[0].child_sid.as_deref().unwrap();
        let entry = children.iter().find(|c| c.session_id == sid).unwrap();
        assert_eq!(entry.title.as_deref(), Some(t.title.as_str()), "{id}");
    }
    // One Turn for the list, one per end it heard at most, one to deliver.
    assert!(st.turn_seq <= 7, "{} Turns", st.turn_seq);
    assert_eq!(st.turns_completed, st.turn_seq);
}

/// 3 / 8: T01 fails, T02 after it is blocked (the end of T01's work session
/// says so) until the plan retries T01; its success starts T02. The
/// plan ends while T03 still runs: the end is accepted, T03's attempt ends
/// stopped and its work session is stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_failed_prerequisite_blocks_until_retried_and_the_end_stops_what_runs() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "a, then b; c on the side")).await;
    let path = sd.path().to_path_buf();
    let saw_block = Arc::new(AtomicBool::new(false));
    let b2 = saw_block.clone();
    let path_c = path.clone();
    let llm = ScriptedLlm::fallible(move |req, _| match role(req) {
        Role::Work => {
            let todo = todo_of(req);
            let attempt2 = user_texts(req).into_iter().next().unwrap().contains("(attempt 2,");
            match todo.as_str() {
                "T01" if !attempt2 => Err(llm_context::error::LLMComputeError::Provider {
                    failure: llm_context::error::ProviderFailure::Permanent,
                    message: "the input of this task is missing".into(),
                }),
                "T03" if has_tool_result(req, "w1").is_none() => {
                    Ok(tool_call("w1", "shell", json!({ "command": "sleep 20" })))
                }
                _ => Ok(tool_call("w2", "report", json!({ "report": format!("{todo} done"), "is_end": true }))),
            }
        }
        Role::Explore => panic!("no explorations"),
        Role::Plan => Ok({
            let list = list_of(&path_c);
            let t01_failed = user_texts(req)
                .iter()
                .any(|u| u.contains("todo T01 (attempt 1) finished (failed)"));
            if has_tool_result(req, "p1").is_none() {
                shell(
                    "p1",
                    "Plan a, b after it, c on the side.",
                    "agent-session todo add \"a\" --content \"Do a.\" && agent-session todo add \"b\" --content \"Do b.\" && agent-session todo add \"c\" --after none --content \"Do c, slowly.\"",
                )
            } else if t01_failed && has_tool_result(req, "p2").is_none() {
                assert_eq!(list.get("T01").unwrap().status, TodoStatus::Failed);
                assert!(!user_texts(req).iter().any(|u| u.contains("<todo id=")), "{:?}", user_texts(req));
                b2.store(true, Ordering::SeqCst);
                assert!(
                    user_texts(req).iter().any(|u| u.contains("todo T01 (attempt 1) finished (failed)") && u.contains("Todos after it: T02 blocked")),
                    "{:?}",
                    user_texts(req)
                );
                shell("p2", "T01 failed: try it again.", "agent-session todo retry T01 --reason \"the input is there now\"")
            } else if right_after(req, "p2") {
                let r = plan_result(req, "p2").unwrap();
                assert_eq!((r["scheduled"][0]["todo_id"].as_str(), r["attempt"].as_u64()), (Some("T01"), Some(2)), "{r}");
                assert_eq!(list.get("T02").unwrap().status, TodoStatus::Pending, "unblocked, waits for T01");
                text("T01 runs again.")
            } else if list.get("T02").is_some_and(|t| t.status == TodoStatus::Completed) && has_tool_result(req, "p3").is_none() {
                assert_eq!(list.get("T03").unwrap().status, TodoStatus::Running);
                tool_call("p3", "report", json!({ "intent": "a and b are done; c is not needed.", "report": "a and b done; c stopped", "is_end": true }))
            } else {
                text("Work runs.")
            }
        }),
    });
    let out = drive_to_end(&env, llm, &sd).await;
    let st = sd.state().unwrap();
    assert!(finished_ok(&out.result), "{:?} {:?}", out.result, st.last_error);
    assert!(saw_block.load(Ordering::SeqCst));
    let list = list_of(&path);
    let t01 = list.get("T01").unwrap();
    assert_eq!(
        t01.attempts.iter().map(|a| a.status).collect::<Vec<_>>(),
        [TodoStatus::Failed, TodoStatus::Completed]
    );
    assert_eq!(t01.attempts[1].reason.as_deref(), Some("the input is there now"));
    assert_eq!(list.get("T02").unwrap().status, TodoStatus::Completed);
    let t03 = list.get("T03").unwrap();
    assert_eq!(t03.status, TodoStatus::Stopped);
    assert!(t03.latest().unwrap().error.as_deref().unwrap_or_default().contains("plan session ended"));
    assert_eq!(todo_summary(&sd).as_deref(), Some("3 todos: 2 completed, 1 stopped"));
    let child = children_of(&list, "T03").pop().unwrap();
    let agent = env.agent();
    let mut stopped = false;
    for _ in 0..200 {
        let e = agent.sessions().lookup(&child).await.unwrap().unwrap();
        if e.status.run_state == RunState::Finished {
            assert_eq!(e.status.outcome, Some(Outcome::Stopped));
            stopped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(stopped, "T03's work session is stopped with the plan");
}

/// 4: with room for one work session, a ready Todo is held back without a
/// record and starts as soon as the running one ends, before the plan
/// infers again.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn the_limit_holds_ready_todos_back_until_one_ends() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    std::env::set_var(libopendan::todos::ENV_MAX_CHILDREN, "1");
    let sd = env.create_work(plan_spec(&env, "two independent things")).await;
    let path = sd.path().to_path_buf();
    let seen = Arc::new(Mutex::new(None));
    let s2 = seen.clone();
    let path_c = path.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Work => {
            let todo = todo_of(req);
            if has_tool_result(req, "w1").is_none() {
                return tool_call("w1", "shell", json!({ "command": "sleep 0.5" }));
            }
            tool_call("w2", "report", json!({ "report": format!("{todo} done"), "is_end": true }))
        }
        Role::Explore => panic!("no explorations"),
        Role::Plan => {
            let list = list_of(&path_c);
            if has_tool_result(req, "p1").is_none() {
                return shell(
                    "p1",
                    "Two independent Todos.",
                    "agent-session todo add \"a\" --content \"Do a.\" && agent-session todo add \"b\" --after none --content \"Do b.\"",
                );
            }
            if right_after(req, "p1") {
                let outs = jsons_in(&has_tool_result(req, "p1").unwrap());
                assert_eq!(outs[0]["scheduled"][0]["result"], "dispatched", "{}", outs[0]);
                assert_eq!(outs[1]["scheduled"][0]["result"], "held", "{}", outs[1]);
                assert_eq!(outs[1]["status"], "pending");
                return text("a runs, b waits for room.");
            }
            if list.get("T01").unwrap().status == TodoStatus::Completed && s2.lock().unwrap().is_none() {
                *s2.lock().unwrap() = Some(list.get("T02").unwrap().latest().is_some());
            }
            if list.todos.iter().all(|t| t.status == TodoStatus::Completed) {
                return tool_call("p2", "report", json!({ "intent": "Both done.", "report": "a and b done", "is_end": true }));
            }
            text("Work runs.")
        }
    });
    let out = drive_to_end(&env, llm, &sd).await;
    std::env::remove_var(libopendan::todos::ENV_MAX_CHILDREN);
    assert!(finished_ok(&out.result), "{:?}", out.result);
    assert_eq!(*seen.lock().unwrap(), Some(true), "T02 started before the plan saw T01's end");
    let list = list_of(&path);
    let (a, b) = (list.get("T01").unwrap(), list.get("T02").unwrap());
    assert!(b.attempts[0].started_at_ms >= a.attempts[0].ended_at_ms.unwrap(), "one at a time");
    assert_eq!(b.attempts.len(), 1);
    let raw = std::fs::read_to_string(path.join(".opendan_agent_session/todos.json")).unwrap();
    assert!(!raw.contains("refusal") && !raw.contains("block_reason"), "held back without a record: {raw}");
}

/// 5 / 7: the user's message reaches the plan through its queue; the plan
/// updates the running Todo (a placement change is refused while it runs);
/// the work session cannot finish before it took the update in, gets it as
/// its next input with the whole new task and reports against it.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn an_update_reaches_the_running_work_session() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "write a greeting")).await;
    let path = sd.path().to_path_buf();
    let (queue_dir, plan_queue) = (env.queue_dir.clone(), queue_of(&sd));
    let posted = Arc::new(AtomicBool::new(false));
    let saw_update = Arc::new(Mutex::new(String::new()));
    let (p2, u2) = (posted.clone(), saw_update.clone());
    let plan_path = path.clone();
    let path_c = path.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Work => {
            let updated = !list_of(&plan_path).get("T01").unwrap().updates.is_empty();
            if !p2.swap(true, Ordering::SeqCst) {
                post_blocking(&queue_dir, &plan_queue, msg("Please write it in Rust instead."));
            }
            let input = last_input(req);
            if input.contains("was updated") {
                *u2.lock().unwrap() = input;
                return tool_call("w-new", "report", json!({ "report": "UPDATED: written in Rust", "is_end": true }));
            }
            let done = req.messages.iter().filter(|m| m.role == AiRole::Tool).count();
            if !updated {
                return tool_call(&format!("s{done}"), "shell", json!({ "command": "sleep 0.3" }));
            }
            match has_tool_result(req, "w-end") {
                None => tool_call("w-end", "report", json!({ "report": "written in Python", "is_end": true })),
                Some(r) => {
                    assert!(r.contains("your Todo was updated"), "the end waits for the update: {r}");
                    text("The task changed; adjusting.")
                }
            }
        }
        Role::Explore => panic!("no explorations"),
        Role::Plan => {
            let list = list_of(&path_c);
            if has_tool_result(req, "p1").is_none() {
                return shell("p1", "One Todo.", "agent-session todo add \"greeting\" --content \"Write a greeting in Python.\" --accept \"prints hello\"");
            }
            if right_after(req, "p1") {
                return text("T01 runs.");
            }
            if has_tool_result(req, "p2").is_none() && user_texts(req).iter().any(|u| u.contains("in Rust instead")) {
                let input = last_input(req);
                assert!(input.contains("The user added to or changed the work"), "{input}");
                return calls(&[
                    ("p2", "shell", json!({ "intent": "Tell T01 the user wants Rust.", "command": "agent-session todo update T01 --content \"Write a greeting in Rust.\" --reason \"the user wants Rust\"" })),
                    ("p2b", "shell", json!({ "intent": "Its order cannot change while it runs.", "command": "agent-session todo update T01 --after none --reason \"no prerequisite\"" })),
                ]);
            }
            if right_after(req, "p2") {
                let r = plan_result(req, "p2").unwrap();
                let child = children_of(&list, "T01").pop().unwrap();
                assert_eq!((r["told"].as_str(), r["changed"].clone()), (Some(child.as_str()), json!(["content"])), "{r}");
                assert!(has_tool_result(req, "p2b").unwrap().contains("is running"), "{:?}", has_tool_result(req, "p2b"));
                return text("T01 was told.");
            }
            if list.get("T01").unwrap().status == TodoStatus::Completed {
                return tool_call("p3", "report", json!({ "intent": "Done in Rust.", "report": "greeting written in Rust", "is_end": true }));
            }
            text("T01 runs.")
        }
    });
    let out = drive_to_end(&env, llm, &sd).await;
    let st = sd.state().unwrap();
    assert!(finished_ok(&out.result), "{:?} {:?}", out.result, st.last_error);
    let update = saw_update.lock().unwrap().clone();
    assert!(update.contains("Todo T01 was updated (content): the user wants Rust"), "{update}");
    assert!(update.contains("Your task now:") && update.contains("Write a greeting in Rust."), "{update}");
    assert!(update.contains("The task you are working on was updated"), "the work_simple branch: {update}");
    let list = list_of(&path);
    let t = list.get("T01").unwrap();
    assert_eq!(t.updates.len(), 1);
    assert_eq!((t.updates[0].attempt, t.updates[0].fields.clone()), (Some(1), vec!["content".to_string()]));
    assert_eq!(t.latest().unwrap().summary.as_deref(), Some("UPDATED: written in Rust"));
    // Taken in at the work session's next batch (its Turn was open: the
    // refused end, the reply and the correction came first).
    let child = children_of(&list, "T01").pop().unwrap();
    let e = env.agent().sessions().lookup(&child).await.unwrap().unwrap();
    let log = read_worklog(&SessionDir::open(&e.location).unwrap());
    assert!(
        log.iter().any(|x| matches!(&x.body, WorklogBody::InputBatch { inputs, .. } if inputs.iter().any(|i| i.key.contains("#updated@")))),
        "{:?}",
        kinds(&log)
    );
}

/// 6: `todo pause` stops the work session's inference after its call in
/// flight (its run stays at the checkpoint, the Turn open, the registry
/// says paused); `todo resume` goes on from there in the same run; `todo
/// stop` ends an attempt as stopped and `todo retry` starts a new one.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn pause_resume_and_stop_reach_the_work_session() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "a, paused once; b, stopped once")).await;
    let path = sd.path().to_path_buf();
    let (queue_dir, plan_queue) = (env.queue_dir.clone(), queue_of(&sd));
    let started = Arc::new(AtomicBool::new(false));
    let s2 = started.clone();
    let path_c = path.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Work => {
            let todo = todo_of(req);
            if todo == "T02" {
                let attempt2 = user_texts(req).into_iter().next().unwrap().contains("(attempt 2,");
                if !attempt2 && has_tool_result(req, "b1").is_none() {
                    return tool_call("b1", "shell", json!({ "command": "sleep 20" }));
                }
                return tool_call("b2", "report", json!({ "report": "T02 done", "is_end": true }));
            }
            match has_tool_result(req, "w1") {
                None => {
                    if !s2.swap(true, Ordering::SeqCst) {
                        post_blocking(&queue_dir, &plan_queue, msg("Hold T01 for a moment."));
                    }
                    tool_call("w1", "shell", json!({ "command": "sleep 1; echo slept" }))
                }
                Some(r) => {
                    assert!(r.contains("slept"), "{r}");
                    tool_call("w2", "report", json!({ "report": "T01 done after its pause", "is_end": true }))
                }
            }
        }
        Role::Explore => panic!("no explorations"),
        Role::Plan => {
            let list = list_of(&path_c);
            let said = |t: &str| user_texts(req).iter().any(|u| u.contains(t));
            if has_tool_result(req, "p1").is_none() {
                return shell("p1", "One Todo.", "agent-session todo add \"a\" --content \"Do a.\"");
            }
            if has_tool_result(req, "p2").is_none() && said("Hold T01") {
                return calls(&[
                    ("p2", "shell", json!({ "intent": "Pause T01 as asked.", "command": "agent-session todo pause T01 --reason \"the user asked to hold it\"" })),
                    ("p2b", "shell", json!({ "intent": "Pausing a Todo that is not running is refused.", "command": "agent-session todo pause T09 --reason \"no such todo\"" })),
                ]);
            }
            if right_after(req, "p2") {
                let r = plan_result(req, "p2").unwrap();
                assert_eq!((r["todo_id"].as_str(), r["sent"].as_str()), (Some("T01"), Some("pause")), "{r}");
                assert!(has_tool_result(req, "p2b").unwrap().contains("not found"), "{:?}", has_tool_result(req, "p2b"));
                return text("T01 is paused.");
            }
            if has_tool_result(req, "p3").is_none() && said("Go on with T01") {
                return shell("p3", "Resume T01.", "agent-session todo resume T01 --reason \"the user said go on\"");
            }
            if list.get("T01").unwrap().status == TodoStatus::Completed && has_tool_result(req, "p4").is_none() {
                return shell("p4", "Add b.", "agent-session todo add \"b\" --content \"Do b.\"");
            }
            if right_after(req, "p4") {
                return shell("p5", "Stop b: not needed this way.", "agent-session todo stop T02 --reason \"wrong approach\"");
            }
            let t02 = list.get("T02").map(|t| t.status);
            if t02 == Some(TodoStatus::Stopped) && has_tool_result(req, "p6").is_none() {
                return shell("p6", "Run b again.", "agent-session todo retry T02 --reason \"the right approach now\"");
            }
            if t02 == Some(TodoStatus::Completed) {
                return tool_call("p7", "report", json!({ "intent": "a and b done.", "report": "a and b done", "is_end": true }));
            }
            text("Work runs.")
        }
    });
    let agent = env.agent();
    let monitor = {
        let path = path.clone();
        let agent = agent.clone();
        let llm = llm.clone();
        let (queue_dir, plan_queue) = (env.queue_dir.clone(), queue_of(&sd));
        tokio::spawn(async move {
            // T01's work session gets paused …
            let mut child = String::new();
            wait_until("T01 to be paused", || {
                let list = list_of(&path);
                match list.get("T01").and_then(|t| t.latest()) {
                    Some(a) if a.paused() => {
                        child = a.child_sid.clone().unwrap_or_default();
                        true
                    }
                    _ => false,
                }
            })
            .await;
            let mut paused = false;
            for _ in 0..200 {
                if agent.sessions().lookup(&child).await.unwrap().unwrap().status.paused.is_some() {
                    paused = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(paused, "the work session reports it is paused");
            // … and infers no more while paused, its call in flight done.
            let work = |llm: &ScriptedLlm| llm.requests.lock().unwrap().iter().filter(|r| role(r) == Role::Work && todo_of(r) == "T01").count();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let before = work(&llm);
            tokio::time::sleep(Duration::from_millis(800)).await;
            assert_eq!(work(&llm), before, "no inference while paused");
            post_blocking(&queue_dir, &plan_queue, msg("Go on with T01 now."));
            child
        })
    };
    let out = drive_to_end(&env, llm.clone(), &sd).await;
    let child = monitor.await.unwrap();
    let st = sd.state().unwrap();
    assert!(finished_ok(&out.result), "{:?} {:?}", out.result, st.last_error);
    let list = list_of(&path);
    let t01 = list.get("T01").unwrap();
    let a = t01.latest().unwrap();
    assert!(a.paused_at_ms.is_some() && a.resumed_at_ms.is_some() && !a.paused());
    assert_eq!(t01.status, TodoStatus::Completed);
    // Resumed from its checkpoint: the same run, the same Turn.
    let e = agent.sessions().lookup(&child).await.unwrap().unwrap();
    let cd = SessionDir::open(&e.location).unwrap();
    let cst = cd.state().unwrap();
    assert_eq!((cst.turn_seq, cst.paused.is_none()), (1, true), "{cst:?}");
    let k = kinds(&read_worklog(&cd));
    assert_eq!(k.iter().filter(|x| **x == "turn_started").count(), 1, "{k:?}");
    let t01_reqs: Vec<LlmInferenceRequest> = llm.requests.lock().unwrap().iter().filter(|r| role(r) == Role::Work && todo_of(r) == "T01").cloned().collect();
    assert_eq!(t01_reqs.len(), 2, "one inference before the pause, one after the resume");
    assert!(has_tool_result(&t01_reqs[1], "w1").is_some());
    let t02 = list.get("T02").unwrap();
    assert_eq!(
        t02.attempts.iter().map(|a| a.status).collect::<Vec<_>>(),
        [TodoStatus::Stopped, TodoStatus::Completed]
    );
}

/// 6 (between Turns): a paused plan opens no Turn — a user message waits —
/// and its pause reaches its running work session (which stops at its next
/// inference boundary); resuming the plan resumes that work session too,
/// and what waited opens the next Turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_paused_plan_holds_its_inputs_and_pauses_its_work() {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    let sd = env.create_work(plan_spec(&env, "one slow thing")).await;
    let path = sd.path().to_path_buf();
    let path_c = path.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Work => match has_tool_result(req, "w1") {
            None => tool_call("w1", "shell", json!({ "command": "sleep 1.5; echo slept" })),
            Some(_) => tool_call("w2", "report", json!({ "report": "T01 done", "is_end": true })),
        },
        Role::Explore => panic!("no explorations"),
        Role::Plan => {
            if has_tool_result(req, "p1").is_none() {
                return shell("p1", "One Todo.", "agent-session todo add \"slow\" --content \"Do it slowly.\"");
            }
            let list = list_of(&path_c);
            let heard = user_texts(req).iter().any(|u| u.contains("Note: no rush."));
            if heard && list.get("T01").unwrap().status == TodoStatus::Completed {
                return tool_call("p2", "report", json!({ "intent": "Done.", "report": "done", "is_end": true }));
            }
            text("Work runs.")
        }
    });
    let agent = env.agent();
    let (queue_dir, plan_queue, plan_sid) = (env.queue_dir.clone(), queue_of(&sd), sd.sid().to_string());
    let monitor = {
        let (agent, llm, path) = (agent.clone(), llm.clone(), path.clone());
        tokio::spawn(async move {
            wait_until("T01 to run", || {
                list_of(&path)
                    .get("T01")
                    .and_then(|t| t.latest())
                    .is_some_and(|a| a.dispatch == Some(libopendan::todos::DispatchState::Dispatched))
            })
            .await;
            let child = children_of(&list_of(&path), "T01").pop().unwrap();
            let ctl = |key: &str, command: ControlCommand| PostedInput::control(APP, key, command);
            post_blocking(&queue_dir, &plan_queue, ctl("pause-1", ControlCommand::Pause { reason: Some("hold on".into()) }));
            for _ in 0..200 {
                let plan = agent.sessions().lookup(&plan_sid).await.unwrap().unwrap();
                let work = agent.sessions().lookup(&child).await.unwrap().unwrap();
                if plan.status.paused.is_some() && work.status.paused.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let work = agent.sessions().lookup(&child).await.unwrap().unwrap();
            assert_eq!(work.status.paused, Some(PauseSource::Parent), "the plan's pause reaches its work session");
            post_blocking(&queue_dir, &plan_queue, msg("Note: no rush."));
            let plans = |llm: &ScriptedLlm| llm.requests.lock().unwrap().iter().filter(|r| role(r) == Role::Plan).count();
            let before = plans(&llm);
            tokio::time::sleep(Duration::from_millis(2500)).await;
            assert_eq!(plans(&llm), before, "a paused plan opens no Turn");
            assert_eq!(list_of(&path).get("T01").unwrap().status, TodoStatus::Running, "its work waits too");
            post_blocking(&queue_dir, &plan_queue, ctl("resume-1", ControlCommand::Resume { reason: Some("go on".into()) }));
        })
    };
    let out = drive_to_end(&env, llm.clone(), &sd).await;
    monitor.await.unwrap();
    let st = sd.state().unwrap();
    assert!(finished_ok(&out.result), "{:?} {:?}", out.result, st.last_error);
    assert!(st.paused.is_none());
    let log = read_worklog(&sd);
    let controls: Vec<String> = log
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::ControlApplied { command, .. } => Some(command.clone()),
            _ => None,
        })
        .collect();
    assert!(controls.contains(&"pause".to_string()) && controls.contains(&"resume".to_string()), "{controls:?}");
    let child = children_of(&list_of(&path), "T01").pop().unwrap();
    let e = agent.sessions().lookup(&child).await.unwrap().unwrap();
    assert_eq!(e.status.outcome, Some(Outcome::Succeeded));
    let clog = read_worklog(&SessionDir::open(&e.location).unwrap());
    assert!(
        clog.iter().any(|x| matches!(&x.body, WorklogBody::ControlApplied { command, detail, .. } if command == "resume" && detail["applied"] == true)),
        "the plan's resume reached its work session"
    );
}
