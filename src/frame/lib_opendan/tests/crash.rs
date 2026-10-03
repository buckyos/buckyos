//! A-03 / §8.3 crash windows: a child process drives the session and dies
//! (fault injection = abort, or `kill -9` while a tool runs); the parent
//! recovers in-process and checks the protocol invariants.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::SessionDir;
use llm_context::deps::LlmInferenceRequest;
use serde_json::json;

const ENV_ROOT: &str = "LIBOPENDAN_TEST_ROOT";
const ENV_SESSION: &str = "LIBOPENDAN_TEST_SESSION";
const ENV_SCENARIO: &str = "LIBOPENDAN_TEST_SCENARIO";

/// Content based scripts (the parent and the child must agree).
fn script(name: &str) -> Arc<ScriptedLlm> {
    script_for(name, None)
}

fn script_for(name: &str, _ctx: Option<(PathBuf, String)>) -> Arc<ScriptedLlm> {
    match name {
        "transient" => ScriptedLlm::fallible(|req: &LlmInferenceRequest, _| {
            if render(&req.messages).contains("second message") {
                Ok(text("got both"))
            } else {
                Err(llm_context::error::LLMComputeError::Provider {
                    failure: llm_context::error::ProviderFailure::Transient,
                    message: "busy".into(),
                })
            }
        }),
        "simple_tool" => ScriptedLlm::new(|req: &LlmInferenceRequest, _| {
            if has_tool_result(req, "c1").is_some() {
                text("all done")
            } else {
                tool_call("c1", "shell", json!({ "command": "echo step >> steps.log" }))
            }
        }),
        "exec_sleep" => {
            ScriptedLlm::new(
                |req: &LlmInferenceRequest, _| match has_tool_result(req, "c1") {
                    Some(r) => text(&format!("recovered: {r}")),
                    None => tool_call(
                        "c1",
                        "shell",
                        json!({ "command": "echo start >> marker; sleep 30; echo done >> marker" }),
                    ),
                },
            )
        }
        "context_limit" => ScriptedLlm::fallible(|req: &LlmInferenceRequest, _| {
            let all = render(&req.messages);
            if has_tool_result(req, "c2").is_some() {
                Ok(text("all done"))
            } else if has_tool_result(req, "c1").is_some() {
                Err(llm_context::error::LLMComputeError::Provider {
                    failure: llm_context::error::ProviderFailure::ContextLimit,
                    message: "context_length_exceeded".into(),
                })
            } else if all.contains("<session_history>") && all.contains("[result #c1") {
                Ok(tool_call(
                    "c2",
                    "shell",
                    json!({ "command": "echo two >> steps.log" }),
                ))
            } else {
                Ok(tool_call(
                    "c1",
                    "shell",
                    json!({ "command": "echo one >> steps.log" }),
                ))
            }
        }),
        "answer" => ScriptedLlm::new(|_, _| text("plain answer")),
        "wait" => ScriptedLlm::new(|_, _| {
            text("<response><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        }),
        "switch" => ScriptedLlm::new(|req: &LlmInferenceRequest, _| {
            if render(&req.messages).contains("context_switch to=\"do\"") {
                text("<response><report><![CDATA[both phases done]]></report></response>")
            } else {
                text("<response><next_behavior>do</next_behavior></response>")
            }
        }),
        "fork" => ScriptedLlm::new(|req: &LlmInferenceRequest, _| {
            let all = render(&req.messages);
            if all.contains("research result X") {
                text("<response><report><![CDATA[final answer]]></report></response>")
            } else if all.contains("context_switch to=\"research\"") {
                text("<response><report><![CDATA[research result X]]></report></response>")
            } else {
                text("<response><next_behavior>research</next_behavior></response>")
            }
        }),
        "subcall" => ScriptedLlm::new(|req: &LlmInferenceRequest, _| {
            if has_tool_result(req, "f1").is_some() {
                text("final answer")
            } else if last_user_text(req).contains("<sub_task") {
                text("X is 42")
            } else {
                tool_call(
                    "f1",
                    "call_behavior",
                    json!({ "behavior": "research", "task": "find X" }),
                )
            }
        }),
        other => panic!("unknown scenario {other}"),
    }
}

/// Child entry point: a no-op unless spawned by `spawn_child`.
#[tokio::test]
async fn child_driver() {
    let (Ok(root), Ok(session), Ok(scenario)) = (
        std::env::var(ENV_ROOT),
        std::env::var(ENV_SESSION),
        std::env::var(ENV_SCENARIO),
    ) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    let sd = SessionDir::open(&session).unwrap();
    let deps = env.deps(script_for(
        &scenario,
        Some((env.queue_dir.clone(), queue_of(&sd))),
    ));
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    eprintln!("child drive result: {r:?}");
}

fn spawn_child(env: &Env, sd: &SessionDir, scenario: &str, fault: Option<&str>) -> Child {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "child_driver", "--nocapture", "--test-threads=1"])
        .env(ENV_ROOT, &env.root)
        .env(ENV_SESSION, sd.path())
        .env(ENV_SCENARIO, scenario)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(f) = fault {
        cmd.env("LIBOPENDAN_FAULT", f);
    } else {
        cmd.env_remove("LIBOPENDAN_FAULT");
    }
    cmd.spawn().unwrap()
}

fn wait_exit(child: &mut Child, max: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        if start.elapsed() > max {
            let _ = child.kill();
            panic!("child did not exit in time");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_worklog_contiguous(sd: &SessionDir) {
    let wl = read_worklog(sd);
    for (i, e) in wl.iter().enumerate() {
        assert_eq!(e.seq, i as u64 + 1, "worklog seq gap / duplicate: {wl:#?}");
    }
    let st = sd.state().unwrap();
    assert_eq!(st.worklog.committed_seq, wl.len() as u64);
    assert_eq!(
        st.worklog.committed_bytes,
        std::fs::metadata(sd.worklog().path()).unwrap().len()
    );
}

fn count_kind(sd: &SessionDir, kind: &str) -> usize {
    read_worklog(sd)
        .iter()
        .filter(|e| e.body.kind() == kind)
        .count()
}

async fn post_msg(env: &Env, sd: &SessionDir, _key: &str, text: &str) {
    let ch = env.channels();
    let q = sd.config().unwrap().channels.kmsg().unwrap().1.to_string();
    libopendan::channel::kmsg::post_to_queue(&ch.client(), &q, &msg(text))
        .await
        .unwrap();
}

/// Crash at a commit window of the first input batch, then recover
/// in-process.
async fn crash_window(fault: &str) -> (Env, SessionDir, Arc<ScriptedLlm>) {
    let env = Env::new();
    let sd = env
        .create_work(work_spec("append a line to steps.log"))
        .await;
    post_msg(&env, &sd, "m-1", "please do it").await;
    let mut child = spawn_child(&env, &sd, "simple_tool", Some(fault));
    let st = wait_exit(&mut child, Duration::from_secs(60));
    assert!(!st.success(), "child must die at {fault}");
    let llm = script("simple_tool");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{fault}: {r:?}");
    assert_worklog_contiguous(&sd);
    // Exactly one Turn with exactly one copy of its input message.
    assert_eq!(count_kind(&sd, "turn_started"), 1, "{fault}");
    assert_eq!(count_kind(&sd, "turn_ended"), 1, "{fault}");
    assert_eq!(count_kind(&sd, "user_message"), 1, "{fault}");
    if llm.count() > 0 {
        let transcript = llm.transcript(llm.count() - 1);
        assert_eq!(
            transcript.matches("please do it").count(),
            1,
            "{fault}: {transcript}"
        );
    }
    let msg = read_worklog(&sd)
        .into_iter()
        .find_map(|e| match e.body {
            WorklogBody::UserMessage { content, .. } => Some(content),
            _ => None,
        })
        .unwrap();
    assert_eq!(msg.matches("please do it").count(), 1, "{fault}: {msg}");
    // The input was consumed once and acknowledged.
    let st = sd.state().unwrap();
    assert_eq!(st.source("q").acked_index, 1, "{fault}");
    let q = libopendan::channel::DirMsgQueue::new(&env.queue_dir).unwrap();
    let sub = match &sd.config().unwrap().channels.inputs[0] {
        InputSourceConfig::Kmsg { subscriber, .. } => subscriber.clone(),
    };
    assert_eq!(q.cursor(&sub), Some(2), "{fault}: kmsg ack");
    // Only the last run survives.
    let runs = sd.runs().list().unwrap();
    assert_eq!(runs, vec![st.last_run.clone().unwrap()], "{fault}");
    (env, sd, llm)
}

#[tokio::test]
async fn crash_after_input_checkpoint_of_new_run() {
    // Orphan run (gate set, state never referenced it): removed, the input
    // is fetched again and processed once.
    let (_env, sd, _llm) = crash_window("input_batch:after_input_checkpoint").await;
    assert!(std::fs::read_to_string(sd.path().join("steps.log")).is_ok());
}

#[tokio::test]
async fn crash_after_state_commit_before_gate_clear() {
    crash_window("input_batch:after_state_commit").await;
}

#[tokio::test]
async fn crash_after_gate_clear_before_confirm() {
    crash_window("input_batch:after_gate_clear").await;
}

#[tokio::test]
async fn crash_after_flush_before_commit() {
    // The flushed tail was never committed: it is truncated and rewritten
    // once from the kept run.
    let (_env, sd, _) = crash_window("finish_run:after_flush").await;
    assert_eq!(count_kind(&sd, "outcome"), 1);
    assert_eq!(count_kind(&sd, "assistant_message"), 2);
    assert_eq!(
        count_kind(&sd, "step"),
        0,
        "function call responses are not Steps"
    );
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
}

#[tokio::test]
async fn crash_after_finish_commit() {
    let (env, sd, llm) = crash_window("finish_run:after_commit").await;
    // Finished by the child; the parent only caught up the reports.
    assert_eq!(llm.count(), 0);
    let agent = env.agent();
    use libopendan::state::AgentStateClient;
    let e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert_eq!(e.status.run_state, RunState::Finished);
    assert_eq!(e.status.rev, sd.state().unwrap().rev);
    assert!(agent.perception().last_seq(sd.sid()).await.unwrap() >= 2);
}

/// Crash inside a mid-run context-limit rewrite, then recover in-process:
/// no tool runs twice, every record is in the worklog once.
async fn context_limit_crash(fault: &str) {
    let env = Env::new();
    let sd = env.create_work(work_spec("two tool steps")).await;
    let mut child = spawn_child(&env, &sd, "context_limit", Some(fault));
    let st = wait_exit(&mut child, Duration::from_secs(60));
    assert!(!st.success(), "child must die at {fault}");
    let llm = script("context_limit");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{fault}: {r:?}");
    assert_worklog_contiguous(&sd);
    let log = std::fs::read_to_string(sd.path().join("steps.log")).unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec!["one", "two"],
        "{fault}"
    );
    assert_eq!(count_kind(&sd, "turn_started"), 1, "{fault}");
    assert_eq!(count_kind(&sd, "user_message"), 1, "{fault}");
    assert_eq!(count_kind(&sd, "assistant_message"), 3, "{fault}");
    let results: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::ActionResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(results, vec!["c1", "c2"], "{fault}");
    let rewritten = read_worklog(&sd)
        .iter()
        .filter(
            |e| matches!(&e.body, WorklogBody::Outcome { kind, .. } if kind == "context_rewritten"),
        )
        .count();
    assert_eq!(rewritten, 1, "{fault}");
    let st = sd.state().unwrap();
    assert_eq!(
        sd.runs().list().unwrap(),
        vec![st.last_run.clone().unwrap()],
        "{fault}"
    );
}

#[tokio::test]
async fn crash_after_context_limit_flush() {
    context_limit_crash("context_limit:after_flush").await;
}

#[tokio::test]
async fn crash_after_context_limit_compaction() {
    context_limit_crash("context_limit:after_compact").await;
}

#[tokio::test]
async fn crash_after_context_limit_rewrite_published() {
    context_limit_crash("context_limit:after_publish").await;
}

#[tokio::test]
async fn kill_9_during_shell_reports_interrupted_result_and_leaves_the_command_alone() {
    let env = Env::new();
    let sd = env.create_work(work_spec("long running command")).await;
    let mut child = spawn_child(&env, &sd, "exec_sleep", None);
    // Wait until the command started.
    let marker = sd.path().join("marker");
    let start = Instant::now();
    while !std::fs::read_to_string(&marker)
        .unwrap_or_default()
        .contains("start")
    {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "tool never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    let _ = child.wait();
    // The runner is gone; its in-flight record is not.
    let st = sd.state().unwrap();
    let live = st.live_run.clone().expect("live run");
    let rec = sd.runs().record(&live.run_id).unwrap();
    assert_eq!(
        rec.inflight.len(),
        1,
        "inflight persisted before the tool ran"
    );
    // Same identity, other process: takes over without probing or killing
    // anything (standard process semantics, long-tool TODO §3.2).
    let llm = script("exec_sleep");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    // The tool was not re-run; its result says the executor exited while
    // it ran and leaves the judgement to the model.
    let m = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(m.matches("start").count(), 1, "{m}");
    let result = has_tool_result(&llm.requests.lock().unwrap()[0], "c1").unwrap();
    assert!(result.contains("result unknown"), "{result}");
    assert!(result.contains("previous executor exited"), "{result}");
    assert!(result.contains("shell (native)"), "{result}");
    assert_eq!(llm.count(), 1);
    let wl = read_worklog(&sd);
    assert!(wl.iter().any(|e| matches!(&e.body,
        WorklogBody::ActionResult { status, .. } if status == "unresolved")));
    assert_worklog_contiguous(&sd);
    let last = sd.state().unwrap().last_run.unwrap();
    let rec = sd.runs().record(&last).unwrap();
    assert!(rec.inflight.is_empty());
    // Leave no process behind in the test environment: the orphaned
    // command belongs to nobody, so the test kills it itself.
    let _ = Command::new("pkill")
        .args(["-KILL", "-f", &format!("{}", marker.display())])
        .status();
}

#[tokio::test]
async fn unsupported_snapshot_version_blocks_recovery_and_keeps_everything() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let mut child = spawn_child(
        &env,
        &sd,
        "simple_tool",
        Some("input_batch:after_gate_clear"),
    );
    wait_exit(&mut child, Duration::from_secs(60));
    let st = sd.state().unwrap();
    let run_id = st.live_run.clone().unwrap().run_id;
    let rec = sd.runs().record(&run_id).unwrap();
    let idx = rec.latest_snapshot_idx.unwrap();
    let snap_path: PathBuf = sd
        .runs_dir()
        .join(&run_id)
        .join("snapshots")
        .join(format!("{idx:04}.json"));
    let mut v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snap_path).unwrap()).unwrap();
    v["state"]["snapshot_version"] = json!(99);
    std::fs::write(&snap_path, serde_json::to_vec(&v).unwrap()).unwrap();
    let llm = script("simple_tool");
    match drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await {
        DriveResult::RecoveryBlocked(b) => assert!(b.reason.contains("not supported"), "{b:?}"),
        r => panic!("{r:?}"),
    }
    assert_eq!(llm.count(), 0, "no inference");
    let after = sd.state().unwrap();
    assert_eq!(after.live_run, st.live_run, "live run kept");
    assert_eq!(after.inputs, st.inputs, "consumption untouched");
    assert!(after.last_error.is_some());
    assert!(sd.runs().exists(&run_id));
    // Repair → retry works.
    v["state"]["snapshot_version"] = json!(llm_context::SNAPSHOT_FORMAT_VERSION);
    std::fs::write(&snap_path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(drive(&sd, &env.deps(llm), StopWhen::Finished)
        .await
        .is_finished());
}

#[tokio::test]
async fn xllm_takes_over_a_native_run_and_drive_writes_back() {
    use agent_tool::xllm::{ResumeLimits, ResumeStart, RunStatus, RunStore, XllmDeps, XllmRun};
    let env = Env::new();
    let sd = env.create_work(work_spec("answer")).await;
    let mut child = spawn_child(&env, &sd, "answer", Some("input_batch:after_gate_clear"));
    wait_exit(&mut child, Duration::from_secs(60));
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let store = RunStore::disk(sd.runs_dir());
    let llm = ScriptedLlm::new(|req, _| {
        if let Some(result) = has_tool_result(req, "helper-check") {
            assert!(result.contains("runtime-bin"), "{result}");
            assert!(!result.contains("missing"), "{result}");
            text("plain answer")
        } else {
            tool_call(
                "helper-check",
                "shell",
                json!({"command": if cfg!(windows) { r#"if test "${PATH%%:*}" -ef "$(cygpath -u "$OPENDAN_SESSION_DIR")/.runtime/bin"; then echo runtime-bin; else printf 'missing PATH=%s DIR=%s\n' "$PATH" "$OPENDAN_SESSION_DIR"; exit 1; fi; test -n "$OPENDAN_RUNTIME_ID""# } else { r#"case $PATH in *"$OPENDAN_SESSION_DIR/.runtime/bin"*) echo runtime-bin;; *) echo missing; exit 1;; esac; test -n "$OPENDAN_RUNTIME_ID""# }}),
            )
        }
    });
    let manifest_path = sd.runtime_bin_dir().join(".manifest.json");
    let manifest = std::fs::read(&manifest_path).unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    changed["entries"]["changed-helper"] = json!("bad");
    std::fs::write(&manifest_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let deps = XllmDeps::default().with_llm(llm.clone());
    let rejected = XllmRun::resume(
        &store,
        Some(&run_id),
        None,
        ResumeLimits::default(),
        deps.clone(),
    )
    .await;
    assert!(matches!(
        rejected,
        Err(agent_tool::xllm::XllmError::RecoveryBlocked(_))
    ));
    assert_eq!(llm.count(), 0);
    std::fs::write(&manifest_path, manifest).unwrap();
    let started = XllmRun::resume(&store, Some(&run_id), None, ResumeLimits::default(), deps)
        .await
        .unwrap();
    let ResumeStart::Run(mut run) = started else {
        panic!("terminal?")
    };
    let out = run.execute().await.unwrap();
    assert_eq!(out.status(), RunStatus::Completed);
    drop(run);
    assert_eq!(llm.count(), 2);
    // xllm never touched state.json / worklog.
    assert!(sd.state().unwrap().live_run.is_some());
    let r = drive(&sd, &env.deps(script("answer")), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(sd.report().unwrap().contains("plain answer"));
    assert_eq!(count_kind(&sd, "outcome"), 1);
    assert_worklog_contiguous(&sd);
}

#[tokio::test]
async fn xllm_refuses_run_with_pending_host_commit() {
    use agent_tool::xllm::{ResumeLimits, RunStore, XllmDeps, XllmRun};
    let env = Env::new();
    let sd = env.create_work(work_spec("answer")).await;
    let mut child = spawn_child(
        &env,
        &sd,
        "answer",
        Some("input_batch:after_input_checkpoint"),
    );
    wait_exit(&mut child, Duration::from_secs(60));
    let runs = sd.runs().list().unwrap();
    assert_eq!(runs.len(), 1);
    let rec = sd.runs().record(&runs[0]).unwrap();
    assert!(rec.host_commit_pending.is_some());
    let store = RunStore::disk(sd.runs_dir());
    let deps = XllmDeps::default().with_llm(script("answer"));
    let err = XllmRun::resume(&store, Some(&runs[0]), None, ResumeLimits::default(), deps)
        .await
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("not committed"), "{err}");
}

/// A batch of two messages (semi-subscription snapshot + controlled
/// input) committed into the snapshot, crash before state.json: recovery
/// completes state from the receipt — both messages stay once, exactly the
/// injected state version is cleared, the reply path is restored — without
/// rendering again or reading the bus.
#[tokio::test]
async fn crash_after_snapshot_batch_checkpoint_does_not_reinject() {
    let env = Env::new();
    let mut spec = work_spec("watch");
    spec.subscriptions.push(Subscription {
        id: "s2".into(),
        mode: SubscriptionMode::Semi,
        source: SubscriptionSource::ObjectEvent {
            object: "https://cam/01".into(),
            event: String::new(),
        },
        watch: vec![],
    });
    let sd = env.create_work(spec).await;
    match drive(&sd, &env.deps(script("transient")), StopWhen::Finished).await {
        DriveResult::Error { error, .. } => assert_eq!(error["recoverable"], json!(true)),
        r => panic!("{r:?}"),
    }
    let agent = env.agent();
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &event("cam:7", Some("s2"), "object", "https://cam/01", Some(7), "motion at door"),
    )
    .await
    .unwrap();
    let second = msg("second message");
    libopendan::post_input(agent.as_ref(), sd.sid(), &second).await.unwrap();
    let mut child = spawn_child(
        &env,
        &sd,
        "transient",
        Some("input_batch:after_input_checkpoint"),
    );
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    // The event was saved (and its delivery consumed) before the batch; the
    // batch itself is only in the snapshot.
    let st = sd.state().unwrap();
    assert_eq!(st.live_run.clone().unwrap().applied_input_seq, 1);
    assert_eq!(st.pending_events.len(), 1);
    assert_eq!(st.source("q").acked_index, 1);
    assert!(st.reply.is_none());
    // A newer version arrives before the recovery.
    libopendan::post_input(
        agent.as_ref(),
        sd.sid(),
        &event("cam:8", Some("s2"), "object", "https://cam/01", Some(8), "door closed"),
    )
    .await
    .unwrap();
    let llm = script("transient");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert!(r.is_finished(), "{r:?}");
    let t = llm.transcript(llm.count() - 1);
    assert_eq!(t.matches("motion at door").count(), 1, "{t}");
    assert_eq!(t.matches("second message").count(), 1, "{t}");
    assert!(!t.contains("door closed"), "v8 was not part of the committed batch: {t}");
    let st = sd.state().unwrap();
    assert_eq!(st.source("q").acked_index, 3);
    assert_eq!(
        st.reply,
        Some(ReplyRoute::Message {
            to: USER.into(),
            to_session: None,
            kind: "chat".into(),
            reply_to: Some(second.key.clone()),
            tunnel: None,
        })
    );
    let users = read_worklog(&sd)
        .into_iter()
        .filter(|e| matches!(e.body, WorklogBody::UserMessage { .. }))
        .count();
    assert_eq!(users, 3, "on_init, snapshot, on_input");
    assert_eq!(count_kind(&sd, "turn_started"), 1);
    assert_eq!(count_kind(&sd, "input_batch"), 1);
    assert_worklog_contiguous(&sd);
}

#[tokio::test]
async fn new_input_into_a_resumed_run_survives_a_crash_once() {
    let env = Env::new();
    let sd = env.create_work(work_spec("needs two messages")).await;
    // Transient provider failure: the run is kept (paused), state ready.
    match drive(&sd, &env.deps(script("transient")), StopWhen::Finished).await {
        DriveResult::Error { error, .. } => assert_eq!(error["recoverable"], json!(true)),
        r => panic!("{r:?}"),
    }
    let st = sd.state().unwrap();
    let run_id = st.live_run.clone().unwrap().run_id;
    assert_eq!(st.run_state, RunState::Ready);
    post_msg(&env, &sd, "m-2", "second message").await;
    // The child resumes the same run, injects the message, crashes before
    // committing state.
    let mut child = spawn_child(
        &env,
        &sd,
        "transient",
        Some("input_batch:after_input_checkpoint"),
    );
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    assert_eq!(sd.state().unwrap().live_run.unwrap().applied_input_seq, 1);
    let llm = script("transient");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let t = llm.transcript(llm.count() - 1);
    assert_eq!(t.matches("second message").count(), 1, "{t}");
    let st = sd.state().unwrap();
    assert_eq!(
        st.last_run.as_deref(),
        Some(run_id.as_str()),
        "same run continued"
    );
    assert_eq!(st.source("q").acked_index, 1);
    // The retryable error kept the Turn open: the second message joined it.
    assert_eq!(count_kind(&sd, "turn_started"), 1);
    assert_eq!(count_kind(&sd, "input_batch"), 1);
    assert_eq!(count_kind(&sd, "user_message"), 2);
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let ended: Vec<String> = read_worklog(&sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::TurnEnded { turn, .. } => Some(format!("ended {turn}")),
            _ => None,
        })
        .collect();
    assert_eq!(ended, vec!["ended 1"]);
    assert_worklog_contiguous(&sd);
}

#[tokio::test]
async fn active_sessions_on_the_same_workspace_see_each_other() {
    let env = Env::new();
    let ws = env.root.join("snake");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    let mk = |obj: &str, path: &str| {
        let mut s = work_spec(obj);
        s.workspace = Some(WorkspaceRef::External {
            path: ws.display().to_string(),
        });
        s.scope = Some(Scope {
            paths: vec![path.into()],
            objects: vec![],
        });
        s
    };
    let a = env
        .create_work(mk("A: wall wrap mode", "ws:snake/src/"))
        .await;
    let mut child = spawn_child(&env, &a, "answer", Some("input_batch:after_gate_clear"));
    wait_exit(&mut child, Duration::from_secs(60));
    assert_eq!(a.state().unwrap().run_state, RunState::Running);
    let b = env
        .create_work(mk("B: collision tweak", "ws:snake/src/collision.js"))
        .await;
    let a_sid = a.sid().to_string();
    let llm = ScriptedLlm::new(move |req, _| {
        let u = last_user_text(req);
        assert!(u.contains("<active_sessions>"), "{u}");
        assert!(u.contains(&a_sid), "{u}");
        assert!(u.contains("relation=\"same_target\""), "{u}");
        assert!(u.contains("<overlap>"), "{u}");
        text("I will wait for the other session")
    });
    let r = drive(&b, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 1);
}

fn behavior_spec(obj: &str, modes: serde_json::Value) -> libopendan::api::SessionSpec {
    let mut spec = work_spec(obj);
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    spec.prompt.behavior = Some("plan".into());
    spec.extensions
        .insert("opendan".into(), json!({ "behaviors": modes }));
    spec
}

// Review regressions -------------------------------------------------------

#[tokio::test]
async fn redo_of_finish_keeps_the_answer() {
    let env = Env::new();
    let sd = env.create_work(work_spec("answer")).await;
    post_msg(&env, &sd, "m-1", "question").await;
    let mut child = spawn_child(&env, &sd, "answer", Some("finish_run:after_flush"));
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    let llm = script("answer");
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(llm.count(), 0);
    assert!(sd.report().unwrap().contains("plain answer"));
    let st = sd.state().unwrap();
    assert_eq!(st.result.unwrap()["answer"], json!("plain answer"));
    assert!(st.one_line_status.contains("plain answer"));
}

#[tokio::test]
async fn redo_of_finish_keeps_waiting() {
    let env = Env::new();
    let mut spec = work_spec("wait");
    spec.prompt.llm_context = json!({
        "loop_model": "behavior",
        "tools": { "enabled": true, "tools2actions": true }
    });
    let sd = env.create_work(spec).await;
    let mut child = spawn_child(&env, &sd, "wait", Some("finish_run:after_flush"));
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    let r = drive(&sd, &env.deps(script("wait")), StopWhen::Idle).await;
    assert!(!r.is_finished(), "{r:?}");
    assert_eq!(sd.state().unwrap().run_state, RunState::Waiting);
}

#[tokio::test]
async fn fork_return_survives_a_crash_after_the_child_finish() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "research then answer",
            json!({ "research": { "mode": "fork" } }),
        ))
        .await;
    let mut child = spawn_child(&env, &sd, "fork", Some("finish_run:after_commit"));
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    // One commit: child finished AND parent live again with the result.
    let st = sd.state().unwrap();
    assert!(st.process_stack.is_empty(), "{:?}", st.process_stack);
    assert!(st.live_run.is_some());
    assert!(st.process_result.is_some());
    let llm = script("fork");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert!(sd.report().unwrap().contains("final answer"));
    assert_eq!(llm.count(), 1, "only the parent's final step");
}

#[tokio::test]
async fn crash_while_committing_the_switch_hand_over_does_not_repeat_it() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "two phases",
            json!({ "do": { "mode": "switch_context" } }),
        ))
        .await;
    // Hit #2 = the hand-over batch entering `do`'s own context (same Turn).
    let mut child = spawn_child(
        &env,
        &sd,
        "switch",
        Some("input_batch:after_input_checkpoint#2"),
    );
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    let llm = script("switch");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let t = llm.transcript(llm.count() - 1);
    assert_eq!(t.matches("context_switch to=\"do\"").count(), 1, "{t}");
    assert!(sd.state().unwrap().internal_continuation.is_none());
    assert_eq!(count_kind(&sd, "turn_started"), 1);
    assert_eq!(count_kind(&sd, "input_batch"), 1);
    assert_worklog_contiguous(&sd);
}

// Context scheduling (context switch TODO V6 / V7) ----------------------------

fn subcall_spec() -> libopendan::api::SessionSpec {
    let mut spec = work_spec("fork from a tool call");
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [ { "groupname": "bash" }, { "name": "call_behavior" } ] }
    });
    spec.extensions.insert(
        "opendan".into(),
        json!({ "behaviors": { "research": { "mode": "fork" } } }),
    );
    spec
}

fn outcome_kinds(sd: &SessionDir) -> Vec<String> {
    read_worklog(sd)
        .into_iter()
        .filter_map(|e| match e.body {
            WorklogBody::Outcome { kind, .. } => Some(kind),
            _ => None,
        })
        .collect()
}

/// V7: a crash at every stage of a tool-triggered sub context — after the
/// caller suspended on the call, while the child's hand-over batch commits,
/// after the child finished, after its result was filled into the caller —
/// recovers into the same child / result: one child run, the result handed
/// back exactly once, the caller's Turn kept.
#[tokio::test]
async fn sub_context_call_survives_a_crash_at_every_stage() {
    for (fault, left) in [
        ("outcome:after_checkpoint", 2),
        ("input_batch:after_input_checkpoint#2", 2),
        ("finish_run:after_commit", 1),
        ("sub_return:after_fill", 1),
    ] {
        let env = Env::new();
        let sd = env.create_work(subcall_spec()).await;
        let mut child = spawn_child(&env, &sd, "subcall", Some(fault));
        assert!(
            !wait_exit(&mut child, Duration::from_secs(60)).success(),
            "{fault}: the fault point was not reached"
        );
        let llm = script("subcall");
        let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
        assert!(r.is_finished(), "{fault}: {r:?}");
        assert_eq!(llm.count(), left, "{fault}: nothing is inferred twice");
        assert!(sd.report().unwrap().contains("final answer"), "{fault}");
        assert_eq!(
            outcome_kinds(&sd),
            vec!["suspended", "process_done", "done"],
            "{fault}"
        );
        let results: Vec<(String, String)> = read_worklog(&sd)
            .into_iter()
            .filter_map(|e| match e.body {
                WorklogBody::ActionResult { call_id, result, .. } => Some((call_id, result)),
                _ => None,
            })
            .collect();
        assert_eq!(
            results,
            vec![("f1".to_string(), "X is 42".to_string())],
            "{fault}: the result is handed back once"
        );
        let st = sd.state().unwrap();
        assert!(st.process_stack.is_empty() && st.process_result.is_none(), "{fault}");
        assert_eq!((st.turn_seq, st.turns_completed), (1, 1), "{fault}");
        assert_eq!(count_kind(&sd, "turn_started"), 1, "{fault}");
        assert_worklog_contiguous(&sd);
        // Referenced runs only: no orphan child run is left behind.
        assert_eq!(sd.runs().list().unwrap(), vec![st.last_run.unwrap()], "{fault}");
    }
}

/// xllm has neither the session's `call_behavior` tool nor a resolver for
/// its task: it refuses to take over a run suspended on a sub context
/// instead of dropping the call as an unknown local task.
#[tokio::test]
async fn xllm_refuses_a_run_suspended_on_a_sub_context() {
    use agent_tool::xllm::{ResumeLimits, RunStore, XllmDeps, XllmRun};
    let env = Env::new();
    let sd = env.create_work(subcall_spec()).await;
    let mut child = spawn_child(&env, &sd, "subcall", Some("outcome:after_checkpoint"));
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let store = RunStore::disk(sd.runs_dir());
    let llm = script("subcall");
    let deps = XllmDeps::default().with_llm(llm.clone());
    let r = XllmRun::resume(&store, Some(&run_id), None, ResumeLimits::default(), deps).await;
    assert!(r.is_err(), "xllm must refuse: {r:?}");
    assert_eq!(llm.count(), 0);
    // The session still completes the call.
    let llm = script("subcall");
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    assert_eq!(llm.count(), 2);
}

/// V6: a crash between the hand-over checkpoint and the state commit. The
/// run stopped at the hand-over point: recovery commits the transfer, it
/// does not infer on the run again.
#[tokio::test]
async fn crash_at_the_hand_over_point_commits_the_transfer_once() {
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "two phases",
            json!({ "do": { "mode": "switch_context" } }),
        ))
        .await;
    let mut child = spawn_child(&env, &sd, "switch", Some("outcome:after_checkpoint"));
    assert!(!wait_exit(&mut child, Duration::from_secs(60)).success());
    let st = sd.state().unwrap();
    let run_id = st.live_run.clone().unwrap().run_id;
    let rec = sd.runs().record(&run_id).unwrap();
    assert_eq!(rec.status, agent_tool::xllm::RunStatus::Paused);
    assert_eq!(rec.handover.as_ref().unwrap().next_behavior, "do");
    assert!(st.process_stack.is_empty(), "the transfer is not committed yet");
    let llm = script("switch");
    let r = drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 1, "only `do` runs");
    let t = llm.transcript(0);
    assert_eq!(t.matches("context_switch to=\"do\"").count(), 1, "{t}");
    assert_eq!(outcome_kinds(&sd), vec!["suspended", "done"]);
    assert_eq!(count_kind(&sd, "turn_started"), 1);
    assert_eq!(count_kind(&sd, "turn_ended"), 1);
    assert_worklog_contiguous(&sd);
}

/// V6: xllm takes over a hosted run whose model hands over to another
/// behavior. The run yields at the hand-over point (not completed), every
/// further `xllm --resume` stays there, and the session commits the transfer
/// exactly once.
#[tokio::test]
async fn xllm_yields_at_a_hand_over_and_the_session_commits_it_once() {
    use agent_tool::xllm::{ResumeLimits, ResumeStart, RunStatus, RunStore, XllmDeps, XllmRun};
    let env = Env::new();
    let sd = env
        .create_work(behavior_spec(
            "two phases",
            json!({ "do": { "mode": "switch_context" } }),
        ))
        .await;
    let mut child = spawn_child(&env, &sd, "switch", Some("input_batch:after_gate_clear"));
    wait_exit(&mut child, Duration::from_secs(60));
    let run_id = sd.state().unwrap().live_run.unwrap().run_id;
    let store = RunStore::disk(sd.runs_dir());
    let llm = script("switch");
    let deps = XllmDeps::default().with_llm(llm.clone());
    let ResumeStart::Run(mut run) =
        XllmRun::resume(&store, Some(&run_id), None, ResumeLimits::default(), deps.clone())
            .await
            .unwrap()
    else {
        panic!("terminal?")
    };
    let out = run.execute().await.unwrap();
    assert_eq!(out.status(), RunStatus::Paused, "a hand-over is not a completed run");
    assert_eq!(out.record().handover.as_ref().unwrap().next_behavior, "do");
    drop(run);
    assert_eq!(llm.count(), 1);
    // Repeated resume stays at the hand-over point.
    for _ in 0..2 {
        let err = XllmRun::resume(&store, Some(&run_id), None, ResumeLimits::default(), deps.clone())
            .await
            .err()
            .expect("must not continue past the hand-over");
        assert!(err.to_string().contains("handed over"), "{err}");
    }
    assert_eq!(llm.count(), 1);
    // The session commits the transfer and runs the target.
    let llm2 = script("switch");
    let r = drive(&sd, &env.deps(llm2.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm2.count(), 1, "only `do` runs");
    assert!(sd.report().unwrap().contains("both phases done"));
    assert_eq!(outcome_kinds(&sd), vec!["suspended", "done"]);
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    assert_eq!(count_kind(&sd, "turn_ended"), 1, "the hand-over did not close the Turn");
    assert_worklog_contiguous(&sd);
}
