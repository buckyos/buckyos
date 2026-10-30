//! Goal Sessions (notepads/opendan-goal-session-todo.md, phase 1): Goals and
//! their Goal Sessions hosted by a Supervisor with a hand-moved clock and a
//! scripted LLM. Acceptance ids (V01…) refer to §5 of the TODO.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use libopendan::api::{create_sub_session, SubSessionSpec};
use libopendan::host::goals::{GoalHost, GoalHostOptions, ManualClock};
use libopendan::host::{HostDeps, Supervisor};
use libopendan::protocol::*;
use libopendan::runner::DefaultAssembler;
use libopendan::state::{
    ActivationQuery, AgentStateClient, GoalCaller, GoalCreate, GoalReopen, GoalRestart,
    GoalRevise, GoalStatusChange,
};
use libopendan::{FsAgentStateClient, OpenDanError, SessionDir};
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

const SLOT: u64 = 300;
/// A statistics window no test run crosses.
const FOREVER: u64 = 1_000_000_000;

struct Hosted {
    sup: Arc<Supervisor>,
    clock: Arc<ManualClock>,
    agent: Arc<FsAgentStateClient>,
}

fn host_deps(env: &Env, llm: Arc<ScriptedLlm>) -> HostDeps {
    let deps = env.deps(llm);
    HostDeps {
        who: APP.into(),
        agent: deps.agent.clone(),
        inputs: deps.inputs.clone(),
        waker: deps.waker.clone(),
        xllm: deps.xllm.clone(),
        assembler: Arc::new(DefaultAssembler::default()),
        session_cli: None,
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        options: deps.options.clone(),
        max_child_concurrency: 4,
        bridges: Vec::new(),
        outbound: None,
        objects: None,
        turn_tasks: None,
        notify: None,
        share: None,
        runtime_id: None,
        notifier: None,
    }
}

fn hosted(env: &Env, llm: Arc<ScriptedLlm>) -> Hosted {
    hosted_with(env, llm, json!({ "tools": { "enabled": true } }))
}

fn hosted_with(env: &Env, llm: Arc<ScriptedLlm>, llm_context: Value) -> Hosted {
    let clock = ManualClock::at(libopendan::now_ms());
    let mut opts = GoalHostOptions::new(env.agent_root.join("sessions"), llm_context);
    opts.clock = clock.clone();
    let sup = Supervisor::new(host_deps(env, llm), None).with_goals(GoalHost::new(opts));
    Hosted {
        sup,
        clock,
        agent: env.agent(),
    }
}

impl Hosted {
    /// One host pass: sessions, then goals.
    async fn tick(&self) {
        self.sup.tick().await.unwrap();
        self.sup.tick_goals().await.unwrap();
    }

    /// Tick until `f` holds (panics after ~10s).
    async fn until(&self, what: &str, f: impl Fn(&Hosted) -> bool) {
        for _ in 0..400 {
            self.tick().await;
            if f(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn goal(&self, id: &str) -> GoalDefinition {
        self.agent.goals().get(id).await.unwrap().unwrap()
    }

    fn goal_now(&self, id: &str) -> GoalDefinition {
        block(self.agent.goals().get(id)).unwrap().unwrap()
    }

    fn activations(&self, id: &str) -> Vec<GoalActivation> {
        block(self.agent.goals().activations(id, &ActivationQuery::default())).unwrap()
    }

    fn state(&self, id: &str) -> Option<GoalState> {
        block(self.agent.goals().state(id)).unwrap()
    }

    fn session(&self, id: &str) -> SessionDir {
        let g = self.goal_now(id);
        let e = block(self.agent.sessions().lookup(&g.session.sid)).unwrap().unwrap();
        SessionDir::open(&e.location).unwrap()
    }

    async fn shutdown(&self) {
        self.sup.shutdown().await;
    }
}

/// Run a future to completion from synchronous code (a closure of a test).
fn block<T: Send + 'static>(f: impl std::future::Future<Output = T> + Send) -> T {
    std::thread::scope(|s| {
        s.spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(f)
        })
        .join()
        .unwrap()
    })
}

fn owner() -> GoalCaller {
    GoalCaller::owner(APP)
}

fn goal_req(objective: &str) -> GoalCreate {
    GoalCreate {
        objective: objective.into(),
        distance_evaluation: "count open items".into(),
        ..Default::default()
    }
}

fn is_goal_req(req: &LlmInferenceRequest) -> bool {
    render(&req.messages).contains("(goal session)")
}

fn report(result: Value) -> buckyos_api::AiResponse {
    tool_call(
        "r1",
        "report",
        json!({ "report": "looked; nothing to do", "is_end": true, "result": result }),
    )
}

fn no_action() -> buckyos_api::AiResponse {
    report(json!({ "decision": "no_action", "distance": "nothing open" }))
}

/// The goal header of the newest goal-session request.
fn last_header(llm: &ScriptedLlm) -> String {
    let reqs = llm.requests.lock().unwrap();
    reqs.iter()
        .rev()
        .filter(|r| is_goal_req(r))
        .flat_map(user_texts)
        .find(|t| t.starts_with("<goal_activation"))
        .unwrap_or_default()
}

/// V01 / V02: the host creates the Goal Session from the Goal (once, with a
/// derived id and no parent); the same key creates no second Goal.
#[tokio::test]
async fn the_host_creates_one_goal_session_per_goal_and_epoch() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|_, _| no_action());
    let h = hosted(&env, llm);
    let mut req = goal_req("keep TODO.md short");
    req.key = Some("k1".into());
    let g1 = h.agent.goals().create(&req, &owner()).await.unwrap();
    let g2 = h.agent.goals().create(&req, &owner()).await.unwrap();
    assert_eq!(g1.goal_id, g2.goal_id);
    assert_eq!(g1.session.sid, g2.session.sid);
    assert_eq!(h.agent.goals().list().await.unwrap().len(), 1);
    // Written, not hosted yet: no session.
    assert!(h.agent.sessions().lookup(&g1.session.sid).await.unwrap().is_none());
    assert_eq!(
        g1.session.sid,
        h.agent
            .sessions()
            .reserve_id(
                SessionKind::Goal,
                &libopendan::ids::goal_session_key(AGENT, &g1.goal_id, 1),
                None,
            )
            .await
            .unwrap()
    );
    let (date, number) = g1.session.sid.strip_prefix("goal-").unwrap().split_once('-').unwrap();
    assert_eq!(date.len(), 8);
    assert!(chrono::NaiveDate::parse_from_str(date, "%Y%m%d").is_ok());
    assert!(number.parse::<u64>().unwrap() > 0);
    h.tick().await;
    h.tick().await;
    let goal_sessions: Vec<RegistryEntry> = h
        .agent
        .sessions()
        .query(&RegistryQuery {
            kind: Some(SessionKind::Goal),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(goal_sessions.len(), 1);
    let e = &goal_sessions[0];
    assert_eq!(e.session_id, g1.session.sid);
    assert_eq!(Path::new(&e.location).file_name().unwrap().to_str(), Some(e.session_id.as_str()));
    assert_eq!(e.goal_id.as_deref(), Some(g1.goal_id.as_str()));
    assert!(e.origin.is_none(), "a goal session has no parent");
    let cfg = SessionDir::open(&e.location).unwrap().config().unwrap();
    assert_eq!(
        cfg.session.goal,
        Some(GoalBinding {
            goal_id: g1.goal_id.clone(),
            epoch: 1
        })
    );
    // The definition is not copied into the session.
    assert!(!cfg.session.objective.contains("TODO.md"));
    // Without start_on_create nothing runs.
    assert!(h.activations(&g1.goal_id).is_empty());
    // A goal session is never created like an ordinary one.
    let agent = env.agent();
    let mut spec = libopendan::SessionTemplate::load("goal", None).unwrap().spec("x");
    spec.prompt.llm_context = json!({});
    let err = libopendan::create_session(
        &env.app_dir,
        spec,
        agent.as_ref(),
        APP,
        env.channels().as_ref(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, OpenDanError::InvalidArgument(_)), "{err}");
    h.shutdown().await;
}

/// V03 / V05: a scheduled slot gives one Activation with the full Goal
/// header; `no_action` keeps the Goal active and the session waiting for
/// its next run opportunity.
#[tokio::test]
async fn a_scheduled_slot_runs_one_activation_and_no_action_is_valid() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|req, _| {
        assert!(is_goal_req(req));
        no_action()
    });
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("keep TODO.md short");
    req.title = Some("todo keeper".into());
    req.schedule.every_secs = Some(SLOT);
    req.initial_skills = vec![GoalSkill {
        name: "triage".into(),
        reference: Some("skills/triage.md".into()),
        version: None,
        note: None,
    }];
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.tick().await;
    h.tick().await;
    assert_eq!(llm.count(), 0, "nothing before the first slot");
    h.clock.advance(SLOT * 1000);
    h.until("first activation", |h| h.activations(&g.goal_id).len() == 1)
        .await;
    let header = last_header(&llm);
    for needle in [
        "<goal_activation",
        "kind=\"scheduled\"",
        "<title>todo keeper</title>",
        "<objective>keep TODO.md short</objective>",
        "<skill name=\"triage\" ref=\"skills/triage.md\"",
        "<budget ",
        "<result_format>",
        "none: this is the first activation",
    ] {
        assert!(header.contains(needle), "{needle} missing in\n{header}");
    }
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!((a.epoch, a.seq), (1, 1));
    assert_eq!(a.outcome, ActivationOutcome::Completed);
    assert_eq!(a.decision, ActivationDecision::NoAction);
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Scheduled);
    assert!(a.cost.total_tokens > 0);
    let st = h.state(&g.goal_id).unwrap();
    assert_eq!(st.distance.as_ref().unwrap().text, "nothing open");
    assert_eq!(st.last_activation.as_ref().unwrap().decision, ActivationDecision::NoAction);
    assert_eq!(st.usage.activations, 1);
    // Waiting for the next run opportunity, not for a person.
    let sd = h.session(&g.goal_id);
    h.until("waiting", |h| {
        let s = h.session(&g.goal_id).state().unwrap();
        s.run_state == RunState::Waiting
            && s.waiting_for.as_ref().map(|w| w.kind) == Some(WaitingKind::Event)
            && s.goal.as_ref().is_some_and(|g| !g.needs_flush())
    })
    .await;
    assert_eq!(h.goal(&g.goal_id).await.status, GoalStatus::Active);
    // The next slot: the second Activation sees the first one's state.
    h.clock.advance(SLOT * 1000);
    h.until("second activation", |h| h.activations(&g.goal_id).len() == 2)
        .await;
    let header = last_header(&llm);
    assert!(header.contains(" activation=\"2\" revision="), "{header}");
    assert!(header.contains("<distance"), "{header}");
    assert!(header.contains("decision=\"no_action\""), "{header}");
    assert_eq!(sd.state().unwrap().turn_seq, 2);
    h.shutdown().await;
}

/// V04: slots missed while nothing hosted the Goal fold into one fire.
#[tokio::test]
async fn missed_slots_fold_into_one_activation() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|_, _| no_action());
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("watch the queue");
    req.schedule.every_secs = Some(SLOT);
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.tick().await;
    // Down for three slots.
    h.clock.advance(3 * SLOT * 1000);
    h.until("one activation", |h| h.activations(&g.goal_id).len() == 1)
        .await;
    for _ in 0..10 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let acts = h.activations(&g.goal_id);
    assert_eq!(acts.len(), 1, "{acts:?}");
    assert_eq!(acts[0].reasons.len(), 1);
    assert_eq!(acts[0].reasons[0].missed, Some(3));
    assert!(last_header(&llm).contains("missed_slots=\"3\""));
    h.shutdown().await;
}

/// V06: an Activation out of budget ends; the session does not, and the
/// next run opportunity has a budget of its own.
#[tokio::test]
async fn an_activation_out_of_budget_ends_alone() {
    let env = Env::new();
    // Never reports: every round is a shell call (15 tokens each).
    let llm = ScriptedLlm::new(|req, n| {
        if act(req) == 2 {
            return no_action();
        }
        tool_call(&format!("s{n}"), "shell", json!({ "command": "true" }))
    });
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("busy goal");
    req.budget.activation.max_total_tokens = Some(40);
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("first activation", |h| h.activations(&g.goal_id).len() == 1)
        .await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.outcome, ActivationOutcome::BudgetExhausted);
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Created);
    let sd = h.session(&g.goal_id);
    let st = sd.state().unwrap();
    assert!(!st.is_finished());
    assert!(st.last_error.is_none(), "{:?}", st.last_error);
    // A manual run opportunity runs normally.
    h.sup.goal_activate(&g.goal_id, "r-1", "look again").await.unwrap();
    h.until("second activation", |h| h.activations(&g.goal_id).len() == 2)
        .await;
    let a = &h.activations(&g.goal_id)[1];
    assert_eq!(a.outcome, ActivationOutcome::Completed);
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Manual);
    h.shutdown().await;
}

/// The Goal result is checked when it is reported: a wrong one is refused
/// to the model, which can correct it.
#[tokio::test]
async fn a_wrong_goal_result_is_refused_to_the_model() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|req, _| {
        match has_tool_result(req, "r1") {
            Some(r) => {
                assert!(r.contains("decision"), "{r}");
                tool_call(
                    "r2",
                    "report",
                    json!({ "report": "fixed", "is_end": true, "result": { "decision": "acted", "pending": ["write summary"] } }),
                )
            }
            None => report(json!({ "decision": "done" })),
        }
    });
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("result check");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1)
        .await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.decision, ActivationDecision::Acted);
    assert_eq!(h.state(&g.goal_id).unwrap().pending, vec!["write summary".to_string()]);
    h.shutdown().await;
}

/// Create a Work of the goal session `parent` from inside an LLM script.
fn create_work(root: &Path, parent: &str, objective: &str, key: &str) -> Result<String, OpenDanError> {
    let env = Env::at(root);
    block(async move {
        let agent = env.agent();
        let ch = env.channels();
        create_sub_session(
            agent.as_ref(),
            APP,
            parent,
            SubSessionSpec {
                objective: objective.into(),
                msgs: vec![format!("please {objective}")],
                key: Some(key.into()),
                ..Default::default()
            },
            ch.as_ref(),
        )
        .await
        .map(|sd| sd.sid().to_string())
    })
}

fn session_id_of(req: &LlmInferenceRequest) -> String {
    let all = render(&req.messages);
    let start = all.find("working in session `").unwrap() + "working in session `".len();
    all[start..].split('`').next().unwrap().to_string()
}

/// V09 / V10: an Activation creates Work and ends while it runs; the Work's
/// end is a later Activation's input, absorbed once, with its tokens; a
/// revision made meanwhile is flagged in the next header.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn work_created_by_an_activation_is_absorbed_later() {
    let env = Env::new();
    let root = env.root.clone();
    let release = env.root.join("release");
    let rel = release.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        if !is_goal_req(req) {
            return work_reply(req, &rel);
        }
        if act(req) == 1 && has_tool_result(req, "r1").is_none() {
            let sid = session_id_of(req);
            create_work(&root, &sid, "check item 1", "w1").unwrap();
            create_work(&root, &sid, "check item 2", "w2").unwrap();
            return report(json!({ "decision": "created_work", "pending": ["items 1 and 2 in progress"] }));
        }
        report(json!({ "decision": "no_action" }))
    });
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("finish the TODO list");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("first activation", |h| h.activations(&g.goal_id).len() == 1)
        .await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.decision, ActivationDecision::CreatedWork);
    assert_eq!(a.works_created.len(), 2, "{a:?}");
    let works: Vec<RegistryEntry> = h
        .agent
        .sessions()
        .query(&RegistryQuery {
            goal_id: Some(g.goal_id.clone()),
            kind: Some(SessionKind::Work),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(works.len(), 2);
    for w in &works {
        let o = w.origin.as_ref().unwrap().goal.as_ref().unwrap();
        assert_eq!((o.goal_id.as_str(), o.revision, o.epoch, o.activation_seq), (g.goal_id.as_str(), 1, 1, 1));
        assert_ne!(w.status.run_state, RunState::Finished);
    }
    // Revised while the Work runs.
    let g2 = h
        .agent
        .goals()
        .revise(
            &g.goal_id,
            &GoalRevise {
                expected_revision: 1,
                objective: Some("finish the TODO list, oldest first".into()),
                ..Default::default()
            },
            &owner(),
        )
        .await
        .unwrap();
    assert_eq!(g2.revision, 2);
    std::fs::write(&release, "go").unwrap();
    h.until("works absorbed", |h| {
        h.state(&g.goal_id)
            .is_some_and(|s| s.absorbed.len() == 2 && s.active_works.is_empty())
            && h.activations(&g.goal_id)
                .iter()
                .map(|a| a.works_absorbed.len())
                .sum::<usize>()
                >= 2
    })
    .await;
    let acts = h.activations(&g.goal_id);
    let absorbed: Vec<&String> = acts.iter().flat_map(|a| &a.works_absorbed).collect();
    assert_eq!(absorbed.len(), 2, "each work absorbed once: {acts:?}");
    assert!(acts[1..]
        .iter()
        .all(|a| a.reasons.iter().all(|r| r.kind == ActivationReasonKind::Child)));
    // The first Activation after the revision is told what changed; the
    // ones after it are not told again.
    let headers: Vec<String> = llm
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| is_goal_req(r))
        .map(header_of)
        .filter(|h| h.lines().next().unwrap_or_default().contains(" revision=\"2\""))
        .collect();
    assert!(headers[0].contains("changed_since_last_activation=\"objective\""), "{}", headers[0]);
    assert!(headers[0].contains("goal_revision=\"1\" outdated=\"true\""), "{}", headers[0]);
    let last = headers.last().unwrap();
    if !std::ptr::eq(last, &headers[0]) && act_of(last) != act_of(&headers[0]) {
        assert!(!last.contains("changed_since_last_activation"), "{last}");
    }
    let st = h.state(&g.goal_id).unwrap();
    assert!(st.usage.work_tokens > 0);
    assert_eq!(st.recent_works.len(), 2);
    // The Work's result names its Goal.
    let w = SessionDir::open(&works[0].location).unwrap();
    assert_eq!(w.state().unwrap().result.unwrap()["goal"]["goal_id"], g.goal_id);
    assert!(w.report().unwrap().contains(&g.goal_id));
    h.shutdown().await;
}

/// A Work that runs until the test lets it (a shell command waiting for a
/// file), then reports.
fn work_reply(req: &LlmInferenceRequest, release: &Path) -> buckyos_api::AiResponse {
    match has_tool_result(req, "w") {
        Some(_) => text("work done: item checked"),
        None => tool_call(
            "w",
            "shell",
            json!({ "command": format!("while [ ! -f {} ]; do sleep 0.05; done; echo released", release.display()) }),
        ),
    }
}

/// The Activation number of the request's Goal header (0: none).
fn act(req: &LlmInferenceRequest) -> u64 {
    act_of(&header_of(req))
}

fn act_of(h: &str) -> u64 {
    let root = h.lines().next().unwrap_or_default();
    root.split(" activation=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn header_of(req: &LlmInferenceRequest) -> String {
    user_texts(req)
        .into_iter()
        .find(|t| t.starts_with("<goal_activation"))
        .unwrap_or_default()
}

fn works_of(h: &Hosted, goal_id: &str) -> Vec<RegistryEntry> {
    block(h.agent.sessions().query(&RegistryQuery {
        goal_id: Some(goal_id.to_string()),
        kind: Some(SessionKind::Work),
        ..Default::default()
    }))
    .unwrap()
}

/// V07: at the period limit scheduled fires are skipped (and recorded) and
/// no Work is created; the end of a Work created before is still absorbed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_period_limit_skips_fires_and_refuses_work_but_absorbs_results() {
    let env = Env::new();
    let root = env.root.clone();
    let release = env.root.join("release");
    let rel = release.clone();
    let refused = Arc::new(std::sync::Mutex::new(None::<String>));
    let seen = refused.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        if !is_goal_req(req) {
            return work_reply(req, &rel);
        }
        let header = header_of(req);
        if act(req) == 1 && has_tool_result(req, "r1").is_none() {
            create_work(&root, &session_id_of(req), "check item 1", "w1").unwrap();
            return report(json!({ "decision": "created_work" }));
        }
        if act(req) == 2 && has_tool_result(req, "r1").is_none() {
            let err = create_work(&root, &session_id_of(req), "check item 2", "w2").unwrap_err();
            *seen.lock().unwrap() = Some(err.to_json()["kind"].as_str().unwrap().to_string());
        }
        report(json!({ "decision": "no_action" }))
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("bounded goal");
    req.schedule.every_secs = Some(SLOT);
    req.schedule.min_interval_secs = Some(0);
    req.budget.period.window_secs = Some(FOREVER);
    req.budget.period.max_activations = Some(2);
    req.budget.period.max_works = Some(1);
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.tick().await;
    h.clock.advance(SLOT * 1000);
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    h.clock.advance(SLOT * 1000);
    h.until("activation 2", |h| h.activations(&g.goal_id).len() == 2).await;
    assert_eq!(refused.lock().unwrap().as_deref(), Some("goal_budget_exhausted"));
    // Over the limit: the next slot is skipped and recorded.
    h.clock.advance(SLOT * 1000);
    h.until("skip recorded", |h| {
        block(h.agent.goals().schedule(&g.goal_id))
            .unwrap()
            .is_some_and(|s| s.skipped.iter().any(|k| k.reason == "period_budget"))
    })
    .await;
    assert_eq!(h.activations(&g.goal_id).len(), 2);
    assert_eq!(works_of(&h, &g.goal_id).len(), 1);
    // The Work's end still comes in.
    std::fs::write(&release, "go").unwrap();
    h.until("work absorbed", |h| h.state(&g.goal_id).is_some_and(|s| s.absorbed.len() == 1))
        .await;
    let last = h.activations(&g.goal_id).pop().unwrap();
    assert_eq!(last.reasons[0].kind, ActivationReasonKind::Child);
    h.shutdown().await;
}

/// V08: a question nobody answers ends the Activation (needs input); it is
/// recorded in the Goal State, the session goes on.
#[tokio::test]
async fn a_question_ends_the_activation_and_is_recorded() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|_, _| {
        text("<response><report><![CDATA[Which repository should I watch?]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
    });
    let h = hosted_with(
        &env,
        llm,
        json!({ "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true } }),
    );
    let mut req = goal_req("watch a repository");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.outcome, ActivationOutcome::NeedsInput);
    assert_eq!(a.decision, ActivationDecision::NeedsInfo);
    let st = h.state(&g.goal_id).unwrap();
    assert_eq!(st.questions.len(), 1);
    assert!(st.questions[0].text.contains("Which repository"));
    let s = h.session(&g.goal_id).state().unwrap();
    assert!(!s.is_finished());
    assert!(s.open_turn.is_none());
    h.shutdown().await;
}

/// V11: a paused Goal opens no Activation (fires skipped, a Work's end
/// waits); after resume the waiting inputs make one Activation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_goal_holds_its_inputs_until_resumed() {
    let env = Env::new();
    let root = env.root.clone();
    let release = env.root.join("release");
    let rel = release.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        if !is_goal_req(req) {
            return work_reply(req, &rel);
        }
        if act(req) == 1 && has_tool_result(req, "r1").is_none() {
            create_work(&root, &session_id_of(req), "check item 1", "w1").unwrap();
            return report(json!({ "decision": "created_work" }));
        }
        no_action()
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("pausable");
    req.schedule.start_on_create = true;
    req.schedule.every_secs = Some(SLOT);
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    h.agent
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Paused,
                reason: Some("holiday".into()),
                evidence: None,
            },
            &owner(),
        )
        .await
        .unwrap();
    std::fs::write(&release, "go").unwrap();
    h.until("work finished", |h| {
        works_of(h, &g.goal_id)
            .iter()
            .all(|w| w.status.run_state == RunState::Finished)
    })
    .await;
    h.clock.advance(SLOT * 1000);
    for _ in 0..20 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(h.activations(&g.goal_id).len(), 1, "nothing runs while paused");
    let sch = h.agent.goals().schedule(&g.goal_id).await.unwrap().unwrap();
    assert!(sch.skipped.iter().any(|k| k.reason == "paused"), "{sch:?}");
    h.agent
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Active,
                reason: None,
                evidence: None,
            },
            &owner(),
        )
        .await
        .unwrap();
    h.until("activation 2", |h| h.activations(&g.goal_id).len() == 2).await;
    for _ in 0..10 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let acts = h.activations(&g.goal_id);
    assert_eq!(acts.len(), 2);
    assert!(acts[1].reasons.iter().any(|r| r.kind == ActivationReasonKind::Child));
    h.shutdown().await;
}

/// V12: archiving a Goal ends its Goal Session (stopped) and stops its
/// running Work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn archive_stops_the_session_and_its_work() {
    let env = Env::new();
    let root = env.root.clone();
    let release = env.root.join("never");
    let llm = ScriptedLlm::new(move |req, _| {
        if !is_goal_req(req) {
            return work_reply(req, &release);
        }
        if act(req) == 1 && has_tool_result(req, "r1").is_none() {
            create_work(&root, &session_id_of(req), "long work", "w1").unwrap();
            return report(json!({ "decision": "created_work" }));
        }
        no_action()
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("archivable");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    h.until("work running", |h| {
        works_of(h, &g.goal_id)
            .iter()
            .any(|w| w.status.run_state == RunState::Running)
    })
    .await;
    h.agent
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Archived,
                reason: Some("no longer needed".into()),
                evidence: None,
            },
            &owner(),
        )
        .await
        .unwrap();
    h.until("ended", |h| {
        let sd = h.session(&g.goal_id);
        let s = sd.state().unwrap();
        s.outcome == Some(Outcome::Stopped)
            && works_of(h, &g.goal_id)
                .iter()
                .all(|w| w.status.outcome == Some(Outcome::Stopped))
            && h.goal_now(&g.goal_id).epochs[0].ended_at_ms.is_some()
    })
    .await;
    let goal = h.goal(&g.goal_id).await;
    assert_eq!(goal.epochs[0].end, Some(EpochEnd::Archived));
    h.shutdown().await;
}

/// V13: a Goal Session archives its own Goal (recorded as its own change);
/// the owner reopens it: a new epoch keeps the Goal State and starts with a
/// `reopened` Activation.
#[tokio::test]
async fn a_goal_archived_by_itself_is_reopened_in_a_new_epoch() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|req, _| {
        if header_of(req).contains("kind=\"reopened\"") {
            assert!(header_of(req).contains("still relevant"));
            return no_action();
        }
        report(json!({ "decision": "acted", "pending": ["one thing left"] }))
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("self archiving");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    let me = GoalCaller::from_session(APP, &g.session.sid);
    // What a goal session may not do.
    assert!(matches!(
        h.agent
            .goals()
            .set_status(&g.goal_id, &GoalStatusChange { status: GoalStatus::Archived, reason: None, evidence: None }, &me)
            .await,
        Err(OpenDanError::InvalidArgument(_))
    ));
    h.agent
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Archived,
                reason: Some("looks done".into()),
                evidence: None,
            },
            &me,
        )
        .await
        .unwrap();
    let changes = h.agent.goals().changes(&g.goal_id, 0).await.unwrap();
    assert_eq!(changes.last().unwrap().via, format!("goal_session:{}", g.session.sid));
    h.until("epoch 1 ended", |h| h.goal_now(&g.goal_id).epochs[0].ended_at_ms.is_some())
        .await;
    assert_eq!(h.session(&g.goal_id).state().unwrap().outcome, Some(Outcome::Stopped));
    let g2 = h
        .agent
        .goals()
        .reopen(
            &g.goal_id,
            &GoalReopen {
                reason: "still relevant".into(),
                fresh_state: false,
            },
            &owner(),
        )
        .await
        .unwrap();
    assert_eq!((g2.session.epoch, g2.status), (2, GoalStatus::Active));
    assert_ne!(g2.session.sid, g.session.sid);
    h.until("reopened activation", |h| h.activations(&g.goal_id).len() == 2).await;
    let a = &h.activations(&g.goal_id)[1];
    assert_eq!((a.epoch, a.seq), (2, 1));
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Reopened);
    let st = h.state(&g.goal_id).unwrap();
    assert_eq!(st.epoch, 2);
    assert_eq!(st.pending, vec!["one thing left".to_string()], "the state is kept");
    assert!(h.agent.goals().epoch_state(&g.goal_id, 1).await.unwrap().is_some());
    h.shutdown().await;
}

/// V14: a restart ends the epoch's session (superseded) without stopping
/// its Work, archives its Goal State and starts a fresh context; the Work's
/// end is taken in by the new epoch, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_starts_a_fresh_context_that_takes_over_the_work() {
    let env = Env::new();
    let root = env.root.clone();
    let release = env.root.join("release");
    let rel = release.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        if !is_goal_req(req) {
            return work_reply(req, &rel);
        }
        let header = header_of(req);
        if header.contains("kind=\"created\"") && has_tool_result(req, "r1").is_none() {
            create_work(&root, &session_id_of(req), "slow work", "w1").unwrap();
            return report(json!({ "decision": "created_work", "pending": ["context of epoch 1"] }));
        }
        no_action()
    });
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("restartable");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    let g2 = h
        .agent
        .goals()
        .restart(
            &g.goal_id,
            &GoalRestart {
                reason: "fresh start".into(),
                carry_state: false,
            },
            &owner(),
        )
        .await
        .unwrap();
    assert_eq!(g2.session.epoch, 2);
    h.until("restarted activation", |h| {
        h.activations(&g.goal_id)
            .iter()
            .any(|a| a.epoch == 2 && a.reasons.iter().any(|r| r.kind == ActivationReasonKind::Restarted))
    })
    .await;
    // The old session ended as superseded; the Work runs on.
    let old = SessionDir::open(
        &h.agent.sessions().lookup(&g.session.sid).await.unwrap().unwrap().location,
    )
    .unwrap();
    assert_eq!(old.state().unwrap().outcome, Some(Outcome::Stopped));
    let w = &works_of(&h, &g.goal_id)[0];
    assert_ne!(w.status.run_state, RunState::Finished);
    let archived = h.agent.goals().epoch_state(&g.goal_id, 1).await.unwrap().unwrap();
    assert_eq!(archived.pending, vec!["context of epoch 1".to_string()]);
    let st = h.state(&g.goal_id).unwrap();
    assert_eq!(st.epoch, 2);
    assert!(st.pending.is_empty(), "only the facts are kept");
    assert_eq!(st.active_works.len(), 1);
    // A fresh context: nothing of epoch 1 in the new session's request.
    {
        let reqs = llm.requests.lock().unwrap();
        let first_of_2 = reqs
            .iter()
            .find(|r| header_of(r).contains("kind=\"restarted\""))
            .unwrap();
        assert!(!render(&first_of_2.messages).contains("context of epoch 1\"]"));
        assert_eq!(
            user_texts(first_of_2).iter().filter(|t| t.starts_with("<goal_activation")).count(),
            1
        );
    }
    std::fs::write(&release, "go").unwrap();
    h.until("absorbed by epoch 2", |h| h.state(&g.goal_id).is_some_and(|s| s.absorbed.len() == 1))
        .await;
    for _ in 0..10 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let acts = h.activations(&g.goal_id);
    let takers: Vec<(u64, u64)> = acts
        .iter()
        .filter(|a| !a.works_absorbed.is_empty())
        .map(|a| (a.epoch, a.seq))
        .collect();
    assert_eq!(takers.len(), 1, "{acts:?}");
    assert_eq!(takers[0].0, 2);
    let goal = h.goal(&g.goal_id).await;
    assert_eq!(goal.epochs[0].end, Some(EpochEnd::Restarted));
    assert!(goal.epochs[0].ended_at_ms.is_some());
    h.shutdown().await;
}

/// V17: stopping an Activation (a TaskMgr cancel, `session.stop`) closes
/// only its Turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_an_activation_keeps_the_goal_session() {
    let env = Env::new();
    let release = env.root.join("never");
    let llm = ScriptedLlm::new(move |req, _| {
        if act(req) == 2 {
            return no_action();
        }
        match has_tool_result(req, "long") {
            Some(_) => no_action(),
            None => tool_call(
                "long",
                "shell",
                json!({ "command": format!("while [ ! -f {} ]; do sleep 0.05; done", release.display()) }),
            ),
        }
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("stoppable");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("running", |h| {
        block(h.agent.sessions().lookup(&h.goal_now(&g.goal_id).session.sid))
            .unwrap()
            .is_some_and(|e| e.status.run_state == RunState::Running)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.agent
        .sessions()
        .post_input(&g.session.sid, &PostedInput::control(APP, "stop-1", ControlCommand::stop(Some("enough".into()))))
        .await
        .unwrap();
    h.until("activation stopped", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.outcome, ActivationOutcome::Stopped);
    let s = h.session(&g.goal_id).state().unwrap();
    assert!(!s.is_finished());
    assert!(!s.turn_stop_requested);
    h.sup.goal_activate(&g.goal_id, "again", "go on").await.unwrap();
    h.until("activation 2", |h| h.activations(&g.goal_id).len() == 2).await;
    assert_eq!(h.activations(&g.goal_id)[1].outcome, ActivationOutcome::Completed);
    h.shutdown().await;
}

/// V18: later Activations carry earlier Goal headers as one line of history;
/// everything needed comes back in the new header.
#[tokio::test]
async fn earlier_headers_are_one_line_in_history() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|_, _| report(json!({ "decision": "acted", "pending": ["keep going"], "skills": { "add": ["notes"] } })));
    let h = hosted(&env, llm.clone());
    let mut req = goal_req("long lived");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    for i in 2..=3 {
        h.sup.goal_activate(&g.goal_id, &format!("r{i}"), "again").await.unwrap();
        h.until("next activation", |h| h.activations(&g.goal_id).len() == i).await;
    }
    let reqs = llm.requests.lock().unwrap();
    let last = reqs.last().unwrap();
    let all = render(&last.messages);
    assert_eq!(
        user_texts(last).iter().filter(|t| t.contains("<goal_activation goal=")).count(),
        1,
        "{all}"
    );
    assert!(all.contains("goal activation 1 of goal"), "{all}");
    assert!(all.contains("goal activation 2 of goal"), "{all}");
    let header = header_of(last);
    assert!(header.contains("<skill name=\"notes\""), "{header}");
    assert!(header.contains("<item>keep going</item>"), "{header}");
    drop(reqs);
    h.shutdown().await;
}

/// V19: what each kind of caller may do with a Goal.
#[tokio::test]
async fn callers_are_kept_to_their_kind() {
    let env = Env::new();
    let llm = ScriptedLlm::new(|_, _| no_action());
    let h = hosted(&env, llm);
    // A UI session creates and steers; a Work only reads.
    let ui = env.create_work({
        let mut spec = libopendan::SessionTemplate::load("ui", None).unwrap().spec("chat");
        spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
        spec
    })
    .await;
    let work = env.create_work(work_spec("a task")).await;
    let from_ui = GoalCaller::from_session(APP, ui.sid());
    let from_work = GoalCaller::from_session(APP, work.sid());
    let g = h.agent.goals().create(&goal_req("ui goal"), &from_ui).await.unwrap();
    assert_eq!(g.created_by.via, format!("ui_session:{}", ui.sid()));
    assert!(matches!(
        h.agent.goals().create(&goal_req("work goal"), &from_work).await,
        Err(OpenDanError::Forbidden(_))
    ));
    h.tick().await;
    let me = GoalCaller::from_session(APP, &g.session.sid);
    let revise = GoalRevise {
        expected_revision: 1,
        objective: Some("bigger".into()),
        ..Default::default()
    };
    assert!(matches!(h.agent.goals().revise(&g.goal_id, &revise, &me).await, Err(OpenDanError::Forbidden(_))));
    let budget = GoalRevise {
        expected_revision: 1,
        budget: Some(GoalBudget::default()),
        ..Default::default()
    };
    assert!(matches!(h.agent.goals().revise(&g.goal_id, &budget, &me).await, Err(OpenDanError::Forbidden(_))));
    let pause = GoalStatusChange {
        status: GoalStatus::Paused,
        reason: None,
        evidence: None,
    };
    assert!(matches!(h.agent.goals().set_status(&g.goal_id, &pause, &me).await, Err(OpenDanError::Forbidden(_))));
    let achieved = GoalStatusChange {
        status: GoalStatus::Achieved,
        reason: None,
        evidence: Some("done".into()),
    };
    assert!(matches!(
        h.agent.goals().set_status(&g.goal_id, &achieved, &me).await,
        Err(OpenDanError::Forbidden(_))
    ), "a maintain goal is never achieved by itself");
    let restart = GoalRestart {
        reason: "x".into(),
        carry_state: false,
    };
    assert!(matches!(h.agent.goals().restart(&g.goal_id, &restart, &me).await, Err(OpenDanError::Forbidden(_))));
    assert!(matches!(
        h.agent.goals().reopen(&g.goal_id, &GoalReopen { reason: "x".into(), fresh_state: false }, &me).await,
        Err(OpenDanError::Forbidden(_))
    ));
    // The UI that created it revises it; a stale revision is a conflict.
    let g2 = h.agent.goals().revise(&g.goal_id, &revise, &from_ui).await.unwrap();
    assert_eq!(g2.revision, 2);
    assert!(matches!(
        h.agent.goals().revise(&g.goal_id, &revise, &from_ui).await,
        Err(OpenDanError::GoalConflict(_))
    ));
    assert!(h.agent.goals().revision(&g.goal_id, 1).await.unwrap().unwrap().objective == "ui goal");
    // An achieve goal marks itself achieved with evidence, not with open work.
    let mut areq = goal_req("one-off");
    areq.lifecycle = GoalLifecycle::Achieve;
    let a = h.agent.goals().create(&areq, &owner()).await.unwrap();
    h.tick().await;
    let a_me = GoalCaller::from_session(APP, &a.session.sid);
    let done = h.agent.goals().set_status(&a.goal_id, &achieved, &a_me).await.unwrap();
    assert_eq!(done.status, GoalStatus::Achieved);
    h.until("achieved session ends", |h| {
        let sd = h.session(&a.goal_id);
        sd.state().unwrap().outcome == Some(Outcome::Succeeded)
    })
    .await;
    h.shutdown().await;
}

/// The Agent State facade over kRPC: Goals are read and changed through it;
/// what only a driver or the host writes is refused.
#[tokio::test]
async fn goals_over_krpc() {
    use libopendan::state::krpc::{serve_call, KrpcAgentStateClient, StateTransport};
    struct Loop(Arc<FsAgentStateClient>);
    #[async_trait::async_trait]
    impl StateTransport for Loop {
        async fn call(&self, method: &str, params: Value) -> libopendan::Result<Value> {
            serve_call(self.0.as_ref(), APP, method, &params)
                .await
                .unwrap_or_else(|| Err(OpenDanError::InvalidArgument(format!("unknown {method}"))))
                .map_err(|e| libopendan::state::krpc::error_from_wire(&libopendan::state::krpc::error_to_wire(&e)))
        }
    }
    let env = Env::new();
    let remote = KrpcAgentStateClient::connect(AGENT, Arc::new(Loop(env.agent()))).await.unwrap();
    let g = remote.goals().create(&goal_req("remote goal"), &owner()).await.unwrap();
    assert_eq!(remote.goals().list().await.unwrap().len(), 1);
    assert_eq!(remote.goals().get(&g.goal_id).await.unwrap().unwrap().objective, "remote goal");
    let st = GoalState::initial(&g, 1, 1);
    let lease_free = serve_call(env.agent().as_ref(), APP, "goals.write_state", &json!({ "state": st })).await;
    assert!(matches!(lease_free, Some(Err(_))), "{lease_free:?}");
    assert!(remote.goals().state(&g.goal_id).await.unwrap().is_none());
    let err = remote
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Active,
                reason: None,
                evidence: None,
            },
            &owner(),
        )
        .await
        .unwrap();
    assert_eq!(err.status, GoalStatus::Active);
    assert!(matches!(
        remote.goals().reopen(&g.goal_id, &GoalReopen { reason: "x".into(), fresh_state: false }, &owner()).await,
        Err(OpenDanError::GoalConflict(_))
    ));
}

/// The Goal's current skills change from inside an Activation (a control
/// the driver applies), and a UI's feedback is a run opportunity of its own.
#[tokio::test]
async fn skills_control_and_feedback() {
    let env = Env::new();
    let qd = env.queue_dir.clone();
    let root = env.root.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        let header = header_of(req);
        if act(req) == 1 && has_tool_result(req, "r1").is_none() {
            let sid = session_id_of(req);
            let queue = {
                let env = Env::at(&root);
                let e = block(async move { env.agent().sessions().lookup(&sid).await })
                    .unwrap()
                    .unwrap();
                queue_of(&SessionDir::open(&e.location).unwrap())
            };
            post_blocking(
                &qd,
                &queue,
                PostedInput::control(
                    APP,
                    "skills-1",
                    ControlCommand::GoalSkills {
                        add: vec![GoalSkill {
                            name: "triage".into(),
                            reference: Some("skills/triage.md".into()),
                            version: None,
                            note: None,
                        }],
                        remove: Vec::new(),
                    },
                ),
            );
            return tool_call("s1", "shell", json!({ "command": "true" }));
        }
        if header.contains("kind=\"feedback\"") {
            assert!(render(&req.messages).contains("please also check the wiki"));
        }
        no_action()
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("skillful");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    h.until("skill", |h| h.state(&g.goal_id).is_some_and(|s| s.current_skills.iter().any(|k| k.name == "triage")))
        .await;
    h.sup
        .goal_feedback(&g.goal_id, "please also check the wiki", "ui_session:ui-1")
        .await
        .unwrap();
    h.until("activation 2", |h| h.activations(&g.goal_id).len() == 2).await;
    let a = &h.activations(&g.goal_id)[1];
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Feedback);
    assert!(a.reasons[0].note.as_deref().unwrap().contains("ui_session:ui-1"));
    h.shutdown().await;
}

// ---- crash windows: a child process hosts the goal and dies ----

const ENV_ROOT: &str = "LIBOPENDAN_GOAL_TEST_ROOT";
const ENV_SCENARIO: &str = "LIBOPENDAN_GOAL_TEST_SCENARIO";

/// Content based scripts the child and the parent share.
fn crash_script(scenario: &str, root: &Path) -> Arc<ScriptedLlm> {
    let marker = root.join("marker");
    match scenario {
        // The Activation runs a long tool call (the parent kills the child
        // meanwhile), then reports.
        "slow" => ScriptedLlm::new(move |req, _| match has_tool_result(req, "long") {
            Some(_) => no_action(),
            None if act(req) > 1 => no_action(),
            None => tool_call(
                "long",
                "shell",
                json!({ "command": format!("touch {}; sleep 30", marker.display()) }),
            ),
        }),
        // Recovery must not infer again.
        "silent" => ScriptedLlm::new(|req, _| panic!("no inference expected: {}", render(&req.messages))),
        _ => ScriptedLlm::new(|_, _| no_action()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn child_goal_host() {
    let (Ok(root), Ok(scenario)) = (std::env::var(ENV_ROOT), std::env::var(ENV_SCENARIO)) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    let h = hosted(&env, crash_script(&scenario, Path::new(&root)));
    let started = std::time::Instant::now();
    let mut advanced = false;
    while started.elapsed() < Duration::from_secs(60) {
        h.tick().await;
        if scenario == "fire" && !advanced {
            h.clock.advance(SLOT * 1000);
            advanced = true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn spawn_host(env: &Env, scenario: &str, fault: Option<&str>) -> std::process::Child {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "child_goal_host", "--nocapture", "--test-threads=1"])
        .env(ENV_ROOT, &env.root)
        .env(ENV_SCENARIO, scenario)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match fault {
        Some(f) => cmd.env("LIBOPENDAN_FAULT", f),
        None => cmd.env_remove("LIBOPENDAN_FAULT"),
    };
    cmd.spawn().unwrap()
}

fn wait_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
    let start = std::time::Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("child did not exit in time");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn goal_sessions(h: &Hosted) -> usize {
    block(h.agent.sessions().query(&RegistryQuery {
        kind: Some(SessionKind::Goal),
        ..Default::default()
    }))
    .unwrap()
    .len()
}

/// V02: the host dies after the Goal is written and its epoch prepared, before
/// the Goal Session exists: the next host creates it, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_dying_before_the_session_exists_is_caught_up() {
    let env = Env::new();
    let agent = env.agent();
    let mut req = goal_req("crash before create");
    req.schedule.start_on_create = true;
    let g = agent.goals().create(&req, &owner()).await.unwrap();
    let mut child = spawn_host(&env, "quick", Some("goal:before_session_create"));
    assert!(!wait_exit(&mut child).success());
    assert!(agent.sessions().lookup(&g.session.sid).await.unwrap().is_none());
    let h = hosted(&env, crash_script("quick", &env.root));
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1).await;
    assert_eq!(goal_sessions(&h), 1);
    h.shutdown().await;
}

/// V15: an Activation killed in the middle goes on in the same (epoch, seq):
/// its header is not assembled again and one record is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_activation_killed_mid_run_continues_where_it_was() {
    let env = Env::new();
    let agent = env.agent();
    let mut req = goal_req("crash mid run");
    req.schedule.start_on_create = true;
    let g = agent.goals().create(&req, &owner()).await.unwrap();
    let mut child = spawn_host(&env, "slow", None);
    let marker = env.root.join("marker");
    let start = std::time::Instant::now();
    while !marker.exists() {
        assert!(start.elapsed() < Duration::from_secs(30), "the tool never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    let _ = child.wait();
    let llm = crash_script("slow", &env.root);
    let h = hosted(&env, llm.clone());
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!((a.epoch, a.seq, a.outcome), (1, 1, ActivationOutcome::Completed));
    let last = llm.requests.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        user_texts(&last).iter().filter(|t| t.contains("<goal_activation goal=")).count(),
        1
    );
    assert_eq!(h.session(&g.goal_id).state().unwrap().turn_seq, 1);
    for _ in 0..10 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(h.activations(&g.goal_id).len(), 1);
    h.shutdown().await;
}

/// V16: the Turn's close is committed but the Goal State / Activation record
/// are not (fully) written: the next drive writes them once, without
/// inferring again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_activation_is_written_back_once_after_a_crash() {
    for fault in [
        "finish_run:after_commit",
        "goal:after_state_publish",
        "goal:after_activation_append",
    ] {
        let env = Env::new();
        let agent = env.agent();
        let mut req = goal_req("crash at write back");
        req.schedule.start_on_create = true;
        let g = agent.goals().create(&req, &owner()).await.unwrap();
        let mut child = spawn_host(&env, "quick", Some(fault));
        assert!(!wait_exit(&mut child).success(), "{fault}");
        let h = hosted(&env, crash_script("silent", &env.root));
        h.until("written back", |h| {
            h.activations(&g.goal_id).len() == 1
                && h.state(&g.goal_id).is_some_and(|s| s.usage.activations == 1)
                && h.session(&g.goal_id)
                    .state()
                    .unwrap()
                    .goal
                    .is_some_and(|x| !x.needs_flush())
        })
        .await;
        for _ in 0..10 {
            h.tick().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(h.activations(&g.goal_id).len(), 1, "{fault}");
        assert_eq!(h.state(&g.goal_id).unwrap().last_activation.unwrap().seq, 1);
        h.shutdown().await;
    }
}

/// A fire recorded as pending but not posted is posted again with the same
/// key: one Activation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pending_fire_is_posted_once_after_a_crash() {
    let env = Env::new();
    let agent = env.agent();
    let mut req = goal_req("crash at fire");
    req.schedule.every_secs = Some(SLOT);
    let g = agent.goals().create(&req, &owner()).await.unwrap();
    let mut child = spawn_host(&env, "fire", Some("goal:after_fire_pending"));
    assert!(!wait_exit(&mut child).success());
    let pending = agent.goals().schedule(&g.goal_id).await.unwrap().unwrap().pending.unwrap();
    let h = hosted(&env, crash_script("quick", &env.root));
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.reasons[0].key, pending.key);
    let sch = h.agent.goals().schedule(&g.goal_id).await.unwrap().unwrap();
    assert!(sch.pending.is_none());
    assert_eq!(sch.last_posted_key.as_deref(), Some(pending.key.as_str()));
    h.shutdown().await;
}

/// A kill -9 in the middle of a tool call, and a run opportunity queued
/// meanwhile: the resumed run first finishes its tool batch (the input
/// cannot be placed in the middle of it), the queued fire is the next
/// Activation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn input_queued_while_a_killed_activation_was_down_waits_for_its_batch() {
    let env = Env::new();
    let agent = env.agent();
    let mut req = goal_req("queued while down");
    req.schedule.start_on_create = true;
    let g = agent.goals().create(&req, &owner()).await.unwrap();
    let mut child = spawn_host(&env, "slow", None);
    let marker = env.root.join("marker");
    let start = std::time::Instant::now();
    while !marker.exists() {
        assert!(start.elapsed() < Duration::from_secs(30), "the tool never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    let _ = child.wait();
    let key = libopendan::goal::scheduled_key(&g.goal_id, 42, 3);
    libopendan::goal::post_activation(
        agent.as_ref(),
        &g,
        APP,
        ActivationReasonKind::Scheduled,
        &key,
        "scheduled run (every 300s); 3 slots passed",
    )
    .await
    .unwrap();
    let h = hosted(&env, crash_script("slow", &env.root));
    h.until("two activations", |h| h.activations(&g.goal_id).len() == 2).await;
    let acts = h.activations(&g.goal_id);
    assert_eq!(acts[0].reasons[0].kind, ActivationReasonKind::Created);
    assert_eq!(acts[0].outcome, ActivationOutcome::Completed);
    assert_eq!(acts[1].reasons[0].key, key);
    assert_eq!(acts[1].reasons[0].missed, Some(3));
    assert!(h.session(&g.goal_id).state().unwrap().last_error.is_none());
    h.shutdown().await;
}

/// A run opportunity taken in while an Activation is open (its run resumes
/// after a retryable error) joins that Activation; the record lists it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_opportunity_joining_an_open_activation_is_one_of_its_reasons() {
    let env = Env::new();
    let ok = env.root.join("provider_ok");
    let flag = ok.clone();
    let llm = ScriptedLlm::fallible(move |_, _| {
        if flag.exists() {
            Ok(no_action())
        } else {
            Err(llm_context::error::LLMComputeError::Provider {
                failure: llm_context::error::ProviderFailure::Transient,
                message: "busy".into(),
            })
        }
    });
    let h = hosted(&env, llm);
    let mut req = goal_req("joined reasons");
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation open, provider busy", |h| {
        block(h.agent.sessions().lookup(&h.goal_now(&g.goal_id).session.sid))
            .unwrap()
            .is_some_and(|e| e.status.turn_open && e.status.last_error.is_some())
    })
    .await;
    let key = libopendan::goal::scheduled_key(&g.goal_id, 42, 3);
    libopendan::goal::post_activation(
        h.agent.as_ref(),
        &g,
        APP,
        ActivationReasonKind::Scheduled,
        &key,
        "scheduled run (every 300s); 3 slots passed",
    )
    .await
    .unwrap();
    std::fs::write(&ok, "ok").unwrap();
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1).await;
    for _ in 0..20 {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let acts = h.activations(&g.goal_id);
    assert_eq!(acts.len(), 1, "{acts:?}");
    let kinds: Vec<ActivationReasonKind> = acts[0].reasons.iter().map(|r| r.kind).collect();
    assert_eq!(kinds, vec![ActivationReasonKind::Created, ActivationReasonKind::Scheduled]);
    assert_eq!(acts[0].reasons[1].missed, Some(3));
    h.shutdown().await;
}

/// V14 (notify TODO): the questions of an Activation reach the user as
/// notifications — one per question text, a question that is no longer
/// asked is canceled; the answer comes back as an `answer` Activation and
/// the question records it. In a Goal `ask-user` points to `questions`.
#[tokio::test]
async fn goal_questions_reach_the_user_and_answers_come_back() {
    use libopendan::notify::{LocalNotify, NotifyAnswer, NotifyHost, NotifyListReq, NotifyStatus};
    let env = Env::new();
    let local = Arc::new(LocalNotify::enable(&env.agent_root, AGENT).unwrap());
    let llm = ScriptedLlm::new(|req, _| {
        if !is_goal_req(req) {
            return text("not a goal");
        }
        match act(req) {
            1 if has_tool_result(req, "a1").is_none() => tool_call(
                "a1",
                "shell",
                json!({ "command": "agent-session ask-user 'Which repository?'" }),
            ),
            1 => {
                let r = has_tool_result(req, "a1").unwrap();
                assert!(r.contains("put the question in `questions`"), "{r}");
                report(json!({ "decision": "needs_info",
                               "questions": ["Which repository should I watch?", "How often?"] }))
            }
            2 => report(json!({ "decision": "needs_info",
                                "questions": ["Which repository should I watch?"] })),
            _ => report(json!({ "decision": "acted", "distance": "watching" })),
        }
    });
    let mut deps = host_deps(&env, llm.clone());
    deps.notify = Some(local.clone());
    deps.session_cli = Some(std::path::PathBuf::from(env!("CARGO_BIN_EXE_xagent")));
    let clock = ManualClock::at(libopendan::now_ms());
    let mut opts = GoalHostOptions::new(
        env.agent_root.join("sessions"),
        json!({ "tools": { "enabled": true } }),
    );
    opts.clock = clock.clone();
    opts.answers_every = Duration::ZERO;
    let h = Hosted {
        sup: Supervisor::new(deps, None).with_goals(GoalHost::new(opts)),
        clock,
        agent: env.agent(),
    };
    let mut req = goal_req("watch a repository");
    req.title = Some("repo watcher".into());
    req.schedule.start_on_create = true;
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    h.until("activation 1", |h| h.activations(&g.goal_id).len() == 1).await;
    let sent = local.list(NotifyListReq::default()).await.unwrap();
    assert_eq!(sent.len(), 2, "{sent:#?}");
    for v in &sent {
        assert_eq!(v.source.goal_id.as_deref(), Some(g.goal_id.as_str()));
        assert_eq!(v.source.operation.as_deref(), Some("repo watcher"));
    }
    let st = h.state(&g.goal_id).unwrap();
    assert!(st.questions.iter().all(|q| q.notify_id.is_some()));
    let repo = st.questions[0].notify_id.clone().unwrap();
    let often = st.questions[1].notify_id.clone().unwrap();

    // The same question again is the same notification; the dropped one is
    // canceled.
    h.sup.goal_activate(&g.goal_id, "again", "look again").await.unwrap();
    h.until("activation 2", |h| h.activations(&g.goal_id).len() == 2).await;
    assert_eq!(local.list(NotifyListReq::default()).await.unwrap().len(), 2);
    let dropped = local.state(&often).await.unwrap();
    assert_eq!(dropped.status, NotifyStatus::Canceled);
    assert_eq!(dropped.closed_reason.as_deref(), Some("goal_dropped"));
    let st = h.state(&g.goal_id).unwrap();
    assert_eq!(st.questions.len(), 1);
    assert_eq!(st.questions[0].notify_id.as_deref(), Some(repo.as_str()));

    // The user answers: an `answer` Activation, once.
    local.answer(&repo, NotifyAnswer::text("buckyos/buckyos"), None).await.unwrap();
    h.until("answer activation", |h| h.activations(&g.goal_id).len() == 3).await;
    let a = &h.activations(&g.goal_id)[2];
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Answer);
    assert_eq!(a.reasons[0].reference.as_deref(), Some(repo.as_str()));
    let header = last_header(&llm);
    assert!(header.contains("kind=\"answer\""), "{header}");
    assert!(header.contains("buckyos/buckyos"), "{header}");
    let st = h.state(&g.goal_id).unwrap();
    assert_eq!(st.questions[0].status, Some(GoalQuestionStatus::Answered));
    assert_eq!(st.questions[0].answer.as_deref(), Some("buckyos/buckyos"));
    for _ in 0..10 {
        h.tick().await;
    }
    assert_eq!(h.activations(&g.goal_id).len(), 3, "an answer opens one Activation");
    h.shutdown().await;
}
