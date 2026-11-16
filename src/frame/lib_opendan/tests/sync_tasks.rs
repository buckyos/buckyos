//! Sync tasks (`libopendan::sync`): the runner's permissions on the real
//! Deno, the envelope the host writes, retries and failures, the timetable,
//! and credentials that never leave the program's input.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use libopendan::host::goals::ManualClock;
use libopendan::notify::NotifyHost;
use libopendan::sync::*;
use serde_json::{json, Value};

const SECRET: &str = "s3cr3t-Value-42";

/// A runner that answers from a script and keeps what each run was given.
#[derive(Default)]
struct Scripted {
    answers: Mutex<Vec<ProgramRun>>,
    inputs: Mutex<Vec<Value>>,
}

impl Scripted {
    fn then(self: &Arc<Self>, run: ProgramRun) -> Arc<Self> {
        self.answers.lock().unwrap().push(run);
        self.clone()
    }

    fn calls(&self) -> usize {
        self.inputs.lock().unwrap().len()
    }
}

#[async_trait]
impl SyncRunner for Scripted {
    async fn run(&self, _program: &Path, _hosts: &[String], input: &Value) -> ProgramRun {
        self.inputs.lock().unwrap().push(input.clone());
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            return printed(json!({ "status": "ok", "records": [] }));
        }
        answers.remove(0)
    }
}

fn printed(v: Value) -> ProgramRun {
    ProgramRun { stdout: Ok(v.to_string().into_bytes()), exited_ok: true, log: String::new() }
}

fn transient() -> ProgramRun {
    ProgramRun { stdout: Err((FailureKind::Transient, "the program did not finish within 120 s".into())), exited_ok: false, log: String::new() }
}

struct Zone {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    clock: Arc<ManualClock>,
    secrets: Arc<MemorySecrets>,
    notify: Arc<libopendan::notify::LocalNotify>,
}

const T0: u64 = 1_800_000_000_000;

fn zone() -> Zone {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("data/home/alice/agents/jarvis.test.buckyos.io");
    std::fs::create_dir_all(&root).unwrap();
    let notify = Arc::new(libopendan::notify::LocalNotify::enable(&root, "did:bns:jarvis").unwrap());
    Zone { _tmp: tmp, root, clock: ManualClock::at(T0), secrets: Arc::new(MemorySecrets::default()), notify }
}

impl Zone {
    fn host(&self, runner: Arc<dyn SyncRunner>) -> Arc<SyncHost> {
        let mut o = SyncHostOptions::new(self.root.clone(), self.secrets.clone(), Some(self.notify.clone()));
        o.runner = runner;
        o.clock = self.clock.clone();
        o.retry_delays = vec![Duration::from_millis(5), Duration::from_millis(5)];
        SyncHost::new(o)
    }

    fn file(&self, task: &str, name: &str) -> PathBuf {
        self.root.join("sync").join(task).join(name)
    }

    fn out(&self, task: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.file(task, "out.json")).unwrap()).unwrap()
    }

    async fn notices(&self) -> Vec<libopendan::notify::NotifyView> {
        self.notify.list(Default::default()).await.unwrap()
    }
}

fn put(id: &str, every: u64, secrets: &[&str]) -> PutRequest {
    serde_json::from_value(json!({
        "task_id": id,
        "title": "Quotes",
        "program": "export {};\n",
        "hosts": ["api.example.com"],
        "schedule": { "every_secs": every },
        "inputs": { "symbols": ["AAPL"] },
        "secrets": secrets,
    }))
    .unwrap()
}

const WAIT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn a_good_run_writes_the_envelope_the_state_and_the_record() {
    let z = zone();
    let runner = Arc::new(Scripted::default()).then(printed(json!({
        "status": "ok",
        "business_time": "2026-11-15T21:00:00Z",
        "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }],
        "records": [{ "symbol": "AAPL", "close": 190.5 }],
        "state": { "cursor": 7 },
    })));
    let host = z.host(runner.clone());
    let task = host.put(put("quotes", 900, &[])).await.unwrap();
    assert_eq!((task.version, task.state, task.enabled), (1, "ok", true));
    assert_eq!(task.output["path"], "/home/alice/agents/jarvis.test.buckyos.io/sync/quotes/out.json");

    let run = host.run("quotes", "manual", WAIT).await.unwrap();
    assert_eq!((run.status, run.records, run.trigger.as_str()), (RunStatus::Succeeded, Some(1), "manual"));
    let out = z.out("quotes");
    assert_eq!(out["format"], "aiws.sync.v1");
    assert_eq!(out["status"], "ok");
    assert_eq!(out["program"], json!({ "name": "quotes", "version": "1" }));
    assert_eq!(out["business_time"], "2026-11-15T21:00:00Z");
    assert_eq!(out["records"], json!([{ "symbol": "AAPL", "close": 190.5 }]));
    assert_eq!(out["fields"][0]["field_id"], "symbol");
    assert!(out["produced_at"].as_str().is_some_and(|t| t.ends_with('Z')));
    assert!(!z.file("quotes", "out.json.tmp").exists());
    let state: Value = serde_json::from_slice(&std::fs::read(z.file("quotes", "state.json")).unwrap()).unwrap();
    assert_eq!(state, json!({ "cursor": 7 }));
    // The next run gets the state, the task's inputs and its id.
    host.run("quotes", "manual", WAIT).await.unwrap();
    let given = runner.inputs.lock().unwrap()[1].clone();
    assert_eq!(given, json!({ "task_id": "quotes", "inputs": { "symbols": ["AAPL"] }, "secrets": {}, "state": { "cursor": 7 } }));
    let runs = host.runs("quotes", 10).await.unwrap();
    assert_eq!(runs.len(), 2);
    let task = host.get("quotes").await.unwrap();
    assert_eq!((task.failures, task.state), (0, "ok"));
    assert_eq!(task.last_run.unwrap().run_id, runs[0].run_id);

    // A changed program raises the version; the same one does not.
    let mut changed = put("quotes", 900, &[]);
    changed.program = "export const v = 2;\n".into();
    assert_eq!(host.put(changed.clone()).await.unwrap().version, 2);
    assert_eq!(host.put(changed).await.unwrap().version, 2);
    // A disabled task does not run on request.
    host.set_enabled("quotes", false).await.unwrap();
    assert_eq!(host.get("quotes").await.unwrap().state, "disabled");
    assert!(host.run("quotes", "aiworkspace", WAIT).await.is_err());
}

#[tokio::test]
async fn transient_failures_are_retried_and_a_failed_envelope_is_written_after_the_last() {
    let z = zone();
    let runner = Arc::new(Scripted::default())
        .then(printed(json!({ "status": "ok", "records": [{ "id": 1 }] })))
        .then(transient())
        .then(printed(json!({ "status": "failed", "error": "upstream answered 503", "error_kind": "transient" })))
        .then(transient());
    let host = z.host(runner.clone());
    host.put(put("quotes", 900, &[])).await.unwrap();
    assert_eq!(host.run("quotes", "manual", WAIT).await.unwrap().status, RunStatus::Succeeded);
    let run = host.run("quotes", "manual", WAIT).await.unwrap();
    assert_eq!(runner.calls(), 4, "the first try and two retries");
    assert_eq!((run.status, run.kind), (RunStatus::Failed, Some(FailureKind::Transient)));
    let out = z.out("quotes");
    assert_eq!(out["status"], "failed");
    assert_eq!(out["error"], "the program did not finish within 120 s");
    assert!(out.get("records").is_none());
    let task = host.get("quotes").await.unwrap();
    assert_eq!((task.state, task.failures), ("failing", 1));
    assert!(z.notices().await.is_empty(), "a transient failure tells nobody");

    // A retry that succeeds is one good run.
    let runner = Arc::new(Scripted::default()).then(transient()).then(printed(json!({ "status": "ok", "records": [] })));
    let host = z.host(runner.clone());
    assert_eq!(host.run("quotes", "manual", WAIT).await.unwrap().status, RunStatus::Succeeded);
    assert_eq!(runner.calls(), 2);
    assert_eq!(host.get("quotes").await.unwrap().failures, 0);
}

#[tokio::test]
async fn a_program_that_keeps_failing_is_reported_to_the_owner_once() {
    let z = zone();
    let crash = ProgramRun {
        stdout: Ok(Vec::new()),
        exited_ok: false,
        log: "error: Uncaught (in promise) SyntaxError: Unexpected token\n    at file:///x/program.ts:3:1".into(),
    };
    let runner = Arc::new(Scripted::default())
        .then(crash.clone())
        .then(printed(json!({ "status": "ok" })))
        .then(printed(json!({ "status": "done", "records": [] })))
        .then(crash);
    let host = z.host(runner.clone());
    host.put(put("quotes", 900, &[])).await.unwrap();
    let first = host.run("quotes", "manual", WAIT).await.unwrap();
    assert_eq!(first.kind, Some(FailureKind::Program));
    assert!(first.error.as_deref().unwrap().contains("SyntaxError"), "{first:?}");
    let second = host.run("quotes", "manual", WAIT).await.unwrap();
    assert!(second.error.as_deref().unwrap().contains("`records` is missing"), "{second:?}");
    assert!(z.notices().await.is_empty(), "two in a row are not yet worth a notification");
    let third = host.run("quotes", "manual", WAIT).await.unwrap();
    assert!(third.error.as_deref().unwrap().contains("status `done`"), "{third:?}");
    assert_eq!(runner.calls(), 3, "program failures are not retried");
    let notices = z.notices().await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].prompt.title.as_deref().unwrap().contains("keeps failing"));
    host.run("quotes", "manual", WAIT).await.unwrap();
    assert_eq!(z.notices().await.len(), 1, "once per streak");
    assert_eq!(host.get("quotes").await.unwrap().failures, 4);
}

#[tokio::test]
async fn credentials_reach_the_program_and_nothing_else() {
    let z = zone();
    let leaky = ProgramRun {
        stdout: Ok(json!({ "status": "failed", "error": format!("401 for key {SECRET}"), "error_kind": "auth" }).to_string().into_bytes()),
        exited_ok: true,
        log: format!("calling with {SECRET}"),
    };
    let runner = Arc::new(Scripted::default())
        .then(leaky)
        .then(printed(json!({ "status": "ok", "records": [{ "note": format!("token={SECRET}") }] })));
    let host = z.host(runner.clone());
    host.put(put("quotes", 900, &["API_KEY"])).await.unwrap();

    // Not set yet: the run fails as `auth` without starting the program, and the owner is told at once.
    let run = host.run("quotes", "manual", WAIT).await.unwrap();
    assert_eq!(run.kind, Some(FailureKind::Auth));
    assert!(run.error.as_deref().unwrap().contains("`API_KEY` is not set"));
    assert_eq!(runner.calls(), 0);
    let notices = z.notices().await;
    assert_eq!(notices.len(), 1);
    assert!(notices[0].prompt.title.as_deref().unwrap().contains("needs a credential"));

    assert!(host.set_secret("quotes", "OTHER", Some("x".into())).await.is_err(), "only declared secrets");
    let task = host.set_secret("quotes", "API_KEY", Some(SECRET.into())).await.unwrap();
    assert!(task.secrets[0].set);
    let run = host.run("quotes", "manual", WAIT).await.unwrap();
    assert_eq!(runner.inputs.lock().unwrap()[0]["secrets"], json!({ "API_KEY": SECRET }));
    assert_eq!(run.error.as_deref(), Some("401 for key [secret API_KEY]"));

    let try_it = host.try_run(serde_json::from_value(json!({ "task_id": "quotes" })).unwrap()).await.unwrap();
    assert_eq!(try_it["preview"]["sample"][0]["note"], "token=[secret API_KEY]");

    let mut seen = vec![
        serde_json::to_string(&host.get("quotes").await.unwrap()).unwrap(),
        serde_json::to_string(&host.list().await.unwrap()).unwrap(),
        serde_json::to_string(&host.runs("quotes", 10).await.unwrap()).unwrap(),
        try_it.to_string(),
        z.out("quotes").to_string(),
    ];
    for name in ["task.json", "runs.jsonl", "schedule.json"] {
        seen.push(std::fs::read_to_string(z.file("quotes", name)).unwrap());
    }
    for text in seen {
        assert!(!text.contains(SECRET), "a credential leaked: {text}");
    }
    // Dropping the secret from the task forgets its value; deleting the task forgets them all.
    host.put(put("quotes", 900, &[])).await.unwrap();
    assert!(z.secrets.values("quotes").await.unwrap().is_empty());
    host.delete("quotes").await.unwrap();
    assert!(!z.root.join("sync/quotes").exists());
}

#[tokio::test]
async fn the_timetable_is_at_least_15_minutes_and_missed_slots_are_one_run() {
    let z = zone();
    let runner = Arc::new(Scripted::default());
    let host = z.host(runner.clone());
    let err = host.put(put("quotes", 899, &[])).await.unwrap_err();
    assert!(err.to_string().contains("at least 900"), "{err}");
    assert!(host.put(put("Bad/id", 900, &[])).await.is_err());
    let mut bad_host = put("quotes", 900, &[]);
    bad_host.hosts = vec!["localhost:3180".into()];
    assert!(host.put(bad_host).await.is_err());
    host.put(put("quotes", 900, &[])).await.unwrap();

    let settle = |host: Arc<SyncHost>| async move {
        for _ in 0..200 {
            if host.get("quotes").await.unwrap().last_run.is_some_and(|r| r.status != RunStatus::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    // Counting starts at the slot the task was first seen in.
    host.tick().await.unwrap();
    assert_eq!(runner.calls(), 0);
    z.clock.advance(899_000);
    host.tick().await.unwrap();
    z.clock.advance(1_000);
    host.tick().await.unwrap();
    settle(host.clone()).await;
    assert_eq!(runner.calls(), 1);
    assert_eq!(host.runs("quotes", 1).await.unwrap()[0].trigger, "schedule");
    // Three slots pass while the host is down: the new host runs once.
    z.clock.advance(3 * 900_000);
    let after_restart = z.host(runner.clone());
    after_restart.tick().await.unwrap();
    settle(after_restart.clone()).await;
    after_restart.tick().await.unwrap();
    settle(after_restart.clone()).await;
    assert_eq!(runner.calls(), 2);
    // A disabled task is not run by its timetable.
    after_restart.set_enabled("quotes", false).await.unwrap();
    z.clock.advance(900_000);
    after_restart.tick().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(runner.calls(), 2);
}

#[tokio::test]
async fn try_runs_a_draft_once_and_writes_nothing() {
    let z = zone();
    let runner = Arc::new(Scripted::default()).then(transient());
    let host = z.host(runner.clone());
    let draft = json!({ "task_id": "news", "program": "export {};\n", "hosts": ["feeds.example.org"] });
    let v = host.try_run(serde_json::from_value(draft).unwrap()).await.unwrap();
    assert_eq!(v["run"]["status"], "failed");
    assert_eq!(v["preview"]["kind"], "transient");
    assert_eq!(runner.calls(), 1, "a try is not retried");
    let v = host.try_run(serde_json::from_value(json!({ "task_id": "news", "program": "export {};\n" })).unwrap()).await.unwrap();
    assert_eq!(v["preview"]["status"], "ok");
    assert!(!z.root.join("sync/news").exists(), "nothing of a draft is kept");
    assert!(std::fs::read_dir(z.root.join("sync/.try")).unwrap().next().is_none());
    assert!(host.try_run(serde_json::from_value(json!({ "task_id": "missing" })).unwrap()).await.is_err());
}

/// The real Deno: the program reads its input and reaches the declared host,
/// and nothing else — no environment (the host's own token included), no
/// files, no other host.
#[tokio::test]
async fn the_deno_runner_grants_the_declared_hosts_only() {
    let Some(deno) = DenoSyncRunner::discover() else {
        eprintln!("no Deno on this machine: skipped");
        return;
    };
    std::env::set_var("BUCKYOS_APP_TOKEN", "the-hosts-own-token");
    let allowed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (allowed_port, other_port) = (allowed.local_addr().unwrap().port(), other.local_addr().unwrap().port());
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut s, _)) = allowed.accept().await {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello").await;
        }
    });
    let tmp = tempfile::tempdir().unwrap();
    let secret_file = tmp.path().join("secret.txt");
    std::fs::write(&secret_file, "do not read").unwrap();
    let program = tmp.path().join("program.ts");
    std::fs::write(
        &program,
        format!(
            r#"
const input = JSON.parse(await new Response(Deno.stdin.readable).text());
const probe = async (what: string, f: () => Promise<unknown> | unknown) => {{
  try {{ return {{ what, ok: true, value: String(await f()) }}; }} catch (e) {{ return {{ what, ok: false, value: String(e) }}; }}
}};
console.error("a line of log");
const records = [
  await probe("env", () => Deno.env.get("BUCKYOS_APP_TOKEN")),
  await probe("read", () => Deno.readTextFile({file:?})),
  await probe("write", () => Deno.writeTextFile({out:?}, "x")),
  await probe("run", () => new Deno.Command("/bin/sh", {{ args: ["-c", "echo hi"] }}).output()),
  await probe("declared", async () => (await fetch("http://127.0.0.1:{allowed_port}/")).text()),
  await probe("undeclared", () => fetch("http://127.0.0.1:{other_port}/")),
  await probe("elsewhere", () => fetch("https://example.org/")),
];
console.log(JSON.stringify({{ status: "ok", records, state: {{ got: input.secrets.KEY, task: input.task_id }} }}));
"#,
            file = secret_file.display().to_string(),
            out = tmp.path().join("written.txt").display().to_string(),
        ),
    )
    .unwrap();
    let input = json!({ "task_id": "probe", "inputs": {}, "secrets": { "KEY": "v" }, "state": null });
    let run = deno.run(&program, &[format!("127.0.0.1:{allowed_port}")], &input).await;
    assert!(run.log.contains("a line of log"), "{}", run.log);
    let Attempt::Ok(result) = judge(&run) else {
        panic!("{:?} {}", judge(&run), run.log);
    };
    assert_eq!(result.state, Some(json!({ "got": "v", "task": "probe" })));
    let by: BTreeMap<String, (bool, String)> = result
        .records
        .unwrap()
        .into_iter()
        .map(|r| (r["what"].as_str().unwrap().to_string(), (r["ok"].as_bool().unwrap(), r["value"].as_str().unwrap().to_string())))
        .collect();
    assert_eq!(by["declared"], (true, "hello".to_string()));
    for what in ["env", "read", "write", "run", "undeclared", "elsewhere"] {
        let (ok, value) = &by[what];
        assert!(!ok && value.contains("NotCapable"), "{what}: {value}");
    }
    assert!(!tmp.path().join("written.txt").exists());
    drop(other);

    // A program that never ends is stopped and counts as transient; one that prints too much as a program error.
    let slow = tmp.path().join("slow.ts");
    std::fs::write(&slow, "await new Promise(() => setTimeout(() => {}, 60_000));\n").unwrap();
    let short = DenoSyncRunner { deno: deno.deno.clone(), timeout: Duration::from_secs(2) };
    match judge(&short.run(&slow, &[], &input).await) {
        Attempt::Failed { kind: FailureKind::Transient, error } => assert!(error.contains("within 2 s"), "{error}"),
        other => panic!("{other:?}"),
    }
    let loud = tmp.path().join("loud.ts");
    std::fs::write(&loud, "const line = \"x\".repeat(1 << 20) + \"\\n\";\nfor (let i = 0; i < 40; i++) await Deno.stdout.write(new TextEncoder().encode(line));\n").unwrap();
    match judge(&deno.run(&loud, &[], &input).await) {
        Attempt::Failed { kind: FailureKind::Program, error } => assert!(error.contains("32 MiB"), "{error}"),
        other => panic!("{other:?}"),
    }
}
