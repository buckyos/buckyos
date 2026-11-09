//! Result samples of the Memory consolidation (notepads/agent-memory-
//! capture-retain-weight-todo.md §3, T0, T4): each fixture in
//! `tests/fixtures/memory_consolidation/` is "materials + cognitions already
//! held → the list expected afterwards": `list` (what should be held),
//! `exclude` (inferences that must not be drawn), the dispositions each
//! material may get, and the next task it should change. Only what can be
//! checked without reading is checked mechanically (dispositions, counts,
//! revised / untouched items, literal values); `exclude` is for the reader,
//! since a statement that denies an inference names it too.
//!
//! - `every_fixture_reaches_the_model_with_what_comparison_needs` (no
//!   model): a fixture seeds an isolated AgentRoot, and the batch the
//!   maintenance Goal reads shows every material with its source and every
//!   known cognition with its range, basis and sources.
//! - `replay_with_the_real_model` (ignored): the Jarvis package's
//!   `memory_maintenance` behavior consolidates each fixture through the
//!   local Zone's AICC, the mechanical checks of the fixture run, and the
//!   resulting list is written next to the expectation for review:
//!
//! ```text
//! cargo test -p libopendan --test memory_replay -- --ignored --nocapture
//! MEMORY_REPLAY_ONLY=s1_delivery_merge,s6_tool_counterexample cargo test … -- --ignored
//! ```
//!
//!   Reports land in `target/memory-replay/<fixture>.md`. A passing
//!   mechanical check does not replace reading the list.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::plan::{install_jarvis, PACKAGE};
use common::*;
use libopendan::host::goals::{GoalHost, GoalHostOptions, ManualClock};
use libopendan::host::{HostDeps, Supervisor};
use libopendan::lock::{Acquire, Lease};
use libopendan::memory::{
    ActorKind, Batch, Caller, ConsolidationCommit, ItemStatus, Memory, MemoryConfig, PerceptionInput,
    SourceRef, SourceType, CONSOLIDATION_LEASE,
};
use libopendan::protocol::*;
use libopendan::runner::{
    BehaviorAssembler, DefaultAssembler, RunnerOptions, TOOL_MEMORY_CONSOLIDATION_BEGIN,
    TOOL_MEMORY_CONSOLIDATION_COMMIT,
};
use libopendan::state::AgentStateClient;
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use serde_json::{json, Value};

const OWNER: &str = "did:bns:alice";
const WORKSPACE: &str = "workspace:proj-a";
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/memory_consolidation");

fn fixtures() -> Vec<Value> {
    let only: Vec<String> = std::env::var("MEMORY_REPLAY_ONLY")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(FIXTURES)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| serde_json::from_slice::<Value>(&std::fs::read(p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .filter(|f| only.is_empty() || only.iter().any(|o| f["id"] == o.as_str()))
        .collect()
}

fn principal(s: &str) -> &str {
    match s {
        "owner" => OWNER,
        "agent" => AGENT,
        other => other,
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn holder(name: &str) -> HolderInfo {
    HolderInfo {
        runner_id: name.into(),
        principal: APP.into(),
        host: None,
        pid: std::process::id(),
        runtime_id: None,
    }
}

/// Seed the AgentRoot with the fixture: its known cognitions (one seeding
/// commit under the consolidation lease) and its materials (each written
/// under its session's lease, as `ctl perceive` does). Returns the
/// perception ref of every material key.
fn seed(env: &Env, fx: &Value) -> BTreeMap<String, String> {
    let memory = Memory::open(&env.agent_root, MemoryConfig::default());
    let known = fx["known"].as_array().cloned().unwrap_or_default();
    if !known.is_empty() {
        let mut operations = Vec::new();
        for k in &known {
            let id = k["item_id"].as_str().unwrap();
            let scope = json!({ "subjects": [principal(k["subject"].as_str().unwrap())], "objects": k["objects"] });
            let mut evidence = Vec::new();
            for (i, e) in k["evidence"].as_array().unwrap().iter().enumerate() {
                let obs = format!("obs_{}_{i}", id.trim_start_matches("item_"));
                operations.push(json!({"op": "add_observation", "observation_id": obs, "kind": e["kind"],
                    "content": e["content"], "confidence": 0.8, "scope": scope,
                    "source_ref": {"type": "session_event", "event_ref": e["event"]}}));
                evidence.push(obs);
            }
            operations.push(json!({"op": "put_item", "item_id": id, "kind": "observation_inference",
                "claim": {"type": "observation_inference", "statement": k["statement"]},
                "weight": k["weight"], "confidence": k["confidence"], "evidence": evidence,
                "write_reason": "fixture", "scope": scope, "basis": k["basis"], "explicit": k["explicit"],
                "stability": k["stability"], "last_evidence_at": k["last_evidence_at"],
                "semantic_kind": k["semantic_kind"], "tags": k["tags"]}));
        }
        let plan: ConsolidationCommit = serde_json::from_value(json!({
            "idempotency_key": format!("seed-{}", fx["id"].as_str().unwrap()),
            "summary": "fixture: cognitions already held",
            "produced_by": { "goal_run": "fixture:seed" },
            "operations": operations,
        }))
        .unwrap_or_else(|e| panic!("{}: known cognitions: {e}", fx["id"]));
        let lease = match env.agent().locks().acquire(CONSOLIDATION_LEASE, holder("fixture")).unwrap() {
            Acquire::Acquired(l) => l,
            Acquire::Busy(_) => panic!("consolidation lease busy"),
        };
        let batch = Batch {
            id: "fixture".into(),
            materials: Vec::new(),
            context: Vec::new(),
            truncated: false,
            graph_seq: 0,
        };
        memory
            .consolidator(&lease, "fixture")
            .unwrap()
            .commit(&batch, plan)
            .unwrap_or_else(|e| panic!("{}: seeding known cognitions: {e}", fx["id"]));
    }
    let mut refs = BTreeMap::new();
    let mut leases: BTreeMap<String, Lease> = BTreeMap::new();
    for m in fx["materials"].as_array().unwrap() {
        let sid = m["session"].as_str().unwrap().to_string();
        let lease = leases.entry(sid.clone()).or_insert_with(|| {
            let path = env.root.join("fixture-sessions").join(&sid).join("lease.json");
            match Lease::acquire(&format!("session:{sid}"), &path, holder(&sid)).unwrap() {
                Acquire::Acquired(l) => l,
                Acquire::Busy(_) => panic!("session lease busy"),
            }
        });
        let subject = principal(m["subject"].as_str().unwrap());
        let caller = Caller::new(&sid, &[OWNER, AGENT, WORKSPACE], &[subject]);
        let mut source = SourceRef::event(SourceType::SessionEvent, m["event"].as_str().unwrap());
        source.session_id = Some(sid.clone());
        source.actor_kind = Some(m["actor"].as_str().unwrap().parse::<ActorKind>().unwrap());
        let key = m["key"].as_str().unwrap();
        let input = PerceptionInput {
            content: m["content"].as_str().unwrap().to_string(),
            attributes: Value::Null,
            subjects: Vec::new(),
            objects: strings(&m["objects"]),
            exceptions: Vec::new(),
            tags: strings(&m["tags"]),
            suggested_kind: m["suggest"].as_str().map(str::to_string),
            memory_intent: m["intent"].as_str().map(str::to_string),
            cites: strings(&m["cites"]),
            occurred_at: m["occurred_at"].as_str().map(str::to_string),
            anchors: None,
            mentions: Vec::new(),
            source: Some(source),
            idempotency_key: Some(format!("{}/{key}", fx["id"].as_str().unwrap())),
            kind: None,
            seq: None,
        };
        let receipt = memory
            .record_perception(&caller, lease, input)
            .unwrap_or_else(|e| panic!("{} {key}: {e}", fx["id"]));
        refs.insert(key.to_string(), receipt.reference.expect("recorded"));
    }
    refs
}

struct Hosted {
    sup: Arc<Supervisor>,
    agent: Arc<libopendan::FsAgentStateClient>,
}

fn hosted(env: &Env, xllm: agent_tool::xllm::XllmDeps, llm_context: Value, behaviors: bool, max_wait: Duration) -> Hosted {
    let agent = env.agent();
    let deps = HostDeps {
        who: APP.into(),
        agent: agent.clone(),
        inputs: env.channels(),
        waker: Arc::new(libopendan::channel::PollWaker),
        xllm,
        assembler: if behaviors { Arc::new(BehaviorAssembler::default()) } else { Arc::new(DefaultAssembler::default()) },
        session_cli: None,
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        context_windows: None,
        options: RunnerOptions {
            poll_interval: Duration::from_millis(50),
            max_wait,
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
        notifier: None,
    };
    let mut opts = GoalHostOptions::new(env.agent_root.join("sessions"), llm_context);
    opts.clock = ManualClock::at(libopendan::now_ms());
    Hosted {
        sup: Supervisor::new(deps, None).with_goals(GoalHost::new(opts)),
        agent,
    }
}

impl Hosted {
    async fn goal(&self) -> Option<GoalDefinition> {
        self.agent
            .goals()
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|g| g.is_builtin(BUILTIN_MEMORY_MAINTENANCE))
    }

    /// Tick until the built-in Goal finished `n` Activations.
    async fn activations(&self, n: usize, timeout: Duration) -> (GoalDefinition, Vec<GoalActivation>) {
        let end = std::time::Instant::now() + timeout;
        loop {
            self.sup.tick().await.unwrap();
            self.sup.tick_goals().await.unwrap();
            if let Some(g) = self.goal().await {
                let a = self.agent.goals().activations(&g.goal_id, &Default::default()).await.unwrap();
                if a.len() >= n {
                    return (g, a);
                }
            }
            assert!(std::time::Instant::now() < end, "timed out waiting for activation {n}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn tool_results(req: &LlmInferenceRequest) -> Vec<String> {
    let mut out = Vec::new();
    for m in &req.messages {
        for c in &m.content {
            if let buckyos_api::AiContent::ToolResult { content, .. } = c {
                out.push(content.iter().filter_map(|x| x.text_str()).collect::<Vec<_>>().join(" "));
            }
        }
    }
    out
}

/// T0 acceptance, first half: what `begin` returns is enough to compare
/// materials with each other and with what is already known.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_fixture_reaches_the_model_with_what_comparison_needs() {
    let all = fixtures();
    assert!(all.len() >= 14, "fixtures: {}", all.len());
    for fx in all {
        let id = fx["id"].as_str().unwrap().to_string();
        let env = Env::new();
        std::fs::write(env.agent_root.join("agent.toml"), "[session.goal.maintenance]\nenabled = true\nbehavior = \"\"\n").unwrap();
        let refs = seed(&env, &fx);
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let seen2 = seen.clone();
        let llm = ScriptedLlm::new(move |req, n| {
            let results = tool_results(req);
            let Some(begin) = results.iter().find(|t| t.contains("\"operation_id\":\"mc-")) else {
                return tool_call(&format!("b{n}"), TOOL_MEMORY_CONSOLIDATION_BEGIN, json!({}));
            };
            *seen2.lock().unwrap() = Some(begin.clone());
            if results.iter().any(|t| t.contains("\"abandoned\":true")) {
                return tool_call(
                    &format!("r{n}"),
                    TOOL_REPORT,
                    json!({ "report": "done", "is_end": true, "result": { "decision": "no_action", "distance": "fixture read" } }),
                );
            }
            let op = begin.split("\"operation_id\":\"").nth(1).unwrap().split('"').next().unwrap().to_string();
            tool_call(&format!("a{n}"), TOOL_MEMORY_CONSOLIDATION_COMMIT, json!({ "operation_id": op, "abandon": true, "reason": "fixture read" }))
        });
        let xllm = agent_tool::xllm::XllmDeps::default().with_llm(llm as Arc<dyn LlmClient>);
        let h = hosted(&env, xllm, json!({ "tools": { "enabled": true } }), false, Duration::from_millis(300));
        h.activations(1, Duration::from_secs(20)).await;
        h.sup.shutdown().await;
        let begin: Value = serde_json::from_str(seen.lock().unwrap().as_deref().unwrap_or_else(|| panic!("{id}: no batch"))).unwrap();
        // Every material, with its words, its time and the event it came from.
        let materials = begin["materials"].as_array().unwrap();
        for m in fx["materials"].as_array().unwrap() {
            let r = &refs[m["key"].as_str().unwrap()];
            let shown = materials.iter().find(|x| x["ref"] == r.as_str()).unwrap_or_else(|| panic!("{id}: {r} not in the batch"));
            assert_eq!(shown["content"], m["content"], "{id}");
            assert_eq!(shown["occurred_at"], m["occurred_at"], "{id}");
            assert_eq!(shown["source"]["event_ref"], m["event"], "{id}");
            assert_eq!(shown["source"]["actor_kind"], m["actor"], "{id}");
            assert_eq!(shown["scope"]["subjects"][0], principal(m["subject"].as_str().unwrap()), "{id}");
            if !strings(&m["cites"]).is_empty() {
                assert_eq!(shown["cites"][0]["current_revision"], 1, "{id}: {shown}");
            }
        }
        // Every known cognition, whoever its subject is, with its range,
        // the nature of its evidence and the events behind it.
        let known = begin["known_cognitions"].as_array().unwrap();
        for k in fx["known"].as_array().unwrap() {
            let reference = format!("{}@1", k["item_id"].as_str().unwrap());
            let shown = known.iter().find(|x| x["ref"] == reference.as_str()).unwrap_or_else(|| panic!("{id}: {reference} not shown: {known:?}"));
            assert_eq!(shown["statement"], k["statement"], "{id}");
            assert_eq!(shown["basis"], k["basis"], "{id}");
            assert_eq!(shown["explicit"], k["explicit"], "{id}");
            assert_eq!(shown["scope"]["subjects"][0], principal(k["subject"].as_str().unwrap()), "{id}");
            let events: Vec<String> = k["evidence"].as_array().unwrap().iter().map(|e| e["event"].as_str().unwrap().to_string()).collect();
            assert_eq!(strings(&shown["sources"]), events, "{id}");
            assert_eq!(strings(&shown["tags"]).len(), strings(&k["tags"]).len(), "{id}");
        }
        for rule in ["admission", "evidence", "revision", "statement", "explicit", "weight"] {
            assert!(begin["rules"][rule].is_string(), "{id}: rules.{rule}");
        }
    }
}

/// `a.*b` matches when `a` is followed by `b`; anything else is a substring.
fn matches(pattern: &str, text: &str) -> bool {
    let mut rest = text;
    for part in pattern.split(".*") {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    true
}

struct Outcome {
    report: String,
    failures: Vec<String>,
    tokens: u64,
}

fn judge(fx: &Value, refs: &BTreeMap<String, String>, env: &Env, activation: &GoalActivation, state: Option<GoalState>, goal_session: Option<PathBuf>) -> Outcome {
    let exp = &fx["expected"];
    let view = Memory::open(&env.agent_root, MemoryConfig::default()).graph_view().unwrap();
    let known_ids: Vec<String> = fx["known"].as_array().unwrap().iter().map(|k| k["item_id"].as_str().unwrap().to_string()).collect();
    let mut failures = Vec::new();
    let mut out = String::new();
    out.push_str(&format!("# {} — {}\n\n", fx["id"].as_str().unwrap(), fx["title"].as_str().unwrap()));
    out.push_str(&format!(
        "Activation: decision `{}`, outcome `{}`, {} runs, {} tokens{}\n\n",
        serde_json::to_value(activation.decision).unwrap().as_str().unwrap_or("?"),
        activation.outcome.as_str(),
        activation.cost.runs,
        activation.cost.total_tokens,
        activation.end_reason.as_ref().map(|r| format!(", ended: {r}")).unwrap_or_default(),
    ));
    // The rounds the activation took and the commits that were refused
    // (each refusal costs a whole round).
    if let Some(dir) = goal_session {
        let meta = dir.join(".opendan_agent_session");
        let rounds = std::fs::read_to_string(meta.join("usage.jsonl")).map(|t| t.lines().count()).unwrap_or(0);
        out.push_str(&format!("Rounds: {rounds}\n"));
        for line in std::fs::read_to_string(meta.join("worklog.jsonl")).unwrap_or_default().lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            let text = v.to_string();
            if text.contains("\"status\":\"error\"") {
                let result = v.pointer("/body/result").or_else(|| v.pointer("/result")).map(|r| r.to_string()).unwrap_or(text);
                out.push_str(&format!("- refused: {}\n", result.chars().take(400).collect::<String>()));
            }
        }
        out.push('\n');
    }
    out.push_str("## Expected\n\n");
    for l in strings(&exp["list"]) {
        out.push_str(&format!("- {l}\n"));
    }
    out.push_str("\nMust not appear:\n\n");
    for l in strings(&exp["exclude"]) {
        out.push_str(&format!("- {l}\n"));
    }
    out.push_str(&format!("\nNext task: {}\n\n## Materials\n\n", exp["next_task"].as_str().unwrap_or("")));
    // The plans the model committed (the receipts keep them whole).
    let receipts = state.and_then(|s| s.maintenance).map(|m| m.receipts).unwrap_or_default();
    let mut disposed: BTreeMap<String, Value> = BTreeMap::new();
    let mut summaries = Vec::new();
    for r in &receipts {
        if let Some(plan) = &r.plan {
            if r.stage != ReceiptStage::Prepared && r.stage != ReceiptStage::Failed {
                summaries.push(plan["summary"].as_str().unwrap_or("").to_string());
                for d in plan["dispositions"].as_array().cloned().unwrap_or_default() {
                    disposed.insert(d["perception_ref"].as_str().unwrap_or("").to_string(), d);
                }
            }
        }
    }
    for m in fx["materials"].as_array().unwrap() {
        let key = m["key"].as_str().unwrap();
        let d = disposed.get(&refs[key]);
        let outcome = d.and_then(|d| d["outcome"].as_str()).unwrap_or("(not disposed)");
        let allowed = strings(&exp["dispositions"][key]);
        let ok = allowed.iter().any(|a| a == outcome);
        if !ok {
            failures.push(format!("{key}: disposed `{outcome}`, expected one of {allowed:?}"));
        }
        out.push_str(&format!(
            "- {} **{key}** → `{outcome}`{}{}{}\n  > {}\n",
            if ok { "✓" } else { "✗" },
            d.map(|d| strings(&d["cognition_refs"])).filter(|r| !r.is_empty()).map(|r| format!(" {r:?}")).unwrap_or_default(),
            d.and_then(|d| d["reason"].as_str()).map(|r| format!(" — {r}")).unwrap_or_default(),
            d.filter(|d| d["deadline"].is_string()).map(|d| format!(" (until {}; when: {:?})", d["deadline"].as_str().unwrap_or(""), strings(&d["reevaluate_when"]))).unwrap_or_default(),
            m["content"].as_str().unwrap(),
        ));
    }
    out.push_str("\n## Resulting list\n\n");
    let mut active: Vec<&libopendan::memory::MemoryItem> = Vec::new();
    for item in view.items() {
        let is_new = !known_ids.contains(&item.item_id);
        if item.status == ItemStatus::Active {
            active.push(item);
        }
        out.push_str(&format!(
            "- `{}` [{}{}] basis={} explicit={} stability={} weight={} confidence={}{}{}\n  scope={} tags={:?} last_evidence={}\n  > {}\n",
            item.reference(),
            serde_json::to_value(item.status).unwrap().as_str().unwrap_or("?"),
            if is_new { ", new" } else if item.revision > 1 { ", revised" } else { ", as seeded" },
            item.basis.map(|b| b.as_str()).unwrap_or("-"),
            item.explicit,
            item.stability.as_str(),
            item.weight,
            item.confidence,
            item.valid_until.as_ref().map(|t| format!(" valid_until={t}")).unwrap_or_default(),
            item.review_at.as_ref().map(|t| format!(" review_at={t}")).unwrap_or_default(),
            serde_json::to_string(&item.scope).unwrap(),
            item.tags,
            item.last_evidence_at.as_deref().unwrap_or("-"),
            item.statement(),
        ));
    }
    out.push_str("\n## Observations added\n\n");
    for o in view.observations() {
        if !o.observation_id.contains("_0") || !known_ids.iter().any(|k| o.observation_id.starts_with(&format!("obs_{}", k.trim_start_matches("item_")))) {
            out.push_str(&format!("- `{}`: {}\n", o.observation_id, o.content));
        }
    }
    for s in summaries {
        out.push_str(&format!("\nCommit summary: {s}\n"));
    }
    if let Some(r) = &activation.result {
        out.push_str(&format!("\nGoal result: {r}\n"));
    }

    let new: Vec<&&libopendan::memory::MemoryItem> = active.iter().filter(|i| !known_ids.contains(&i.item_id)).collect();
    if let Some(max) = exp["max_new_items"].as_u64() {
        if new.len() as u64 > max {
            failures.push(format!("{} new cognitions, at most {max} expected", new.len()));
        }
    }
    if let Some(min) = exp["min_new_items"].as_u64() {
        if (new.len() as u64) < min {
            failures.push(format!("{} new cognitions, at least {min} expected", new.len()));
        }
    }
    for id in strings(&exp["revised"]) {
        let item = view.item(&id).expect("seeded");
        if item.revision == 1 && item.status == ItemStatus::Active {
            failures.push(format!("{id} was neither revised nor withdrawn"));
        }
    }
    for id in strings(&exp["untouched"]) {
        let item = view.item(&id).expect("seeded");
        if item.revision != 1 || item.status != ItemStatus::Active {
            failures.push(format!("{id} was changed although this batch says nothing about it"));
        }
    }
    for id in strings(&exp["still_active"]) {
        if view.item(&id).expect("seeded").status != ItemStatus::Active {
            failures.push(format!("{id} is no longer active"));
        }
    }
    for h in exp["must_have"].as_array().cloned().unwrap_or_default() {
        let any = strings(&h["any_of"]);
        if !active.iter().any(|i| any.iter().any(|a| i.statement().contains(a.as_str()))) {
            failures.push(format!("no active cognition mentions any of {any:?} ({})", h["why"].as_str().unwrap_or("")));
        }
    }
    for p in strings(&exp["must_not_match"]) {
        if let Some(i) = active.iter().find(|i| matches(&p, &i.statement())) {
            failures.push(format!("{} matches the excluded `{p}`", i.reference()));
        }
    }
    for s in strings(&exp["observation_must_contain"]) {
        if !view.observations().any(|o| o.content.contains(&s)) {
            failures.push(format!("no observation keeps the exact value `{s}`"));
        }
    }
    for (basis, why) in exp["forbid_basis"].as_object().cloned().unwrap_or_default() {
        if let Some(i) = new.iter().find(|i| i.basis.map(|b| b.as_str()) == Some(basis.as_str())) {
            failures.push(format!("{} has basis {basis}: {}", i.reference(), why.as_str().unwrap_or("")));
        }
    }
    out.push_str("\n## Mechanical checks\n\n");
    if failures.is_empty() {
        out.push_str("all passed (the list above still needs reading)\n");
    }
    for f in &failures {
        out.push_str(&format!("- ✗ {f}\n"));
    }
    Outcome {
        report: out,
        failures,
        tokens: activation.cost.total_tokens,
    }
}

/// T4: the semantic replay. Needs the local Zone (AICC) and spends model
/// tokens; run on purpose.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "calls the real model through the local Zone's AICC; writes target/memory-replay/<fixture>.md"]
async fn replay_with_the_real_model() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/memory-replay");
    std::fs::create_dir_all(&out_dir).unwrap();
    let keep = std::env::var("MEMORY_REPLAY_KEEP").ok().map(PathBuf::from);
    let mut failed: Vec<String> = Vec::new();
    let mut tokens = 0u64;
    for fx in fixtures() {
        let id = fx["id"].as_str().unwrap().to_string();
        let env = match &keep {
            Some(dir) => {
                let root = dir.join(&id);
                let _ = std::fs::remove_dir_all(&root);
                Env::at(&root)
            }
            None => Env::new(),
        };
        install_jarvis(&env);
        assert!(Path::new(PACKAGE).join("behaviors/memory_maintenance.toml").exists());
        let refs = seed(&env, &fx);
        let h = hosted(
            &env,
            agent_tool::xllm::XllmDeps::default(),
            json!({ "model": "llm.chat", "max_tokens": 8192, "provider": { "type": "buckyos" }, "tools": { "enabled": true } }),
            true,
            Duration::from_secs(600),
        );
        let (goal, activations) = h.activations(1, Duration::from_secs(900)).await;
        let state = h.agent.goals().state(&goal.goal_id).await.unwrap();
        h.sup.shutdown().await;
        let session = h.agent.sessions().lookup(&activations[0].session_id).await.unwrap().map(|e| PathBuf::from(&e.location));
        let outcome = judge(&fx, &refs, &env, &activations[0], state, session);
        std::fs::write(out_dir.join(format!("{id}.md")), &outcome.report).unwrap();
        println!("{}", outcome.report);
        tokens += outcome.tokens;
        if !outcome.failures.is_empty() {
            failed.push(format!("{id}: {}", outcome.failures.join("; ")));
        }
    }
    println!("total tokens: {tokens}; reports in {}", out_dir.display());
    assert!(failed.is_empty(), "mechanical checks failed:\n{}", failed.join("\n"));
}

// ---------------------------------------------------------------------------
// T1: what a conversation perceives
// ---------------------------------------------------------------------------

const CAPTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/memory_capture/cases.json");

/// A one-to-one conversation of the Jarvis package with its owner, on the
/// real model. Routing work to sub sessions is taken out: the case is what
/// the conversation itself perceives.
async fn conversation(env: &Env) -> (HostDeps, libopendan::SessionDir) {
    conversation_on(env, "dm%3Adid%3Abns%3Aalice", None, None).await
}

/// What the conversation answered, Turn by Turn.
#[derive(Default)]
struct Replies(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl libopendan::runner::OutboundSink for Replies {
    async fn compose(
        &self,
        _cfg: &SessionConfig,
        base: ndn_lib::MsgObject,
        reply: &libopendan::runner::TurnReply,
        objects: Option<&dyn libopendan::runner::ObjectStore>,
    ) -> libopendan::Result<Option<ndn_lib::MsgObject>> {
        self.0.lock().unwrap().push(reply.answer.clone().unwrap_or_default());
        if reply.status != TurnStatus::Completed {
            return Ok(None);
        }
        libopendan::runner::compose_reply(base, reply, objects).await
    }

    async fn send(&self, _sid: &str, _record: &libopendan::bridge::OutboundRecord) -> libopendan::runner::SendResult {
        libopendan::runner::SendResult::Sent {
            msg_id: None,
            deliveries: vec![],
        }
    }
}

async fn conversation_on(
    env: &Env,
    route_key: &str,
    replies: Option<Arc<Replies>>,
    scripted: Option<Arc<ScriptedLlm>>,
) -> (HostDeps, libopendan::SessionDir) {
    install_jarvis(env);
    let route = env.agent_root.join("behaviors/chat_route.toml");
    let text = std::fs::read_to_string(&route).unwrap();
    assert!(text.contains(r#"tool_whitelist = ["group:bash_core", "behavior:analyze_intent"]"#));
    std::fs::write(&route, text.replace(r#", "behavior:analyze_intent"]"#, "]").replace("next = [\"analyze_intent\"]\n", "")).unwrap();
    common::plan::tool_env(env);
    let template = libopendan::SessionTemplate::load("ui", Some(&env.agent_root)).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/{route_key}"));
    spec.owner = Some(OWNER.into());
    spec.outbound = Some(OutboundBinding {
        to: OWNER.into(),
        to_session: None,
        kind: "chat".into(),
    });
    spec.prompt.llm_context = json!({ "model": "llm.chat", "max_tokens": 8192, "provider": { "type": "buckyos" }, "tools": { "enabled": true } });
    spec.freeze = true;
    let sd = env.create_work(spec).await;
    let mut host = common::plan::host(env, ScriptedLlm::new(|_, _| panic!("the real model answers")));
    host.xllm = match scripted {
        Some(llm) => agent_tool::xllm::XllmDeps::default().with_llm(llm as Arc<dyn LlmClient>),
        None => agent_tool::xllm::XllmDeps::default(),
    };
    host.options.max_wait = Duration::from_secs(600);
    host.options.load_hints = true;
    host.outbound = replies.map(|r| r as Arc<dyn libopendan::runner::OutboundSink>);
    (host, sd)
}

async fn say(env: &Env, host: &HostDeps, sd: &libopendan::SessionDir, text: &str) {
    libopendan::post_input(env.agent().as_ref(), sd.sid(), &msg(text)).await.unwrap();
    for _ in 0..2 {
        Box::pin(libopendan::host::run_session(
            host,
            sd,
            libopendan::runner::StopWhen::Idle,
            libopendan::runner::StopSignal::default(),
            false,
        ))
        .await;
    }
}

fn perceived(env: &Env, sd: &libopendan::SessionDir) -> Vec<PerceptionRecord> {
    libopendan::state::perception::read_records(&env.agent().layout().perception_file(sd.sid())).unwrap()
}

fn describe(r: &PerceptionRecord) -> String {
    format!(
        "- {}\n  tags={:?} objects={:?} suggested={:?} intent={:?} cites={:?} occurred_at={:?}\n",
        r.summary, r.tags, r.objects, r.suggested_kind, r.memory_intent, r.cites, r.occurred_at
    )
}

/// T1 acceptance, the prompt itself (no model): a conversation of the
/// Jarvis package is told what to perceive — the route's Memory section and
/// the manual's section under `## Commands` — and how to read a hint.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_jarvis_conversation_is_told_what_to_perceive() {
    let env = Env::new();
    let seen: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let seen2 = seen.clone();
    let llm = ScriptedLlm::new(move |req, _| {
        *seen2.lock().unwrap() = common::plan::system_of(req);
        text(r#"{"reply": "你好", "topic": null, "artifacts": []}"#)
    });
    let (host, sd) = conversation_on(&env, "dm%3Adid%3Abns%3Aalice", None, Some(llm)).await;
    say(&env, &host, &sd, "你好").await;
    let system = seen.lock().unwrap().clone();
    let memory = system.find("## Memory").expect("the route's Memory section");
    let commands = system.find("## Commands").expect("the manual");
    for needle in [
        "Memory: what to perceive",
        "It is the only source of memory material",
        "The same words, different records",
        "Keep exact values exact",
        "Do not perceive:",
    ] {
        assert!(system[commands..].contains(needle), "the manual lacks `{needle}`:\n{system}");
    }
    for needle in ["agent-session ctl perceive", "a finished work included", "the agent's inference"] {
        assert!(system[memory..].contains(needle), "the route lacks `{needle}`:\n{system}");
    }
}

/// T1 acceptance: the same city said four ways leaves four different
/// records (or none), an anomaly without a known cause is still recorded
/// with its conditions, an exact URL survives with its use, the owner's
/// judgment keeps who judged and why. Needs the local Zone (AICC).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "calls the real model through the local Zone's AICC; writes target/memory-replay/capture.md"]
async fn capture_with_the_real_model() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/memory-replay");
    std::fs::create_dir_all(&out_dir).unwrap();
    let only: Vec<String> = std::env::var("MEMORY_REPLAY_ONLY")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let file: Value = serde_json::from_slice(&std::fs::read(CAPTURE).unwrap()).unwrap();
    let mut report = String::from("# What a conversation perceives\n\n");
    let mut failed = Vec::new();
    for case in file["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        if !only.is_empty() && !only.iter().any(|o| o == id) {
            continue;
        }
        let env = Env::new();
        let (host, sd) = conversation(&env).await;
        for text in strings(&case["say"]) {
            say(&env, &host, &sd, &text).await;
        }
        let records = perceived(&env, &sd);
        let all = records.iter().map(|r| r.summary.as_str()).collect::<Vec<_>>().join("\n");
        let mut problems = Vec::new();
        match case["expect"].as_str().unwrap() {
            "none" if !records.is_empty() => problems.push("something was perceived, nothing expected".to_string()),
            "some" if records.is_empty() => problems.push("nothing was perceived".to_string()),
            _ => {}
        }
        if !records.is_empty() {
            for needle in strings(&case["contains_all"]) {
                if !all.contains(&needle) {
                    problems.push(format!("`{needle}` is not kept"));
                }
            }
            let any = strings(&case["contains_any"]);
            if !any.is_empty() && !any.iter().any(|a| all.contains(a.as_str())) {
                problems.push(format!("none of {any:?} is kept"));
            }
        }
        let section = format!(
            "## {id} — {}\n\nSaid: {}\n\nExpected: {} — {}\n\nPerceived ({}):\n\n{}\n{}\n",
            case["covers"].as_str().unwrap_or(""),
            strings(&case["say"]).join(" / "),
            case["expect"].as_str().unwrap(),
            case["why"].as_str().unwrap_or(""),
            records.len(),
            records.iter().map(describe).collect::<String>(),
            if problems.is_empty() { "✓".to_string() } else { format!("✗ {}", problems.join("; ")) },
        );
        println!("{section}");
        report.push_str(&section);
        if !problems.is_empty() {
            failed.push(format!("{id}: {}", problems.join("; ")));
        }
    }
    std::fs::write(out_dir.join("capture.md"), &report).unwrap();
    assert!(failed.is_empty(), "capture checks failed:\n{}", failed.join("\n"));
}

/// T1 → T0 → T2 in one piece, on the real model: what the owner says in a
/// conversation is perceived, consolidated by the maintenance Goal, and
/// comes back as hint lines that say what supports them, where they hold
/// and when they end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "calls the real model through the local Zone's AICC; writes target/memory-replay/pipeline.md"]
async fn conversation_to_hint_with_the_real_model() {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/memory-replay");
    std::fs::create_dir_all(&out_dir).unwrap();
    let env = Env::new();
    let (host, sd) = conversation(&env).await;
    let said = [
        "以后项目 A 的周报控制在一页以内，明细放附件。",
        "顺便说一下，我现在在上海，周五离开。",
        "记一下刚才的情况：你用 tool-x v1 在 Linux 上生成报告失败了两次，加上 --out build/ 之后才成功。到底是不是只和输出目录有关，我们还没查清。",
    ];
    for text in said {
        say(&env, &host, &sd, text).await;
    }
    let records = perceived(&env, &sd);
    let mut report = String::from("# From a conversation to hint lines\n\n## Said\n\n");
    for s in said {
        report.push_str(&format!("- {s}\n"));
    }
    report.push_str(&format!("\n## Perceived ({})\n\n{}", records.len(), records.iter().map(describe).collect::<String>()));
    assert!(!records.is_empty(), "nothing was perceived:\n{report}");
    let h = hosted(
        &env,
        agent_tool::xllm::XllmDeps::default(),
        json!({ "model": "llm.chat", "max_tokens": 8192, "provider": { "type": "buckyos" }, "tools": { "enabled": true } }),
        true,
        Duration::from_secs(600),
    );
    let (_, activations) = h.activations(1, Duration::from_secs(900)).await;
    h.sup.shutdown().await;
    report.push_str(&format!(
        "\n## Consolidated\n\nActivation: decision `{}`, {} tokens\n\n",
        serde_json::to_value(activations[0].decision).unwrap().as_str().unwrap_or("?"),
        activations[0].cost.total_tokens
    ));
    let view = Memory::open(&env.agent_root, MemoryConfig::default()).graph_view().unwrap();
    let mut lines = Vec::new();
    for item in view.items().filter(|i| i.status == ItemStatus::Active) {
        report.push_str(&format!(
            "- `{}` basis={} explicit={} stability={} weight={} valid_until={:?} tags={:?}\n  > {}\n",
            item.reference(),
            item.basis.map(|b| b.as_str()).unwrap_or("-"),
            item.explicit,
            item.stability.as_str(),
            item.weight,
            item.valid_until,
            item.tags,
            item.statement()
        ));
        // The line a conversation whose topic carries one of the item's
        // tags is shown (the runner's recall path).
        let hints = env
            .agent()
            .cognition()
            .recall_hints(&libopendan::state::RecallQuery {
                tags: item.tags.iter().take(1).cloned().collect(),
                max_hints: 5,
            })
            .await
            .unwrap();
        for hint in hints.iter().filter(|x| x.id == item.reference()) {
            lines.push((item.clone(), format!("hint: {} ({})", hint.sentence, hint.note())));
        }
    }
    report.push_str("\n## Hint lines\n\n");
    for (_, line) in &lines {
        report.push_str(&format!("- {line}\n"));
    }
    // The third layer: a new conversation about the same project. Whether
    // the cognition is recalled there (the runner recalls by the topic's
    // tags, so not before the conversation has a topic) and whether the
    // answer uses it are reported, not asserted: a list that is right but
    // not recalled is a recall problem, not a consolidation one.
    let replies = Arc::new(Replies::default());
    let (host2, sd2) = conversation_on(&env, "dm%3Adid%3Abns%3Aalice%2Fnext", Some(replies.clone()), None).await;
    report.push_str("\n## The next conversation\n\n");
    for text in [
        "我们来过一下项目 A 的周报。先别动手，告诉我你记得的要求。",
        "好，按这个要求给我列一个本周周报的提纲，每部分一句话。",
    ] {
        say(&env, &host2, &sd2, text).await;
        let state = sd2.state().unwrap();
        let shown: Vec<&String> = state.background_shown.keys().filter(|k| k.starts_with("hint:")).collect();
        report.push_str(&format!(
            "- Said: {text}\n  topic: {} {:?}; hint lines shown: {shown:?}\n  Answer: {}\n",
            state.topic.title,
            state.topic.tags,
            replies.0.lock().unwrap().last().cloned().unwrap_or_default().replace('\n', " ")
        ));
    }
    // How it got there: the memory commands the conversation ran itself.
    let location = env.agent().sessions().lookup(sd2.sid()).await.unwrap().unwrap().location;
    let worklog = std::fs::read_to_string(Path::new(&location).join(".opendan_agent_session/worklog.jsonl")).unwrap_or_default();
    let mut commands: Vec<String> = Vec::new();
    for needle in ["agent-session recall", "ctl perceive"] {
        for (i, _) in worklog.match_indices(needle) {
            let c: String = worklog[i..].chars().take_while(|c| *c != '"' && *c != '\\').take(120).collect();
            if !commands.contains(&c) {
                commands.push(c);
            }
        }
    }
    report.push_str(&format!("\nMemory commands it ran: {commands:?}\n"));
    println!("{report}");
    std::fs::write(out_dir.join("pipeline.md"), &report).unwrap();
    assert!(!lines.is_empty(), "no cognition came back as a hint");
    for (item, line) in &lines {
        // Never more than the evidence: an inference is named as one, the
        // user's words as theirs, and what ends says when.
        match item.basis.map(|b| b.as_str()) {
            Some("inference") => assert!(line.contains("the agent's inference, not confirmed"), "{line}"),
            Some("user_statement") => assert!(line.contains("stated by the user") || line.contains("the user's explicit statement"), "{line}"),
            _ => {}
        }
        if let Some(t) = &item.valid_until {
            assert!(line.contains(&format!("holds until {t}")), "{line}");
        }
    }
}
