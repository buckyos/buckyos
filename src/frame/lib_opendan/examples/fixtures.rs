//! Generate the protocol fixtures (L6): golden session directories produced
//! by the Rust reference implementation, one per scenario of §11, each with
//! an `expected.json` describing what a conforming runner must parse and do
//! next.
//!
//! ```text
//! cargo run -p libopendan --example fixtures -- <out_dir>
//! ```
//!
//! Absolute paths inside the fixtures are rewritten to `${FIXTURE_ROOT}` (the
//! scenario directory); a consumer substitutes its own copy location.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_tool::local_llm_context::XllmDeps;
use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiUsage};
use libopendan::api::{create_session, SessionSpec};
use libopendan::channel::{kmsg::post_to_queue, KmsgChannels, PollWaker};
use libopendan::protocol::*;
use libopendan::runner::{drive, RunnerDeps, RunnerOptions, StopWhen};
use libopendan::runtime::NativeRuntime;
use libopendan::{FsAgentStateClient, SessionDir};
use llm_context::deps::{LlmClient, LlmInferenceRequest};
use llm_context::error::{LLMComputeError, ProviderFailure};
use serde_json::{json, Value};

const AGENT: &str = "did:bns:jarvis.alice";
const APP: &str = "app:app2@alice";

type R<T> = Result<T, Box<dyn std::error::Error>>;

// ---------------------------------------------------------------------------
// scripted LLM (content based, so a child process and the parent agree)
// ---------------------------------------------------------------------------

struct Script {
    name: String,
    calls: AtomicUsize,
}

fn render(msgs: &[AiMessage]) -> String {
    let mut s = String::new();
    for m in msgs {
        for c in &m.content {
            match c {
                AiContent::Text { text } => s.push_str(text),
                AiContent::ToolResult { call_id, .. } => s.push_str(&format!("<tool_result {call_id}>")),
                _ => {}
            }
        }
        s.push('\n');
    }
    s
}

fn tool(call_id: &str, name: &str, args: Value) -> AiResponse {
    let map = args.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    AiResponse::new(AiMessage::new(AiRole::Assistant, vec![AiContent::tool_use(call_id, name, map)]))
}

fn text(t: &str) -> AiResponse {
    AiResponse::new(AiMessage::text(AiRole::Assistant, t))
}

#[async_trait]
impl LlmClient for Script {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let all = render(&req.messages);
        let mut r = match self.name.as_str() {
            "tool_then_answer" => {
                if all.contains("<tool_result c1>") {
                    text("done: notes.txt written")
                } else {
                    tool("c1", "exec", json!({ "command": "echo note > notes.txt" }))
                }
            }
            "long_exec" => {
                if all.contains("<tool_result c1>") {
                    text("recovered")
                } else {
                    tool("c1", "exec", json!({ "command": "echo start >> marker; sleep 60; echo done >> marker" }))
                }
            }
            "transient" => {
                if all.contains("second message") {
                    text("got both")
                } else {
                    return Err(LLMComputeError::Provider {
                        failure: ProviderFailure::Transient,
                        message: "busy".into(),
                    });
                }
            }
            "fork" => {
                let n = all.matches("<response>").count();
                let _ = n;
                if all.contains("research result") {
                    text("<response><report><![CDATA[final]]></report></response>")
                } else if all.contains("behavior_switch to=\"research\"") {
                    text("<response><report><![CDATA[research result]]></report></response>")
                } else if all.contains("p1-output") {
                    text("<response><next_behavior>research</next_behavior></response>")
                } else {
                    text("<response><actions><exec><![CDATA[echo p1-output]]></exec></actions></response>")
                }
            }
            _ => text("answer"),
        };
        r.usage = Some(AiUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            total_tokens: Some(15),
            ..Default::default()
        });
        Ok(r)
    }
}

// ---------------------------------------------------------------------------
// environment of one scenario
// ---------------------------------------------------------------------------

struct Env {
    root: PathBuf,
}

impl Env {
    fn new(root: &Path) -> Self {
        for d in ["agent_root", "app", "kmsg"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Self { root: root.to_path_buf() }
    }
    fn channels(&self) -> Arc<KmsgChannels> {
        Arc::new(KmsgChannels::dir(self.root.join("kmsg")).unwrap())
    }
    fn agent(&self) -> Arc<FsAgentStateClient> {
        Arc::new(
            FsAgentStateClient::open(
                self.root.join("agent_root"),
                AGENT,
                Some(self.channels().client()),
                Some(Arc::new(PollWaker)),
            )
            .unwrap(),
        )
    }
    fn deps(&self, script: &str) -> RunnerDeps {
        let llm = Arc::new(Script {
            name: script.into(),
            calls: AtomicUsize::new(0),
        });
        RunnerDeps::new(
            APP,
            self.agent(),
            self.channels(),
            Arc::new(NativeRuntime::local("rt-fixture-native", "app:app2")),
            XllmDeps::default().with_llm(llm),
        )
        .with_options(RunnerOptions {
            poll_interval: Duration::from_millis(20),
            max_wait: Duration::from_millis(100),
            load_hints: false,
            ..Default::default()
        })
    }
    async fn create(&self, sid: &str, mut spec: SessionSpec) -> SessionDir {
        spec.session_id = Some(sid.to_string());
        if spec.prompt.llm_context.is_null() {
            spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
        }
        create_session(&self.root.join("app"), spec, self.agent().as_ref(), APP, self.channels().as_ref())
            .await
            .unwrap()
    }
    async fn post(&self, sd: &SessionDir, input: Input) {
        let q = sd.config().unwrap().channels.kmsg().unwrap().1.to_string();
        post_to_queue(&self.channels().client(), &q, &input, APP).await.unwrap();
    }
    /// Drive in a child process (optionally dying at a fault point).
    fn child(&self, sd: &SessionDir, script: &str, fault: Option<&str>) -> std::process::Child {
        let mut c = Command::new(std::env::current_exe().unwrap());
        c.arg("__child")
            .arg(&self.root)
            .arg(sd.path())
            .arg(script)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match fault {
            Some(f) => c.env("LIBOPENDAN_FAULT", f),
            None => c.env_remove("LIBOPENDAN_FAULT"),
        };
        c.spawn().unwrap()
    }
    fn child_wait(&self, sd: &SessionDir, script: &str, fault: &str) {
        let mut ch = self.child(sd, script, Some(fault));
        let _ = ch.wait();
    }
}

fn work(obj: &str) -> SessionSpec {
    SessionSpec::work(obj)
}

// ---------------------------------------------------------------------------
// expectations
// ---------------------------------------------------------------------------

fn observe(sd: &SessionDir) -> Value {
    let st = sd.state().unwrap();
    let wl_len = std::fs::metadata(sd.worklog().path()).map(|m| m.len()).unwrap_or(0);
    let runs = sd.runs().list().unwrap();
    let live = st.live_run.as_ref().map(|l| {
        let rec = sd.runs().record(&l.run_id).ok();
        json!({
            "run_id": l.run_id,
            "applied_input_seq": l.applied_input_seq,
            "run_status": rec.as_ref().map(|r| r.status.as_str()),
            "host_commit_pending": rec.as_ref().and_then(|r| r.host_commit_pending),
            "inflight": rec.as_ref().map(|r| r.inflight.iter().map(|a| a.call_id.clone()).collect::<Vec<_>>()),
            "executions": rec.as_ref().map(|r| r.executions.len()),
        })
    });
    json!({
        "session_id": sd.sid(),
        "state": {
            "rev": st.rev,
            "run_state": st.run_state,
            "outcome": st.outcome,
            "acceptance": st.acceptance,
            "turn_seq": st.turn_seq,
            "open_turn": st.open_turn.as_ref().map(|t| t.index),
            "turns_completed": st.turns_completed,
            "last_run": st.last_run,
            "process_stack": st.process_stack.iter().map(|f| json!({"entry": f.entry, "mode": f.mode, "run_id": f.run_id})).collect::<Vec<_>>(),
            "inputs": st.inputs,
        },
        "live_run": live,
        "runs_on_disk": runs,
        "worklog": {
            "committed_seq": st.worklog.committed_seq,
            "committed_bytes": st.worklog.committed_bytes,
            "file_bytes": wl_len,
            "uncommitted_tail": wl_len > st.worklog.committed_bytes,
        },
    })
}

fn write_expected(dir: &Path, scenario: &str, description: &str, sessions: Vec<Value>, next: Value) {
    let v = json!({
        "scenario": scenario,
        "description": description,
        "generated_by": "libopendan examples/fixtures.rs",
        "sessions": sessions,
        "next": next,
    });
    std::fs::write(dir.join("expected.json"), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

/// Replace absolute paths of the scenario with `${FIXTURE_ROOT}`.
fn relativize(dir: &Path) {
    let root = dir.display().to_string();
    let canon = dir.canonicalize().map(|p| p.display().to_string()).unwrap_or(root.clone());
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(s) = std::fs::read_to_string(&p) {
                if s.contains(&canon) || s.contains(&root) {
                    let r = s.replace(&canon, "${FIXTURE_ROOT}").replace(&root, "${FIXTURE_ROOT}");
                    std::fs::write(&p, r).unwrap();
                }
            }
        }
    }
    // kmsg lock file is runtime only.
    let _ = std::fs::remove_file(dir.join("kmsg").join(".lock"));
}

// ---------------------------------------------------------------------------
// scenarios
// ---------------------------------------------------------------------------

async fn gen(out: &Path) -> R<()> {
    std::fs::create_dir_all(out)?;
    let scen = |name: &str| {
        let d = out.join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    };

    // 1. new work session (directory outside the AgentRoot)
    {
        let d = scen("01_new_work_session");
        let env = Env::new(&d);
        let mut spec = work("write notes.txt");
        spec.scope = Some(Scope { paths: vec!["ws:notes.txt".into()], objects: vec![] });
        let sd = env.create("work-fixture-new", spec).await;
        write_expected(&d, "new_work_session",
            "Freshly created work session located outside the AgentRoot; registered, queue created, no binding yet.",
            vec![observe(&sd)],
            json!({ "action": "bind_runtime_then_input_batch", "hook": "on_init", "expect_binding_json": false }));
        relativize(&d);
    }
    // 2. finished work session
    {
        let d = scen("02_finished_work_session");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-finished", work("write notes.txt")).await;
        drive(&sd, &env.deps("tool_then_answer"), StopWhen::Finished).await;
        write_expected(&d, "finished_work_session",
            "One run with an exec call, finished (acceptance pending). last_run kept, report.md written.",
            vec![observe(&sd)],
            json!({ "action": "idle", "accepts": ["control:decide"], "rejects": ["msg", "event", "change"] }));
        relativize(&d);
    }
    // 3. crash after the input checkpoint of a new run (orphan run)
    {
        let d = scen("03_orphan_run_pending_host_commit");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-orphan", work("x")).await;
        env.post(&sd, Input::msg("m-1", "please do it")).await;
        env.child_wait(&sd, "tool_then_answer", "input_batch:after_input_checkpoint");
        write_expected(&d, "orphan_run_pending_host_commit",
            "Snapshot + run.json(host_commit_pending) were written for a new run, state.json never referenced it.",
            vec![observe(&sd)],
            json!({ "action": "remove_unreferenced_run_then_new_input_batch", "refetch_inputs": ["q#1"], "xllm_resume": "refuse" }));
        relativize(&d);
    }
    // 4. crash after the state commit, before clearing the gate
    {
        let d = scen("04_gate_pending_after_state_commit");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-gate", work("x")).await;
        env.post(&sd, Input::msg("m-1", "please do it")).await;
        env.child_wait(&sd, "tool_then_answer", "input_batch:after_state_commit");
        write_expected(&d, "gate_pending_after_state_commit",
            "state.json consumed input q#1 (applied batch 1); run.json still has host_commit_pending=1; ack not confirmed.",
            vec![observe(&sd)],
            json!({ "action": "clear_gate_confirm_ack_resume_run", "confirm_ack": 1, "reappend_message": false }));
        relativize(&d);
    }
    // 5. crash after flushing the run history, before the commit
    {
        let d = scen("05_uncommitted_worklog_tail");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-tail", work("x")).await;
        env.child_wait(&sd, "tool_then_answer", "finish_run:after_flush");
        write_expected(&d, "uncommitted_worklog_tail",
            "The run reached its terminal outcome and its history was appended, but state.json was not committed.",
            vec![observe(&sd)],
            json!({ "action": "truncate_worklog_to_committed_then_redo_finish", "decision_source": "run.json host.extra.finish", "llm_calls": 0 }));
        relativize(&d);
    }
    // 6. runner killed while exec runs (in-flight + execution identity)
    {
        let d = scen("06_killed_during_exec");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-kill", work("long command")).await;
        let mut ch = env.child(&sd, "long_exec", None);
        let start = Instant::now();
        while !std::fs::read_to_string(sd.path().join("marker")).unwrap_or_default().contains("start") {
            if start.elapsed() > Duration::from_secs(60) {
                return Err("exec never started".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        unsafe { libc::kill(ch.id() as i32, libc::SIGKILL) };
        let _ = ch.wait();
        // Leave no process behind in the generator environment.
        let st = sd.state()?;
        let rec = sd.runs().record(&st.live_run.clone().unwrap().run_id)?;
        for e in &rec.executions {
            let _ = agent_tool::exec_tracking::stop_execution(e, None, Duration::from_secs(5)).await;
        }
        write_expected(&d, "killed_during_exec",
            "The runner was killed while `exec` ran: run.json holds the in-flight call c1 and its execution identity; the latest snapshot has no result for c1.",
            vec![observe(&sd)],
            json!({ "action": "confirm_execution_stopped_then_resume", "materialize_unresolved": ["c1"], "rerun_tool": false,
                    "if_execution_unverifiable": "recovery_blocked" }));
        relativize(&d);
    }
    // 7. new input into a resumed run: receipt ahead of state
    {
        let d = scen("07_receipt_ahead_of_state");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-receipt", work("needs two messages")).await;
        drive(&sd, &env.deps("transient"), StopWhen::Finished).await;
        env.post(&sd, Input::msg("m-2", "second message")).await;
        env.child_wait(&sd, "transient", "input_batch:after_input_checkpoint");
        write_expected(&d, "receipt_ahead_of_state",
            "A paused run was resumed with input q#1; the snapshot carries receipt batch 2 (and the message), state.json only applied batch 1.",
            vec![observe(&sd)],
            json!({ "action": "apply_receipts_from_snapshot_then_clear_gate", "apply_batches": [2], "reappend_message": false, "fetch_new_inputs_after": true }));
        relativize(&d);
    }
    // 8. finished session with a decide waiting in the queue
    {
        let d = scen("08_finished_with_decide");
        let env = Env::new(&d);
        let mut spec = work("x");
        spec.artifact_id = Some("demo".into());
        let sd = env.create("work-fixture-decide", spec).await;
        drive(&sd, &env.deps("tool_then_answer"), StopWhen::Finished).await;
        env.post(&sd, Input::control("d-1", &ControlCommand::Decide { decision: "accept".into(), by: "did:user:alice".into(), note: None })).await;
        env.post(&sd, Input::msg("m-late", "one more thing")).await;
        write_expected(&d, "finished_with_decide",
            "Finished work session (artifact demo, version produced) with control(decide: accept) and a late msg in the queue.",
            vec![observe(&sd)],
            json!({ "action": "apply_decide_reject_late_msg", "acceptance_after": "accepted", "artifact_head_after": "v-work-fixture-decide", "rejected": ["q#2"] }));
        relativize(&d);
    }
    // 9. semi-subscription pulled by registry rev
    {
        let d = scen("09_semi_subscription");
        let env = Env::new(&d);
        let a = env.create("work-fixture-sub-a", work("A")).await;
        drive(&a, &env.deps("answer"), StopWhen::Finished).await;
        let mut spec = work("B waits for A");
        spec.subscriptions.push(Subscription {
            id: "sa".into(),
            mode: SubscriptionMode::Semi,
            source: SubscriptionSource::Session { session_ref: a.sid().into() },
            watch: vec!["run_state".into(), "outcome".into()],
        });
        let b = env.create("work-fixture-sub-b", spec).await;
        write_expected(&d, "semi_subscription",
            "B semi-subscribes to A; A is finished (registry status rev > B's cursor, which is empty).",
            vec![observe(&a), observe(&b)],
            json!({ "action": "inject_change_on_next_input_batch", "session": b.sid(), "change_id_prefix": "sa@", "extra_inference": false }));
        relativize(&d);
    }
    // 10. two active sessions touching the same workspace
    {
        let d = scen("10_active_overlap");
        let env = Env::new(&d);
        let ws = d.join("ws");
        std::fs::create_dir_all(&ws)?;
        let mk = |p: &str| {
            let mut s = work("edit");
            s.workspace = Some(WorkspaceRef::External { path: ws.display().to_string() });
            s.scope = Some(Scope { paths: vec![p.into()], objects: vec![] });
            s
        };
        let a = env.create("work-fixture-active-a", mk("ws:snake/src/")).await;
        env.child_wait(&a, "answer", "input_batch:after_gate_clear");
        let b = env.create("work-fixture-active-b", mk("ws:snake/src/collision.js")).await;
        write_expected(&d, "active_overlap",
            "A is running (registry status running, touching ws:snake/src/); B is created on the same workspace.",
            vec![observe(&a), observe(&b)],
            json!({ "action": "render_active_sessions", "session": b.sid(), "sees": [a.sid()], "relation": "same_target", "overlap": ["ws:snake/src/"] }));
        relativize(&d);
    }
    // 11. large worklog with a summary start point
    {
        let d = scen("11_worklog_with_summary");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-summary", work("long history")).await;
        let lease = match sd.acquire(env.deps("answer").holder())? {
            libopendan::lock::Acquire::Acquired(l) => l,
            _ => return Err("busy".into()),
        };
        let mut s = sd.load(&lease)?;
        let mut offsets = Vec::new();
        for i in 0..200u64 {
            offsets.push(s.worklog_end());
            s.append_worklog(&lease, vec![WorklogBody::UserMessage { run_id: "r-old".into(), turn: i, content: format!("old message {i}") }])?;
        }
        s.commit_state(&lease)?;
        let mut sm = s.summary()?;
        sm.history_summary = "Summary of turns 0..189.".into();
        sm.start_offset = offsets[190];
        sm.start_seq = 192;
        sm.made_at_seq = s.state.worklog.committed_seq;
        sm.made_by = "manual".into();
        s.write_summary(&lease, &sm)?;
        drop(s);
        lease.release();
        write_expected(&d, "worklog_with_summary",
            "summary.json start point near the end of a 200 entry worklog.",
            vec![observe(&sd)],
            json!({ "action": "build_history_reverse_read", "stop_at_offset": offsets[190], "raw_entries": 10, "summary": "Summary of turns 0..189." }));
        relativize(&d);
    }
    // 12. fork child running, parent suspended
    {
        let d = scen("12_fork_child_live");
        let env = Env::new(&d);
        let mut spec = work("research then answer");
        spec.prompt.llm_context = json!({ "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true } });
        spec.prompt.behavior = Some("plan".into());
        spec.extensions.insert("opendan".into(), json!({ "process_modes": { "research": "fork" } }));
        let sd = env.create("work-fixture-fork", spec).await;
        env.child_wait(&sd, "fork", "input_batch:after_gate_clear#2");
        write_expected(&d, "fork_child_live",
            "The plan process was suspended into process_stack (fork) and the research child run is live.",
            vec![observe(&sd)],
            json!({ "action": "resume_child_run", "keep_runs": "live child + suspended parent", "on_child_done": "pop_parent_inject_process_result" }));
        relativize(&d);
    }
    // 13. unsupported snapshot version
    {
        let d = scen("13_unsupported_snapshot_version");
        let env = Env::new(&d);
        let sd = env.create("work-fixture-blocked", work("x")).await;
        env.child_wait(&sd, "tool_then_answer", "input_batch:after_gate_clear");
        let run_id = sd.state()?.live_run.unwrap().run_id;
        let rec = sd.runs().record(&run_id)?;
        let p = sd.runs_dir().join(&run_id).join("snapshots").join(format!("{:04}.json", rec.latest_snapshot_idx.unwrap()));
        let mut v: Value = serde_json::from_slice(&std::fs::read(&p)?)?;
        v["state"]["snapshot_version"] = json!(99);
        std::fs::write(&p, serde_json::to_vec(&v)?)?;
        write_expected(&d, "unsupported_snapshot_version",
            "The published snapshot of the live run has snapshot_version 99.",
            vec![observe(&sd)],
            json!({ "action": "recovery_blocked", "keep": ["live_run", "run directory", "consumption"], "infer": false }));
        relativize(&d);
    }
    // JSON Schemas next to the fixtures.
    let schema_dir = out.join("..").join("schema");
    std::fs::create_dir_all(&schema_dir)?;
    for (name, s) in libopendan::protocol::json_schemas() {
        std::fs::write(schema_dir.join(format!("{name}.schema.json")), serde_json::to_vec_pretty(&s)?)?;
    }
    println!("fixtures written to {}", out.display());
    Ok(())
}

async fn child(root: &str, session: &str, script: &str) {
    let env = Env::new(Path::new(root));
    let sd = SessionDir::open(session).unwrap();
    let _ = drive(&sd, &env.deps(script), StopWhen::Finished).await;
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("__child") {
        child(&args[1], &args[2], &args[3]).await;
        return;
    }
    let out = PathBuf::from(args.first().cloned().unwrap_or_else(|| "fixtures".into()));
    if let Err(e) = gen(&out).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
