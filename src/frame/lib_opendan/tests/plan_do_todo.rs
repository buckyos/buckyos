//! TodoList-driven PLAN / DO (notepads/opendan-plan-do-todo-list-todo.md §6):
//! a PLAN that turns its objective into Todos, dispatches explorations as
//! parallel sub sessions through the real `agent-session todo` commands,
//! waits by not deciding, reviews each report and runs a serial Todo in DO.
//!
//! The models are scripted (content based); everything else is real: the
//! session's shell runs the `xagent` binary as `agent-session`, sub
//! sessions are driven by the host's ChildDriver in parallel, the list,
//! the registry and the worklogs are the committed files.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use buckyos_api::{AiResponse, AiRole};
use common::*;
use libopendan::host::{run_session, HostDeps, RunOutcome};
use libopendan::protocol::*;
use libopendan::runner::{BehaviorAssembler, RunnerOptions, StopSignal, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::todos::{store_of, ReviewState, TodoList, TodoMode, TodoStatus};
use libopendan::{SessionDir, SessionTemplate};
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

const PLAN: &str = r#"
[meta]
objective = "plan with todos"
next = ["do"]

[prompt]
mode = "behavior"
system = "PLAN-TEST"

[capabilities]
tool_whitelist = []
action_whitelist = ["group:bash"]
"#;

const DO: &str = r#"
[prompt]
mode = "behavior"
system = "DO-TEST"

[entry]
mode = "create_sub_context"
inherit = "steps"
todo = true

[capabilities]
tool_whitelist = []
action_whitelist = ["group:bash"]
"#;

const EXPLORE: &str = r#"
[prompt]
mode = "function_call"
system = "EXPLORE-TEST"

[capabilities]
tool_whitelist = ["group:bash", "report"]
"#;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn write_catalog(env: &Env) {
    write(&env.agent_root.join("role.md"), "I am Jarvis.");
    write(&env.agent_root.join("behaviors/plan.toml"), PLAN);
    write(&env.agent_root.join("behaviors/do.toml"), DO);
    write(&env.agent_root.join("behaviors/explore.toml"), EXPLORE);
}

fn plan_spec(objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("work", None).unwrap().spec(objective);
    // As the Jarvis package: tools in the base, each behavior chooses its loop.
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.prompt.behavior = Some("plan".into());
    spec.freeze = true;
    spec
}

/// The tool processes of the session (`agent-session`) act as the driver.
fn tool_env(env: &Env, max_children: Option<usize>) {
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    std::env::remove_var("LIBOPENDAN_FAULT");
    match max_children {
        Some(n) => std::env::set_var(libopendan::todos::ENV_MAX_CHILDREN, n.to_string()),
        None => std::env::remove_var(libopendan::todos::ENV_MAX_CHILDREN),
    }
}

fn host(env: &Env, llm: Arc<dyn llm_context::deps::LlmClient>) -> HostDeps {
    let agent: Arc<dyn AgentStateClient> = env.agent();
    HostDeps {
        who: APP.into(),
        agent,
        inputs: env.channels(),
        waker: Arc::new(libopendan::channel::PollWaker),
        xllm: agent_tool::xllm::XllmDeps::default().with_llm(llm),
        assembler: Arc::new(BehaviorAssembler::default()),
        session_cli: Some(PathBuf::from(env!("CARGO_BIN_EXE_xagent"))),
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        options: RunnerOptions {
            poll_interval: Duration::from_millis(50),
            max_wait: Duration::from_secs(60),
            load_hints: false,
            ..Default::default()
        },
        max_child_concurrency: 4,
        bridges: Vec::new(),
        outbound: None,
        objects: None,
        turn_tasks: None,
        notify: None,
        share: None,
        runtime_id: None,
    }
}

async fn drive_to_end(env: &Env, llm: Arc<dyn llm_context::deps::LlmClient>, sd: &SessionDir) -> RunOutcome {
    let h = host(env, llm);
    Box::pin(run_session(&h, sd, StopWhen::Finished, StopSignal::default(), false)).await
}

fn system_of(req: &LlmInferenceRequest) -> String {
    req.messages
        .iter()
        .filter(|m| m.role == AiRole::System)
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Plan,
    Do,
    Explore,
}

fn role(req: &LlmInferenceRequest) -> Role {
    let s = system_of(req);
    if s.contains("EXPLORE-TEST") {
        Role::Explore
    } else if s.contains("DO-TEST") {
        Role::Do
    } else {
        assert!(s.contains("PLAN-TEST"), "{s}");
        Role::Plan
    }
}

/// A behavior step: shell actions (CDATA bodies) and an optional report.
fn actions(cmds: &[String], report: Option<&str>) -> AiResponse {
    let mut x = String::from("<response><intent>act</intent><actions>");
    for c in cmds {
        x.push_str(&format!("<shell><![CDATA[{c}]]></shell>"));
    }
    x.push_str("</actions>");
    if let Some(r) = report {
        x.push_str(&format!("<report><![CDATA[{r}]]></report>"));
    }
    x.push_str("</response>");
    text(&x)
}

fn stage(report: &str) -> AiResponse {
    text(&format!("<response><intent>wait</intent><report><![CDATA[{report}]]></report></response>"))
}

fn handover(report: &str, to: &str) -> AiResponse {
    text(&format!(
        "<response><intent>hand over</intent><report><![CDATA[{report}]]></report><next_behavior>{to}</next_behavior></response>"
    ))
}

fn end(report: &str, artifacts: &[&str]) -> AiResponse {
    let a = if artifacts.is_empty() {
        String::new()
    } else {
        format!("<artifacts>{}</artifacts>", json!(artifacts))
    };
    text(&format!(
        "<response><intent>finish</intent><report end=\"true\"><![CDATA[{report}]]></report>{a}</response>"
    ))
}

/// What a failed scenario left, for the test log.
fn dump(sd: &SessionDir, llm: &ScriptedLlm) {
    let st = sd.state().unwrap();
    eprintln!("state: run_state={:?} waiting_for={:?} last_error={:?} one_line={}", st.run_state, st.waiting_for, st.last_error, st.one_line_status);
    for e in read_worklog(sd) {
        eprintln!("  wl {}", serde_json::to_string(&e.body).unwrap().chars().take(600).collect::<String>());
    }
    eprintln!("list: {}", list_of(sd.path()).render_view());
    for t in list_of(sd.path()).todos {
        for a in &t.attempts {
            if let Some(c) = &a.child_sid {
                let dir = sd.path().parent().unwrap().join(c);
                match SessionDir::open(&dir).and_then(|d| d.state()) {
                    Ok(cs) => eprintln!("  child {c}: {:?} {:?} err={:?} {}", cs.run_state, cs.outcome, cs.last_error, cs.one_line_status),
                    Err(e) => eprintln!("  child {c}: {e}"),
                }
            }
        }
    }
    for (i, r) in llm.requests.lock().unwrap().iter().enumerate() {
        eprintln!("--- request {i} ({:?}) last user: {}", role(r), last_user_text(r).chars().take(1500).collect::<String>());
    }
}

/// The model as the host calls it: an exploration's first inference waits
/// (asynchronously, with a deadline) until `barrier` explorations started —
/// a barrier that only orders the test and produces no result; a serial
/// dispatch would never pass it in time.
struct Gate {
    inner: Arc<ScriptedLlm>,
    timeline: Arc<Timeline>,
    barrier: usize,
}

#[async_trait::async_trait]
impl llm_context::deps::LlmClient for Gate {
    async fn infer(
        &self,
        req: LlmInferenceRequest,
    ) -> Result<AiResponse, llm_context::error::LLMComputeError> {
        if role(&req) == Role::Explore && has_tool_result(&req, "x1").is_none() {
            let sys = system_of(&req);
            if let Some(m) = ["MARK-A", "MARK-B", "MARK-D"].into_iter().find(|m| sys.contains(m)) {
                self.timeline.start(&m[5..]);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.timeline.started() < self.barrier && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        self.inner.infer(req).await
    }
}

fn list_of(path: &Path) -> TodoList {
    store_of(&SessionDir::open(path).unwrap()).load().unwrap()
}

/// `starts` / `ends`: the wall clock of each exploration's first and last
/// inference.
#[derive(Default)]
struct Timeline {
    starts: Mutex<Vec<(String, u128)>>,
    ends: Mutex<Vec<(String, u128)>>,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

impl Timeline {
    fn start(&self, who: &str) {
        let mut s = self.starts.lock().unwrap();
        if !s.iter().any(|(w, _)| w == who) {
            s.push((who.to_string(), now_ms()));
        }
    }

    fn started(&self) -> usize {
        self.starts.lock().unwrap().len()
    }

    fn end(&self, who: &str) {
        self.ends.lock().unwrap().push((who.to_string(), now_ms()));
    }

    fn overlapped(&self) -> bool {
        let starts = self.starts.lock().unwrap();
        let ends = self.ends.lock().unwrap();
        let last_start = starts.iter().map(|(_, t)| *t).max().unwrap_or(0);
        let first_end = ends.iter().map(|(_, t)| *t).min().unwrap_or(0);
        starts.len() >= 2 && ends.len() >= 2 && last_start <= first_end
    }
}

/// An exploration child: one shell action, then its final report. The
/// report's first line is its summary; `DETAIL-<m>` is only in its body.
fn explore_child(req: &LlmInferenceRequest, timeline: &Timeline) -> AiResponse {
    let sys = system_of(req);
    let marker = ["MARK-A", "MARK-B", "MARK-D"]
        .into_iter()
        .find(|m| sys.contains(m))
        .unwrap_or_else(|| panic!("an exploration without its todo body: {sys}"));
    let first = user_texts(req).into_iter().next().unwrap_or_default();
    assert!(first.contains(marker), "the first input carries the todo body: {first}");
    let m = &marker[5..];
    if has_tool_result(req, "x1").is_none() {
        timeline.start(m);
        return tool_call(
            "x1",
            "shell",
            json!({ "command": format!("echo CHILD-OUT-{m} > repro_{m}.txt && cat repro_{m}.txt") }),
        );
    }
    timeline.end(m);
    tool_call(
        "x2",
        "report",
        json!({
            "report": format!("{m}: the defect reproduces\n\nDETAIL-{m}-FULL the file, the line and the repro output"),
            "artifacts": [format!("repro_{m}.txt")],
            "is_end": true,
        }),
    )
}

fn do_child(req: &LlmInferenceRequest) -> AiResponse {
    let u = render(&req.messages);
    assert!(u.contains("<sub_task mode=\"create_sub_context\">"), "{u}");
    if !u.contains("[result") && !u.contains("verified-C") {
        assert!(u.contains("Todo T03 (attempt 1, serial)"), "the sub task is the bound todo: {u}");
        assert!(u.contains("MARK-C"), "only the todo body names MARK-C: {u}");
        assert!(u.contains("Builds on these reviewed results"), "{u}");
        return actions(&["echo verified-C > verify.txt".into()], None);
    }
    end("T03: both findings hold\n\nDETAIL-C-FULL public entry checked", &["verify.txt"])
}

fn explore_cmd(title: &str, marker: &str) -> String {
    format!(
        "agent-session todo explore \"{title}\" --content \"check {marker} in scope\" --accept \"a reproduction for {marker}\" --scope \"fixture/\""
    )
}

fn review_cmds(id: &str) -> Vec<String> {
    vec![
        format!("agent-session todo show {id} --report"),
        format!("agent-session todo review {id} --accept --reason \"the repro holds\""),
    ]
}

/// V02 / V03 / V05 / V06 / V12: two explorations run in parallel as their own
/// sub sessions while the PLAN waits by not deciding; their ends wake it,
/// it reads each report on demand, reviews it, and only then creates the
/// serial verification Todo, which DO executes from its bound body.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explorations_in_parallel_then_a_reviewed_serial_todo() {
    let env = Env::new();
    write_catalog(&env);
    tool_env(&env, None);
    let sd = env.create_work(plan_spec("review the fixture")).await;
    let path = sd.path().to_path_buf();
    let timeline = Arc::new(Timeline::default());
    let tl = timeline.clone();
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => explore_child(req, &tl),
        Role::Do => do_child(req),
        Role::Plan => {
            let list = list_of(&path);
            if list.todos.is_empty() {
                return actions(
                    &[explore_cmd("inventory check", "MARK-A"), explore_cmd("idempotency check", "MARK-B")],
                    Some("dispatching T01 and T02 in parallel"),
                );
            }
            if let Some(t) = list.todos.iter().find(|t| t.review_state() == ReviewState::Pending) {
                return actions(&review_cmds(&t.todo_id), Some("reviewing what came back"));
            }
            if !list.explorations_in_flight().is_empty() {
                return stage("waiting for the explorations that still run");
            }
            match list.get("T03") {
                None => actions(
                    &["agent-session todo add \"verify\" --content \"verify both through the public entry MARK-C\" --accept \"public entry reproduces\" --after T01,T02".into()],
                    Some("T01 and T02 accepted: verify serially"),
                ),
                Some(t) if t.status == TodoStatus::Pending => handover("T03 next", "do"),
                Some(_) => end("final review: two defects confirmed", &[]),
            }
        }
    });
    let gate = Arc::new(Gate { inner: llm.clone(), timeline: timeline.clone(), barrier: 2 });
    let out = drive_to_end(&env, gate, &sd).await;
    if !matches!(out.result, libopendan::runner::DriveResult::Finished { .. }) {
        dump(&sd, &llm);
        panic!("{:?}", out.result);
    }
    assert!(out.children.is_empty());

    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Succeeded));
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1), "one Turn throughout");

    // The list: two explorations and one serial Todo, all reviewed.
    let list = list_of(sd.path());
    assert_eq!(list.todos.len(), 3);
    let (a, b, c) = (list.get("T01").unwrap(), list.get("T02").unwrap(), list.get("T03").unwrap());
    for t in [a, b] {
        assert_eq!((t.mode, t.status), (TodoMode::Explore, TodoStatus::Completed));
        assert_eq!(t.review_state(), ReviewState::Accepted);
    }
    let (ca, cb) = (
        a.latest().unwrap().child_sid.clone().unwrap(),
        b.latest().unwrap().child_sid.clone().unwrap(),
    );
    assert_ne!(ca, cb, "one sub session per exploration");
    assert_eq!((c.mode, c.status), (TodoMode::Serial, TodoStatus::Completed));
    assert_eq!(c.review_state(), ReviewState::Accepted);
    let ca3 = c.latest().unwrap();
    assert!(ca3.caller.as_ref().unwrap().call_id.starts_with("step:"));
    assert!(ca3.run_id.is_some(), "the sub context's run is recorded");
    assert_eq!(ca3.summary.as_deref(), Some("T03: both findings hold"));
    // C was created after both reviews.
    for t in [a, b] {
        assert!(c.created_at_ms >= t.reviews[0].at_ms, "C follows the reviews");
    }

    // Each exploration: its own directory, accepted, its report readable.
    let agent = env.agent();
    for (t, sid) in [(a, &ca), (b, &cb)] {
        let e = agent.sessions().lookup(sid).await.unwrap().unwrap();
        assert_eq!(e.origin.as_ref().unwrap().parent_session.as_deref(), Some(sd.sid()));
        assert_eq!(e.status.acceptance, Acceptance::Accepted, "the review is the sub session's acceptance");
        let r = libopendan::todos::read_report(agent.as_ref(), &sd, t.latest().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(r.report.contains("FULL"), "{r:?}");
        assert!(r.session_dir.unwrap().ends_with(sid));
    }
    let rc = libopendan::todos::read_report(agent.as_ref(), &sd, ca3).await.unwrap().unwrap();
    assert!(rc.report.contains("DETAIL-C-FULL"), "the serial report outlives its run");

    // Real parallelism: both explorations ran before either ended.
    assert!(timeline.overlapped(), "starts {:?} ends {:?}", timeline.starts.lock().unwrap(), timeline.ends.lock().unwrap());

    // The PLAN waited by not deciding: its run ended, the Turn stayed open
    // waiting for the children, their ends joined that Turn.
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(|e| matches!(&e.body, WorklogBody::Outcome { kind, .. } if kind == "wait")));
    assert!(wl.iter().any(|e| matches!(&e.body, WorklogBody::InputBatch { inputs, .. }
        if inputs.iter().any(|i| i.src == INTERNAL_CHILD_SRC))));
    assert!(!wl.iter().any(|e| matches!(&e.body, WorklogBody::TurnEnded { status, .. } if *status != TurnStatus::Completed)));

    // V12: the PLAN's context holds the view, never the children's tool
    // output, and a full report only after it asked for it.
    let reqs = llm.requests.lock().unwrap().clone();
    let plans: Vec<String> = reqs.iter().filter(|r| role(r) == Role::Plan).map(|r| render(&r.messages)).collect();
    assert!(plans.iter().any(|p| p.contains("<todo_list rev=")), "the view is assembled");
    for p in &plans {
        assert!(!p.contains("CHILD-OUT-"), "no child tool output in the PLAN: {p}");
        for (m, id) in [("A", "T01"), ("B", "T02"), ("C", "T03")] {
            if p.contains(&format!("DETAIL-{m}-FULL")) {
                assert!(p.contains(&format!("todo show {id} --report")), "the full report of {id} only after asking: {p}");
            }
        }
    }
    // V05: DO's request carried the bound todo.
    assert!(reqs.iter().any(|r| role(r) == Role::Do && render(&r.messages).contains("MARK-C")));
}

fn finished(out: &RunOutcome, sd: &SessionDir, llm: &ScriptedLlm) {
    if !matches!(out.result, libopendan::runner::DriveResult::Finished { .. }) {
        dump(sd, llm);
        panic!("{:?}", out.result);
    }
}

fn gate(llm: &Arc<ScriptedLlm>, barrier: usize) -> Arc<Gate> {
    Arc::new(Gate {
        inner: llm.clone(),
        timeline: Arc::new(Timeline::default()),
        barrier,
    })
}

fn plan_requests(llm: &ScriptedLlm) -> Vec<String> {
    llm.requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| role(r) == Role::Plan)
        .map(|r| render(&r.messages))
        .collect()
}

fn corrections(sd: &SessionDir) -> Vec<String> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Step { correction: true, .. } => Some(serde_json::to_string(&e.body).unwrap()),
            _ => None,
        })
        .collect()
}

/// V01: `next_behavior = do` without a ready serial Todo. With nothing
/// running it is refused correctably — the same run goes on, the Turn does
/// not fail, no DO context exists; with an exploration running it waits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn do_without_a_ready_todo_is_corrected_or_waits() {
    let env = Env::new();
    write_catalog(&env);
    tool_env(&env, None);
    let sd = env.create_work(plan_spec("handle the work")).await;
    let path = sd.path().to_path_buf();
    let tl = Arc::new(Timeline::default());
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => explore_child(req, &tl),
        Role::Do => do_t02(req),
        Role::Plan => {
            let list = list_of(&path);
            let u = render(&req.messages);
            if list.todos.is_empty() && !u.contains("Decision refused") {
                return handover("nothing planned yet", "do");
            }
            if list.todos.is_empty() {
                return actions(&[explore_cmd("inventory check", "MARK-A")], Some("dispatching T01"));
            }
            if let Some(t) = list.todos.iter().find(|t| t.review_state() == ReviewState::Pending) {
                return actions(&review_cmds(&t.todo_id), Some("reviewing"));
            }
            if !list.explorations_in_flight().is_empty() {
                // Nothing serial while T01 runs: this hand-over is a wait.
                return handover("T01 runs; DO has nothing yet", "do");
            }
            match list.get("T02") {
                None => actions(&["agent-session todo add \"serial work\" --content \"do the serial part MARK-S\" --after T01".into()], Some("adding T02")),
                Some(t) if t.status == TodoStatus::Pending => handover("T02 next", "do"),
                Some(_) => end("done", &[]),
            }
        }
    });
    let out = drive_to_end(&env, gate(&llm, 1), &sd).await;
    finished(&out, &sd, &llm);
    let fixes = corrections(&sd);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    assert!(fixes[0].contains("<next_behavior>do</next_behavior>"), "{fixes:?}");
    assert!(
        plan_requests(&llm).iter().any(|p| p.contains("executes the next serial Todo, and none is ready")),
        "the refusal is fed back to the same run"
    );
    let wl = read_worklog(&sd);
    assert!(!wl.iter().any(|e| matches!(&e.body, WorklogBody::TurnEnded { status, .. } if *status != TurnStatus::Completed)));
    // The refused hand-over and the one that waited created no DO run; DO
    // ran once, for T02.
    let do_runs: std::collections::BTreeSet<String> = wl
        .iter()
        .filter_map(|e| match &e.body {
            WorklogBody::Step { run_id, behavior: Some(b), .. } if b == "do" => Some(run_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(do_runs.len(), 1, "{do_runs:?}");
    assert!(wl.iter().any(|e| matches!(&e.body, WorklogBody::Outcome { kind, .. } if kind == "wait")));
    let list = list_of(sd.path());
    assert_eq!(list.get("T02").unwrap().attempts.len(), 1);
    assert_eq!(list.get("T01").unwrap().review_state(), ReviewState::Accepted);
}

fn do_t02(req: &LlmInferenceRequest) -> AiResponse {
    let u = render(&req.messages);
    assert!(u.contains("MARK-S") && u.contains("Todo T02"), "{u}");
    if u.contains("[result") || u.contains("done-S") {
        return end("T02: serial part done", &[]);
    }
    actions(&["echo done-S > s.txt".into()], None)
}

/// V04: both concurrency limits refuse a dispatch observably — the Todo
/// stays pending with the refusal — and the PLAN dispatches it again once
/// the first exploration is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dispatch_limits_are_observable_and_recoverable() {
    for (variant, policy_max, host_max) in [("max_sub_sessions", Some(1u8), None), ("--max-children", None, Some(1usize))] {
        let env = Env::new();
        write_catalog(&env);
        tool_env(&env, host_max);
        let mut spec = plan_spec("two explorations, one slot");
        if let Some(n) = policy_max {
            spec.policy.max_sub_sessions = n;
        }
        let sd = env.create_work(spec).await;
        let path = sd.path().to_path_buf();
        let tl = Arc::new(Timeline::default());
        let llm = ScriptedLlm::new(move |req, _| match role(req) {
            Role::Explore => explore_child(req, &tl),
            Role::Do => panic!("no serial work here"),
            Role::Plan => {
                let list = list_of(&path);
                if list.todos.is_empty() {
                    return actions(
                        &[explore_cmd("inventory check", "MARK-A"), explore_cmd("idempotency check", "MARK-B")],
                        Some("dispatching T01 and T02"),
                    );
                }
                if let Some(t) = list.todos.iter().find(|t| t.review_state() == ReviewState::Pending) {
                    return actions(&review_cmds(&t.todo_id), Some("reviewing"));
                }
                if !list.explorations_in_flight().is_empty() {
                    return stage("waiting for the running exploration");
                }
                if list.get("T02").unwrap().status == TodoStatus::Pending {
                    return actions(&["agent-session todo dispatch T02".into()], Some("a slot is free: T02"));
                }
                end("both explored", &[])
            }
        });
        let out = drive_to_end(&env, gate(&llm, 1), &sd).await;
        finished(&out, &sd, &llm);
        let list = list_of(sd.path());
        let (a, b) = (list.get("T01").unwrap(), list.get("T02").unwrap());
        assert_eq!(b.refusals.len(), 1, "{variant}: {:?}", b.refusals);
        assert!(b.refusals[0].reason.contains(variant), "{variant}: {}", b.refusals[0].reason);
        assert_eq!(b.attempts.len(), 1);
        assert!(b.attempts[0].started_at_ms >= a.attempts[0].ended_at_ms.unwrap(), "{variant}: dispatched after the slot was free");
        // The refusal was in the PLAN's view of its own action.
        assert!(plan_requests(&llm).iter().any(|p| p.contains("\"dispatched\": false")), "{variant}");
        let agent = env.agent();
        assert_eq!(agent.sessions().children_of(&[sd.sid().to_string()]).await.unwrap().len(), 2);
    }
    tool_env(&Env::new(), None);
}

/// V07 / V10: an exploration fails; the PLAN rejects it and retries. The
/// new attempt has its own sub session; the failed one keeps its record and
/// its sub session is discarded. A Todo that is not executed is cancelled
/// with its reason, never deleted; ids are never reused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_exploration_is_rejected_and_retried() {
    let env = Env::new();
    write_catalog(&env);
    tool_env(&env, None);
    let sd = env.create_work(plan_spec("explore with a failing branch")).await;
    let path = sd.path().to_path_buf();
    let tl = Arc::new(Timeline::default());
    let llm = ScriptedLlm::fallible(move |req, _| match role(req) {
        Role::Explore if system_of(req).contains("attempt 1,") => Err(llm_context::error::LLMComputeError::Provider {
            failure: llm_context::error::ProviderFailure::Permanent,
            message: "the input of this branch is missing".into(),
        }),
        Role::Explore => Ok(explore_child(req, &tl)),
        Role::Do => panic!("no serial work here"),
        Role::Plan => Ok({
            let list = list_of(&path);
            if list.todos.is_empty() {
                actions(
                    &[
                        explore_cmd("inventory check", "MARK-A"),
                        "agent-session todo add \"maybe later\" --content \"optional follow-up\"".into(),
                    ],
                    Some("T01 explores, T02 may follow"),
                )
            } else if let Some(t) = list.todos.iter().find(|t| t.review_state() == ReviewState::Pending) {
                let attempt = t.latest().unwrap();
                if attempt.status == TodoStatus::Failed {
                    actions(
                        &[
                            format!("agent-session todo show {} --report", t.todo_id),
                            format!("agent-session todo review {} --reject --reason \"the branch failed: no evidence\"", t.todo_id),
                            format!("agent-session todo retry {} --reason \"retry with the input restored\"", t.todo_id),
                        ],
                        Some("T01 failed: rejected and retried"),
                    )
                } else {
                    actions(&review_cmds(&t.todo_id), Some("reviewing the retry"))
                }
            } else if !list.explorations_in_flight().is_empty() {
                stage("waiting for T01")
            } else if list.get("T02").unwrap().status == TodoStatus::Pending {
                actions(&["agent-session todo cancel T02 --reason \"not needed after T01\"".into()], Some("dropping T02"))
            } else {
                end("explored after one retry", &[])
            }
        }),
    });
    let out = drive_to_end(&env, gate(&llm, 1), &sd).await;
    finished(&out, &sd, &llm);
    let list = list_of(sd.path());
    let a = list.get("T01").unwrap();
    assert_eq!(a.attempts.len(), 2);
    let (first, second) = (&a.attempts[0], &a.attempts[1]);
    assert_eq!(first.status, TodoStatus::Failed);
    assert!(first.error.as_deref().unwrap_or_default().contains("missing"), "{first:?}");
    assert_eq!(second.status, TodoStatus::Completed);
    assert_eq!(second.reason.as_deref(), Some("retry with the input restored"));
    assert_ne!(first.child_sid, second.child_sid, "a new attempt is a new sub session");
    assert_eq!(a.reviews.len(), 2);
    assert_eq!(a.reviews[0].verdict, libopendan::todos::ReviewVerdict::Reject);
    assert_eq!(a.review_state(), ReviewState::Accepted);
    let b = list.get("T02").unwrap();
    assert_eq!(b.status, TodoStatus::Cancelled);
    assert_eq!(b.cancel_reason.as_deref(), Some("not needed after T01"));
    assert_eq!(list.next_seq, 3);
    let agent = env.agent();
    let failed = agent.sessions().lookup(first.child_sid.as_deref().unwrap()).await.unwrap().unwrap();
    assert_eq!(failed.status.outcome, Some(Outcome::Failed));
    assert_eq!(failed.status.acceptance, Acceptance::Discarded, "a rejected exploration is discarded");
}

/// The tool process as the PLAN's shell runs it.
fn agent_session(env: &Env, sd: &SessionDir, args: &[&str], fault: Option<&str>) -> (i32, Value) {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"));
    c.args(args)
        .env("OPENDAN_SESSION_ID", sd.sid())
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .env("LIBOPENDAN_WHO", APP)
        .env("LIBOPENDAN_QUEUE_DIR", &env.queue_dir)
        .env_remove(libopendan::todos::ENV_MAX_CHILDREN);
    match fault {
        Some(f) => c.env("LIBOPENDAN_FAULT", f),
        None => c.env_remove("LIBOPENDAN_FAULT"),
    };
    let out = c.output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    (
        out.status.code().unwrap_or(-1),
        serde_json::from_str(&text)
            .unwrap_or_else(|_| Value::String(format!("{text}{}", String::from_utf8_lossy(&out.stderr)))),
    )
}

/// V08 / V09: replays and crashes of the tool layer. Two dispatches in one
/// shell command get two sub sessions; dispatching a running Todo again or
/// repeating a review changes nothing; a dispatch killed after its record
/// is withdrawn by the host's sync, one killed after creating its sub
/// session is bound by it — never a second sub session.
#[tokio::test]
async fn tool_layer_replays_and_crashes() {
    let env = Env::new();
    write_catalog(&env);
    let sd = env.create_work(plan_spec("tool layer")).await;
    let agent = env.agent();
    // One shell batch, two explorations.
    let (code, v) = agent_session(&env, &sd, &["todo", "explore", "a", "--content", "MARK-A body"], None);
    assert_eq!(code, 0, "{v}");
    let (code, w) = agent_session(&env, &sd, &["todo", "explore", "b", "--content", "MARK-B body"], None);
    assert_eq!(code, 0, "{w}");
    assert_ne!(v["child_sid"], w["child_sid"]);
    // Dispatching a running Todo again: refused, nothing created.
    let (code, again) = agent_session(&env, &sd, &["todo", "dispatch", "T01"], None);
    assert_eq!(code, 3, "{again}");
    assert_eq!(again["dispatched"], false);
    let children = || async { agent.sessions().children_of(&[sd.sid().to_string()]).await.unwrap().len() };
    assert_eq!(children().await, 2);
    assert!(list_of(sd.path()).get("T01").unwrap().refusals.is_empty(), "a refused replay is not a limit hit");

    // Killed after the dispatch record: no sub session; the host withdraws it.
    agent_session(&env, &sd, &["todo", "add", "c", "--content", "MARK-C body"], None);
    let (code, _) = agent_session(&env, &sd, &["todo", "dispatch", "T03"], Some("todo:after_dispatch_record"));
    assert_ne!(code, 0);
    let t = list_of(sd.path()).get("T03").unwrap().clone();
    assert_eq!(t.latest().unwrap().dispatch, Some(libopendan::todos::DispatchState::Dispatching));
    assert_eq!(children().await, 2);
    let state = sd.state().unwrap();
    assert!(libopendan::todos::sync(agent.as_ref(), &sd, &state).await.unwrap());
    let t = list_of(sd.path()).get("T03").unwrap().clone();
    assert_eq!((t.status, t.attempts.len()), (TodoStatus::Pending, 0));
    assert!(t.refusals[0].reason.contains("interrupted"));

    // Killed after creating the sub session: bound, by a retried dispatch
    // as by the host's sync — the same sub session.
    let (code, _) = agent_session(&env, &sd, &["todo", "dispatch", "T03"], Some("todo:after_child_created"));
    assert_ne!(code, 0);
    assert_eq!(children().await, 3);
    let child = list_of(sd.path()).get("T03").unwrap().latest().unwrap().child_sid.clone().unwrap();
    let (code, r) = agent_session(&env, &sd, &["todo", "dispatch", "T03"], None);
    assert_eq!(code, 0, "{r}");
    assert_eq!(r["child_sid"], json!(child));
    assert_eq!(r["attempt"], 1);
    assert_eq!(children().await, 3, "never a second sub session");
    let t = list_of(sd.path()).get("T03").unwrap().clone();
    assert_eq!(t.latest().unwrap().dispatch, Some(libopendan::todos::DispatchState::Dispatched));

    // A body on stdin keeps what double quotes would have expanded.
    let mut c = std::process::Command::new("bash");
    c.arg("-c")
        .arg(format!(
            "{} todo add 'quoted body' --content - <<'EOF'\nprint a line `ACCEPT-X-1` with $HOME untouched\nEOF",
            env!("CARGO_BIN_EXE_xagent")
        ))
        .env("OPENDAN_SESSION_ID", sd.sid())
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .env("LIBOPENDAN_WHO", APP)
        .env("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    let out = c.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let t = list_of(sd.path()).todos.last().unwrap().clone();
    assert_eq!(t.content, "print a line `ACCEPT-X-1` with $HOME untouched");

    // Owner side: read only.
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"));
    let out = c
        .args(["todo", "list", sd.sid(), "--json"])
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .env_remove("OPENDAN_SESSION_ID")
        .output()
        .unwrap();
    let full: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(full["todos"].as_array().unwrap().len(), 4);
    assert_eq!(full["todos"][0]["attempts"][0]["child_sid"], v["child_sid"]);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xagent"))
        .args(["todo", "add", "x", "--content", "y", "--session", sd.sid()])
        .env("OPENDAN_AGENT_DID", AGENT)
        .env("OPENDAN_AGENT_ROOT", &env.agent_root)
        .env_remove("OPENDAN_SESSION_ID")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "writes only from the session's own runs");
}

/// V11: the session cannot end while its list has not converged (the end
/// is refused correctably and the PLAN reviews first); stopping the parent
/// stops its explorations, closes the waiting Turn, and the list follows.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn end_needs_a_converged_list_and_stop_cascades() {
    // The end check.
    let env = Env::new();
    write_catalog(&env);
    tool_env(&env, None);
    let sd = env.create_work(plan_spec("end check")).await;
    let path = sd.path().to_path_buf();
    let tl = Arc::new(Timeline::default());
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => explore_child(req, &tl),
        Role::Do => panic!("no serial work"),
        Role::Plan => {
            let list = list_of(&path);
            let u = render(&req.messages);
            if list.todos.is_empty() {
                return actions(&[explore_cmd("inventory check", "MARK-A")], Some("T01"));
            }
            if !list.explorations_in_flight().is_empty() {
                return stage("waiting for T01");
            }
            if list.get("T01").unwrap().review_state() == ReviewState::Pending && !u.contains("has not converged") {
                return end("finished without looking", &[]);
            }
            if list.get("T01").unwrap().review_state() == ReviewState::Pending {
                return actions(&review_cmds("T01"), Some("reviewing first"));
            }
            end("finished after review", &[])
        }
    });
    let out = drive_to_end(&env, gate(&llm, 1), &sd).await;
    finished(&out, &sd, &llm);
    let fixes = corrections(&sd);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    assert!(fixes[0].contains("finished without looking"), "{fixes:?}");
    assert!(
        plan_requests(&llm)
            .iter()
            .any(|p| p.contains("has not converged") && p.contains("the result of T01 (attempt 1) is not reviewed")),
        "the end is refused with the reason"
    );
    assert_eq!(sd.state().unwrap().final_report.unwrap().report, "finished after review");

    // The stop cascade: the exploration never gets past its gate.
    let env = Env::new();
    write_catalog(&env);
    tool_env(&env, None);
    let sd = env.create_work(plan_spec("stop me")).await;
    let path = sd.path().to_path_buf();
    let tl = Arc::new(Timeline::default());
    let llm = ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => explore_child(req, &tl),
        Role::Do => panic!("no serial work"),
        Role::Plan => {
            if list_of(&path).todos.is_empty() {
                actions(&[explore_cmd("inventory check", "MARK-A")], Some("T01"))
            } else {
                stage("waiting for T01")
            }
        }
    });
    let stop = StopSignal::default();
    let h = host(&env, gate(&llm, 99));
    let stopper = {
        let stop = stop.clone();
        let path = sd.path().to_path_buf();
        tokio::spawn(async move {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                let st = SessionDir::open(&path).unwrap().state().unwrap();
                if st.waiting_for.as_ref().is_some_and(|w| w.kind == WaitingKind::Children) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            stop.request();
        })
    };
    let out = Box::pin(run_session(&h, &sd, StopWhen::Finished, stop, false)).await;
    stopper.await.unwrap();
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Stopped), "{:?}", out.result);
    assert!(st.open_turn.is_none(), "the waiting Turn is closed");
    let child = list_of(sd.path()).get("T01").unwrap().latest().unwrap().child_sid.clone().unwrap();
    let agent = env.agent();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let e = agent.sessions().lookup(&child).await.unwrap().unwrap();
        if e.status.run_state == RunState::Finished {
            assert_eq!(e.status.outcome, Some(Outcome::Stopped));
            break;
        }
        assert!(Instant::now() < deadline, "the exploration was not stopped: {:?}", e.status);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    libopendan::todos::sync(agent.as_ref(), &sd, &st).await.unwrap();
    assert_eq!(list_of(sd.path()).get("T01").unwrap().status, TodoStatus::Stopped);
}

// ---------------------------------------------------------------------------
// V09: crash windows of the runner (a child process drives and dies)
// ---------------------------------------------------------------------------

const ENV_ROOT: &str = "PLAN_DO_TEST_ROOT";
const ENV_SESSION: &str = "PLAN_DO_TEST_SESSION";
const ENV_SCENARIO: &str = "PLAN_DO_TEST_SCENARIO";

/// One serial Todo, executed by DO and reviewed (content based: the crashed
/// process and the recovering one agree).
fn serial_script(path: PathBuf) -> Arc<ScriptedLlm> {
    ScriptedLlm::new(move |req, _| match role(req) {
        Role::Do => {
            let u = render(&req.messages);
            if u.contains("[result") || u.contains("done-S") {
                end("T01: serial part done", &[])
            } else {
                actions(&["echo done-S >> s.txt".into()], None)
            }
        }
        Role::Explore => panic!("no explorations"),
        Role::Plan => {
            let list = list_of(&path);
            match list.get("T01") {
                None => actions(&["agent-session todo add \"serial\" --content \"the serial part MARK-S\"".into()], Some("T01")),
                Some(t) if t.status == TodoStatus::Pending => handover("T01 next", "do"),
                Some(t) if t.review_state() == ReviewState::Pending => actions(&review_cmds("T01"), Some("reviewing")),
                Some(_) => end("serial done", &[]),
            }
        }
    })
}

/// Two explorations then the end (the parent waits in between).
fn explore_script(path: PathBuf) -> Arc<ScriptedLlm> {
    let tl = Arc::new(Timeline::default());
    ScriptedLlm::new(move |req, _| match role(req) {
        Role::Explore => explore_child(req, &tl),
        Role::Do => panic!("no serial work"),
        Role::Plan => {
            let list = list_of(&path);
            if list.todos.is_empty() {
                return actions(
                    &[explore_cmd("inventory check", "MARK-A"), explore_cmd("idempotency check", "MARK-B")],
                    Some("T01 and T02"),
                );
            }
            if let Some(t) = list.todos.iter().find(|t| t.review_state() == ReviewState::Pending) {
                return actions(&review_cmds(&t.todo_id), Some("reviewing"));
            }
            if !list.explorations_in_flight().is_empty() {
                return stage("waiting");
            }
            end("explored", &[])
        }
    })
}

/// The driving child process of the crash tests (does nothing in a normal
/// test run).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_child() {
    let Ok(root) = std::env::var(ENV_ROOT) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    std::env::set_var("LIBOPENDAN_WHO", APP);
    std::env::set_var("LIBOPENDAN_QUEUE_DIR", &env.queue_dir);
    let sid = std::env::var(ENV_SESSION).unwrap();
    let e = env.agent().sessions().lookup(&sid).await.unwrap().unwrap();
    let sd = SessionDir::open(&e.location).unwrap();
    let path = sd.path().to_path_buf();
    let (llm, barrier) = match std::env::var(ENV_SCENARIO).unwrap().as_str() {
        "serial" => (serial_script(path), 1),
        // The explorations hold at their gate until the process is killed.
        _ => (explore_script(path), 99),
    };
    let out = drive_to_end(&env, gate(&llm, barrier), &sd).await;
    panic!("the crash child was expected to die, it ended with {:?}", out.result);
}

fn spawn_child(env: &Env, sd: &SessionDir, scenario: &str, fault: Option<&str>) -> std::process::Child {
    let mut c = std::process::Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env(ENV_ROOT, &env.root)
        .env(ENV_SESSION, sd.sid())
        .env(ENV_SCENARIO, scenario)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match fault {
        Some(f) => c.env("LIBOPENDAN_FAULT", f),
        None => c.env_remove("LIBOPENDAN_FAULT"),
    };
    c.spawn().unwrap()
}

/// A crash right after a serial Todo was bound to the hand-over (before the
/// suspension committed), or right after the returning result was written
/// back (before the return committed): recovery redoes the same transfer —
/// the same attempt, no second one, DO not executed twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serial_bind_and_writeback_survive_a_crash() {
    for fault in ["todo:after_serial_bind", "todo:after_serial_writeback"] {
        let env = Env::new();
        write_catalog(&env);
        let sd = env.create_work(plan_spec("serial crash")).await;
        let status = spawn_child(&env, &sd, "serial", Some(fault)).wait().unwrap();
        assert!(!status.success(), "{fault}: the child must die");
        let list = list_of(sd.path());
        let t = list.get("T01").unwrap();
        assert_eq!(t.attempts.len(), 1, "{fault}");
        if fault == "todo:after_serial_writeback" {
            assert_eq!(t.latest().unwrap().status, TodoStatus::Completed, "{fault}: written back before the crash");
        } else {
            assert_eq!(t.latest().unwrap().status, TodoStatus::Running, "{fault}");
        }
        tool_env(&env, None);
        let llm = serial_script(sd.path().to_path_buf());
        let out = drive_to_end(&env, gate(&llm, 1), &sd).await;
        finished(&out, &sd, &llm);
        let list = list_of(sd.path());
        let t = list.get("T01").unwrap();
        assert_eq!(t.attempts.len(), 1, "{fault}: no second attempt");
        assert_eq!((t.status, t.review_state()), (TodoStatus::Completed, ReviewState::Accepted), "{fault}");
        let do_calls = llm.requests.lock().unwrap().iter().filter(|r| role(r) == Role::Do).count();
        if fault == "todo:after_serial_writeback" {
            assert_eq!(do_calls, 0, "{fault}: DO is not executed again");
        }
        let lines = std::fs::read_to_string(sd.path().join("s.txt")).unwrap();
        assert_eq!(lines.lines().count(), 1, "{fault}: DO's action ran once");
    }
}

/// The host dies while the parent waits for its explorations (and they are
/// mid-run): a new host takes the same sub sessions on, their ends wake the
/// same open Turn, nothing is dispatched twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_killed_while_waiting_resumes_the_same_explorations() {
    let env = Env::new();
    write_catalog(&env);
    let sd = env.create_work(plan_spec("explore and be killed")).await;
    let mut child = spawn_child(&env, &sd, "explore", None);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let st = sd.state().unwrap();
        let both_started = list_of(sd.path()).explorations_in_flight().len() == 2;
        if both_started && st.waiting_for.as_ref().is_some_and(|w| w.kind == WaitingKind::Children) {
            break;
        }
        assert!(Instant::now() < deadline, "the parent never waited: {st:?}");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let before: Vec<String> = list_of(sd.path())
        .todos
        .iter()
        .map(|t| t.latest().unwrap().child_sid.clone().unwrap())
        .collect();
    tool_env(&env, None);
    let llm = explore_script(sd.path().to_path_buf());
    let out = drive_to_end(&env, gate(&llm, 2), &sd).await;
    finished(&out, &sd, &llm);
    let list = list_of(sd.path());
    let after: Vec<String> = list.todos.iter().map(|t| t.latest().unwrap().child_sid.clone().unwrap()).collect();
    assert_eq!(before, after, "the same sub sessions");
    for t in &list.todos {
        assert_eq!(t.attempts.len(), 1);
        assert_eq!(t.reviews.len(), 1, "reviewed once");
        assert_eq!((t.status, t.review_state()), (TodoStatus::Completed, ReviewState::Accepted));
    }
    let agent = env.agent();
    assert_eq!(agent.sessions().children_of(&[sd.sid().to_string()]).await.unwrap().len(), 2);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1), "the same Turn");
}

/// A context that only reports while nothing runs keeps its first reports
/// (the old behavior), and the third in a row is refused instead of
/// re-inferring without end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_progress_reports_are_bounded() {
    let env = Env::new();
    write_catalog(&env);
    tool_env(&env, None);
    let sd = env.create_work(plan_spec("idle reports")).await;
    let llm = ScriptedLlm::new(move |req, n| match (role(req), n) {
        (Role::Plan, 0..=2) => stage(&format!("thinking {n}")),
        (Role::Plan, _) => {
            assert!(render(&req.messages).contains("another reply with only a progress report"));
            end("done thinking", &[])
        }
        _ => panic!("no sub work"),
    });
    let out = drive_to_end(&env, gate(&llm, 1), &sd).await;
    finished(&out, &sd, &llm);
    assert_eq!(llm.count(), 4);
    let fixes = corrections(&sd);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    assert!(fixes[0].contains("thinking 2"), "{fixes:?}");
}
