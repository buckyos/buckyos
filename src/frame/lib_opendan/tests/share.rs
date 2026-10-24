//! "Agent shares its results" (notepads/agent-homestation-auto-share-todo.md
//! §5): a user task — a Work created under a UI session with `--report
//! final` — that ends with a final report becomes one post of the agent,
//! written by the model without private details; nothing else is shared.

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use libopendan::api::{create_sub_session, SessionSpec, SubSessionSpec, SubWorkspace};
use libopendan::protocol::*;
use libopendan::runner::{drive, RunnerDeps, StopWhen};
use libopendan::share::{LocalShare, SharePost, SHARE_MARKER, SHARE_TAG};
use libopendan::{SessionDir, SessionTemplate};
use serde_json::json;

fn ui_spec() -> SessionSpec {
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({
        "tools": { "enabled": true, "tools": [{ "groupname": "bash" }, { "name": "report" }] }
    });
    spec
}

/// The Work intent analysis creates: a sub session of the UI session that
/// reports its end to it.
async fn user_task(env: &Env, ui: &SessionDir, objective: &str, report: ReportMode) -> SessionDir {
    let sub = SubSessionSpec {
        objective: objective.into(),
        msgs: vec!["the user's answers".into()],
        workspace: SubWorkspace::None,
        report,
        key: Some(format!("k-{objective}")),
        ..Default::default()
    };
    create_sub_session(env.agent().as_ref(), APP, ui.sid(), sub, env.channels().as_ref())
        .await
        .unwrap()
}

fn is_compose(req: &llm_context::deps::LlmInferenceRequest) -> bool {
    render(&req.messages).contains(SHARE_MARKER)
}

fn final_report(report: &str) -> buckyos_api::AiResponse {
    tool_call(
        "final",
        "report",
        json!({ "report": report, "artifacts": ["answer.txt"], "is_end": true }),
    )
}

fn composed(share: bool, text: &str) -> buckyos_api::AiResponse {
    common::text(
        &json!({
            "share": share,
            "reason": if share { "reusable approach" } else { "personal matter" },
            "text": text,
            "tags": ["Rust", " testing ", SHARE_TAG]
        })
        .to_string(),
    )
}

fn deps_with_share(env: &Env, llm: Arc<ScriptedLlm>) -> (RunnerDeps, LocalShare) {
    let local = LocalShare::enable(&env.agent_root).unwrap();
    let mut deps = env.deps(llm);
    deps.share = Some(Arc::new(local.clone()));
    (deps, local)
}

/// The share runs after the run's end, off the drive: wait for it.
async fn wait_shares(local: &LocalShare, n: usize) -> Vec<SharePost> {
    let started = Instant::now();
    loop {
        let all = local.all().unwrap();
        if all.len() >= n || started.elapsed() > Duration::from_secs(10) {
            return all;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn settled(llm: &ScriptedLlm, calls: usize) {
    let started = Instant::now();
    while llm.count() < calls && started.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// run.json `host.extra.shared` of the session's last run, once it is set.
async fn wait_shared(sd: &SessionDir) -> bool {
    let started = Instant::now();
    loop {
        let state = sd.state().unwrap();
        let record = sd.runs().record(state.last_run.as_ref().unwrap()).unwrap();
        let shared = record.host.and_then(|h| h.extra.get("shared").cloned()) == Some(json!(true));
        if shared || started.elapsed() > Duration::from_secs(10) {
            return shared;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn work_report_spec(objective: &str) -> SessionSpec {
    let mut spec = work_spec(objective);
    spec.prompt.llm_context = ui_spec().prompt.llm_context;
    spec
}

/// A successful user task with a final report is composed into one post:
/// the model sees the objective, the report and the artifact names (never
/// paths), the post carries the fixed tag first, the model's tags cleaned,
/// and the run remembers it was shared.
#[tokio::test]
async fn a_finished_user_task_is_shared_once_as_the_agents_post() {
    let env = Env::new();
    let ui = env.create_work(ui_spec()).await;
    let work = user_task(&env, &ui, "sort the photo archive by year", ReportMode::Final).await;
    std::fs::write(work.path().join("answer.txt"), "done\n").unwrap();
    let llm = ScriptedLlm::new(|req, _| {
        if is_compose(req) {
            let prompt = last_user_text(req);
            assert!(prompt.contains("sort the photo archive by year"), "{prompt}");
            assert!(prompt.contains("renamed 300 files"), "{prompt}");
            assert!(prompt.contains("answer.txt"), "{prompt}");
            assert!(!prompt.contains(".opendan_agent_session"), "{prompt}");
            assert!(prompt.contains("Never include names of people"), "{prompt}");
            assert!(req.force_json);
            return composed(true, "Sorted a photo archive by year: read EXIF dates, moved files into year folders, verified counts per folder.");
        }
        final_report("renamed 300 files into year folders")
    });
    let (deps, local) = deps_with_share(&env, llm.clone());
    let result = drive(&work, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    let posts = wait_shares(&local, 1).await;
    assert_eq!(posts.len(), 1, "{posts:?}");
    let post = &posts[0];
    assert_eq!(post.key, format!("share:{}", work.sid()));
    assert_eq!(post.session_id, work.sid());
    assert!(post.text.starts_with("Sorted a photo archive"), "{}", post.text);
    assert_eq!(post.tags, vec![SHARE_TAG, "rust", "testing"]);
    assert!(post.artifacts.is_empty(), "file artifacts without an object are named, not attached: {post:?}");
    assert_eq!(llm.count(), 2);
    assert!(wait_shared(&work).await);
    // The UI session itself never shares its replies.
    assert!(!local.all().unwrap().iter().any(|p| p.session_id == ui.sid()));
}

/// The model may refuse: a personal matter stays unshared, and the run is
/// not asked again.
#[tokio::test]
async fn the_model_can_decline_to_share() {
    let env = Env::new();
    let ui = env.create_work(ui_spec()).await;
    let work = user_task(&env, &ui, "book the dentist appointment", ReportMode::Final).await;
    std::fs::write(work.path().join("answer.txt"), "booked\n").unwrap();
    let llm = ScriptedLlm::new(|req, _| {
        if is_compose(req) {
            return composed(false, "");
        }
        final_report("booked for Tuesday")
    });
    let (deps, local) = deps_with_share(&env, llm.clone());
    assert!(drive(&work, &deps, StopWhen::Finished).await.is_finished());
    assert!(wait_shared(&work).await);
    assert_eq!(llm.count(), 2);
    assert!(local.all().unwrap().is_empty());
}

/// Not user tasks: a Work without a UI parent, a sub session that does not
/// report its end, a UI session's own turn, a failed user task, a host
/// without a share service. None of them asks the model for a post.
#[tokio::test]
async fn only_successful_reporting_user_tasks_are_shared() {
    let env = Env::new();
    let ui = env.create_work(ui_spec()).await;
    let llm = ScriptedLlm::new(|req, _| {
        assert!(!is_compose(req), "nothing here may be shared");
        final_report("plain result")
    });
    let (deps, local) = deps_with_share(&env, llm.clone());

    let direct = env.create_work(work_report_spec("a work of its own")).await;
    std::fs::write(direct.path().join("answer.txt"), "x\n").unwrap();
    assert!(drive(&direct, &deps, StopWhen::Finished).await.is_finished());

    let silent = user_task(&env, &ui, "a quiet sub task", ReportMode::None).await;
    std::fs::write(silent.path().join("answer.txt"), "x\n").unwrap();
    assert!(drive(&silent, &deps, StopWhen::Finished).await.is_finished());

    let unserved = user_task(&env, &ui, "a task of a host without sharing", ReportMode::Final).await;
    std::fs::write(unserved.path().join("answer.txt"), "x\n").unwrap();
    assert!(drive(&unserved, &env.deps(llm.clone()), StopWhen::Finished).await.is_finished());

    // A failed user task (the Work tried to wait for the user).
    let mut behavior_ui = ui_spec();
    behavior_ui.prompt.llm_context =
        json!({ "loop_model": "behavior", "tools": { "enabled": true, "tools2actions": true } });
    behavior_ui.route_key = Some("behavior-ui".into());
    let ui2 = env.create_work(behavior_ui).await;
    let failing = user_task(&env, &ui2, "pick a color", ReportMode::Final).await;
    let waiting = ScriptedLlm::new(|req, _| {
        assert!(!is_compose(req), "a failed task is not shared");
        common::text("<response><report><![CDATA[which color?]]></report><next_behavior>WAIT_USER_MSG</next_behavior></response>")
    });
    let (deps2, _) = deps_with_share(&env, waiting.clone());
    assert!(drive(&failing, &deps2, StopWhen::Finished).await.is_finished());
    assert_eq!(failing.state().unwrap().outcome, Some(Outcome::Failed));

    settled(&llm, 3).await;
    assert_eq!(llm.count(), 3);
    assert_eq!(waiting.count(), 1);
    assert!(local.all().unwrap().is_empty());
}

const CRASH_ROOT: &str = "LIBOPENDAN_SHARE_TEST_ROOT";
const CRASH_SESSION: &str = "LIBOPENDAN_SHARE_TEST_SESSION";

fn crash_script() -> Arc<ScriptedLlm> {
    ScriptedLlm::new(|req, _| {
        if is_compose(req) {
            return composed(true, "Recovered and shared once.");
        }
        final_report("committed before the crash")
    })
}

#[tokio::test]
async fn share_crash_child_driver() {
    let (Ok(root), Ok(session)) = (std::env::var(CRASH_ROOT), std::env::var(CRASH_SESSION)) else {
        return;
    };
    let env = Env::at(Path::new(&root));
    let sd = SessionDir::open(&session).unwrap();
    let (deps, _) = deps_with_share(&env, crash_script());
    let result = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(result.is_finished(), "{result:?}");
    // Keep the process alive for the share that follows the run's end.
    tokio::time::sleep(Duration::from_secs(5)).await;
}

/// The end of a run redone after a crash (before its commit) shares exactly
/// once; a run that already shared before the crash is not asked again.
/// (A crash between the commit and the share loses the post: like the
/// perception and report that follow the commit, sharing is best effort.)
#[tokio::test]
async fn a_redone_run_end_shares_exactly_once() {
    for (fault, shared_before) in [("finish_run:after_flush", false), ("share:after_publish", true)] {
        let env = Env::new();
        let ui = env.create_work(ui_spec()).await;
        let work = user_task(&env, &ui, "survive a crash", ReportMode::Final).await;
        std::fs::write(work.path().join("answer.txt"), "x\n").unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "share_crash_child_driver", "--nocapture", "--test-threads=1"])
            .env(CRASH_ROOT, &env.root)
            .env(CRASH_SESSION, work.path())
            .env("LIBOPENDAN_FAULT", fault)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child did not reach {fault}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(!status.success(), "fault not reached: {fault}");
        let local = LocalShare::enable(&env.agent_root).unwrap();
        assert_eq!(local.all().unwrap().len(), usize::from(shared_before), "{fault}");
        let llm = ScriptedLlm::new(move |req, n| {
            assert!(is_compose(req), "{fault}: only the post may be composed on the redo ({n})");
            assert!(!shared_before, "{fault}: a shared run is not composed again");
            composed(true, "Recovered and shared once.")
        });
        let (deps, _) = deps_with_share(&env, llm.clone());
        // A session that finished (and shared) before the crash has nothing to redo.
        let result = drive(&work, &deps, StopWhen::Finished).await;
        assert!(result.is_finished(), "{fault}: {result:?}");
        let posts = wait_shares(&local, 1).await;
        assert_eq!(posts.len(), 1, "{fault}: {posts:?}");
        assert_eq!(posts[0].text, "Recovered and shared once.");
        assert!(wait_shared(&work).await, "{fault}");
        settled(&llm, usize::from(!shared_before)).await;
        assert_eq!(llm.count(), usize::from(!shared_before), "{fault}");
    }
}
