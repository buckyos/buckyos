//! L6: the reference runner behaves as each fixture's `expected.json` says
//! (the same check other language runners run against the fixtures).

mod common;
#[path = "../examples/support/fixture_paths.rs"]
mod fixture_paths;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::SessionDir;
use serde_json::{json, Value};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../doc/opendan/protocol/fixtures")
}

/// Copy a fixture into a temp dir and substitute `${FIXTURE_ROOT}`.
fn load(name: &str) -> (tempfile::TempDir, Env, Value) {
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join(name);
    copy_dir(&fixtures_dir().join(name), &dst, libopendan::now_ms());
    if name == "10_active_overlap" {
        std::fs::create_dir_all(dst.join("ws")).unwrap();
    }
    let root = dst.display().to_string();
    let host = libopendan::runtime::native_host_id();
    let uid = fixture_paths::uid();
    let hostname = fixture_paths::hostname();
    let os = if cfg!(windows) {
        "windows".to_string()
    } else {
        String::from_utf8_lossy(
            &std::process::Command::new("uname")
                .arg("-s")
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_ascii_lowercase()
    };
    fixture_paths::rewrite(
        &dst,
        &[
            ("${FIXTURE_ROOT}", &root),
            ("${FIXTURE_HOST}", &host),
            ("${FIXTURE_UID}", &uid),
            ("${FIXTURE_HOSTNAME}", &hostname),
        ],
    );
    localize(&dst, &root, &os);
    let expected: Value =
        serde_json::from_slice(&std::fs::read(dst.join("expected.json")).unwrap()).unwrap();
    let env = Env {
        root: dst.clone(),
        agent_root: dst.join("agent_root"),
        app_dir: dst.join("app"),
        queue_dir: dst.join("kmsg"),
        ..Env::at(&dst)
    };
    (tmp, env, expected)
}

fn localize(dir: &Path, root: &str, os: &str) {
    fn value(v: &mut Value, root: &str, os: &str, canonical: bool) {
        match v {
            Value::Object(map) => {
                if map.get("os").and_then(Value::as_str) == Some("linux") {
                    map.insert("os".into(), os.into());
                }
                for (key, v) in map.iter_mut() {
                    value(v, root, os, key == "workdir" || key == "cwd");
                }
            }
            Value::Array(values) => {
                for v in values {
                    value(v, root, os, canonical);
                }
            }
            Value::String(path) if cfg!(windows) && path.starts_with(root) => {
                let native: PathBuf = Path::new(path).components().collect();
                let native = if canonical {
                    native.canonicalize().unwrap_or(native)
                } else {
                    native
                };
                *path = native.display().to_string();
            }
            _ => {}
        }
    }
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            localize(&path, root, os);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            value(&mut v, root, os, false);
            std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        }
    }
}

fn copy_dir(src: &Path, dst: &Path, started_at_ms: u64) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        let t = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t, started_at_ms);
        } else {
            std::fs::copy(&p, &t).unwrap();
            if src.file_name().is_some_and(|n| n == "snapshots")
                && p.extension().is_some_and(|n| n == "json")
            {
                let mut snapshot: Value =
                    serde_json::from_slice(&std::fs::read(&t).unwrap()).unwrap();
                snapshot["state"]["started_at_ms"] = started_at_ms.into();
                std::fs::write(&t, serde_json::to_vec(&snapshot).unwrap()).unwrap();
            }
            #[cfg(unix)]
            {
                let mode = std::fs::metadata(&p).unwrap().permissions();
                std::fs::set_permissions(&t, mode).unwrap();
            }
        }
    }
}

/// Runner deps on the runtime the fixtures were bound to.
fn fdeps(env: &Env, llm: Arc<ScriptedLlm>) -> libopendan::runner::RunnerDeps {
    let mut d = env.deps(llm);
    d.runtime = Arc::new(libopendan::runtime::NativeRuntime::local(
        "rt-fixture-native",
        "app:app2",
    ));
    d
}

fn session(env: &Env, sid: &str) -> SessionDir {
    SessionDir::open(env.app_dir.join(sid)).unwrap()
}

fn script(name: &'static str) -> Arc<ScriptedLlm> {
    ScriptedLlm::new(move |req, _| {
        let all = render(&req.messages);
        match name {
            "tool_then_answer" => {
                if has_tool_result(req, "c1").is_some() {
                    text("done: notes.txt written")
                } else {
                    tool_call("c1", "shell", json!({ "command": "echo note > notes.txt" }))
                }
            }
            "long_exec" => {
                let r = has_tool_result(req, "c1").expect("resumed with c1 materialized");
                assert!(r.contains("unknown"), "{r}");
                text("recovered")
            }
            "transient" => {
                assert_eq!(all.matches("second message").count(), 1, "{all}");
                text("got both")
            }
            "fork" => {
                if all.contains("research result") {
                    text("<response><report><![CDATA[final]]></report></response>")
                } else {
                    text("<response><report><![CDATA[research result]]></report></response>")
                }
            }
            _ => text("answer"),
        }
    })
}

#[tokio::test]
async fn f01_new_work_session() {
    let (_t, env, e) = load("01_new_work_session");
    assert_eq!(e["next"]["action"], "bind_runtime_then_input_batch");
    let sd = session(&env, "work-fixture-new");
    assert!(sd.binding_opt().unwrap().is_none());
    let llm = script("answer");
    let r = drive(&sd, &fdeps(&env, llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(llm.transcript(0).contains("on_init"));
    assert!(sd.binding_opt().unwrap().is_some());
}

#[tokio::test]
async fn f02_finished_work_session() {
    let (_t, env, _) = load("02_finished_work_session");
    let sd = session(&env, "work-fixture-finished");
    let llm = script("answer");
    assert!(drive(&sd, &fdeps(&env, llm.clone()), StopWhen::Idle)
        .await
        .is_finished());
    assert_eq!(llm.count(), 0);
}

#[tokio::test]
async fn f03_orphan_run() {
    let (_t, env, _) = load("03_orphan_run_pending_host_commit");
    let sd = session(&env, "work-fixture-orphan");
    let orphan = sd.runs().list().unwrap();
    assert_eq!(orphan.len(), 1);
    assert!(drive(
        &sd,
        &fdeps(&env, script("tool_then_answer")),
        StopWhen::Finished
    )
    .await
    .is_finished());
    let st = sd.state().unwrap();
    assert_eq!(
        sd.runs().list().unwrap(),
        vec![st.last_run.clone().unwrap()]
    );
    assert_ne!(
        st.last_run.clone().unwrap(),
        orphan[0],
        "orphan removed, new run used"
    );
    assert_eq!(st.source("q").acked_index, 1);
}

#[tokio::test]
async fn f04_gate_pending() {
    let (_t, env, _) = load("04_gate_pending_after_state_commit");
    let sd = session(&env, "work-fixture-gate");
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    assert!(drive(
        &sd,
        &fdeps(&env, script("tool_then_answer")),
        StopWhen::Finished
    )
    .await
    .is_finished());
    let st = sd.state().unwrap();
    assert_eq!(
        st.last_run.as_deref(),
        Some(run_id.as_str()),
        "same run resumed"
    );
    assert_eq!(st.source("q").acked_index, 1);
    let users = read_worklog(&sd)
        .iter()
        .filter(|e| e.body.kind() == "user_message")
        .count();
    assert_eq!(users, 1);
}

#[tokio::test]
async fn f05_uncommitted_tail() {
    let (_t, env, _) = load("05_uncommitted_worklog_tail");
    let sd = session(&env, "work-fixture-tail");
    let llm = script("tool_then_answer");
    assert!(drive(&sd, &fdeps(&env, llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(llm.count(), 0, "finish redone from the terminal record");
    let outcomes = read_worklog(&sd)
        .iter()
        .filter(|e| e.body.kind() == "outcome")
        .count();
    assert_eq!(outcomes, 1);
}

#[tokio::test]
async fn f06_killed_during_exec() {
    let (_t, env, _) = load("06_killed_during_exec");
    let sd = session(&env, "work-fixture-kill");
    let llm = script("long_exec");
    assert!(drive(&sd, &fdeps(&env, llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    let marker = std::fs::read_to_string(sd.path().join("marker")).unwrap();
    assert_eq!(marker.matches("start").count(), 1, "tool not re-run");
}

#[tokio::test]
async fn f07_receipt_ahead_of_state() {
    let (_t, env, _) = load("07_receipt_ahead_of_state");
    let sd = session(&env, "work-fixture-receipt");
    let llm = script("transient");
    assert!(drive(&sd, &fdeps(&env, llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn f08_finished_with_decide() {
    let (_t, env, e) = load("08_finished_with_decide");
    let sd = session(&env, "work-fixture-decide");
    assert!(drive(&sd, &fdeps(&env, script("answer")), StopWhen::Idle)
        .await
        .is_finished());
    assert_eq!(sd.state().unwrap().acceptance, Acceptance::Accepted);
    let head = env.agent().artifacts().head("demo").await.unwrap().unwrap();
    assert_eq!(
        head.head.as_deref(),
        e["next"]["artifact_head_after"].as_str()
    );
    assert_eq!(
        read_worklog(&sd).last().unwrap().body.kind(),
        "input_rejected"
    );
}

#[tokio::test]
async fn f09_semi_subscription() {
    let (_t, env, _) = load("09_semi_subscription");
    let b = session(&env, "work-fixture-sub-b");
    let llm = ScriptedLlm::new(|req, _| {
        let u = last_user_text(req);
        assert!(u.contains("work-fixture-sub-a is finished"), "{u}");
        text("seen")
    });
    assert!(drive(&b, &fdeps(&env, llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(llm.count(), 1);
}

#[tokio::test]
async fn f10_active_overlap() {
    let (_t, env, _) = load("10_active_overlap");
    let b = session(&env, "work-fixture-active-b");
    let llm = ScriptedLlm::new(|req, _| {
        let u = last_user_text(req);
        assert!(
            u.contains("work-fixture-active-a") && u.contains("same_target"),
            "{u}"
        );
        text("avoid")
    });
    let result = drive(&b, &fdeps(&env, llm), StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
}

#[tokio::test]
async fn f11_worklog_with_summary() {
    let (_t, env, e) = load("11_worklog_with_summary");
    let sd = session(&env, "work-fixture-summary");
    let lease = match sd.acquire(fdeps(&env, script("answer")).holder()).unwrap() {
        libopendan::lock::Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    let s = sd.load(&lease).unwrap();
    let sm = s.summary().unwrap();
    assert_eq!(
        sm.start_offset,
        e["next"]["stop_at_offset"].as_u64().unwrap()
    );
    let w = libopendan::runner::history::read_window(&s, &sm, 100_000).unwrap();
    assert!(w.reached_start);
    assert_eq!(
        w.lines.len() as u64,
        e["next"]["raw_entries"].as_u64().unwrap()
    );
}

#[tokio::test]
async fn f12_fork_child_live() {
    let (_t, env, _) = load("12_fork_child_live");
    let sd = session(&env, "work-fixture-fork");
    assert_eq!(sd.state().unwrap().process_stack.len(), 1);
    let r = drive(&sd, &fdeps(&env, script("fork")), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(sd.state().unwrap().process_stack.is_empty());
    assert!(sd.report().unwrap().contains("final"));
}

#[tokio::test]
async fn f13_unsupported_snapshot_version() {
    let (_t, env, _) = load("13_unsupported_snapshot_version");
    let sd = session(&env, "work-fixture-blocked");
    let llm = script("answer");
    assert!(matches!(
        drive(&sd, &fdeps(&env, llm.clone()), StopWhen::Finished).await,
        DriveResult::RecoveryBlocked(_)
    ));
    assert_eq!(llm.count(), 0);
}
