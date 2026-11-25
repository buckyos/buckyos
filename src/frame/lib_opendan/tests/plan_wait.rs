//! A plan session that waits for long (notepads/jarvis-plan-work-simple-todo.md):
//! its timer backs off and also runs while nothing of the plan does, a reply
//! then is a wait after one correction, a tick tells how long each Todo runs
//! and its latest progress, and a plan past its lifetime wraps up.
//!
//! The Jarvis package is the AgentRoot; the model is scripted. Time is
//! simulated (`common::pass_time`) between drives of the plan alone.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::plan::*;
use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, RunnerDeps, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::todos::TodoStatus;
use libopendan::SessionDir;
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

fn setup() -> Env {
    let env = Env::new();
    install_jarvis(&env);
    tool_env(&env);
    env
}

fn deps(env: &Env, llm: Arc<ScriptedLlm>) -> RunnerDeps {
    env.deps(llm).with_session_cli(std::path::PathBuf::from(env!("CARGO_BIN_EXE_xagent")))
}

async fn look(env: &Env, sd: &SessionDir, llm: &Arc<ScriptedLlm>) -> DriveResult {
    drive(sd, &deps(env, llm.clone()), StopWhen::Idle).await
}

fn report(id: &str, text: &str, outcome: &str) -> buckyos_api::AiResponse {
    tool_call(id, "report", json!({ "intent": "End.", "report": text, "outcome": outcome, "is_end": true }))
}

fn names(req: &LlmInferenceRequest) -> Vec<String> {
    req.tool_specs.iter().map(|s| s.name.clone()).collect()
}

/// Let `hours` pass in steps of ten minutes, the plan looked at after each:
/// the minutes at which it inferred.
async fn wait_hours(env: &Env, sd: &SessionDir, llm: &Arc<ScriptedLlm>, hours: u64) -> Vec<u64> {
    let mut at = Vec::new();
    for step in 1..=hours * 6 {
        pass_time(sd, 600);
        let before = llm.count();
        look(env, sd, llm).await;
        for _ in before..llm.count() {
            at.push(step * 10);
        }
    }
    at
}

/// D3: the file this plan waits for comes tomorrow. Nothing of it runs: one
/// correction, then its reply is a wait. In 24 hours its timer makes it
/// infer 8 times at most, each tick after twice the time, up to 4 hours, with
/// no further correction; the user's word resets the interval, and the plan
/// ends when the file is there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plan_waits_for_tomorrows_file_with_few_ticks() {
    let env = setup();
    let sd = env.create_work(plan_spec(&env, "publish tomorrow's file")).await;
    let arrived = Arc::new(AtomicBool::new(false));
    let seen = arrived.clone();
    let inputs: Arc<Mutex<Vec<String>>> = Arc::default();
    let kept = inputs.clone();
    let llm = ScriptedLlm::new(move |req, n| {
        let input = last_input(req);
        kept.lock().unwrap().push(input.clone());
        if n == 1 {
            assert!(input.contains("<runner_notice>Nothing you started is running"), "{input}");
            assert!(input.contains("reply with what you are waiting for"), "{input}");
        }
        if seen.load(Ordering::SeqCst) {
            return report("end", "published", "succeeded");
        }
        text("Waiting for tomorrow's file.")
    });
    let r = look(&env, &sd, &llm).await;
    assert!(matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }), "{r:?}");
    assert_eq!(llm.count(), 2, "its first reply is corrected once, the second is a wait");
    let st = sd.state().unwrap();
    assert!(st.task_watch.idle && st.open_turn.is_none() && st.last_error.is_none(), "{st:?}");
    assert_eq!(st.turns_completed, 1);
    let status = env.agent().sessions().lookup(sd.sid()).await.unwrap().unwrap().status;
    assert!(!status.needs_input, "its timer moves it, nobody has to");
    assert!(status.one_line_status.contains("Waiting for tomorrow's file."), "{}", status.one_line_status);
    // Looking again changes nothing: it is not due.
    look(&env, &sd, &llm).await;
    assert_eq!(llm.count(), 2);

    let at = wait_hours(&env, &sd, &llm, 24).await;
    assert_eq!(at, [30, 90, 210, 450, 690, 930, 1170, 1410], "minutes");
    let ticks: Vec<String> = inputs.lock().unwrap().iter().filter(|i| i.contains("<tick timer=")).cloned().collect();
    assert_eq!(ticks.len(), 8);
    assert!(ticks[0].contains("Nothing you started is running; no other input came for 30m"), "{}", ticks[0]);
    assert!(ticks[0].contains("The next tick comes in 1h00m"), "{}", ticks[0]);
    assert!(ticks[7].contains("The next tick comes in 4h00m"), "{}", ticks[7]);
    assert!(inputs.lock().unwrap().iter().filter(|i| i.contains("<runner_notice>")).count() == 1);

    // Another input resets the interval (and the correction).
    env.agent().sessions().post_input(sd.sid(), &msg("Any news?")).await.unwrap();
    let before = llm.count();
    look(&env, &sd, &llm).await;
    assert_eq!(llm.count(), before + 2, "answered, corrected once, waiting again");
    assert_eq!(sd.state().unwrap().task_watch.ticks, 0);
    let at = wait_hours(&env, &sd, &llm, 1).await;
    assert_eq!(at, [30]);

    // The file is there: the next tick ends the plan.
    arrived.store(true, Ordering::SeqCst);
    let at = wait_hours(&env, &sd, &llm, 1).await;
    assert_eq!(at, [30]);
    let st = sd.state().unwrap();
    assert_eq!((st.run_state, st.outcome), (RunState::Finished, Some(Outcome::Succeeded)), "{:?}", st.last_error);
    assert!(list_of(sd.path()).todos.is_empty(), "nothing was dispatched meanwhile");
}

/// D2 / D4: a Todo runs for three hours. The plan gets a chance after 30
/// minutes and after 1.5 hours; both times it is told how long the Todo has
/// run and the latest progress of its work session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tick_tells_how_long_a_todo_runs_and_its_progress() {
    let env = setup();
    let sd = env.create_work(plan_spec(&env, "a long job")).await;
    let ticks: Arc<Mutex<Vec<String>>> = Arc::default();
    let kept = ticks.clone();
    let llm = ScriptedLlm::new(move |req, n| match role(req) {
        Role::Plan if n == 0 => shell("p1", "One long Todo.", "agent-session todo add \"long job\" --content \"Do it.\" --accept \"done\""),
        Role::Plan => {
            let input = last_input(req);
            if input.contains("<tick timer=") {
                kept.lock().unwrap().push(input);
            }
            text("It runs.")
        }
        _ => {
            if has_tool_result(req, "w1").is_none() {
                return tool_call("w1", "report", json!({ "report": "copied 1 of 5 disks", "is_end": false }));
            }
            if has_tool_result(req, "w2").is_none() {
                return tool_call("w2", "shell", json!({ "command": "for i in $(seq 1 600); do test -f done && exit 0; sleep 0.1; done" }));
            }
            text("Step 2 of 5: the disks are being copied.")
        }
    });
    look(&env, &sd, &llm).await;
    let list = list_of(sd.path());
    let t = list.get("T01").unwrap();
    assert_eq!(t.status, TodoStatus::Running);
    let child_sid = t.attempts[0].child_sid.clone().unwrap();
    let entry = env.agent().sessions().lookup(&child_sid).await.unwrap().unwrap();
    let child = SessionDir::open(&entry.location).unwrap();
    let mut cdeps = deps(&env, llm.clone());
    cdeps.options.poll_interval = std::time::Duration::from_millis(50);
    let mut cfg: Value = serde_json::from_slice(&std::fs::read(child.file("session_config.json")).unwrap()).unwrap();
    cfg["prompt"]["llm_context"]["runtime"] = json!({ "task_wait_ms": 300 });
    std::fs::write(child.file("session_config.json"), serde_json::to_vec_pretty(&cfg).unwrap()).unwrap();
    let r = drive(&child, &cdeps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { run_state: RunState::Waiting, .. }), "{r:?}");

    let mut at = Vec::new();
    for step in 1..=18 {
        pass_time(&child, 600);
        pass_time(&sd, 600);
        let before = llm.count();
        look(&env, &sd, &llm).await;
        for _ in before..llm.count() {
            at.push(step * 10);
        }
    }
    assert_eq!(at, [30, 90], "minutes");
    let ticks = ticks.lock().unwrap().clone();
    for (i, ran) in [(0, "30m"), (1, "1h30m")] {
        assert!(ticks[i].contains(&format!("T01 long job (attempt 1): running for {ran}")), "{}", ticks[i]);
        assert!(ticks[i].contains(&format!("latest progress, {ran}")), "{}", ticks[i]);
        assert!(ticks[i].contains("Step 2 of 5: the disks are being copied."), "{}", ticks[i]);
    }
    std::fs::write(std::path::Path::new(&entry.location).join("done"), "").unwrap();
}

/// D3: a plan waits; its host restarts. It was going to infer by itself: it
/// stays interrupted until its owner resumes it, and waits on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_plan_is_interrupted_by_a_restart_and_waits_on_after_a_resume() {
    let env = setup();
    let sd = env.create_work(plan_spec(&env, "wait for it")).await;
    let llm = ScriptedLlm::new(|_, _| text("Waiting for it."));
    look(&env, &sd, &llm).await;
    assert_eq!(llm.count(), 2);

    let mut restarted = deps(&env, llm.clone());
    restarted.recovering = Some(libopendan::runner::Recovering::new(libopendan::now_ms()));
    drive(&sd, &restarted, StopWhen::Idle).await;
    assert_eq!(sd.state().unwrap().paused.as_ref().map(|p| p.source), Some(PauseSource::Interrupted));
    pass_time(&sd, 7200);
    look(&env, &sd, &llm).await;
    assert_eq!(llm.count(), 2, "interrupted: no tick");

    assert_eq!(resume_interrupted(env.agent().as_ref()).await, 1);
    look(&env, &sd, &llm).await;
    assert_eq!(llm.count(), 3, "the tick that came due meanwhile");
    assert!(sd.state().unwrap().task_watch.idle, "it waits on");
    assert_eq!(wait_hours(&env, &sd, &llm, 1).await, [60]);
}

/// The plan may still be stopped, and changed, while it waits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_plan_takes_changes_and_a_stop() {
    let env = setup();
    let sd = env.create_work(plan_spec(&env, "wait for it")).await;
    let llm = ScriptedLlm::new(|req, _| {
        if last_input(req).contains("do this first") && has_tool_result(req, "p1").is_none() {
            return shell("p1", "The user wants something now.", "agent-session todo add \"this first\" --content \"Do it.\" --accept \"done\"");
        }
        text("Waiting.")
    });
    look(&env, &sd, &llm).await;
    env.agent().sessions().post_input(sd.sid(), &msg("do this first")).await.unwrap();
    look(&env, &sd, &llm).await;
    assert_eq!(list_of(sd.path()).get("T01").unwrap().status, TodoStatus::Running);
    let st = sd.state().unwrap();
    assert!(!st.task_watch.idle && !st.wait_children.is_empty(), "it waits for its Todo now: {st:?}");
    let stop = PostedInput::control(APP, "stop-1", ControlCommand::stop(Some("not needed".into())));
    env.agent().sessions().post_input(sd.sid(), &stop).await.unwrap();
    look(&env, &sd, &llm).await;
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
    assert_eq!(list_of(sd.path()).get("T01").unwrap().status, TodoStatus::Stopped);
}

/// D3: 48 hours after its creation a plan gets its last Turn, whatever the
/// interval says: only `report` is left, what still runs stops with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plan_past_its_lifetime_only_reports() {
    let env = setup();
    let sd = env.create_work(plan_spec(&env, "a very long job")).await;
    let last: Arc<Mutex<Vec<(Vec<String>, String)>>> = Arc::default();
    let kept = last.clone();
    let llm = ScriptedLlm::new(move |req, n| match n {
        0 => shell("p1", "One Todo.", "agent-session todo add \"long job\" --content \"Do it.\" --accept \"done\""),
        1 => text("It runs."),
        _ => {
            let texts = user_texts(req);
            let budget = texts.iter().rev().find(|t| t.starts_with("<<budget>>")).cloned().unwrap_or_default();
            kept.lock().unwrap().push((names(req), budget));
            if texts.iter().any(|t| t.contains("reached its lifetime (48h00m since it was created): it ends now")) {
                report("end", "half of it is done", "failed")
            } else {
                text("It runs.")
            }
        }
    });
    look(&env, &sd, &llm).await;
    assert_eq!(list_of(sd.path()).get("T01").unwrap().status, TodoStatus::Running);
    // Almost 48 hours: its ordinary ticks, with all its tools.
    pass_time(&sd, 47 * 3600);
    look(&env, &sd, &llm).await;
    assert_eq!(llm.count(), 3);
    assert!(last.lock().unwrap()[0].0.len() > 1, "{:?}", last.lock().unwrap()[0]);
    pass_time(&sd, 3600);
    let r = look(&env, &sd, &llm).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 4, "one inference: the report");
    let (tools, budget) = last.lock().unwrap().last().cloned().unwrap();
    assert_eq!(tools, ["report"]);
    assert!(budget.contains("This session has reached its lifetime (48h00m since it was created)"), "{budget}");
    assert!(budget.contains("end with `report` (is_end: true)"), "{budget}");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Failed));
    assert_eq!(st.final_report.as_ref().map(|r| r.report.as_str()), Some("half of it is done"));
    let t01 = list_of(sd.path()).get("T01").unwrap().clone();
    assert_eq!(t01.status, TodoStatus::Stopped);
    let wrapped = read_worklog(&sd)
        .iter()
        .filter(|e| matches!(&e.body, WorklogBody::ControlApplied { command, detail, .. } if command == "budget_wrap_up" && detail["which"] == "lifetime"))
        .count();
    assert_eq!(wrapped, 1);
}

/// A plan past its lifetime waits for nothing: a reply is corrected once,
/// then the plan ends as failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plan_past_its_lifetime_waits_for_nothing() {
    let env = setup();
    let sd = env.create_work(plan_spec(&env, "a very long job")).await;
    let llm = ScriptedLlm::new(|req, n| match n {
        0 => shell("p1", "One Todo.", "agent-session todo add \"long job\" --content \"Do it.\" --accept \"done\""),
        3 => {
            assert!(last_input(req).contains("past its lifetime and waits for nothing any more"), "{}", last_input(req));
            assert_eq!(names(req), ["report"]);
            text("Still running.")
        }
        _ => text("It runs."),
    });
    look(&env, &sd, &llm).await;
    pass_time(&sd, 48 * 3600);
    let r = look(&env, &sd, &llm).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 4);
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Failed));
    assert_eq!(st.last_error.as_ref().unwrap()["kind"], "missing_end_report");
    assert_eq!(list_of(sd.path()).get("T01").unwrap().status, TodoStatus::Stopped);
}

/// A plan in the middle of a Turn when its lifetime ends: from its next
/// inference on only `report` is left.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lifetime_that_ends_inside_a_turn_leaves_only_the_report() {
    let env = setup();
    let mut spec = plan_spec(&env, "look around");
    spec.policy.max_life_secs = 2;
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, _| {
        if has_tool_result(req, "p1").is_none() && names(req).len() > 1 {
            return shell("p1", "Look for a while.", "sleep 2.5; echo looked");
        }
        assert_eq!(names(req), ["report"]);
        assert!(user_texts(req).iter().any(|t| t.starts_with("<<budget>> This session has reached its lifetime (2s")), "{:?}", user_texts(req));
        report("end", "looked", "succeeded")
    });
    let r = look(&env, &sd, &llm).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    let st = sd.state().unwrap();
    assert_eq!((st.outcome, st.turn_seq), (Some(Outcome::Succeeded), 1));
}

/// A Work that does not wait for outside things is as before: a reply with
/// nothing running is corrected once, then it fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_work_that_may_not_wait_still_fails_without_an_end() {
    let env = setup();
    let mut spec = plan_spec(&env, "wait for it");
    spec.policy.waits_idle = false;
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, _| text("Waiting."));
    let r = look(&env, &sd, &llm).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 2);
    assert_eq!(sd.state().unwrap().last_error.as_ref().unwrap()["kind"], "missing_end_report");
}
