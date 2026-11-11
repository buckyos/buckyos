//! Cards of the work a UI Turn started (notepads/
//! messagehub-worksession-ui-todo.md §3.3–§3.5): states and their line,
//! usage from `usage.jsonl`, a plan's TodoList with bound / unbound /
//! retried / cancelled Todos, the Turn filter and `complete`.

mod common;

use common::*;
use libopendan::lock::Acquire;
use libopendan::protocol::*;
use libopendan::session::Session;
use libopendan::state::AgentStateClient;
use libopendan::work_card::{work_cards, work_tree, CardShape, CardState, WorkCard};
use libopendan::SessionDir;
use serde_json::json;

const UI_TASK: &str = "t-ui-1";

fn holder() -> HolderInfo {
    HolderInfo {
        runner_id: "rn-cards".into(),
        principal: APP.into(),
        host: None,
        pid: std::process::id(),
        runtime_id: None,
    }
}

/// Change a session's committed state (and its files) as its driver would.
fn edit(sd: &SessionDir, f: impl FnOnce(&mut Session, &libopendan::lock::Lease)) {
    let Acquire::Acquired(lease) = sd.acquire(holder()).unwrap() else {
        panic!("session busy")
    };
    let mut s = sd.load(&lease).unwrap();
    f(&mut s, &lease);
    s.commit_state(&lease).unwrap();
}

fn open_turn(index: u64) -> OpenTurn {
    OpenTurn {
        index,
        run_id: "r-1".into(),
        input_seq: 1,
        hook: None,
        inputs: Vec::new(),
        has_msg: true,
        msg_keys: Vec::new(),
        at_ms: 1,
    }
}

fn binding(turn: u64, task_id: &str, closed: Option<TurnStatus>, summary: Option<&str>) -> TurnTask {
    TurnTask {
        turn,
        task_id: task_id.into(),
        has_msg: false,
        opened_at_ms: 1,
        placeholder: None,
        closed,
        summary: summary.map(str::to_string),
        error: None,
        reported: closed.is_some(),
    }
}

async fn ui_session(env: &Env) -> SessionDir {
    let sd = env.create_work(work_spec("the conversation")).await;
    edit(&sd, |s, _| {
        s.state.run_state = RunState::Running;
        s.state.open_turn = Some(open_turn(1));
        s.state.turn_seq = 1;
        s.state.turn_tasks = vec![binding(1, UI_TASK, None, None)];
    });
    sd
}

async fn child(env: &Env, parent: &SessionDir, task: &str, title: Option<&str>, class: &str, behavior: Option<&str>) -> SessionDir {
    let mut spec = work_spec("\n  Build the crate and run its tests\nin the workspace");
    spec.title = title.map(str::to_string);
    spec.class = class.into();
    spec.prompt.behavior = behavior.map(str::to_string);
    spec.origin = Some(Origin {
        parent_session: Some(parent.sid().to_string()),
        parent_task: Some(task.into()),
        ..Default::default()
    });
    env.create_work(spec).await
}

fn usage(sd: &SessionDir, lines: &[(&str, u64, u64)]) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(sd.file(USAGE_FILE))
        .unwrap();
    for (i, (model, input, output)) in lines.iter().enumerate() {
        let r = UsageRecord {
            at_ms: i as u64,
            model: model.to_string(),
            run_id: None,
            input_tokens: *input,
            output_tokens: *output,
            total_tokens: input + output,
        };
        writeln!(f, "{}", serde_json::to_string(&r).unwrap()).unwrap();
    }
}

async fn cards(env: &Env, ui: &SessionDir) -> libopendan::work_card::WorkCards {
    work_cards(env.agent().as_ref(), ui.sid(), UI_TASK).await.unwrap()
}

fn card<'a>(all: &'a [WorkCard], sid: &str) -> &'a WorkCard {
    all.iter().find(|c| c.session_id == sid).expect("card")
}

#[tokio::test]
async fn every_state_has_its_line() {
    let env = Env::new();
    let ui = ui_session(&env).await;
    let queued = child(&env, &ui, UI_TASK, Some("Queued"), "work", None).await;
    let live = child(&env, &ui, UI_TASK, Some("Live"), "work", None).await;
    edit(&live, |s, _| {
        s.state.run_state = RunState::Running;
        s.state.turn_seq = 2;
        s.state.open_turn = Some(open_turn(2));
        s.state.one_line_status = "build the crate".into();
    });
    let between = child(&env, &ui, UI_TASK, Some("Between"), "work", None).await;
    edit(&between, |s, _| {
        s.state.run_state = RunState::Waiting;
        s.state.turn_seq = 2;
        s.state.watched_tasks = vec!["local:shell:c1".into()];
        s.state.one_line_status = "waiting for input: two builds are still running".into();
        s.state.turn_tasks = vec![
            binding(1, "t-w-1", Some(TurnStatus::Completed), Some("started two builds")),
            binding(2, "t-w-2", Some(TurnStatus::Completed), Some("two builds are still running")),
        ];
    });
    let asking = child(&env, &ui, UI_TASK, Some("Asking"), "work", None).await;
    edit(&asking, |s, _| {
        s.state.run_state = RunState::Waiting;
        s.state.turn_seq = 1;
    });
    let paused = child(&env, &ui, UI_TASK, Some("Paused"), "work", None).await;
    edit(&paused, |s, _| {
        s.state.run_state = RunState::Waiting;
        s.state.turn_seq = 1;
        s.state.paused = Some(Paused { source: PauseSource::User, at_ms: 1, reason: Some("the owner asked".into()), by: "x".into(), in_run: false });
    });
    let done = child(&env, &ui, UI_TASK, Some("Done"), "work", None).await;
    edit(&done, |s, lease| {
        s.write_report(lease, "\n# Built\nall 12 tests pass").unwrap();
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Succeeded);
        s.state.finished_at_ms = Some(s.state.updated_at_ms + 5000);
    });
    let answered = child(&env, &ui, UI_TASK, Some("Answered"), "work", None).await;
    edit(&answered, |s, lease| {
        s.write_report(lease, "# Report — Build the crate\n\n- session: `w`\n- outcome: succeeded\n- turns: 2\n\nA: build-A-done\nB: build-B-done\n").unwrap();
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Succeeded);
        s.state.finished_at_ms = Some(9);
    });
    let reported = child(&env, &ui, UI_TASK, Some("Reported"), "work", None).await;
    edit(&reported, |s, lease| {
        s.write_report(lease, "# Report — Build the crate\n\n- session: `w`\n- outcome: succeeded\n- turns: 2\n\nignored").unwrap();
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Succeeded);
        s.state.finished_at_ms = Some(9);
        s.state.result = Some(json!({ "answer": "\n\nBoth builds passed.\nDetails follow." }));
    });
    let failed = child(&env, &ui, UI_TASK, Some("Failed"), "work", None).await;
    edit(&failed, |s, lease| {
        s.write_report(lease, "a report").unwrap();
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Failed);
        s.state.last_error = Some(json!({ "kind": "x", "message": "the build broke\nat line 3" }));
        s.state.finished_at_ms = Some(9);
    });
    let stopped = child(&env, &ui, UI_TASK, Some("Stopped"), "work", None).await;
    edit(&stopped, |s, _| {
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Stopped);
        s.state.finished_at_ms = Some(9);
    });

    let all = cards(&env, &ui).await;
    assert_eq!(all.session_id, ui.sid());
    assert_eq!(all.task_id, UI_TASK);
    assert_eq!(all.sessions.len(), 10);
    assert!(all.sessions.windows(2).all(|w| w[0].created_at_ms <= w[1].created_at_ms));
    let expect = [
        (&queued, CardState::Queued, ""),
        (&live, CardState::Running, "build the crate"),
        (&between, CardState::Running, "two builds are still running"),
        (&asking, CardState::NeedsInput, ""),
        (&paused, CardState::Paused, "the owner asked"),
        (&done, CardState::Succeeded, "Built"),
        (&answered, CardState::Succeeded, "A: build-A-done"),
        (&reported, CardState::Succeeded, "Both builds passed."),
        (&failed, CardState::Failed, "the build broke"),
        (&stopped, CardState::Stopped, ""),
    ];
    for (sd, state, line) in expect {
        let c = card(&all.sessions, sd.sid());
        assert_eq!((c.state, c.line.as_str()), (state, line), "{}", c.title);
        assert_eq!(c.shape, CardShape::Simple);
        assert!(c.todos.is_none() && c.subtree_usage.is_none());
        assert_eq!(c.finished_at_ms.is_some(), state.is_terminal(), "{}", c.title);
    }
    assert_eq!(card(&all.sessions, live.sid()).title, "Live");
    // Running cards: not complete.
    assert!(!all.complete);
}

#[tokio::test]
async fn usage_sums_the_rounds_and_names_the_last_model() {
    let env = Env::new();
    let ui = ui_session(&env).await;
    let w = child(&env, &ui, UI_TASK, None, "work", Some("explore")).await;
    usage(&w, &[("gpt-6.1-sol@cliproxy", 1000, 100), ("llm.code", 50, 5), ("gpt-6.1-sol@cliproxy", 2000, 300)]);
    // A line being appended is not counted.
    std::fs::OpenOptions::new()
        .append(true)
        .open(w.file(USAGE_FILE))
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"{\"at_ms\": 9, \"mod"))
        .unwrap();
    let all = cards(&env, &ui).await;
    let c = &all.sessions[0];
    assert_eq!(c.shape, CardShape::Explore);
    // No title: the objective's first line.
    assert_eq!(c.title, "Build the crate and run its tests");
    assert_eq!((c.usage.input_tokens, c.usage.output_tokens, c.usage.rounds), (3050, 405, 3));
    assert_eq!(c.usage.models.len(), 2);
    assert_eq!((c.usage.models[0].model.as_str(), c.usage.models[0].output_tokens), ("gpt-6.1-sol@cliproxy", 400));
    assert_eq!(c.model.as_deref(), Some("gpt-6.1-sol@cliproxy"));
    // No Round yet.
    let fresh = child(&env, &ui, UI_TASK, Some("Fresh"), "work", None).await;
    let all = cards(&env, &ui).await;
    let f = card(&all.sessions, fresh.sid());
    assert_eq!((f.usage.rounds, f.model.as_deref()), (0, None));
}

fn todo(id: &str, seq: u64, title: &str, status: &str, attempts: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut t = json!({
        "todo_id": id, "seq": seq, "rev": 1, "status": status, "mode": "serial",
        "title": title, "content": "do it", "attempts": attempts,
        "created_at_ms": 1, "updated_at_ms": 1
    });
    for (k, v) in extra.as_object().unwrap() {
        t[k] = v.clone();
    }
    t
}

fn attempt(n: u32, sid: &str, status: &str) -> serde_json::Value {
    json!({ "attempt": n, "mode": "serial", "child_sid": sid, "status": status, "started_at_ms": 1 })
}

#[tokio::test]
async fn a_plan_card_carries_its_todos_and_their_cards() {
    let env = Env::new();
    let ui = ui_session(&env).await;
    let plan = child(&env, &ui, UI_TASK, Some("Rewrite the design doc"), "plan_simple", None).await;
    edit(&plan, |s, _| {
        s.state.run_state = RunState::Running;
        s.state.turn_seq = 1;
        s.state.open_turn = Some(open_turn(1));
        s.state.turn_tasks = vec![binding(1, "t-plan-1", None, None)];
        s.state.one_line_status = "T02 in progress".into();
    });
    usage(&plan, &[("gpt-6.1-sol@cliproxy", 100, 10)]);
    let t1 = child(&env, &plan, "t-plan-1", Some("Survey"), "work", Some("work_simple")).await;
    edit(&t1, |s, lease| {
        s.write_report(lease, "found 3 mismatches").unwrap();
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Succeeded);
        s.state.finished_at_ms = Some(5);
    });
    usage(&t1, &[("gpt-6.1-sol@cliproxy", 1000, 200)]);
    let t2_old = child(&env, &plan, "t-plan-1", Some("Rewrite §3"), "work", None).await;
    edit(&t2_old, |s, _| {
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Failed);
    });
    usage(&t2_old, &[("gpt-6.1-sol@cliproxy", 30, 3)]);
    let t2 = child(&env, &plan, "t-plan-1", Some("Rewrite §3"), "work", None).await;
    edit(&t2, |s, _| {
        s.state.run_state = RunState::Running;
        s.state.turn_seq = 1;
        s.state.open_turn = Some(open_turn(1));
        s.state.one_line_status = "edit doc/llm_context/design.md".into();
    });
    usage(&t2, &[("gpt-6.1-sol@cliproxy", 400, 50)]);
    let list = json!({
        "schema": "opendan.todo_list/1", "next_seq": 6, "rev": 7,
        "todos": [
            todo("T03", 3, "Add tests", "blocked", json!([]), json!({ "blocked_by": ["T02"] })),
            todo("T01", 1, "Survey", "completed", json!([attempt(1, t1.sid(), "completed")]), json!({})),
            todo("T02", 2, "Rewrite §3", "running",
                 json!([attempt(1, t2_old.sid(), "failed"), attempt(2, t2.sid(), "running")]), json!({})),
            todo("T04", 4, "Accept", "pending", json!([]), json!({})),
            todo("T05", 5, "Old idea", "cancelled", json!([]), json!({ "cancel_reason": "not needed" })),
        ]
    });
    std::fs::write(
        plan.state_dir().join("todos.json"),
        serde_json::to_vec_pretty(&list).unwrap(),
    )
    .unwrap();

    let all = cards(&env, &ui).await;
    assert_eq!(all.sessions.len(), 1, "only the work the Turn created");
    let p = &all.sessions[0];
    assert_eq!((p.shape, p.state, p.line.as_str()), (CardShape::Plan, CardState::Running, "T02 in progress"));
    assert_eq!(p.usage.output_tokens, 10);
    let sub = p.subtree_usage.as_ref().unwrap();
    assert_eq!((sub.input_tokens, sub.output_tokens, sub.rounds), (1530, 263, 4));
    let todos = p.todos.as_ref().unwrap();
    assert_eq!((todos.rev, todos.total, todos.done), (7, 4, 1));
    let ids: Vec<&str> = todos.items.iter().map(|t| t.todo_id.as_str()).collect();
    assert_eq!(ids, ["T01", "T02", "T03", "T04", "T05"]);
    let c1 = todos.items[0].card.as_ref().unwrap();
    assert_eq!((c1.state, c1.line.as_str(), c1.title.as_str()), (CardState::Succeeded, "found 3 mismatches", "Survey"));
    assert!(c1.todos.is_none());
    // A retried Todo: its latest attempt only.
    let t = &todos.items[1];
    assert_eq!(t.attempt, 2);
    let c2 = t.card.as_ref().unwrap();
    assert_eq!((c2.session_id.as_str(), c2.state), (t2.sid(), CardState::Running));
    assert_eq!(c2.line, "edit doc/llm_context/design.md");
    assert_eq!(c2.usage.output_tokens, 50);
    // Not bound: titles only.
    assert!(todos.items[2].card.is_none());
    assert_eq!(todos.items[2].blocked_by, ["T02"]);
    assert_eq!(todos.items[2].attempt, 0);
    assert!(todos.items[3].card.is_none() && todos.items[4].card.is_none());
    assert_eq!(serde_json::to_value(todos.items[4].status).unwrap(), "cancelled");
    assert!(!all.complete);
}

#[tokio::test]
async fn only_the_turns_work_and_complete_when_all_ended() {
    let env = Env::new();
    let ui = ui_session(&env).await;
    let mine = child(&env, &ui, UI_TASK, Some("Mine"), "work", None).await;
    let other = child(&env, &ui, "t-ui-2", Some("Other Turn"), "work", None).await;
    let all = cards(&env, &ui).await;
    assert_eq!(all.sessions.iter().map(|c| c.session_id.as_str()).collect::<Vec<_>>(), [mine.sid()]);
    assert!(!all.complete);
    let none = work_cards(env.agent().as_ref(), ui.sid(), "t-ui-9").await.unwrap();
    assert!(none.sessions.is_empty() && !none.complete);
    edit(&mine, |s, _| {
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Succeeded);
    });
    // The Turn is still open: it may start more work.
    assert!(!cards(&env, &ui).await.complete);
    edit(&ui, |s, _| {
        s.state.open_turn = None;
        s.state.run_state = RunState::Waiting;
        s.state.turn_tasks = vec![binding(1, UI_TASK, Some(TurnStatus::Completed), Some("started"))];
    });
    assert!(cards(&env, &ui).await.complete);
    let _ = other;
}

#[tokio::test]
async fn a_todo_session_has_the_todo_title() {
    use libopendan::api::{create_sub_session, SubSessionSpec};
    let env = Env::new();
    let parent = env.create_work(work_spec("parent")).await;
    let sub = SubSessionSpec {
        objective: "Todo T01 (attempt 1, serial): Survey\nlong task text".into(),
        title: Some("  Survey   the\nsources ".into()),
        key: Some("k".into()),
        ..Default::default()
    };
    let sd = create_sub_session(env.agent().as_ref(), APP, parent.sid(), sub, env.channels().as_ref()).await.unwrap();
    assert_eq!(sd.config().unwrap().session.title.as_deref(), Some("Survey the sources"));
    let entry = env.agent().sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert_eq!(entry.title.as_deref(), Some("Survey the sources"));
}

fn finish(sd: &SessionDir) {
    edit(sd, |s, _| {
        s.state.run_state = RunState::Finished;
        s.state.outcome = Some(Outcome::Succeeded);
        s.state.finished_at_ms = Some(9);
    });
}

fn running(sd: &SessionDir) {
    edit(sd, |s, _| {
        s.state.run_state = RunState::Running;
        s.state.turn_seq = 1;
        s.state.open_turn = Some(open_turn(1));
    });
}

/// SC5: the work below a conversation as a tree — two works side by side, a
/// plan with three Todos, one of whose work sessions started a work of its
/// own; unfinished only, and complete once everything ended.
#[tokio::test]
async fn the_work_below_a_session_is_a_tree() {
    let env = Env::new();
    let agent = env.agent();
    let ui = ui_session(&env).await;
    let done = child(&env, &ui, UI_TASK, Some("Done before"), "work", None).await;
    finish(&done);
    let side = child(&env, &ui, "t-ui-0", Some("Side job"), "work", None).await;
    running(&side);
    edit(&side, |s, _| {
        s.state.paused = Some(Paused { source: PauseSource::Interrupted, at_ms: 2, reason: Some("interrupted by a restart of the host".into()), by: APP.into(), in_run: false });
    });
    let plan = child(&env, &ui, UI_TASK, Some("The plan"), "plan_simple", None).await;
    running(&plan);
    let t1 = child(&env, &plan, "t-plan-1", Some("One"), "work", None).await;
    finish(&t1);
    let t2_old = child(&env, &plan, "t-plan-1", Some("Two"), "work", None).await;
    finish(&t2_old);
    let t2 = child(&env, &plan, "t-plan-1", Some("Two"), "work", None).await;
    running(&t2);
    let deep = child(&env, &t2, "t-t2-1", Some("Deeper"), "work", None).await;
    running(&deep);
    let list = json!({
        "schema": "opendan.todo_list/1", "next_seq": 4, "rev": 3,
        "todos": [
            todo("T01", 1, "One", "completed", json!([attempt(1, t1.sid(), "completed")]), json!({})),
            todo("T02", 2, "Two", "running", json!([attempt(1, t2_old.sid(), "failed"), attempt(2, t2.sid(), "running")]), json!({})),
            todo("T03", 3, "Three", "pending", json!([]), json!({})),
        ]
    });
    std::fs::write(plan.state_dir().join("todos.json"), serde_json::to_vec_pretty(&list).unwrap()).unwrap();

    let all = work_tree(agent.as_ref(), ui.sid(), false).await.unwrap();
    assert!(!all.complete);
    let ids: Vec<&str> = all.sessions.iter().map(|c| c.session_id.as_str()).collect();
    assert_eq!(ids, [done.sid(), side.sid(), plan.sid()], "every work of the session, whichever Turn created it");
    assert_eq!((all.sessions[1].state, all.sessions[1].paused), (CardState::Paused, Some(PauseSource::Interrupted)));

    let tree = work_tree(agent.as_ref(), ui.sid(), true).await.unwrap();
    assert!(!tree.complete);
    let ids: Vec<&str> = tree.sessions.iter().map(|c| c.session_id.as_str()).collect();
    assert_eq!(ids, [side.sid(), plan.sid()], "a finished branch is left out");
    let p = &tree.sessions[1];
    assert!(p.children.is_empty(), "a Todo's attempts are not listed beside its Todos: {:?}", p.children);
    let todos = p.todos.as_ref().unwrap();
    assert_eq!(todos.items.len(), 3, "a plan in progress keeps its whole list");
    let c2 = todos.items[1].card.as_ref().unwrap();
    assert_eq!((c2.session_id.as_str(), c2.title.as_str(), c2.state), (t2.sid(), "Two", CardState::Running));
    assert_eq!(c2.children.len(), 1, "what a Todo's work session started hangs below it");
    assert_eq!((c2.children[0].session_id.as_str(), c2.children[0].title.as_str()), (deep.sid(), "Deeper"));
    assert!(todos.items[0].card.as_ref().unwrap().children.is_empty());

    for sd in [&side, &plan, &t2, &deep] {
        finish(sd);
    }
    let tree = work_tree(agent.as_ref(), ui.sid(), true).await.unwrap();
    assert!(tree.complete && tree.sessions.is_empty(), "{tree:?}");
}
