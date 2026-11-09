//! R0 of the recall wiring TODO (notepads/agent-memory-recall-wiring-
//! todo.md §3.1, §4): how many held cognitions a whole message lets in as a
//! full-text entry, and how many of the first eight are about it. No model:
//! the library is the active list the 14 consolidation fixtures left on
//! 2026-11-09 (`tests/fixtures/memory_recall_entry/library.json`, taken from
//! the reports in `notepads/evidence/agent-memory-capture-retain-weight/
//! 2026-11-09/`), the messages are hand-written: related ones, unrelated
//! ones and near misses that share words with what is held.
//!
//! The current gate (two shared tokens, [`fts_enters`]) is measured through
//! the real `query_topic`; the alternatives are scored the same way by hand
//! (full text only, which is all a new conversation has) and checked against
//! the real path where they coincide. The report lands in
//! `target/memory-replay/recall_entry.md`.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use agent_tool::agent_memory::{fts_tokens, item_search_text};
use common::*;
use libopendan::lock::Acquire;
use libopendan::memory::{
    Batch, Budget, Caller, ConsolidationCommit, ItemStatus, Memory, MemoryConfig, TopicQuery,
    TopicScope, CONSOLIDATION_LEASE,
};
use libopendan::protocol::HolderInfo;
use libopendan::state::AgentStateClient;
use serde_json::{json, Value};

const LIBRARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/memory_recall_entry/library.json"
);

fn library() -> Value {
    serde_json::from_slice(&std::fs::read(LIBRARY).unwrap()).unwrap()
}

fn seed(env: &Env, lib: &Value) -> Memory {
    let memory = Memory::open(&env.agent_root, MemoryConfig::default());
    let mut operations = Vec::new();
    for (i, k) in lib["items"].as_array().unwrap().iter().enumerate() {
        // One observation with the statement itself: it adds no token.
        let obs = format!("obs_r0_{i}");
        operations.push(json!({"op": "add_observation", "observation_id": obs, "kind": "explicit_statement",
            "content": k["statement"], "confidence": 0.8, "scope": k["scope"],
            "source_ref": {"type": "session_event", "event_ref": format!("ui-r0:e{i}")}}));
        operations.push(json!({"op": "put_item", "item_id": k["item_id"], "kind": "observation_inference",
            "claim": {"type": "observation_inference", "statement": k["statement"]},
            "weight": k["weight"], "confidence": k["confidence"], "evidence": [obs],
            "write_reason": "fixture", "scope": k["scope"], "basis": k["basis"], "explicit": k["explicit"],
            "stability": k["stability"], "last_evidence_at": k["last_evidence_at"],
            "semantic_kind": "preference", "tags": k["tags"]}));
    }
    let plan: ConsolidationCommit = serde_json::from_value(json!({
        "idempotency_key": "seed-recall-entry",
        "summary": "the lists the consolidation fixtures left",
        "produced_by": { "goal_run": "fixture:seed" },
        "operations": operations,
    }))
    .unwrap();
    let holder = HolderInfo {
        runner_id: "fixture".into(),
        principal: APP.into(),
        host: None,
        pid: std::process::id(),
        runtime_id: None,
    };
    let lease = match env.agent().locks().acquire(CONSOLIDATION_LEASE, holder).unwrap() {
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
    memory.consolidator(&lease, "fixture").unwrap().commit(&batch, plan).unwrap();
    memory
}

/// A full-text gate: does a message of `q` tokens sharing `hits` with an
/// item let it in?
struct Gate {
    name: &'static str,
    enters: fn(usize, usize) -> bool,
}

const GATES: &[Gate] = &[
    Gate { name: "two shared (current)", enters: |q, h| h > 0 && h >= q.min(2) },
    Gate { name: "three shared", enters: |q, h| h > 0 && h >= q.min(3) },
    Gate { name: "a fifth of the message", enters: |q, h| h > 0 && h >= q.min(2).max(q.div_ceil(5)) },
    Gate { name: "a third of the message", enters: |q, h| h > 0 && h >= q.min(2).max(q.div_ceil(3)) },
];

struct Row {
    entered: usize,
    top: Vec<String>,
}

/// Rank by the graph's full-text score (2 per shared token, at most 5) plus
/// weight and confidence, as `AgentMemory::recall` does without tags,
/// objects or fading.
fn rank(memory: &Memory, caller: &Caller, text: &str, gate: &Gate) -> Row {
    let view = memory.graph_view().unwrap();
    let q = fts_tokens(text);
    let scope = libopendan::memory::QueryScope {
        subjects: caller.grants.iter().cloned().collect(),
        objects: Vec::new(),
    };
    let mut scored: Vec<(f64, String)> = Vec::new();
    for item in view.items() {
        if item.status != ItemStatus::Active || !item.visible_to(&caller.grants) {
            continue;
        }
        if item.scope.as_ref().is_some_and(|s| !s.matches(&scope)) {
            continue;
        }
        let (own, obs) = item_search_text(&view, item);
        let mut doc = fts_tokens(&own);
        doc.extend(fts_tokens(&obs));
        let hits = q.intersection(&doc).count();
        if !(gate.enters)(q.len(), hits) {
            continue;
        }
        let score = 2.0 * hits.min(5) as f64 + item.weight * 10.0 + item.confidence * 6.0;
        scored.push((score, item.item_id.clone()));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.1.cmp(&b.1)));
    Row {
        entered: scored.len(),
        top: scored.into_iter().take(8).map(|(_, id)| id).collect(),
    }
}

#[test]
fn whole_messages_as_a_full_text_entry() {
    let env = Env::new();
    let lib = library();
    let memory = seed(&env, &lib);
    let owner = lib["owner"].as_str().unwrap();
    let agent = lib["agent"].as_str().unwrap();
    // A one-to-one conversation with the owner, and a Work in proj-a.
    let callers = [
        ("ui (owner, agent)", Caller::new("ui-r0", &[owner, agent], &[owner])),
        (
            "work (owner, workspace:proj-a, agent)",
            Caller::new("work-r0", &[owner, "workspace:proj-a", agent], &[owner, "workspace:proj-a"]),
        ),
    ];
    let messages = lib["messages"].as_array().unwrap();
    let mut report = String::from("# R0: a whole message as the full-text entry\n\n");
    report.push_str(&format!(
        "Library: {} active cognitions (the lists the 14 consolidation fixtures left, 2026-11-09). Messages: {} ({} related, {} without a related cognition).\n\n",
        lib["items"].as_array().unwrap().len(),
        messages.len(),
        messages.iter().filter(|m| !m["relevant"].as_array().unwrap().is_empty()).count(),
        messages.iter().filter(|m| m["relevant"].as_array().unwrap().is_empty()).count(),
    ));
    for (who, caller) in &callers {
        report.push_str(&format!("## {who}\n\n"));
        let visible: BTreeSet<String> = {
            let view = memory.graph_view().unwrap();
            view.items()
                .filter(|i| i.visible_to(&caller.grants))
                .map(|i| i.item_id.clone())
                .collect()
        };
        report.push_str(&format!("Visible to it: {}\n\n", visible.len()));
        report.push_str("| gate | related: mean entered | related: relevant in top 8 / relevant visible | related: precision of top 8 | unrelated: mean entered | unrelated: messages with any entry |\n|---|---|---|---|---|---|\n");
        let mut detail = String::new();
        for gate in GATES {
            let (mut rel_entered, mut rel_n, mut rel_hit, mut rel_total, mut top_n, mut top_rel) = (0, 0, 0, 0, 0, 0);
            let (mut un_entered, mut un_n, mut un_any) = (0, 0, 0);
            for m in messages {
                let text = m["text"].as_str().unwrap();
                let relevant: BTreeSet<String> = m["relevant"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .filter(|id| visible.contains(id))
                    .collect();
                let row = rank(&memory, caller, text, gate);
                if gate.name.ends_with("(current)") {
                    // The hand ranking is the real path's.
                    let real = memory
                        .query_topic(
                            caller,
                            &TopicQuery::new(
                                TopicScope {
                                    subjects: caller.grants.iter().cloned().collect(),
                                    objects: Vec::new(),
                                    tags: Vec::new(),
                                    title: text.to_string(),
                                },
                                Budget { max_items: 1000, cognitions: 1000, perceptions: 1000 },
                            ),
                        )
                        .unwrap();
                    let real_ids: BTreeSet<String> = real.cognitions.iter().map(|h| h.id.clone()).collect();
                    let mine: BTreeSet<String> = rank(&memory, caller, text, gate)
                        .top
                        .into_iter()
                        .chain(std::iter::empty())
                        .collect();
                    assert_eq!(real.cognitions.len(), row.entered, "{text}: {real_ids:?}");
                    assert!(mine.is_subset(&real_ids), "{text}: {mine:?} vs {real_ids:?}");
                    detail.push_str(&format!(
                        "- {} `{}` → {} entered; top: {}\n",
                        if m["relevant"].as_array().unwrap().is_empty() { "✗" } else { "✓" },
                        text,
                        row.entered,
                        row.top
                            .iter()
                            .map(|id| if relevant.contains(id) { format!("**{id}**") } else { id.clone() })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if m["relevant"].as_array().unwrap().is_empty() {
                    un_n += 1;
                    un_entered += row.entered;
                    un_any += (row.entered > 0) as usize;
                } else {
                    rel_n += 1;
                    rel_entered += row.entered;
                    rel_total += relevant.len();
                    rel_hit += row.top.iter().filter(|id| relevant.contains(*id)).count();
                    top_n += row.top.len();
                    top_rel += row.top.iter().filter(|id| relevant.contains(*id)).count();
                }
            }
            report.push_str(&format!(
                "| {} | {:.1} | {rel_hit}/{rel_total} | {} | {:.1} | {un_any}/{un_n} |\n",
                gate.name,
                rel_entered as f64 / rel_n.max(1) as f64,
                if top_n == 0 { "-".to_string() } else { format!("{top_rel}/{top_n} ({:.0}%)", 100.0 * top_rel as f64 / top_n as f64) },
                un_entered as f64 / un_n.max(1) as f64,
            ));
        }
        report.push_str(&format!("\nPer message, current gate (relevant in bold):\n\n{detail}\n"));
    }
    println!("{report}");
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/memory-replay");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("recall_entry.md"), &report).unwrap();
}
