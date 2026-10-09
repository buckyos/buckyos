//! Process level acceptance of the `xagent` executable (xAgent §8, §10):
//! exit codes, one JSON document on stdout, state on disk. The LLM is an
//! OpenAI compatible mock served by the test.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libopendan::protocol::*;
use libopendan::SessionDir;
use serde_json::{json, Value};

const AGENT: &str = "did:bns:jarvis.alice";
const WHO: &str = "app:xagent-test@alice";

/// An OpenAI compatible chat completion endpoint: `script(request, n)`
/// returns the assistant message, or an HTTP status to fail with.
struct MockLlm {
    port: u16,
    calls: Arc<AtomicUsize>,
}

impl MockLlm {
    fn start(script: impl Fn(&Value, usize) -> Result<Value, u16> + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let script = Arc::new(script);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let script = script.clone();
                let counter = counter.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut len = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let l = line.trim().to_ascii_lowercase();
                        if l.is_empty() {
                            break;
                        }
                        if let Some(v) = l.strip_prefix("content-length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; len];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    let (status, payload) = match script(&request, n) {
                        Ok(message) => (
                            200,
                            json!({
                                "id": format!("mock-{n}"), "object": "chat.completion", "model": "mock",
                                "choices": [{ "index": 0, "message": message, "finish_reason": "stop" }],
                                "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
                            }),
                        ),
                        Err(code) => (code, json!({ "error": { "message": "mock failure", "type": "server_error" } })),
                    };
                    let text = payload.to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                        text.len()
                    );
                });
            }
        });
        Self { port, calls }
    }

    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn llm_context(&self, extra: Value) -> String {
        let mut v = json!({
            "provider": { "type": "openai", "base_url": format!("http://127.0.0.1:{}/v1", self.port) },
            "model": "mock",
            "tools": { "enabled": true },
        });
        for (k, val) in extra.as_object().cloned().unwrap_or_default() {
            v[k] = val;
        }
        v.to_string()
    }
}

fn say(text: &str) -> Result<Value, u16> {
    Ok(json!({ "role": "assistant", "content": text }))
}

fn call_shell(id: &str, command: &str) -> Result<Value, u16> {
    Ok(json!({ "role": "assistant", "content": null, "tool_calls": [{
        "id": id, "type": "function",
        "function": { "name": "shell", "arguments": json!({ "command": command }).to_string() }
    }]}))
}

fn text_of(m: &Value) -> String {
    match &m["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn messages(req: &Value) -> Vec<Value> {
    req["messages"].as_array().cloned().unwrap_or_default()
}

fn system_of(req: &Value) -> String {
    messages(req)
        .iter()
        .filter(|m| m["role"] == "system")
        .map(text_of)
        .collect::<Vec<_>>()
        .join("\n")
}

fn last_user(req: &Value) -> String {
    messages(req)
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .map(text_of)
        .unwrap_or_default()
}

fn tool_result(req: &Value, call_id: &str) -> Option<String> {
    messages(req)
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == call_id)
        .map(text_of)
}

struct Cli {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Cli {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        for d in ["agent_root", "sessions", "kmsg"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Self { _tmp: tmp, root }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_xagent"));
        c.args(args)
            .env("OPENDAN_AGENT_ROOT", self.root.join("agent_root"))
            .env("OPENDAN_AGENT_DID", AGENT)
            .env("LIBOPENDAN_QUEUE_DIR", self.root.join("kmsg"))
            .env("LIBOPENDAN_WHO", WHO)
            .env("OPENAI_API_KEY", "test")
            .env_remove("OPENDAN_SESSION_ID")
            .env_remove("LIBOPENDAN_FAULT")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    fn run(&self, args: &[&str]) -> (i32, Value, String) {
        let out = self.command(args).output().unwrap();
        parse(out)
    }

    fn sessions(&self) -> String {
        self.root.join("sessions").display().to_string()
    }

    /// `new --no-run`, returning the session directory.
    fn create(&self, args: &[&str]) -> SessionDir {
        let parent = self.sessions();
        let mut all = vec!["new", "--parent", parent.as_str(), "--no-run", "--poll-ms", "50"];
        all.extend_from_slice(args);
        let (code, v, err) = self.run(&all);
        assert_eq!(code, 0, "{v} {err}");
        SessionDir::open(v["path"].as_str().unwrap()).unwrap()
    }
}

/// stdout must be exactly one JSON document (or nothing); logs go to stderr.
fn parse(out: Output) -> (i32, Value, String) {
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let v = if stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{stdout}\n--- stderr\n{stderr}"))
    };
    (out.status.code().unwrap_or(-1), v, stderr)
}

fn worklog(sd: &SessionDir) -> Vec<WorklogEntry> {
    sd.worklog().read_all_for_audit().unwrap()
}

/// E13 / E28: the default work session has no queue. Its first input is
/// persisted by `new --no-run`, a later `run` (another process) consumes it
/// and closes the Turn; nothing can be posted to it afterwards.
#[test]
fn new_no_run_then_run_a_work_session_without_a_queue() {
    let cli = Cli::new();
    let llm = MockLlm::start(|req, _| {
        assert!(last_user(req).contains("the note: buy milk"), "{}", last_user(req));
        // G3: a session is not worded as a one-shot task.
        assert!(system_of(req).contains("You are running inside an Agent Session"));
        assert!(!system_of(req).contains("one-shot task runner"));
        say("milk")
    });
    let lc = llm.llm_context(json!({}));
    let sd = cli.create(&["--objective", "summarize the note", "--llm-context", &lc, "--msg", "the note: buy milk"]);
    let cfg = sd.config().unwrap();
    assert!(cfg.channels.kmsg().is_none());
    assert_eq!(cfg.prompt.initial_inputs.len(), 1);
    assert!(cfg.prompt.frozen.is_some(), "frozen at creation (identity; no behavior)");
    assert!(sd.binding_opt().unwrap().is_none(), "nothing is bound before the first drive");
    assert_eq!(llm.count(), 0);

    let dir = sd.path().display().to_string();
    // A session without a queue takes no delivery on `run` or `post`.
    let (code, _, err) = cli.run(&["run", &dir, "--msg", "more"]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("no input queue"), "{err}");
    let (code, _, err) = cli.run(&["post", sd.sid(), "--msg", "more"]);
    assert_eq!(code, 2, "{err}");
    let (code, _, err) = cli.run(&["ctl", sd.sid(), "stop"]);
    assert_eq!(code, 2, "{err}");

    let (code, v, err) = cli.run(&["run", &dir, "--poll-ms", "50"]);
    assert_eq!(code, 0, "{v} {err}");
    assert_eq!(v["kind"], "turn_closed");
    assert_eq!((v["turn"].as_u64(), v["status"].as_str()), (Some(1), Some("completed")));
    assert_eq!(v["answer"], "milk");
    assert_eq!(llm.count(), 1);
    let st = sd.state().unwrap();
    assert!(st.is_finished() && st.source(BOOTSTRAP_SRC).is_consumed(1));

    // By session id, finished: nothing to close, the session's result.
    let (code, v, _) = cli.run(&["run", sd.sid(), "--poll-ms", "50"]);
    assert_eq!((code, v["kind"].as_str()), (0, Some("finished")));
    // Read-only commands.
    let (code, v, _) = cli.run(&["status", sd.sid(), "--report", "--worklog", "3", "--run", "--events"]);
    assert_eq!(code, 0);
    assert_eq!(v["entry"]["status"]["run_state"], "finished");
    assert!(v["report"].as_str().unwrap().contains("milk"));
    assert_eq!(v["turn"]["completed"], 1);
    assert_eq!(v["run"]["status"], "completed");
    let (code, v, _) = cli.run(&["list"]);
    assert_eq!((code, v.as_array().map(Vec::len)), (0, Some(1)));
    let (code, _, err) = cli.run(&["xllm", sd.sid()]);
    assert_eq!(code, 2, "{err}");
    let out = cli.root.join("schema");
    let o = cli.command(&["schema", out.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success());
    assert!(out.join("session_config.schema.json").is_file());
    let (code, _, _) = cli.run(&["nonsense"]);
    assert_eq!(code, 2);
    let (code, _, err) = cli.run(&["run", "work-does-not-exist"]);
    assert_eq!(code, 2, "{err}");
}

/// E6 / E9 / E15: a session whose template allows waiting. `run --msg`
/// leaves the Turn open (exit 3) when the agent asks; a posted answer is
/// consumed by the next `run` in the same Turn; control records are a
/// separate protocol; `stop` ends the session (exit 4).
#[test]
fn ui_session_waits_for_input_and_control_is_separate() {
    let cli = Cli::new();
    let llm = MockLlm::start(|req, _| {
        let u = last_user(req);
        if u.contains("blue") {
            say("<response><report><![CDATA[blue it is]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        } else if u.contains("thanks") {
            say("<response><report><![CDATA[welcome]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        } else {
            say("<response><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        }
    });
    let lc = llm.llm_context(json!({ "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true } }));
    let sd = cli.create(&["--class", "ui", "--objective", "chat", "--llm-context", &lc]);
    assert!(sd.config().unwrap().channels.kmsg().is_some());
    let sid = sd.sid().to_string();

    // Nothing to do yet: no Turn is made up.
    let (code, v, _) = cli.run(&["run", &sid, "--poll-ms", "50"]);
    assert_eq!((code, v["kind"].as_str()), (3, Some("turn_open")), "{v}");
    assert_eq!(v["waiting_for"]["kind"], "input", "the bootstrap Turn waits for input: {v}");

    let (code, v, err) = cli.run(&["run", &sid, "--msg", "pick a color for me", "--poll-ms", "50"]);
    assert_eq!((code, v["kind"].as_str()), (3, Some("turn_open")), "{v} {err}");
    let turn = v["turn"].as_u64().unwrap();
    assert!(sd.state().unwrap().open_turn.is_some());

    // The same delivery as a posted record, then a plain `run`.
    let (code, v, err) = cli.run(&["post", &sid, "--msg", "blue"]);
    assert_eq!(code, 0, "{v} {err}");
    let (code, v, err) = cli.run(&["run", &sid, "--poll-ms", "50"]);
    assert_eq!((code, v["kind"].as_str()), (0, Some("turn_closed")), "{v} {err}");
    assert_eq!((v["turn"].as_u64(), v["answer"].as_str()), (Some(turn), Some("blue it is")));
    let st = sd.state().unwrap();
    assert_eq!(st.turns_completed, 1);
    assert!(!st.is_finished());

    // Idle: no open Turn and nothing closed.
    let (code, v, _) = cli.run(&["run", &sid, "--poll-ms", "50"]);
    assert_eq!((code, v["kind"].as_str()), (3, Some("idle")), "{v}");

    // A logical record from a file: msg accepted, control refused.
    let rec = cli.root.join("rec.json");
    let me = "did:bns:alice";
    std::fs::write(
        &rec,
        json!({ "type": "control", "key": "c-1", "payload": { "command": "stop" } }).to_string(),
    )
    .unwrap();
    let (code, _, err) = cli.run(&["post", &sid, "--json", rec.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("ctl"), "{err}");
    let (code, v, err) = cli.run(&["post", &sid, "--msg", "thanks", "--from", me]);
    assert_eq!(code, 0, "{v} {err}");

    // Controls never enter the context and open no Turn.
    let (code, _, err) = cli.run(&["ctl", &sid, "activity", "--summary", "chatting", "--touch", "ws:notes.md"]);
    assert_eq!(code, 0, "{err}");
    let (code, v, err) = cli.run(&["run", &sid, "--poll-ms", "50"]);
    assert_eq!((code, v["answer"].as_str()), (0, Some("welcome")), "{v} {err}");
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (turn + 1, 2));
    assert_eq!(st.activity.summary, "chatting");

    let before = llm.count();
    let (code, _, err) = cli.run(&["ctl", &sid, "stop"]);
    assert_eq!(code, 0, "{err}");
    let (code, v, err) = cli.run(&["run", &sid, "--poll-ms", "50"]);
    assert_eq!(code, 4, "{v} {err}");
    assert_eq!(llm.count(), before, "a control is not an input");
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
    assert_eq!(sd.state().unwrap().turn_seq, turn + 1, "no Turn was opened for it");
    // A late input after the end.
    let (code, _, err) = cli.run(&["post", &sid, "--msg", "anyone?"]);
    assert_eq!(code, 1, "{err}");
}

/// E21 / E5: a sub session created by the layer ② tool through `shell`
/// (`agent-session create-worksession --wait`). The parent run is suspended
/// on `session:<sid>`, the same process drives the child, and its end is
/// the call's tool result.
#[test]
fn a_sub_session_is_created_and_waited_for_through_the_shell_tool() {
    let cli = Cli::new();
    let llm = MockLlm::start(|req, _| {
        if system_of(req).contains("compute the answer") {
            return say("forty-two");
        }
        match tool_result(req, "w1") {
            Some(r) => {
                assert!(r.contains("forty-two"), "{r}");
                say("the child said forty-two")
            }
            None => call_shell(
                "w1",
                "agent-session create-worksession --objective 'compute the answer' --msg 'be brief' --wait",
            ),
        }
    });
    let lc = llm.llm_context(json!({}));
    let parent = cli.sessions();
    let (code, v, err) = cli.run(&[
        "new", "--parent", &parent, "--objective", "delegate", "--llm-context", &lc, "--poll-ms", "50",
        "--max-wait", "60",
    ]);
    assert_eq!(code, 0, "{v}\n{err}");
    assert_eq!(v["kind"], "turn_closed");
    assert_eq!(v["answer"], "the child said forty-two");
    assert!(v.get("children").is_none(), "{v}");
    let sd = SessionDir::open(v["path"].as_str().unwrap()).unwrap();
    assert!(sd.runtime_bin_dir().join("agent-session").is_file());

    let (code, list, _) = cli.run(&["sessions", "--children", "--of", sd.sid()]);
    assert_eq!(code, 0);
    let children = list.as_array().unwrap();
    assert_eq!(children.len(), 1, "{list}");
    assert_eq!(children[0]["run_state"], "finished");
    let child = children[0]["session_id"].as_str().unwrap();
    let (_, st, _) = cli.run(&["status", child, "--report"]);
    let origin = &st["entry"]["origin"];
    assert_eq!(origin["parent_session"], sd.sid());
    assert_eq!(origin["report"], "final");
    assert!(origin["created_by_call"].as_str().unwrap().ends_with("/w1"), "{origin}");
    assert!(st["report"].as_str().unwrap().contains("forty-two"));
    // One call, one result, no event for the same end.
    let results: Vec<_> = worklog(&sd)
        .into_iter()
        .filter(|e| matches!(&e.body, WorklogBody::ActionResult { call_id, .. } if call_id == "w1"))
        .collect();
    assert_eq!(results.len(), 1);
    assert!(sd.state().unwrap().is_finished());
    assert_eq!(llm.count(), 3);
}

/// E1: a run stopped by xagent is finished by xllm from the run directory
/// alone; xagent then commits its end exactly once.
#[test]
fn xllm_takes_over_the_live_run_and_xagent_commits_its_end() {
    use agent_tool::xllm::{ResumeLimits, ResumeStart, RunStore, XllmDeps, XllmRun};
    let cli = Cli::new();
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let f2 = fail.clone();
    let llm = MockLlm::start(move |req, _| {
        if f2.load(Ordering::SeqCst) {
            return Err(503);
        }
        assert!(last_user(req).contains("the note"), "{}", last_user(req));
        say("taken over")
    });
    let lc = llm.llm_context(json!({}));
    let sd = cli.create(&["--objective", "summarize", "--llm-context", &lc, "--msg", "the note"]);
    let sid = sd.sid().to_string();
    // The provider is down: the run is kept, the Turn stays open.
    let (code, v, err) = cli.run(&["run", &sid, "--until", "outcomes:1", "--poll-ms", "50"]);
    assert_eq!(code, 1, "{v} {err}");
    assert_eq!(v["kind"], "error");
    let st = sd.state().unwrap();
    let run_id = st.live_run.as_ref().expect("the run is kept").run_id.clone();
    assert!(st.open_turn.is_some());

    let (code, v, _) = cli.run(&["xllm", &sid]);
    assert_eq!(code, 0);
    assert_eq!(v["run_id"], run_id.as_str());
    assert_eq!(v["blockers"], json!([]));
    let command = v["command"].as_str().unwrap();
    assert!(
        command.contains("xllm --resume --run") && command.contains(&run_id)
            && command.contains(".opendan_agent_session/runs"),
        "{command}"
    );

    // xllm continues it from the run directory: no session template, no
    // frozen behavior, no receipt is needed.
    fail.store(false, Ordering::SeqCst);
    let before_system = {
        let (_, snap) = sd.runs().load_checked(&run_id).unwrap();
        serde_json::to_string(&snap.unwrap().request.input[0]).unwrap()
    };
    std::env::set_var("OPENAI_API_KEY", "test");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let store = RunStore::disk(sd.runs_dir());
        let started = XllmRun::resume(&store, Some(&run_id), None, ResumeLimits::default(), XllmDeps::default())
            .await
            .unwrap();
        let ResumeStart::Run(mut run) = started else {
            panic!("the run must be resumable")
        };
        let _ = run.execute().await;
    });
    let (record, snap) = sd.runs().load_checked(&run_id).unwrap();
    assert!(record.status.is_terminal(), "{:?}", record.status);
    assert_eq!(
        serde_json::to_string(&snap.unwrap().request.input[0]).unwrap(),
        before_system,
        "the system section was not re-assembled"
    );
    assert!(sd.state().unwrap().live_run.is_some(), "the session has not committed it yet");

    let (code, v, err) = cli.run(&["run", &sid, "--poll-ms", "50"]);
    assert_eq!(code, 0, "{v} {err}");
    assert_eq!((v["kind"].as_str(), v["answer"].as_str()), (Some("turn_closed"), Some("taken over")));
    let st = sd.state().unwrap();
    assert_eq!((st.turn_seq, st.turns_completed), (1, 1));
    let wl = worklog(&sd);
    assert_eq!(wl.iter().filter(|e| e.body.kind() == "turn_started").count(), 1);
    assert_eq!(wl.iter().filter(|e| e.body.kind() == "turn_ended").count(), 1);
    for (i, e) in wl.iter().enumerate() {
        assert_eq!(e.seq, i as u64 + 1);
    }
}

fn wait_for(what: &str, max: Duration, f: impl Fn() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < max, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// E8 / E28: one driver per session (exit 5 for the second), and SIGINT of
/// the driving process stops the session through the normal stop path
/// (exit 4) although it has no queue.
#[cfg(unix)]
#[test]
fn a_second_driver_is_busy_and_sigint_stops_the_session() {
    let cli = Cli::new();
    let llm = MockLlm::start(|_, _| call_shell("c1", "echo started > started.txt; sleep 60"));
    let lc = llm.llm_context(json!({}));
    let sd = cli.create(&["--objective", "long command", "--llm-context", &lc]);
    let dir = sd.path().display().to_string();
    let child = cli.command(&["run", &dir, "--poll-ms", "50"]).spawn().unwrap();
    wait_for("the tool to start", Duration::from_secs(60), || sd.path().join("started.txt").exists());

    let (code, v, _) = cli.run(&["run", &dir, "--poll-ms", "50"]);
    assert_eq!((code, v["kind"].as_str()), (5, Some("busy")), "{v}");

    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let started = Instant::now();
    let (code, v, err) = parse(child.wait_with_output().unwrap());
    assert!(started.elapsed() < Duration::from_secs(30), "the sleep was interrupted");
    assert_eq!(code, 4, "{v} {err}");
    assert_eq!((v["kind"].as_str(), v["status"].as_str()), (Some("turn_closed"), Some("stopped")), "{v}");
    let st = sd.state().unwrap();
    assert_eq!(st.outcome, Some(Outcome::Stopped));
    assert!(st.live_run.is_none());
}

fn tmux(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// E3: the session's tmux runtime. Its id is the session id; the target is
/// created by the first drive (not by `new --no-run`), reused by later
/// Turns and by another process, never shared between sessions, and a
/// different runtime identity is refused before any inference (exit 6).
#[test]
fn tmux_runtime_belongs_to_the_session() {
    if !tmux(&["-V"]) {
        eprintln!("tmux not available; skipped");
        return;
    }
    let cli = Cli::new();
    let llm = MockLlm::start(|req, _| {
        // The step result of the action is the last user message.
        let u = last_user(req);
        match u.lines().find(|l| l.trim().starts_with("od_")) {
            Some(name) => say(&format!(
                "<response><report><![CDATA[saw {}]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>",
                name.trim()
            )),
            None => say("<response><actions><shell><![CDATA[tmux display-message -p '#S']]></shell></actions></response>"),
        }
    });
    let lc = llm.llm_context(json!({
        "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true },
        "runtime": { "kind": "tmux" }
    }));
    let a = cli.create(&["--class", "ui", "--objective", "tmux a", "--llm-context", &lc]);
    let b = cli.create(&["--class", "ui", "--objective", "tmux b", "--llm-context", &lc]);
    let name = |sd: &SessionDir| libopendan::runtime::tmux::tmux_session_name(sd.sid());
    assert!(!tmux(&["has-session", "-t", &name(&a)]), "`new --no-run` creates no target");

    let (code, v, err) = cli.run(&["run", a.sid(), "--msg", "first", "--poll-ms", "50"]);
    assert_eq!(code, 0, "{v} {err}");
    assert!(tmux(&["has-session", "-t", &name(&a)]));
    let binding = a.binding_opt().unwrap().unwrap();
    assert_eq!((binding.kind.as_str(), binding.runtime_id.as_str()), ("tmux", a.sid()));
    let (_, record) = (0, a.runs().record(a.state().unwrap().last_run.as_deref().unwrap()).unwrap());
    assert_eq!(record.host.as_ref().unwrap().runtime_id.as_deref(), Some(a.sid()));

    // Another Turn, another process: the same target (attach, not create).
    let (code, v, err) = cli.run(&["run", a.sid(), "--msg", "second", "--poll-ms", "50"]);
    assert_eq!(code, 0, "{v} {err}");
    assert_eq!(a.binding_opt().unwrap().unwrap().target, binding.target);

    // Another session: its own target.
    let (code, v, err) = cli.run(&["run", b.sid(), "--msg", "first", "--poll-ms", "50"]);
    assert_eq!(code, 0, "{v} {err}");
    assert!(tmux(&["has-session", "-t", &name(&b)]));
    assert_ne!(b.binding_opt().unwrap().unwrap().target, binding.target);

    // An explicit runtime id must be the session's own.
    let calls = llm.count();
    let (code, v, err) = cli.run(&["run", a.sid(), "--msg", "third", "--runtime", "somewhere-else", "--poll-ms", "50"]);
    assert_eq!(code, 6, "{v} {err}");
    // The bound target is gone: never re-created under the old identity.
    assert!(tmux(&["kill-session", "-t", &name(&a)]));
    let (code, v, err) = cli.run(&["run", a.sid(), "--poll-ms", "50"]);
    assert_eq!(code, 6, "{v} {err}");
    assert_eq!(llm.count(), calls, "refused before any inference");
    let _ = tmux(&["kill-session", "-t", &name(&b)]);
}

/// E9 / E27: resident hosting. A message posted while `serve` runs is
/// handled without restarting anything; the host keeps the session while it
/// waits; a queued `stop` ends the session and with it the serving loop.
#[test]
fn serve_handles_posted_input_until_the_session_is_stopped() {
    let cli = Cli::new();
    let llm = MockLlm::start(|req, _| {
        let u = last_user(req);
        if u.contains("ping") {
            say("<response><report><![CDATA[pong]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        } else {
            say("<response><next_behavior>WAIT_USER_MSG</next_behavior></response>")
        }
    });
    let lc = llm.llm_context(json!({ "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true } }));
    let sd = cli.create(&["--class", "ui", "--objective", "serve me", "--llm-context", &lc]);
    let sid = sd.sid().to_string();
    let child = cli.command(&["serve", &sid, "--poll-ms", "50"]).spawn().unwrap();
    wait_for("the bootstrap Turn", Duration::from_secs(60), || {
        sd.state().is_ok_and(|s| s.bootstrap_done && s.run_state == RunState::Waiting)
    });
    // Driven by the resident host: a second driver is refused only while a
    // drive is in progress; posting is always possible.
    let (code, v, err) = cli.run(&["post", &sid, "--msg", "ping"]);
    assert_eq!(code, 0, "{v} {err}");
    wait_for("the posted message to be answered", Duration::from_secs(60), || {
        sd.state().is_ok_and(|s| s.turns_completed == 1)
    });
    assert!(sd.state().unwrap().open_turn.is_none());
    let (code, _, err) = cli.run(&["ctl", &sid, "stop"]);
    assert_eq!(code, 0, "{err}");
    let (code, v, err) = parse(child.wait_with_output().unwrap());
    assert_eq!(code, 4, "{v} {err}");
    assert_eq!(v[0]["kind"], "finished");
    assert_eq!(v[0]["session_id"], sid.as_str());
    assert_eq!(sd.state().unwrap().outcome, Some(Outcome::Stopped));
}

fn _unused(_: &Path) {}
