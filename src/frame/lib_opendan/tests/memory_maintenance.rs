//! The agent's default Memory maintenance Goal (Jarvis default Goal TODO
//! §9 V01, V04, V07, V08, V09, V17): seeding, the deterministic gate, the
//! consolidation host tools, the lease, anti echo, the report check and
//! crash recovery of a receipt.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use libopendan::host::goals::{GoalHost, GoalHostOptions, ManualClock};
use libopendan::host::{HostDeps, Supervisor};
use libopendan::lock::Acquire;
use libopendan::memory::{Memory, MemoryConfig, CONSOLIDATION_LEASE};
use libopendan::protocol::*;
use libopendan::runner::{
    drive, DefaultAssembler, DriveResult, StopWhen, TOOL_MEMORY_CONSOLIDATION_BEGIN,
    TOOL_MEMORY_CONSOLIDATION_COMMIT,
};
use libopendan::state::{AgentStateClient, FsAgentStateClient, GoalCaller, GoalStatusChange};
use libopendan::SessionDir;
use llm_context::deps::LlmInferenceRequest;
use serde_json::{json, Value};

const SLOT_MS: u64 = 1800 * 1000;
const ENV_ROOT: &str = "MM_TEST_ROOT";
const ENV_SCENARIO: &str = "MM_TEST_SCENARIO";

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

/// `agent.toml` that switches the built-in Goal on (no behavior: the plain
/// function-call prompt with the host tools).
fn enable_maintenance(env: &Env) {
    std::fs::write(
        env.agent_root.join("agent.toml"),
        "[session.goal.maintenance]\nenabled = true\nbehavior = \"\"\n",
    )
    .unwrap();
}

fn hosted(env: &Env, llm: Arc<ScriptedLlm>) -> Hosted {
    let clock = ManualClock::at(libopendan::now_ms());
    let mut opts = GoalHostOptions::new(
        env.agent_root.join("sessions"),
        json!({ "tools": { "enabled": true } }),
    );
    opts.clock = clock.clone();
    let sup = Supervisor::new(host_deps(env, llm), None).with_goals(GoalHost::new(opts));
    Hosted {
        sup,
        clock,
        agent: env.agent(),
    }
}

impl Hosted {
    async fn tick(&self) {
        self.sup.tick().await.unwrap();
        self.sup.tick_goals().await.unwrap();
    }

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

    fn goals(&self) -> Vec<GoalDefinition> {
        block(self.agent.goals().list()).unwrap()
    }

    fn builtin(&self) -> GoalDefinition {
        let goals = self.goals();
        let found: Vec<&GoalDefinition> = goals
            .iter()
            .filter(|g| g.is_builtin(BUILTIN_MEMORY_MAINTENANCE))
            .collect();
        assert_eq!(found.len(), 1, "exactly one built-in goal: {goals:?}");
        found[0].clone()
    }

    fn activations(&self, id: &str) -> Vec<GoalActivation> {
        block(self.agent.goals().activations(id, &Default::default())).unwrap()
    }

    fn state(&self, id: &str) -> Option<GoalState> {
        block(self.agent.goals().state(id)).unwrap()
    }

    fn gate(&self, id: &str) -> MaintenanceGate {
        block(self.agent.goals().schedule(id))
            .unwrap()
            .and_then(|s| s.gate)
            .expect("the built-in goal has a gate")
    }

    fn session(&self, id: &str) -> SessionDir {
        let g = block(self.agent.goals().get(id)).unwrap().unwrap();
        let e = block(self.agent.sessions().lookup(&g.session.sid)).unwrap().unwrap();
        SessionDir::open(&e.location).unwrap()
    }

    async fn shutdown(&self) {
        self.sup.shutdown().await;
    }
}

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

fn report(n: usize, result: Value) -> buckyos_api::AiResponse {
    tool_call(
        &format!("r{n}"),
        TOOL_REPORT,
        json!({ "report": "activation done", "is_end": true, "result": result }),
    )
}

/// Every tool result text of the request, oldest first.
fn tool_results(req: &LlmInferenceRequest) -> Vec<String> {
    let mut out = Vec::new();
    for m in &req.messages {
        for c in &m.content {
            if let buckyos_api::AiContent::ToolResult { content, .. } = c {
                out.push(
                    content
                        .iter()
                        .filter_map(|x| x.text_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }
    out
}

/// `"ref":"<sid>:<seq>"` values of a begin result.
fn refs_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("\"ref\":\"") {
        let s = &rest[i + 7..];
        let Some(j) = s.find('"') else { break };
        let r = &s[..j];
        if r.contains(':') && !r.starts_with("item_") && !out.iter().any(|x| x == r) {
            out.push(r.to_string());
        }
        rest = &s[j..];
    }
    out
}

fn field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let i = text.find(&needle)?;
    let s = &text[i + needle.len()..];
    Some(s[..s.find('"')?].to_string())
}

/// Revision of `item` among the known cognitions of a begin result.
fn known_revision(text: &str, item: &str) -> Option<u64> {
    let needle = format!("\"ref\":\"{item}@");
    let i = text.find(&needle)?;
    let s = &text[i + needle.len()..];
    s[..s.find('"')?].parse().ok()
}

fn consolidation_plan(op: &str, refs: &[String], expected: Option<u64>) -> Value {
    let obs = format!("obs_mm_{}", expected.unwrap_or(0) + 1);
    let mut item = json!({"op": "put_item", "item_id": "item_mm_1", "kind": "observation_inference",
        "claim": {"type": "observation_inference", "statement": "this agent finishes small work sessions in one run"},
        "weight": 0.6, "confidence": 0.7, "evidence": [obs], "write_reason": "pattern across finished work",
        "scope": {"subjects": [AGENT]}, "basis": "tool_observation",
        "last_evidence_at": "2026-10-23T00:00:00Z", "stability": "contextual"});
    if let Some(e) = expected {
        item["expected_revision"] = json!(e);
    }
    json!({
        "operation_id": op,
        "summary": "absorbed the work's own run records into one experience",
        "operations": [
            {"op": "add_observation", "observation_id": obs, "kind": "behavior_signal",
             "content": "work sessions of this agent finish their objective in one run",
             "source_ref": {"type": "session_event", "event_ref": refs[0]}, "confidence": 0.8},
            item
        ],
        "dispositions": refs.iter().map(|r| json!({"perception_ref": r, "outcome": "absorbed", "cognition_refs": ["item_mm_1"]})).collect::<Vec<_>>(),
    })
}

/// The consolidation flow driven by the content of the request alone (so a
/// new process continues where the old one died): begin → commit → report.
fn consolidating_llm() -> Arc<ScriptedLlm> {
    ScriptedLlm::new(|req, n| {
        let results = tool_results(req);
        if results.iter().any(|t| t.contains("\"status\":\"committed\"") || t.contains("\"status\":\"already_completed\"")) {
            return report(n, json!({ "decision": "acted", "distance": "batch committed" }));
        }
        if let Some(begin) = results.iter().rev().find(|t| t.contains("\"operation_id\":\"mc-")) {
            let op = field(begin, "operation_id").unwrap();
            let refs = refs_of(begin);
            assert!(!refs.is_empty(), "begin result without materials: {begin}");
            let expected = known_revision(begin, "item_mm_1");
            return tool_call(&format!("c{n}"), TOOL_MEMORY_CONSOLIDATION_COMMIT, consolidation_plan(&op, &refs, expected));
        }
        if results.iter().any(|t| t.contains("\"empty\":true")) {
            return report(n, json!({ "decision": "no_action", "distance": "nothing waits" }));
        }
        tool_call(&format!("b{n}"), TOOL_MEMORY_CONSOLIDATION_BEGIN, json!({}))
    })
}

fn silent_llm() -> Arc<ScriptedLlm> {
    ScriptedLlm::new(|req, _| panic!("no inference expected: {}", render(&req.messages)))
}

/// A finished Work session leaves run_digest / task_outcome material.
async fn leave_material(env: &Env, objective: &str) -> SessionDir {
    let sd = env.create_work(work_spec(objective)).await;
    assert!(drive(&sd, &env.deps(ScriptedLlm::new(|_, _| text("ok"))), StopWhen::Finished)
        .await
        .is_finished());
    sd
}

fn memory(env: &Env) -> Memory {
    Memory::open(&env.agent_root, MemoryConfig::default())
}

/// V01: one built-in Goal per agent, seeded once across restarts, no model
/// call on an empty Memory; nothing seeded while the switch is off.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_builtin_goal_is_seeded_once_and_costs_no_model_without_material() {
    let env = Env::new();
    // Switched off: no Goal appears.
    let h = hosted(&env, silent_llm());
    h.tick().await;
    assert!(h.goals().is_empty());
    h.shutdown().await;
    enable_maintenance(&env);
    let h = hosted(&env, silent_llm());
    h.tick().await;
    h.tick().await;
    let g = h.builtin();
    assert_eq!(g.idempotency_key.as_deref(), Some(BUILTIN_MEMORY_MAINTENANCE_KEY));
    assert_eq!(g.lifecycle, GoalLifecycle::Maintain);
    assert_eq!(g.owner, AGENT);
    assert_eq!(g.created_by.via, "owner");
    assert_eq!(g.schedule.every_secs, Some(1800));
    assert_eq!(g.budget.activation.max_total_tokens, Some(120_000));
    let ext = g.opendan().unwrap();
    assert!(ext.has_capability(CAP_MEMORY_CONSOLIDATION) && !ext.records_perception);
    assert_eq!(ext.policy_version, MEMORY_MAINTENANCE_POLICY_VERSION);
    // Its session exists; the created event went through the gate: no work.
    assert!(h.agent.sessions().lookup(&g.session.sid).await.unwrap().is_some());
    let gate = h.gate(&g.goal_id);
    assert_eq!(gate.status, "no_work", "{gate:?}");
    assert!(gate.last_full_check_at_ms > 0 && gate.last_inputs_digest.is_some());
    assert!(h.activations(&g.goal_id).is_empty());
    h.shutdown().await;
    // Two more hosts (restarts): the same Goal, one `created` change.
    for _ in 0..2 {
        let h = hosted(&env, silent_llm());
        h.tick().await;
        h.tick().await;
        assert_eq!(h.goals().len(), 1);
        assert_eq!(h.builtin().goal_id, g.goal_id);
        h.shutdown().await;
    }
    let changes = h.agent.goals().changes(&g.goal_id, 0).await.unwrap();
    assert_eq!(changes.iter().filter(|c| c.kind == "created").count(), 1);
    assert!(h.activations(&g.goal_id).is_empty());
    // Two slots without any change: cheap checks only.
    let h = hosted(&env, silent_llm());
    h.tick().await;
    for _ in 0..2 {
        h.clock.advance(SLOT_MS);
        h.tick().await;
        let gate = h.gate(&g.goal_id);
        assert!(matches!(gate.status.as_str(), "no_change" | "no_work"), "{gate:?}");
    }
    assert!(h.activations(&g.goal_id).is_empty());
    // Another agent's root gets its own Goal (isolation by directory).
    let env2 = Env::new();
    enable_maintenance(&env2);
    let h2 = hosted(&env2, silent_llm());
    h2.tick().await;
    assert_eq!(h2.goals().len(), 1);
    // Same agent DID, another directory: the deterministic id repeats, the
    // Goal, its session and its Memory live in their own root.
    assert_ne!(env2.agent_root, env.agent_root);
    assert!(env2.agent_root.join("state/goals").join(&h2.builtin().goal_id).join("goal.json").is_file());
    h2.shutdown().await;
    h.shutdown().await;
}

/// V04 / the main flow: material makes the gate active, the Activation reads
/// a batch and commits it through the host tools; the cognition exists, the
/// material is cleared, the receipt is complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn material_is_consolidated_through_the_host_tools() {
    let env = Env::new();
    enable_maintenance(&env);
    let work = leave_material(&env, "A").await;
    let llm = consolidating_llm();
    let h = hosted(&env, llm.clone());
    h.tick().await;
    let g = h.builtin();
    // Material present at creation: the created event runs an Activation.
    h.until("first activation", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.decision, ActivationDecision::Acted, "{a:?}");
    assert_eq!(a.reasons[0].kind, ActivationReasonKind::Created);
    assert!(a.reasons[0].note.as_deref().unwrap_or("").contains("memory maintenance"));
    assert!(llm.count() >= 3, "begin, commit, report");
    let st = h.state(&g.goal_id).unwrap();
    let m = st.maintenance.expect("maintenance state");
    assert_eq!(m.consolidations, 1);
    assert_eq!(m.receipts.len(), 1);
    assert_eq!(m.receipts[0].stage, ReceiptStage::Completed);
    assert!(!m.receipts[0].cleanup_pending);
    assert_eq!(m.receipts[0].operations, 2);
    assert!(m.last_successful_consolidation_at_ms.is_some());
    // The cognition, with its schema 4.0 fields.
    let view = memory(&env).graph_view().unwrap();
    let item = view.item("item_mm_1").expect("cognition");
    assert_eq!(item.last_evidence_at.as_deref(), Some("2026-10-23T00:00:00Z"));
    assert!(view.item_created_at(item).is_some());
    assert!(item.produced_by.as_ref().unwrap().goal_run.as_deref().unwrap().starts_with(&format!("goal:{}:1:", g.goal_id)));
    // The material's body is gone, its identity and disposition stay.
    let recs = libopendan::state::perception::read_records(&h.agent.layout().perception_file(work.sid())).unwrap();
    assert!(!recs.is_empty());
    assert!(recs.iter().all(|r| r.cleared.is_some() && r.summary.is_empty()), "{recs:?}");
    // The Goal Session itself left no material: nothing to do at the next slot.
    h.clock.advance(SLOT_MS);
    h.tick().await;
    let gate = h.gate(&g.goal_id);
    assert!(matches!(gate.status.as_str(), "no_work" | "no_change"), "{gate:?}");
    assert_eq!(h.activations(&g.goal_id).len(), 1);
    // A second piece of material: the next slot is active and consolidates it.
    leave_material(&env, "B").await;
    h.clock.advance(SLOT_MS);
    h.until("second activation", |h| h.activations(&g.goal_id).len() == 2).await;
    assert_eq!(h.gate(&g.goal_id).status, "active");
    assert_eq!(h.activations(&g.goal_id)[1].reasons[0].kind, ActivationReasonKind::Scheduled);
    let m = h.state(&g.goal_id).unwrap().maintenance.unwrap();
    assert_eq!(m.consolidations, 2);
    assert_eq!(view_item_revision(&env, "item_mm_1"), 2, "the same item was revised, not duplicated: {m:?} {:?}", h.activations(&g.goal_id));
    h.shutdown().await;
}

fn view_item_revision(env: &Env, id: &str) -> u64 {
    memory(env).graph_view().unwrap().item(id).map(|i| i.revision).unwrap_or(0)
}

/// V07: one consolidator at a time; the second driver is busy and calls no
/// model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_consolidation_lease_keeps_a_second_driver_out() {
    let env = Env::new();
    enable_maintenance(&env);
    let h = hosted(&env, silent_llm());
    h.tick().await;
    let g = h.builtin();
    h.shutdown().await;
    let sd = h.session(&g.goal_id);
    let agent = env.agent();
    let other = match agent
        .locks()
        .acquire(CONSOLIDATION_LEASE, env.deps(silent_llm()).holder())
        .unwrap()
    {
        Acquire::Acquired(l) => l,
        Acquire::Busy(_) => panic!("free"),
    };
    let r = drive(&sd, &env.deps(silent_llm()), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Busy { .. }), "{r:?}");
    drop(other);
    let r = drive(&sd, &env.deps(silent_llm()), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    // The lease is released with the drive.
    assert!(matches!(
        agent.locks().acquire(CONSOLIDATION_LEASE, env.deps(silent_llm()).holder()).unwrap(),
        Acquire::Acquired(_)
    ));
}

/// V17 / anti echo: the built-in Goal's session records no perceptions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_builtin_goal_session_records_no_perceptions() {
    let env = Env::new();
    enable_maintenance(&env);
    let h = hosted(&env, silent_llm());
    h.tick().await;
    let g = h.builtin();
    h.shutdown().await;
    let sd = h.session(&g.goal_id);
    let agent = env.agent();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &PostedInput::control(
            APP,
            "p-echo",
            ControlCommand::Perceive {
                kind: "observation".into(),
                summary: "I consolidated memory".into(),
                tags: vec![],
                objects: vec![],
                subjects: vec![],
                cites: vec![],
                source: None,
                occurred_at: None,
                suggested_kind: None,
                memory_intent: None,
                mentions: vec![],
            },
        ),
    )
    .await
    .unwrap();
    let r = drive(&sd, &env.deps(silent_llm()), StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(|e| matches!(&e.body,
        WorklogBody::InputRejected { detail, .. } if detail.contains("does not record perceptions"))), "{:?}", kinds(&wl));
    assert!(!agent.layout().perception_file(sd.sid()).exists());
}

/// §4.1 report check: a batch that was read but not committed cannot be
/// reported as acted; abandoning it and reporting no_action is accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_acted_report_with_an_open_batch_is_refused() {
    let env = Env::new();
    enable_maintenance(&env);
    leave_material(&env, "A").await;
    let llm = ScriptedLlm::new(|req, n| {
        let results = tool_results(req);
        let begun = results.iter().rev().find(|t| t.contains("\"operation_id\":\"mc-")).cloned();
        let refused = results.iter().any(|t| t.contains("was read but not committed"));
        let abandoned = results.iter().any(|t| t.contains("\"abandoned\":true"));
        match (begun, refused, abandoned) {
            (None, _, _) => tool_call(&format!("b{n}"), TOOL_MEMORY_CONSOLIDATION_BEGIN, json!({})),
            (Some(_), false, _) => report(n, json!({ "decision": "acted", "distance": "pretend" })),
            (Some(b), true, false) => tool_call(
                &format!("a{n}"),
                TOOL_MEMORY_CONSOLIDATION_COMMIT,
                json!({ "operation_id": field(&b, "operation_id").unwrap(), "abandon": true, "reason": "test" }),
            ),
            (Some(_), true, true) => report(n, json!({ "decision": "no_action", "distance": "abandoned" })),
        }
    });
    let h = hosted(&env, llm.clone());
    h.tick().await;
    let g = h.builtin();
    h.until("activation", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.decision, ActivationDecision::NoAction, "{a:?}");
    let m = h.state(&g.goal_id).unwrap().maintenance.unwrap();
    assert_eq!(m.abandoned_batches, 1);
    assert!(m.open_receipts().count() == 0);
    assert_eq!(m.consolidations, 0);
    // The material was not disposed of: it is still pending.
    let p = memory(&env).pending_work().unwrap();
    assert!(p.new > 0, "{p:?}");
    h.shutdown().await;
}

/// V02 / V06: a paused Goal shows `paused` in its gate and runs nothing; a
/// resumed one checks again at the next slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_shows_in_the_gate_and_resume_checks_again() {
    let env = Env::new();
    enable_maintenance(&env);
    let h = hosted(&env, silent_llm());
    h.tick().await;
    let g = h.builtin();
    h.agent
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Paused,
                reason: Some("test".into()),
                evidence: None,
            },
            &GoalCaller::owner(AGENT),
        )
        .await
        .unwrap();
    leave_material(&env, "A").await;
    h.clock.advance(SLOT_MS);
    h.tick().await;
    // The admission chain skips a paused goal before the gate; the gate's
    // own view is updated by the restart below.
    assert!(h.activations(&g.goal_id).is_empty());
    h.shutdown().await;
    // Restarting the host does not revive a paused goal.
    let h = hosted(&env, silent_llm());
    h.tick().await;
    assert_eq!(h.builtin().status, GoalStatus::Paused);
    assert!(h.activations(&g.goal_id).is_empty());
    h.shutdown().await;
    h.agent
        .goals()
        .set_status(
            &g.goal_id,
            &GoalStatusChange {
                status: GoalStatus::Active,
                reason: Some("test".into()),
                evidence: None,
            },
            &GoalCaller::owner(AGENT),
        )
        .await
        .unwrap();
    let h = hosted(&env, consolidating_llm());
    h.tick().await;
    // Two slots: the earlier host's clock had already moved one slot ahead
    // of the wall clock (a clock set back replays no slot).
    h.clock.advance(2 * SLOT_MS);
    h.until("activation after resume", |h| h.activations(&g.goal_id).len() == 1).await;
    assert_eq!(h.gate(&g.goal_id).status, "active");
    h.shutdown().await;
}

// ---- crash recovery of a receipt (V09) ---------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn child_maintenance_host() {
    let (Ok(root), Ok(_scenario)) = (std::env::var(ENV_ROOT), std::env::var(ENV_SCENARIO)) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    let h = hosted(&env, consolidating_llm());
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        h.tick().await;
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn spawn_host(env: &Env, fault: &str) -> std::process::Child {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "child_maintenance_host", "--nocapture", "--test-threads=1"])
        .env(ENV_ROOT, &env.root)
        .env(ENV_SCENARIO, "consolidate")
        .env("LIBOPENDAN_FAULT", fault)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
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

async fn crash_then_recover(fault: &str) {
    let env = Env::new();
    enable_maintenance(&env);
    leave_material(&env, "A").await;
    let mut child = spawn_host(&env, fault);
    assert!(!wait_exit(&mut child).success(), "the child aborts at {fault}");
    let h = hosted(&env, consolidating_llm());
    let g = h.builtin();
    h.until("activation after the crash", |h| h.activations(&g.goal_id).len() == 1).await;
    let a = &h.activations(&g.goal_id)[0];
    assert_eq!(a.decision, ActivationDecision::Acted, "{a:?}");
    let m = h.state(&g.goal_id).unwrap().maintenance.unwrap();
    assert_eq!(m.receipts.len(), 1, "{m:?}");
    assert_eq!(m.receipts[0].stage, ReceiptStage::Completed);
    assert_eq!(m.consolidations, 1);
    let view = memory(&env).graph_view().unwrap();
    let item = view.item("item_mm_1").expect("cognition exists once");
    assert_eq!(item.revision, 1, "no duplicate revision after the replay");
    let occasions = view
        .occasions()
        .iter()
        .filter(|o| o.idempotency_key.as_deref() == Some(m.receipts[0].operation_id.as_str()))
        .count();
    assert_eq!(occasions, 1, "one envelope for the receipt");
    assert_eq!(memory(&env).pending_work().unwrap().new, 0, "no material lost or left");
    h.shutdown().await;
}

/// The host dies after the plan is persisted and before the Memory commit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_receipt_with_a_plan_is_committed_once_after_a_crash() {
    crash_then_recover("consolidation:after_plan").await;
}

/// The host dies after the Memory commit and before the receipt advanced.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_committed_receipt_is_finished_without_a_second_envelope() {
    crash_then_recover("consolidation:after_memory_commit").await;
}

/// The host dies right after the batch was read: the batch is dropped by
/// recovery and the material is read again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_read_before_the_crash_is_read_again() {
    crash_then_recover("consolidation:after_begin").await;
}
