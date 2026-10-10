//! L6: the reference runner behaves as each fixture's `expected.json` says
//! (the same check other language runners run against the fixtures).

mod common;
#[path = "../examples/support/fixture_paths.rs"]
mod fixture_paths;
#[path = "../examples/support/input_fixture.rs"]
mod input_fixture;

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
                    text("<response><report end=\"true\"><![CDATA[final]]></report></response>")
                } else {
                    text("<response><report end=\"true\"><![CDATA[research result]]></report></response>")
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
        // The snapshot message comes first, the controlled input after it.
        let users = user_texts(req);
        assert_eq!(users.len(), 2, "{users:?}");
        assert!(users[0].starts_with("<semi_subscription_snapshot>"), "{users:?}");
        assert!(users[0].contains("work-fixture-sub-a is finished"), "{users:?}");
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

#[tokio::test]
async fn f15_report_pending_commit() {
    let (_t, env, expected) = load("15_report_pending_commit");
    let sd = session(&env, "work-fixture-report");
    let before = sd.state().unwrap();
    assert!(!before.is_finished());
    let run_id = before.live_run.as_ref().unwrap().run_id.clone();
    let record = sd.runs().record(&run_id).unwrap();
    assert!(!record.status.is_terminal());
    let submitted: ReportSubmission = serde_json::from_value(
        record.host.as_ref().unwrap().extra["reports"][0].clone(),
    ).unwrap();
    assert!(submitted.is_end);
    assert_eq!(submitted.report, expected["next"]["final_report"]);
    let llm = ScriptedLlm::new(|_, _| panic!("accepted final report recovery must not infer"));
    let deps = fdeps(&env, llm.clone());
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    assert_eq!(llm.count(), expected["next"]["llm_calls"].as_u64().unwrap() as usize);
    let state = sd.state().unwrap();
    assert_eq!(state.outcome, Some(Outcome::Succeeded));
    assert_eq!(state.acceptance, Acceptance::Pending);
    assert_eq!(state.last_run.as_deref(), Some(run_id.as_str()));
    assert_eq!(state.final_report.as_ref(), Some(&submitted));
    assert_eq!(submitted.result.as_ref(), Some(&expected["next"]["result"]));
    assert_eq!(submitted.artifacts.len(), 1);
    let artifact = &submitted.artifacts[0];
    assert_eq!(artifact.path, expected["next"]["artifact"]["path"]);
    assert_eq!(std::fs::read_to_string(sd.path().join(&artifact.reference)).unwrap(),
        expected["next"]["artifact"]["content"]);
    let entries = read_worklog(&sd);
    let deliveries: Vec<_> = entries.iter().filter_map(|e| match &e.body {
        WorklogBody::ReportDelivery { submission, assistant, .. } => Some((submission, assistant)),
        _ => None,
    }).collect();
    assert_eq!(deliveries.len(), expected["next"]["report_deliveries"].as_u64().unwrap() as usize);
    assert_eq!(deliveries[0].0, &submitted);
    assert_eq!(deliveries[0].1, &submitted.delivery_text());
    assert_eq!(entries.iter().filter(|e| matches!(&e.body,
        WorklogBody::ActionResult { call_id, status, .. } if call_id == "report-final" && status == "ok"
    )).count(), 1);
    assert_eq!(sd.statistics().unwrap().rounds, expected["next"]["rounds_after"].as_u64().unwrap());
    assert!(drive(&sd, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(read_worklog(&sd), entries);
}

/// The input bus fixture: every stored record is handled as `expected.json`
/// says, and the renderings are reproduced byte for byte.
#[tokio::test]
async fn f14_input_bus() {
    use libopendan::runner::input_view::{render_event_xml, render_msg_xml};
    use libopendan::runner::render_template;
    let dir = fixtures_dir().join("14_input_bus");
    let read = |rel: &str| std::fs::read_to_string(dir.join(rel)).unwrap();
    let json_of = |rel: &str| -> Value { serde_json::from_str(&read(rel)).unwrap() };
    let expected = json_of("expected.json");
    // Accepted records parse to their declared type, and a producer that
    // re-posts them (CLI `post --json`) gets the same record.
    let mut records = Vec::new();
    for r in expected["records"].as_array().unwrap() {
        let file = r["file"].as_str().unwrap();
        let rec = json_of(&format!("records/{file}"));
        let input = parse_logical_record(&rec).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(input.type_name(), r["type"].as_str().unwrap(), "{file}");
        let posted = PostedInput::from_json(rec.clone(), "app:any@alice").unwrap();
        assert_eq!(serde_json::to_value(&posted).unwrap(), rec, "{file}");
        if let SessionInput::Msg(m) = &input {
            assert_eq!(msg_key(&m.msg), rec["key"].as_str().unwrap(), "{file}");
            let reply = serde_json::to_value(ReplyRoute::of_msg(&posted.key, m)).unwrap();
            for (k, v) in r["reply_after"].as_object().unwrap() {
                assert_eq!(&reply[k], v, "{file} reply.{k}");
            }
        }
        records.push((file.to_string(), rec));
    }
    // Rejections are deterministic; posting refuses what consuming rejects.
    for r in expected["rejected"].as_array().unwrap() {
        let file = r["file"].as_str().unwrap();
        let rec = json_of(&format!("rejected/{file}"));
        let err = parse_logical_record(&rec).expect_err(file);
        assert_eq!(err.reason.as_str(), r["reason"].as_str().unwrap(), "{file}: {err}");
        // (`post --json` fills in an omitted `from` / `at_ms` / `schema`.)
        if !file.contains("missing_from") {
            assert!(PostedInput::from_json(rec, "app:any@alice").is_err(), "{file}");
        }
    }
    let big = parse_logical_record(&input_fixture::too_large()).unwrap_err();
    assert_eq!(big.reason, RejectReason::PayloadTooLarge);
    // Routing of the two events with the fixture's subscriptions.
    let mut cfg_json = serde_json::to_value(
        libopendan::SessionDir::open(
            fixtures_dir().join("01_new_work_session/app/work-fixture-new"),
        )
        .unwrap()
        .config()
        .unwrap(),
    )
    .unwrap();
    cfg_json["subscriptions"] = expected["session"]["subscriptions"].clone();
    let cfg: SessionConfig = serde_json::from_value(cfg_json).unwrap();
    let mode_of = |file: &str| {
        let rec = &records.iter().find(|(n, _)| n == file).unwrap().1;
        let SessionInput::Event(ev) = parse_logical_record(rec).unwrap() else {
            panic!("{file}")
        };
        cfg.subscription_for(
            ev.subscription_id.as_deref(),
            &ev.source.kind,
            &ev.source.id,
            &ev.event,
        )
        .map(|s| s.mode)
    };
    assert_eq!(mode_of("03_event_active_task.json"), Some(SubscriptionMode::Active));
    assert_eq!(mode_of("04_event_semi_object.json"), Some(SubscriptionMode::Semi));
    // Renderings.
    let view = input_fixture::batch_view(&records, HOOK_ON_INPUT);
    assert_eq!(view.text, read("rendering/input_text.xml"));
    let vars = input_fixture::vars(&view);
    assert_eq!(vars, json_of("rendering/vars.json"));
    for t in expected["rendering"]["templates"].as_array().unwrap() {
        let tpl = read(t["template"].as_str().unwrap());
        let out = render_template(&tpl, vars.clone()).await.unwrap();
        assert_eq!(out, read(t["output"].as_str().unwrap()), "{}", t["template"]);
    }
    // `input.xml` is `input.text`; `message.xml` / `event.xml` are its elements.
    let same = render_template("{{ input | render_format: \"input.xml\" }}", vars.clone())
        .await
        .unwrap();
    assert_eq!(same, view.text);
    assert!(view.text.contains(&render_msg_xml(&view.messages[0])));
    assert!(view.text.contains(&render_event_xml(&view.events[0])));
    assert_eq!(
        read("rendering/templates/example_1_builtin_equivalent.out"),
        read("rendering/on_input_builtin.txt")
    );
    // The speaker is `msg.from`, never the poster; text cannot fake structure.
    assert!(view.text.contains("from=\"Bob\""));
    assert!(!view.text.contains("msg-bridge"));
    assert!(view.text.contains("&lt;build&gt;"));
    // input.media: same text; `inline` adds the blocks in attachment order.
    let builtin = read("rendering/on_input_builtin.txt");
    for (name, media) in [("reference", InputMedia::Reference), ("inline", InputMedia::Inline)] {
        let m = input_fixture::user_message(&builtin, &view, media);
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json_of(&format!("rendering/ai_message_{name}.json")),
            "{name}"
        );
    }
    let inline = json_of("rendering/ai_message_inline.json");
    let kinds: Vec<&str> = inline["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["text", "image", "document"]);
    assert_eq!(json_of("rendering/ai_message_reference.json")["content"].as_array().unwrap().len(), 1);
}
