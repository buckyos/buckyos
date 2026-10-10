//! Multi-Session simulation of the Agent Memory component (component stage
//! C of the Memory requirements, notepads/agent-memory-multi-session-
//! simulation-todo.md). No LLM, no Runner: Sessions are `SimSession`s, the
//! consolidation Goal is `SimSi` with preset plans; storage, matching,
//! commits, notifications and recovery are the real component.
//!
//! ```text
//! cargo run -p libopendan --example memory_sessions                 # every scenario
//! cargo run -p libopendan --example memory_sessions -- main_flow    # one scenario
//! cargo run -p libopendan --example memory_sessions -- --keep <dir> # keep the Agent roots
//! cargo run -p libopendan --example memory_sessions -- --quiet      # only the check summary
//! cargo run -p libopendan --example memory_sessions -- --bench      # cost baseline (T04)
//! ```

#[path = "support/memory_scenarios.rs"]
mod memory_scenarios;
#[path = "support/memory_sim.rs"]
mod memory_sim;

use std::path::PathBuf;

use std::time::Instant;

use agent_tool::agent_memory::{AgentMemory, AgentMemoryConfig, ObservationKind};
use libopendan::memory::preview::{render_topic, Names};
use libopendan::memory::*;
use memory_scenarios::SCENARIOS;
use memory_sim::*;

fn main() {
    let mut keep: Option<PathBuf> = None;
    let mut quiet = false;
    let mut only: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--keep" => keep = args.next().map(PathBuf::from),
            "--quiet" => quiet = true,
            "--bench" => {
                let tmp = tempfile::TempDir::new().expect("tempdir");
                bench(&tmp.path().join("bench"));
                return;
            }
            "--list" => {
                for (name, _) in SCENARIOS {
                    println!("{name}");
                }
                return;
            }
            other => only.push(other.to_string()),
        }
    }
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let base = keep.unwrap_or_else(|| tmp.path().to_path_buf());
    let mut total = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for (name, scenario) in SCENARIOS {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        let root = base.join(name);
        let _ = std::fs::remove_dir_all(&root);
        let mut host = Host::new(&root, "2026-10-08T09:00:00+08:00");
        host.trace.verbose = !quiet;
        scenario(&mut host);
        total += host.trace.checks.len();
        for c in host.trace.failed() {
            failed.push(format!("{name}: {} {}", c.id, c.detail));
        }
        if quiet {
            println!(
                "{name}: {} checks, {} failed",
                host.trace.checks.len(),
                host.trace.failed().len()
            );
        }
    }
    println!("\n{total} checks, {} failed", failed.len());
    for f in &failed {
        println!("  ✗ {f}");
    }
    if !failed.is_empty() {
        std::process::exit(1);
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Cost baseline (TD-22 starts from these numbers; nothing here is a
/// performance promise).
fn bench(root: &std::path::Path) {
    const SESSIONS: usize = 20;
    const PER_SESSION: usize = 50;
    let mut h = Host::new(root, "2026-10-09T09:00:00+08:00");
    let writers: Vec<SimSession> = (0..SESSIONS)
        .map(|i| {
            h.session(
                &format!("S{i}"),
                &format!("ui-s{i}"),
                &[U1, A1],
                &[U1],
                &[ALPHA_GAME],
            )
        })
        .collect();
    let t = Instant::now();
    for (i, w) in writers.iter().enumerate() {
        for j in 0..PER_SESSION {
            let e = h.event(&format!("S{i}/e{j}"), &w.sid, ActorKind::User, "x");
            h.memory
                .record_perception(
                    &w.caller,
                    &w.lease,
                    observation(
                        &format!("第 {i} 个会话的第 {j} 条背景观察"),
                        e,
                        &[ALPHA_GAME],
                        &format!("S{i}/e{j}/1"),
                    ),
                )
                .unwrap();
        }
    }
    let writes = SESSIONS * PER_SESSION;
    let write_ms = ms(t);
    let si = h.si("si-bench");
    let c = h.memory.consolidator(&si.lease, &si.sid).unwrap();
    let t = Instant::now();
    let mut commits = 0;
    let mut batch_sizes = Vec::new();
    loop {
        let b = c.begin(&BatchOptions { limit: 50 }).unwrap();
        if b.materials.is_empty() {
            break;
        }
        batch_sizes.push(b.materials.len());
        let first = b.materials[0].record.source_ref.clone().unwrap();
        let id = format!("item_b{commits}");
        let obs = format!("obs_b{commits}");
        let p = plan(
            &b.id.clone(),
            "bench consolidation",
            vec![
                obs_op(
                    &obs,
                    ObservationKind::ExplicitStatement,
                    "批量整理的背景观察",
                    &first,
                    scope(&[U1], &[ALPHA_GAME]),
                ),
                ItemSpec::new(
                    &id,
                    "preference",
                    &format!("第 {commits} 批整理出的背景偏好"),
                    &[obs.as_str()],
                    scope(&[U1], &[ALPHA_GAME]),
                    Basis::UserStatement,
                )
                .op(),
            ],
            b.materials
                .iter()
                .map(|m| absorbed(&m.reference, &[id.as_str()]))
                .collect(),
        );
        c.commit(&b, p).unwrap();
        commits += 1;
    }
    let commit_ms = ms(t);
    let t = Instant::now();
    let cleaned = c.cleanup().unwrap().cleared.len();
    let cleanup_ms = ms(t);
    let mut b = h.session("B", "ui-b", &[U1, A1], &[U1], &[ALPHA_GAME]);
    b.budget = Budget::of(8);
    let t = Instant::now();
    let r = h
        .memory
        .query_topic(
            &b.caller,
            &TopicQuery::new(
                b.obs.query_scope().with_title("调整游戏背景效果"),
                b.budget.clone(),
            ),
        )
        .unwrap();
    let query_ms = ms(t);
    let hint_bytes = render_topic(&Names::default(), "调整游戏背景效果", &r).len();
    b.obs.accept_recall(&r, &b.caller, h.memory.now_ms());
    let w = h.session("W", "ui-w", &[U1, A1], &[U1], &[ALPHA_GAME]);
    let (mut fast, mut slow, mut fast_ms, mut slow_ms) = (0usize, 0usize, 0f64, 0f64);
    for i in 0..100 {
        if i % 10 == 0 {
            let e = h.event(&format!("W/e{i}"), "ui-w", ActorKind::User, "x");
            h.memory
                .record_perception(
                    &w.caller,
                    &w.lease,
                    observation("新的背景观察", e, &[ALPHA_GAME], &format!("W/e{i}/1")),
                )
                .unwrap();
        }
        let t = Instant::now();
        let d = observe(&h.memory, &b.caller, &b.obs, &b.budget, &b.cfg).unwrap();
        let elapsed = ms(t);
        if d.fast_path {
            fast += 1;
            fast_ms += elapsed;
        } else {
            slow += 1;
            slow_ms += elapsed;
        }
        b.obs.accept(&d, &b.cfg);
    }
    let log = std::fs::metadata(root.join("memory/.meta/occasions.jsonl"))
        .unwrap()
        .len();
    let t = Instant::now();
    let fresh = AgentMemory::open_read(AgentMemoryConfig::new(root.join("memory"))).unwrap();
    let view = fresh.view().unwrap();
    let replay_ms = ms(t);
    println!(
        "Memory component cost baseline ({} perceptions, {} sessions)",
        writes, SESSIONS
    );
    println!(
        "  perception write           {:>8.3} ms avg",
        write_ms / writes as f64
    );
    println!(
        "  consolidation commit       {:>8.3} ms avg over {commits} commits (batch sizes {:?})",
        commit_ms / commits.max(1) as f64,
        batch_sizes.iter().max()
    );
    println!("  cleanup                    {cleanup_ms:>8.3} ms for {cleaned} records");
    println!("  query_topic                {:>8.3} ms, {} cognitions + {} perceptions delivered, {} omitted, hint block {} bytes", query_ms, r.cognitions.len(), r.perceptions.len(), r.omitted.len(), hint_bytes);
    println!(
        "  changes_since fast path    {:>8.3} ms avg, hit rate {}/{}",
        fast_ms / fast.max(1) as f64,
        fast,
        fast + slow
    );
    println!(
        "  changes_since slow path    {:>8.3} ms avg",
        slow_ms / slow.max(1) as f64
    );
    println!(
        "  occasion log               {} occasions, {} bytes",
        view.seq(),
        log
    );
    println!("  full replay (cold open)    {:>8.3} ms", replay_ms);
}
