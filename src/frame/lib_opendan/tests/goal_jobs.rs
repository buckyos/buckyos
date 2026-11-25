//! Goal jobs (`libopendan::goal_jobs`): installing and changing them, running
//! them on their timetable without an inference, ending them with their Goal,
//! and their place in the budget. Programs are small shell scripts.
#![cfg(unix)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use libopendan::goal_jobs::*;
use libopendan::host::goals::{GoalClock, GoalHost, GoalHostOptions, ManualClock};
use libopendan::host::{HostDeps, Supervisor};
use libopendan::protocol::*;
use libopendan::runner::DefaultAssembler;
use libopendan::skills::SkillFile;
use libopendan::state::{ActivationQuery, AgentStateClient, GoalCaller, GoalCreate, GoalStatusChange};
use libopendan::{FsAgentStateClient, OpenDanError};
use serde_json::{json, Value};

struct Hosted {
    sup: Arc<Supervisor>,
    clock: Arc<ManualClock>,
    agent: Arc<FsAgentStateClient>,
    root: PathBuf,
}

fn hosted(env: &Env, llm: Arc<ScriptedLlm>) -> Hosted {
    hosted_at(env, llm, ManualClock::at(libopendan::now_ms()))
}

fn hosted_at(env: &Env, llm: Arc<ScriptedLlm>, clock: Arc<ManualClock>) -> Hosted {
    let mut opts = GoalHostOptions::new(env.agent_root.join("sessions"), json!({ "tools": { "enabled": true } }));
    opts.clock = clock.clone();
    opts.budget_every = Duration::ZERO;
    opts.job_retry = vec![Duration::from_millis(50), Duration::from_millis(50)];
    let mut deps = env.deps(llm);
    deps.options.goal_clock = GoalClock(clock.clone());
    let deps = HostDeps {
        who: APP.into(),
        agent: deps.agent.clone(),
        inputs: deps.inputs.clone(),
        waker: deps.waker.clone(),
        xllm: deps.xllm.clone(),
        assembler: Arc::new(DefaultAssembler::default()),
        session_cli: None,
        session_helpers: Vec::new(),
        app_tools: Vec::new(),
        context_windows: None,
        options: deps.options.clone(),
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
    Hosted { sup: Supervisor::new(deps, None).with_goals(GoalHost::new(opts)), clock, agent: env.agent(), root: env.agent_root.clone() }
}

impl Hosted {
    async fn tick(&self) {
        self.sup.tick().await.unwrap();
        self.sup.tick_goals().await.unwrap();
    }

    /// Host passes until the jobs that run have ended.
    async fn settle(&self) {
        self.tick().await;
        self.sup.goals().unwrap().jobs.settled().await;
        self.tick().await;
        self.sup.goals().unwrap().jobs.settled().await;
    }

    async fn until(&self, what: &str, f: impl AsyncFn(&Hosted) -> bool) {
        for _ in 0..400 {
            self.tick().await;
            if f(self).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn runs(&self, goal: &str, job: &str) -> Vec<JobRun> {
        self.agent.goals().job_runs(goal, job, 0).await.unwrap()
    }

    async fn goal(&self, objective: &str) -> GoalDefinition {
        let mut req = GoalCreate { objective: objective.into(), ..Default::default() };
        req.schedule.min_interval_secs = Some(0);
        self.agent.goals().create(&req, &owner()).await.unwrap()
    }

    fn files(&self, goal: &str) -> JobFiles {
        JobFiles::of(&self.root, goal)
    }
}

fn owner() -> GoalCaller {
    GoalCaller::owner(APP)
}

fn script(body: &str) -> JobPut {
    JobPut {
        job_id: "sync".into(),
        command: vec!["run.sh".into()],
        every_secs: Some(300),
        files: [("run.sh".to_string(), SkillFile::of(format!("#!/bin/sh\n{body}\n").as_bytes()))].into_iter().collect(),
        executable: vec!["run.sh".into()],
        ..Default::default()
    }
}

fn quiet() -> Arc<ScriptedLlm> {
    ScriptedLlm::new(|_, _| tool_call("r1", "report", json!({ "report": "looked", "is_end": true, "result": { "decision": "no_action" } })))
}

/// GJ1: what is installed, kept, gone back to and refused.
#[tokio::test]
async fn jobs_are_installed_as_versions() {
    let env = Env::new();
    let h = hosted(&env, quiet());
    let g = h.goal("keep the page up").await;
    let goals = h.agent.goals();
    let job = goals.job_put(&g.goal_id, &script("echo one"), &owner()).await.unwrap();
    assert_eq!((job.version, job.enabled, job.timeout_secs, job.title.as_str()), (1, true, JOB_TIMEOUT_SECS, "sync"));
    let files = h.files(&g.goal_id);
    std::fs::write(files.data_dir("sync").join("kept.txt"), "kept").unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(files.version_dir("sync", 1).join("run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
    for n in 2..=7u64 {
        let job = goals.job_put(&g.goal_id, &JobPut { title: Some("Sync the page".into()), ..script(&format!("echo {n}")) }, &owner()).await.unwrap();
        assert_eq!(job.version, n);
    }
    let v = goals.jobs(&g.goal_id).await.unwrap().remove(0);
    assert_eq!(v.versions, vec![3, 4, 5, 6, 7], "the newest five are kept");
    assert_eq!(v.job.title, "Sync the page");
    assert_eq!(std::fs::read_to_string(files.data_dir("sync").join("kept.txt")).unwrap(), "kept");
    assert_eq!(goals.job_rollback(&g.goal_id, "sync", None, &owner()).await.unwrap().version, 6);
    assert_eq!(goals.job_rollback(&g.goal_id, "sync", Some(3), &owner()).await.unwrap().version, 3);
    assert!(goals.job_rollback(&g.goal_id, "sync", Some(1), &owner()).await.is_err(), "not kept any more");
    assert_eq!(goals.job_put(&g.goal_id, &script("echo 8"), &owner()).await.unwrap().version, 8, "never a number twice");
    assert!(!goals.job_set_enabled(&g.goal_id, "sync", false, &owner()).await.unwrap().enabled);
    assert!(!goals.job_put(&g.goal_id, &script("echo 9"), &owner()).await.unwrap().enabled, "a new version leaves it off");
    let log = goals.changes(&g.goal_id, 0).await.unwrap();
    assert!(log.iter().filter(|c| c.kind == "job").count() >= 10);
    assert_eq!(goals.get(&g.goal_id).await.unwrap().unwrap().revision, 1, "no revision of the goal");
    // Refused.
    let bad = |f: fn(&mut JobPut)| {
        let mut req = script("true");
        f(&mut req);
        req
    };
    for (what, req) in [
        ("an id", bad(|r| r.job_id = "../x".into())),
        ("a command", bad(|r| r.command = vec![])),
        ("no files", bad(|r| r.files.clear())),
        ("a timetable", bad(|r| r.every_secs = None)),
        ("too often", bad(|r| r.every_secs = Some(60))),
        ("both", bad(|r| r.cron = Some("0 9 * * *".into()))),
        ("a zone", bad(|r| { r.every_secs = None; r.cron = Some("0 9 * * *".into()); r.timezone = Some("Mars/Base".into()) })),
        ("too long", bad(|r| r.timeout_secs = Some(601))),
        ("outside", bad(|r| { r.files.insert("../up.sh".into(), SkillFile::of(b"x")); })),
        ("absolute", bad(|r| { r.files.insert("/etc/x".into(), SkillFile::of(b"x")); })),
        ("too large", bad(|r| { r.files.insert("big".into(), SkillFile::of(&vec![b'x'; JOB_PROGRAM_MAX + 1])); })),
    ] {
        assert!(goals.job_put(&g.goal_id, &req, &owner()).await.is_err(), "{what}");
    }
    // A damaged definition is left out, the others stay.
    goals.job_put(&g.goal_id, &JobPut { job_id: "other".into(), ..script("true") }, &owner()).await.unwrap();
    std::fs::write(h.root.join("state/goals").join(&g.goal_id).join("jobs/other/job.json"), "{").unwrap();
    let ids: Vec<String> = goals.jobs(&g.goal_id).await.unwrap().into_iter().map(|j| j.job.job_id).collect();
    assert_eq!(ids, vec!["sync"]);
    goals.job_delete(&g.goal_id, "sync", &owner()).await.unwrap();
    assert!(goals.jobs(&g.goal_id).await.unwrap().is_empty());
    assert!(matches!(goals.job_delete(&g.goal_id, "sync", &owner()).await, Err(OpenDanError::NotFound(_))));
    h.sup.shutdown().await;
}

/// GJ1: installs at the same time each get a version of their own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installs_at_the_same_time_do_not_mix() {
    let env = Env::new();
    let h = hosted(&env, quiet());
    let g = h.goal("busy").await;
    let puts = (0..8).map(|n| {
        let (agent, id) = (h.agent.clone(), g.goal_id.clone());
        tokio::spawn(async move { agent.goals().job_put(&id, &script(&format!("echo {n}")), &owner()).await.unwrap().version })
    });
    let mut versions = Vec::new();
    for p in puts {
        versions.push(p.await.unwrap());
    }
    versions.sort_unstable();
    assert_eq!(versions, (1..=8).collect::<Vec<u64>>());
    h.sup.shutdown().await;
}

/// GJ2: a new version runs at once, then on its timetable (slots that passed
/// count once); the environment, the result, the log; nothing infers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn jobs_run_on_their_timetable_without_an_inference() {
    let env = Env::new();
    let llm = quiet();
    let h = hosted(&env, llm.clone());
    let g = h.goal("keep the page up").await;
    let goals = h.agent.goals();
    std::fs::create_dir_all(env.agent_root.join("tools")).unwrap();
    let tool = env.agent_root.join("tools/hello-tool");
    std::fs::write(&tool, "#!/bin/sh\necho from the tools directory\n").unwrap();
    std::fs::set_permissions(&tool, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let body = r#"hello-tool
echo n >> count.txt
echo "session=[$OPENDAN_SESSION_ID] goal=$OPENDAN_GOAL_ID job=$OPENDAN_JOB_ID agent=$OPENDAN_AGENT_DID root=$OPENDAN_AGENT_ROOT arg=$1" >&2
cat "$OPENDAN_JOB_DIR/data.txt"
echo '{"summary": "2 sites up", "metrics": {"up": 2}}'
"#;
    let mut req = script(body);
    req.command = vec!["run.sh".into(), "{dir}/data.txt".into()];
    req.files.insert("data.txt".into(), SkillFile::of(b"shipped with the program\n"));
    goals.job_put(&g.goal_id, &req, &owner()).await.unwrap();
    std::env::set_var("OPENDAN_SESSION_ID", "must-not-leak");
    h.settle().await;
    std::env::remove_var("OPENDAN_SESSION_ID");
    let runs = h.runs(&g.goal_id, "sync").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    let r = &runs[0];
    assert_eq!((r.status, r.trigger.as_str(), r.version, r.attempts), (JobRunStatus::Ok, "install", 1, 1), "{r:?}");
    assert_eq!((r.summary.as_str(), r.metrics.as_ref().unwrap()["up"].as_u64()), ("2 sites up", Some(2)));
    let log = goals.job_log(&g.goal_id, "sync", None).await.unwrap().unwrap();
    for needle in [
        "from the tools directory",
        "shipped with the program",
        "session=[] ",
        &format!("goal={}", g.goal_id),
        "job=sync",
        &format!("agent={AGENT}"),
        "versions/1/data.txt",
    ] {
        assert!(log.contains(needle), "{needle} missing in\n{log}");
    }
    assert_eq!(std::fs::read_to_string(h.files(&g.goal_id).data_dir("sync").join("count.txt")).unwrap(), "n\n");
    // Not again before its slot; four slots later it runs once.
    h.settle().await;
    assert_eq!(h.runs(&g.goal_id, "sync").await.len(), 1);
    h.clock.advance(4 * 300_000);
    h.settle().await;
    let runs = h.runs(&g.goal_id, "sync").await;
    assert_eq!((runs.len(), runs[1].trigger.as_str()), (2, "schedule"));
    // A new version runs at once, in the same data directory.
    goals.job_put(&g.goal_id, &script("wc -l < count.txt"), &owner()).await.unwrap();
    h.settle().await;
    let runs = h.runs(&g.goal_id, "sync").await;
    assert_eq!((runs.len(), runs[2].trigger.as_str(), runs[2].version), (3, "install", 2));
    assert_eq!(goals.job_log(&g.goal_id, "sync", None).await.unwrap().unwrap().trim(), "2");
    // Asked for: at once, once.
    let req = goals.job_run(&g.goal_id, "sync", &owner()).await.unwrap();
    h.settle().await;
    let runs = h.runs(&g.goal_id, "sync").await;
    assert_eq!((runs.len(), runs[3].trigger.as_str(), runs[3].request_id.as_deref()), (4, "manual", Some(req.request_id.as_str())));
    // Switched off: neither its slots nor a request.
    goals.job_set_enabled(&g.goal_id, "sync", false, &owner()).await.unwrap();
    assert!(goals.job_run(&g.goal_id, "sync", &owner()).await.is_err());
    h.clock.advance(900_000);
    h.settle().await;
    assert_eq!(h.runs(&g.goal_id, "sync").await.len(), 4);
    let v = goals.jobs(&g.goal_id).await.unwrap().remove(0);
    assert_eq!((v.next_at_ms, v.failures), (None, 0));
    assert_eq!(llm.count(), 0, "no inference");
    assert!(goals.activations(&g.goal_id, &ActivationQuery::default()).await.unwrap().is_empty());
    h.sup.shutdown().await;
}

/// GJ2: a passing failure is tried again, any other is not; a run past its
/// time is ended with everything it started; failures are counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failures_are_told_apart() {
    let env = Env::new();
    let h = hosted(&env, quiet());
    let g = h.goal("fails").await;
    let goals = h.agent.goals();
    let transient = r#"echo n >> tries.txt; test "$(wc -l < tries.txt)" -ge 3 && { echo done; exit 0; }
echo '{"kind":"transient","summary":"503"}'; exit 1"#;
    goals.job_put(&g.goal_id, &script(transient), &owner()).await.unwrap();
    goals.job_put(&g.goal_id, &JobPut { job_id: "login".into(), ..script(r#"echo '{"kind":"auth","summary":"login page"}'; exit 4"#) }, &owner()).await.unwrap();
    h.settle().await;
    let r = h.runs(&g.goal_id, "sync").await.remove(0);
    assert_eq!((r.status, r.attempts), (JobRunStatus::Ok, 3), "{r:?}");
    let r = h.runs(&g.goal_id, "login").await.remove(0);
    assert_eq!((r.status, r.attempts, r.kind.as_deref(), r.exit_code, r.summary.as_str()), (JobRunStatus::Failed, 1, Some("auth"), Some(4), "login page"));
    // Always passing: three attempts, then failed.
    goals.job_put(&g.goal_id, &script(r#"echo '{"kind":"transient"}'; exit 1"#), &owner()).await.unwrap();
    h.settle().await;
    let r = h.runs(&g.goal_id, "sync").await.pop().unwrap();
    assert_eq!((r.status, r.attempts, r.kind.as_deref()), (JobRunStatus::Failed, 3, Some("transient")));
    // A program that cannot be started, one that just fails.
    goals.job_put(&g.goal_id, &JobPut { command: vec!["no-such-program".into()], ..script("true") }, &owner()).await.unwrap();
    h.settle().await;
    let r = h.runs(&g.goal_id, "sync").await.pop().unwrap();
    assert_eq!((r.status, r.kind.as_deref()), (JobRunStatus::Failed, Some("program")), "{r:?}");
    goals.job_put(&g.goal_id, &script("echo broken >&2; exit 7"), &owner()).await.unwrap();
    h.settle().await;
    let v = goals.jobs(&g.goal_id).await.unwrap().into_iter().find(|v| v.job.job_id == "sync").unwrap();
    let last = v.last_run.unwrap();
    assert_eq!((v.failures, last.exit_code, last.kind.as_deref(), last.summary.as_str()), (3, Some(7), Some("program"), "exit 7"));
    // Past its time: the whole group ends, what it started included.
    let mark = env.root.join("alive");
    let body = format!("(sleep 3; touch {m}) &\nsleep 30", m = mark.display());
    goals.job_put(&g.goal_id, &JobPut { timeout_secs: Some(1), ..script(&body) }, &owner()).await.unwrap();
    h.settle().await;
    let r = h.runs(&g.goal_id, "sync").await.pop().unwrap();
    assert_eq!((r.status, r.kind.clone()), (JobRunStatus::Timeout, None), "{r:?}");
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert!(!mark.exists(), "its children ended with it");
    h.sup.shutdown().await;
}

/// GJ2: at most two jobs of an agent run at once, never one job twice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_jobs_run_at_once_never_one_twice() {
    let env = Env::new();
    let h = hosted(&env, quiet());
    let (a, b) = (h.goal("a").await, h.goal("b").await);
    let dir = env.root.join("live");
    std::fs::create_dir_all(&dir).unwrap();
    let body = format!("touch {d}/$OPENDAN_JOB_RUN_ID; sleep 0.4; ls {d} | wc -l; rm {d}/$OPENDAN_JOB_RUN_ID", d = dir.display());
    for (g, id) in [(&a, "one"), (&a, "two"), (&b, "three"), (&b, "four")] {
        h.agent.goals().job_put(&g.goal_id, &JobPut { job_id: id.into(), ..script(&body) }, &owner()).await.unwrap();
    }
    let mut most = 0;
    for _ in 0..80 {
        h.tick().await;
        most = most.max(std::fs::read_dir(&dir).unwrap().count());
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(most, JOBS_AT_ONCE);
    for (g, id) in [(&a, "one"), (&a, "two"), (&b, "three"), (&b, "four")] {
        assert_eq!(h.runs(&g.goal_id, id).await.len(), 1, "{id}");
    }
    h.sup.shutdown().await;
}

/// GJ2: a job ends when its Goal sleeps, is switched off or ends, and runs
/// again when it wakes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn jobs_live_with_their_goal() {
    let env = Env::new();
    let h = hosted(&env, quiet());
    let g = h.goal("sleeps").await;
    let goals = h.agent.goals();
    goals.job_put(&g.goal_id, &script("sleep 20"), &owner()).await.unwrap();
    let running = async |h: &Hosted| h.agent.goals().jobs(&g.goal_id).await.unwrap()[0].running.is_some();
    h.until("it runs", running).await;
    let status = |s: GoalStatus| GoalStatusChange { status: s, reason: Some("x".into()), evidence: None };
    goals.set_status(&g.goal_id, &status(GoalStatus::Paused), &owner()).await.unwrap();
    h.until("it ended", async |h| h.runs(&g.goal_id, "sync").await.len() == 1).await;
    let r = h.runs(&g.goal_id, "sync").await.remove(0);
    assert_eq!(r.status, JobRunStatus::Killed);
    h.clock.advance(900_000);
    for _ in 0..5 {
        h.tick().await;
    }
    assert!(!running(&h).await, "a sleeping goal runs none");
    assert!(goals.job_run(&g.goal_id, "sync", &owner()).await.is_err());
    goals.set_status(&g.goal_id, &status(GoalStatus::Active), &owner()).await.unwrap();
    h.until("it runs again", running).await;
    // Switched off while it runs, then on.
    goals.job_set_enabled(&g.goal_id, "sync", false, &owner()).await.unwrap();
    h.until("ended", async |h| h.runs(&g.goal_id, "sync").await.len() == 2).await;
    goals.job_set_enabled(&g.goal_id, "sync", true, &owner()).await.unwrap();
    h.until("on again", running).await;
    goals.set_status(&g.goal_id, &status(GoalStatus::Archived), &owner()).await.unwrap();
    h.until("ended with its goal", async |h| h.runs(&g.goal_id, "sync").await.len() == 3).await;
    assert!(h.runs(&g.goal_id, "sync").await.iter().all(|r| r.status == JobRunStatus::Killed));
    h.sup.shutdown().await;
}

/// GJ2: a run whose host died is recorded as killed by the next one, its
/// process is ended and the job runs again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_whose_host_died_is_recorded_and_made_up() {
    let env = Env::new();
    let h = hosted(&env, quiet());
    let g = h.goal("survives").await;
    let goals = h.agent.goals();
    goals.job_put(&g.goal_id, &script("echo again"), &owner()).await.unwrap();
    h.settle().await;
    // What a killed host leaves: a run going on, its process alive.
    use std::os::unix::process::CommandExt;
    let mut orphan = std::process::Command::new("sleep")
        .arg("60")
        .env(ENV_JOB_RUN_ID, "20261124-000000-dead00")
        .process_group(0)
        .spawn()
        .unwrap();
    let path = h.root.join("state/goals").join(&g.goal_id).join("jobs/sync/schedule.json");
    let mut s: JobSchedule = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    s.running = Some(JobRunning {
        run_id: "20261124-000000-dead00".into(),
        version: 1,
        at_ms: libopendan::now_ms() - 1000,
        trigger: "schedule".into(),
        pid: Some(orphan.id()),
    });
    std::fs::write(&path, serde_json::to_vec(&s).unwrap()).unwrap();
    h.sup.shutdown().await;
    let h = hosted(&env, quiet());
    h.settle().await;
    let runs = h.runs(&g.goal_id, "sync").await;
    let killed = runs.iter().find(|r| r.run_id == "20261124-000000-dead00").unwrap();
    assert_eq!((killed.status, killed.summary.contains("host stopped")), (JobRunStatus::Killed, true));
    assert_eq!(runs.last().unwrap().status, JobRunStatus::Ok, "made up: {runs:?}");
    if cfg!(target_os = "linux") {
        for _ in 0..40 {
            if orphan.try_wait().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(orphan.try_wait().unwrap().is_some(), "the orphan was ended");
    }
    let _ = orphan.kill();
    let _ = orphan.wait();
    h.sup.shutdown().await;
}

/// GJ3: the rules are those of the check script; kRPC carries the calls.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn who_may_change_jobs() {
    use libopendan::state::krpc::{serve_call, KrpcAgentStateClient, StateTransport};
    let env = Env::new();
    let h = hosted(&env, quiet());
    let g = h.goal("owner's").await;
    let other = h.goal("another").await;
    h.tick().await;
    let goals = h.agent.goals();
    let from = |sid: &str| GoalCaller::from_session(APP, sid);
    // The Goal itself.
    goals.job_put(&g.goal_id, &script("true"), &from(&g.session.sid)).await.unwrap();
    assert!(goals.jobs(&g.goal_id).await.unwrap()[0].job.updated_by.starts_with("goal_session:"));
    let work = env.create_work(work_spec("a task")).await;
    for sid in [other.session.sid.as_str(), work.sid()] {
        let c = from(sid);
        assert!(matches!(goals.job_put(&g.goal_id, &script("true"), &c).await, Err(OpenDanError::Forbidden(_))), "{sid}");
        assert!(matches!(goals.job_run(&g.goal_id, "sync", &c).await, Err(OpenDanError::Forbidden(_))), "{sid}");
        assert!(matches!(goals.job_set_enabled(&g.goal_id, "sync", false, &c).await, Err(OpenDanError::Forbidden(_))), "{sid}");
        assert!(matches!(goals.job_rollback(&g.goal_id, "sync", None, &c).await, Err(OpenDanError::Forbidden(_))), "{sid}");
        assert!(matches!(goals.job_delete(&g.goal_id, "sync", &c).await, Err(OpenDanError::Forbidden(_))), "{sid}");
    }
    struct Loop(Arc<FsAgentStateClient>);
    #[async_trait::async_trait]
    impl StateTransport for Loop {
        async fn call(&self, method: &str, params: Value) -> libopendan::Result<Value> {
            serve_call(self.0.as_ref(), APP, method, &params).await.expect("an agent state method")
        }
    }
    let remote = KrpcAgentStateClient::new(AGENT, Arc::new(Loop(h.agent.clone())));
    let job = remote.goals().job_put(&g.goal_id, &script("echo over krpc"), &owner()).await.unwrap();
    assert_eq!(job.version, 2);
    remote.goals().job_run(&g.goal_id, "sync", &owner()).await.unwrap();
    h.settle().await;
    let runs = remote.goals().job_runs(&g.goal_id, "sync", 1).await.unwrap();
    assert_eq!(runs[0].status, JobRunStatus::Ok);
    assert_eq!(remote.goals().job_log(&g.goal_id, "sync", Some(&runs[0].run_id)).await.unwrap().as_deref(), Some("over krpc\n"));
    assert_eq!(remote.goals().jobs(&g.goal_id).await.unwrap()[0].versions, vec![1, 2]);
    assert!(!remote.goals().job_set_enabled(&g.goal_id, "sync", false, &owner()).await.unwrap().enabled);
    assert_eq!(remote.goals().job_rollback(&g.goal_id, "sync", None, &owner()).await.unwrap().version, 1);
    remote.goals().job_delete(&g.goal_id, "sync", &owner()).await.unwrap();
    h.sup.shutdown().await;
}

/// GJ4: the check script and the Goal header see how the jobs went.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_check_and_the_header_see_the_jobs() {
    use libopendan::goal_check::{check_ctx, CheckWhen};
    let env = Env::new();
    let llm = quiet();
    let h = hosted(&env, llm.clone());
    let g = h.goal("watch the sites").await;
    let goals = h.agent.goals();
    goals.job_put(&g.goal_id, &JobPut { title: Some("Check the sites".into()), ..script(r#"echo '{"summary":"site B is down","kind":"program"}'; exit 1"#) }, &owner()).await.unwrap();
    h.settle().await;
    let jobs = goals.jobs(&g.goal_id).await.unwrap();
    let ctx = check_ctx(&g, None, None, &jobs, Some(&h.root), &CheckWhen::new("scheduled"), libopendan::now_ms());
    let j = &ctx["jobs"][0];
    assert_eq!((j["job_id"].as_str(), j["version"].as_u64(), j["failures"].as_u64(), j["enabled"].as_bool()), (Some("sync"), Some(1), Some(1), Some(true)));
    assert_eq!((j["last_run"]["status"].as_str(), j["last_run"]["summary"].as_str()), (Some("failed"), Some("site B is down")));
    assert!(j["next_at_ms"].as_u64().unwrap() > libopendan::now_ms() - 1000);
    h.sup.goal_activate(&g.goal_id, "r1", "look").await.unwrap();
    h.until("the activation", async |h| h.agent.goals().activations(&g.goal_id, &ActivationQuery::default()).await.unwrap().len() == 1).await;
    let header = llm.requests.lock().unwrap().iter().flat_map(user_texts).find(|t| t.starts_with("<goal_activation")).unwrap();
    for needle in ["<jobs run=\"installed copies", "<job id=\"sync\" version=\"1\"", "last=\"failed\"", "failures_in_a_row=\"1\"", "Check the sites", "site B is down", "tokens_used_by_jobs=\"0\""] {
        assert!(header.contains(needle), "{needle} missing in\n{header}");
    }
    h.sup.shutdown().await;
}

/// GJ5: what a job's models use counts for the Goal's period and the agent's
/// total; when it is used up nothing infers, the jobs still run and are told
/// that no model tokens are left.
/// V10: the routine infers nothing; a job that keeps failing wakes its Goal
/// through the check (here the host's clock is behind the Goal's creation).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_that_keeps_failing_wakes_its_goal() {
    let env = Env::new();
    if libopendan::goal_check::DenoCheckRunner::discover(env.root.join("checks")).is_none() {
        eprintln!("no Deno: skipped");
        return;
    }
    let clock = ManualClock::at(libopendan::now_ms() - 120_000);
    let llm = quiet();
    let h = hosted_at(&env, llm.clone(), clock);
    let mut req = GoalCreate {
        objective: "keep the summary".into(),
        check_script: Some("export default (ctx, state) => {\n  const j = ctx.jobs.find((x) => x.job_id === 'sync');\n  if (j && j.failures >= 2 && state?.told !== j.last_run?.run_id) return { run: true, reason: `sync failed ${j.failures} times`, state: { told: j.last_run.run_id } };\n  return { run: false, state };\n};\n".into()),
        ..Default::default()
    };
    req.schedule.every_secs = Some(3600);
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    let ok = env.root.join("ok");
    std::fs::write(&ok, "").unwrap();
    let body = format!("test -e {}", ok.display());
    h.agent.goals().job_put(&g.goal_id, &JobPut { every_secs: Some(3600), ..script(&body) }, &owner()).await.unwrap();
    h.settle().await;
    let acts = async |h: &Hosted| h.agent.goals().activations(&g.goal_id, &ActivationQuery::default()).await.unwrap();
    for _ in 0..3 {
        h.clock.advance(3_600_000);
        h.settle().await;
    }
    assert!(acts(&h).await.is_empty(), "the routine infers nothing");
    assert!(h.runs(&g.goal_id, "sync").await.iter().all(|r| r.status == JobRunStatus::Ok));
    std::fs::remove_file(&ok).unwrap();
    for _ in 0..4 {
        h.clock.advance(3_600_000);
        h.settle().await;
    }
    h.until("the activation", async |h| !acts(h).await.is_empty()).await;
    let a = &acts(&h).await[0];
    assert!(a.reasons[0].note.as_deref().unwrap().contains("sync failed 2 times"), "{:?}", a.reasons);
    h.sup.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn model_tokens_of_jobs_are_the_goals() {
    let env = Env::new();
    let llm = quiet();
    let h = hosted(&env, llm.clone());
    let mut req = GoalCreate { objective: "translate".into(), ..Default::default() };
    req.schedule.min_interval_secs = Some(0);
    req.schedule.every_secs = Some(300);
    req.budget.period.max_total_tokens = Some(1000);
    let g = h.agent.goals().create(&req, &owner()).await.unwrap();
    let goals = h.agent.goals();
    h.tick().await;
    let body = r#"echo "budget=$OPENDAN_MODEL_BUDGET"
test "$OPENDAN_MODEL_BUDGET" -gt 0 || { echo "no tokens left"; exit 1; }
echo "{\"at_ms\": NOW, \"model\": \"m\", \"run_id\": \"$OPENDAN_JOB_RUN_ID\", \"input_tokens\": 600, \"output_tokens\": 0, \"total_tokens\": 600}" >> "$OPENDAN_USAGE_LOG""#;
    let body = body.replace("NOW", &libopendan::now_ms().to_string());
    goals.job_put(&g.goal_id, &script(&body), &owner()).await.unwrap();
    h.settle().await;
    let r = h.runs(&g.goal_id, "sync").await.remove(0);
    assert_eq!((r.status, r.tokens), (JobRunStatus::Ok, 600));
    assert!(goals.job_log(&g.goal_id, "sync", None).await.unwrap().unwrap().contains("budget=1000"));
    goals.job_run(&g.goal_id, "sync", &owner()).await.unwrap();
    h.settle().await;
    assert!(goals.job_log(&g.goal_id, "sync", None).await.unwrap().unwrap().contains("budget=400"));
    // 1200 of 1000: nothing infers, the job runs and ends for the budget.
    h.clock.advance(300_000);
    h.settle().await;
    let runs = h.runs(&g.goal_id, "sync").await;
    let last = runs.last().unwrap();
    assert_eq!((runs.len(), last.status, last.kind.as_deref()), (3, JobRunStatus::Failed, Some("budget")), "{last:?}");
    assert!(goals.job_log(&g.goal_id, "sync", None).await.unwrap().unwrap().contains("budget=0"));
    let s = goals.schedule(&g.goal_id).await.unwrap().unwrap();
    assert!(s.skipped.iter().any(|k| k.reason == "period_budget"), "{s:?}");
    assert!(goals.activations(&g.goal_id, &ActivationQuery::default()).await.unwrap().is_empty());
    assert_eq!(llm.count(), 0);
    let t = goals.stats(&g.goal_id, 1).await.unwrap();
    assert_eq!((t.periods[0].tokens_by_jobs, t.periods[0].job_runs_ok, t.periods[0].job_runs_failed), (1200, 2, 1));
    // Deleting the job takes none of it back.
    goals.job_delete(&g.goal_id, "sync", &owner()).await.unwrap();
    assert_eq!(goals.job_usage(&g.goal_id, 0).await.unwrap().len(), 2);
    h.sup.shutdown().await;
}

#[test]
fn a_command_line_is_split_without_a_shell() {
    let w = |l: &str| split_command(l).unwrap();
    assert_eq!(w("deno run -A {dir}/main.ts"), ["deno", "run", "-A", "{dir}/main.ts"]);
    assert_eq!(w(r#"run.sh "two words" 'a "b"' c\ d """#), ["run.sh", "two words", "a \"b\"", "c d", ""]);
    assert!(split_command("run.sh 'open").is_err());
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("lib")).unwrap();
    std::fs::write(dir.path().join("lib/a.txt"), "a").unwrap();
    std::fs::write(dir.path().join("run.sh"), "x").unwrap();
    std::fs::set_permissions(dir.path().join("run.sh"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let (files, exec) = read_program_dir(dir.path()).unwrap();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["lib/a.txt", "run.sh"]);
    assert_eq!(exec, ["run.sh"]);
    std::os::unix::fs::symlink("/etc/passwd", dir.path().join("link")).unwrap();
    assert!(read_program_dir(dir.path()).is_err());
}

