//! The Memory multi-Session scenarios (component stage), run against real
//! storage in a temp Agent root. Every check recorded by a scenario must
//! pass; the same scenarios print their trace through
//! `cargo run -p libopendan --example memory_sessions`.

#[path = "../examples/support/memory_scenarios.rs"]
mod memory_scenarios;
#[path = "../examples/support/memory_sim.rs"]
mod memory_sim;

use memory_scenarios::{Scenario, SCENARIOS};
use memory_sim::Host;

fn run(name: &str) {
    let scenario: Scenario = SCENARIOS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f)
        .expect("scenario");
    let tmp = tempfile::TempDir::new().unwrap();
    let mut host = Host::new(&tmp.path().join(name), "2026-10-08T09:00:00+08:00");
    scenario(&mut host);
    assert!(!host.trace.checks.is_empty(), "{name} recorded no checks");
    let failed: Vec<String> = host
        .trace
        .failed()
        .iter()
        .map(|c| format!("{} {}", c.id, c.detail))
        .collect();
    assert!(
        failed.is_empty(),
        "{name}: failed checks:\n{}\n\ntrace:\n{}",
        failed.join("\n"),
        host.trace.lines.join("\n")
    );
}

#[test]
fn main_flow() {
    run("main_flow");
}

#[test]
fn explicit_request() {
    run("explicit_request");
}

#[test]
fn correction_fallback() {
    run("correction_fallback");
}

#[test]
fn tool_experience() {
    run("tool_experience");
}

#[test]
fn world_selection() {
    run("world_selection");
}

#[test]
fn task_revision_boundary() {
    run("task_revision_boundary");
}

#[test]
fn read_set_topic_switch() {
    run("read_set_topic_switch");
}

#[test]
fn deferral_expiry() {
    run("deferral_expiry");
}

#[test]
fn ambiguity_and_time() {
    run("ambiguity_and_time");
}

#[test]
fn mixed_batch() {
    run("mixed_batch");
}

#[test]
fn every_scenario_has_a_test() {
    let here = include_str!("memory_sessions.rs");
    for (name, _) in SCENARIOS {
        assert!(
            here.contains(&format!("run(\"{name}\")")),
            "no test for {name}"
        );
    }
}
