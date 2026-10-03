//! L1 / L2: file primitives, locks, session directory, inputs, registry,
//! activity view, runtime binding.

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::*;
use libopendan::channel::{DirMsgQueue, InputSource};
use libopendan::lock::{Acquire, FileLock, Lease};
use libopendan::protocol::*;
use libopendan::runner::history::{build_history, read_window, Summarizer};
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::{fsutil, SessionDir};
use serde_json::json;

fn holder(tag: &str) -> HolderInfo {
    HolderInfo {
        runner_id: format!("rn-{tag}"),
        principal: APP.into(),
        host: None,
        pid: std::process::id(),
        runtime_id: None,
    }
}

#[cfg(unix)]
fn inode(p: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).unwrap().ino()
}

#[test]
fn lease_epoch_monotonic_and_lock_file_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("lease.json");
    let mut epochs = Vec::new();
    let mut ino = None;
    for i in 0..3 {
        let Acquire::Acquired(l) = Lease::acquire("session:x", &p, holder(&i.to_string())).unwrap()
        else {
            panic!()
        };
        // Second acquisition while held (another descriptor) is busy.
        match Lease::acquire("session:x", &p, holder("other")).unwrap() {
            Acquire::Busy(Some(info)) => assert_eq!(info.epoch, l.epoch()),
            _ => panic!("expected busy with holder info"),
        }
        epochs.push(l.epoch());
        let now = inode(&p);
        assert_eq!(*ino.get_or_insert(now), now, "lock file replaced");
        l.release();
        let info: LockInfo = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(info.released_at_ms.is_some());
    }
    assert_eq!(epochs, vec![1, 2, 3]);
}

#[test]
fn lock_child_process_does_not_inherit_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("lease.json");
    let Acquire::Acquired(l) = Lease::acquire("session:x", &p, holder("a")).unwrap() else {
        panic!()
    };
    // A long-lived child started while the lock is held (like shell).
    let mut child = Command::new("sleep").arg("5").spawn().unwrap();
    drop(l);
    // The lock is free although the child still runs (CLOEXEC).
    let again = FileLock::try_acquire(&p).unwrap();
    assert!(again.is_some(), "child inherited the flock");
    let _ = child.kill();
    let _ = child.wait();
}

/// Child entry: hold a lease until killed.
#[test]
fn lock_holder_child() {
    let Ok(p) = std::env::var("LIBOPENDAN_TEST_LOCK") else {
        return;
    };
    let Acquire::Acquired(_l) =
        Lease::acquire("session:x", Path::new(&p), holder("child")).unwrap()
    else {
        panic!("child could not lock")
    };
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn kill_9_of_holder_releases_the_lease_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("lease.json");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "lock_holder_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("LIBOPENDAN_TEST_LOCK", &p)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Some(info) = libopendan::lock::read_holder_info(&p) {
            if info.holder.runner_id == "rn-child" {
                break;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "child never locked"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(matches!(
        Lease::acquire("session:x", &p, holder("p")).unwrap(),
        Acquire::Busy(_)
    ));
    unsafe {
        libc::kill(child.id() as i32, libc::SIGKILL);
    }
    let _ = child.wait();
    let Acquire::Acquired(l) = Lease::acquire("session:x", &p, holder("p")).unwrap() else {
        panic!("lock not released by kill -9")
    };
    assert_eq!(l.epoch(), 2);
}

#[tokio::test]
async fn worklog_reverse_read_is_bounded_by_the_start_point() {
    let env = Env::new();
    let sd = env.create_work(work_spec("big history")).await;
    let lease = match sd.acquire(holder("w")).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    let mut s = sd.load(&lease).unwrap();
    let body = "x".repeat(900);
    let mut offsets = Vec::new();
    for i in 0..4000u64 {
        offsets.push(s.worklog_end());
        s.append_worklog(
            &lease,
            vec![WorklogBody::UserMessage {
                run_id: "r".into(),
                turn: i,
                content: format!("{i}:{body}"),
            }],
        )
        .unwrap();
    }
    s.commit_state(&lease).unwrap();
    let size = fsutil::file_len(sd.worklog().path()).unwrap();
    assert!(size > 3_500_000);
    // Summary start near the end: only the tail is read.
    let start_idx = 3990usize;
    let mut sm = s.summary().unwrap();
    sm.history_summary = "earlier work".into();
    sm.start_offset = offsets[start_idx];
    sm.start_seq = start_idx as u64 + 2;
    s.write_summary(&lease, &sm).unwrap();
    let w = read_window(&s, &sm, 1_000_000).unwrap();
    assert!(w.reached_start);
    assert_eq!(w.lines.len(), 4000 - start_idx);
    assert!(w.bytes_read <= 2 * 64 * 1024, "read {} bytes", w.bytes_read);
    // Uncommitted tail is truncated on the next load.
    s.append_worklog(
        &lease,
        vec![WorklogBody::Compaction {
            summary_start_seq: 1,
            made_by: "x".into(),
        }],
    )
    .unwrap();
    drop(s);
    let mut s2 = sd.load(&lease).unwrap();
    assert!(s2.truncate_uncommitted(&lease).unwrap());
    assert_eq!(fsutil::file_len(sd.worklog().path()).unwrap(), size);
}

struct FakeSummarizer;

#[async_trait]
impl Summarizer for FakeSummarizer {
    async fn summarize(&self, previous: &str, segment: &str) -> libopendan::Result<String> {
        Ok(format!(
            "{previous}|{} lines",
            segment.lines().filter(|l| !l.is_empty()).count()
        ))
    }
}

#[tokio::test]
async fn budget_exhaustion_compacts_without_gap() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let lease = match sd.acquire(holder("w")).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    let mut s = sd.load(&lease).unwrap();
    for i in 0..200u64 {
        s.append_worklog(
            &lease,
            vec![WorklogBody::UserMessage {
                run_id: "r".into(),
                turn: i,
                content: format!("message {i} {}", "y".repeat(200)),
            }],
        )
        .unwrap();
    }
    s.commit_state(&lease).unwrap();
    let (msg, w) = build_history(&mut s, &lease, Some(&FakeSummarizer), 2_000)
        .await
        .unwrap();
    assert!(
        w.reached_start,
        "after compaction the window reaches the start"
    );
    let sm = sd.summary_opt().unwrap().expect("summary.json written");
    assert!(sm.history_summary.contains("lines"));
    // No gap: the oldest rendered entry is exactly the new start.
    assert_eq!(w.oldest_offset, Some(sm.start_offset));
    let text = msg.unwrap().text_content();
    assert!(text.contains("message 199"));
    assert!(!text.contains("message 0 "));
    // Deterministic rendering for the same inputs.
    let (msg2, _) = build_history(&mut s, &lease, Some(&FakeSummarizer), 2_000)
        .await
        .unwrap();
    assert_eq!(text, msg2.unwrap().text_content());
    assert_eq!(read_worklog(&sd).last().unwrap().body.kind(), "compaction");
}

async fn kmsg_rules(client: Arc<buckyos_api::msg_queue::MsgQueueClient>) {
    use libopendan::channel::kmsg::*;
    // create / subscribe twice: "already exists" is success.
    let q1 = ensure_queue(&client, "opendan.session.s1", "app2", "alice")
        .await
        .unwrap();
    let q2 = ensure_queue(&client, "opendan.session.s1", "app2", "alice")
        .await
        .unwrap();
    assert_eq!(q1, q2);
    for _ in 0..2 {
        ensure_subscription(
            &client,
            &q1,
            "opendan.jarvis.s1",
            "alice",
            "app2",
            buckyos_api::msg_queue::SubPosition::Earliest,
        )
        .await
        .unwrap();
    }
    for i in 1..=5 {
        post_to_queue(
            &client,
            &q1,
            &Input::msg(format!("k{i}"), format!("m{i}")),
            APP,
        )
        .await
        .unwrap();
    }
    let input = KmsgInput::new("q", &q1, "opendan.jarvis.s1", APP, client.clone());
    let mut p = SourceProgress::default();
    let got = input.fetch(&p, 100).await.unwrap();
    assert_eq!(
        got.iter().map(|m| m.index).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    // Selective consumption: 1,2,4 consumed → ack stays at 2, 3 is kept.
    p.mark(1);
    p.mark(2);
    p.mark(4);
    assert_eq!(p.acked_index, 2);
    let left = input.fetch(&p, 100).await.unwrap();
    assert_eq!(left.iter().map(|m| m.index).collect::<Vec<_>>(), vec![3, 5]);
    input.confirm(&p).await.unwrap();
    p.mark(3);
    assert_eq!(p.acked_index, 4, "contiguous prefix folds consumed_above");
    input.confirm(&p).await.unwrap();
    // The ack never moves back: marking lower indexes is a no-op.
    p.mark(1);
    assert_eq!(p.acked_index, 4);
    let first = input.first_available().await.unwrap();
    assert_eq!(first, Some(1));
}

#[tokio::test]
async fn kmsg_rules_hold_for_dir_queue() {
    let dir = tempfile::tempdir().unwrap();
    kmsg_rules(Arc::new(DirMsgQueue::client(dir.path()).unwrap())).await;
}

#[tokio::test]
async fn kmsg_rules_hold_for_real_kmsg_handler() {
    // The sled implementation of the kmsg service, in process.
    let dir = tempfile::tempdir().unwrap();
    let h = kmsg::sled_msg_queue::SledMsgQueue::new_in_dir(dir.path()).unwrap();
    let client = buckyos_api::msg_queue::MsgQueueClient::new_in_process(Box::new(h));
    kmsg_rules(Arc::new(client)).await;
}

#[tokio::test]
async fn lost_subscription_is_recreated_at_the_acked_position() {
    let dir = tempfile::tempdir().unwrap();
    let dq = DirMsgQueue::new(dir.path()).unwrap();
    let client = Arc::new(DirMsgQueue::client(dir.path()).unwrap());
    use libopendan::channel::kmsg::*;
    let q = ensure_queue(&client, "opendan.session.s", "app2", "alice")
        .await
        .unwrap();
    ensure_subscription(
        &client,
        &q,
        "sub-s",
        "alice",
        "app2",
        buckyos_api::msg_queue::SubPosition::Earliest,
    )
    .await
    .unwrap();
    for i in 1..=3 {
        post_to_queue(&client, &q, &Input::msg(format!("k{i}"), "m"), APP)
            .await
            .unwrap();
    }
    let input = KmsgInput::new("q", &q, "sub-s", APP, client.clone());
    let mut p = SourceProgress::default();
    p.mark(1);
    p.mark(2);
    dq.forget_subscriptions().unwrap(); // kmsg restarted, cursors lost (D-09)
    input.confirm(&p).await.unwrap();
    assert_eq!(dq.cursor("sub-s"), Some(3), "re-subscribed At(acked + 1)");
}

#[tokio::test]
async fn session_moves_with_its_directory() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let other = env.root.join("moved");
    std::fs::create_dir_all(&other).unwrap();
    let new_path = other.join(sd.sid());
    std::fs::rename(sd.path(), &new_path).unwrap();
    let moved = SessionDir::open(&new_path).unwrap();
    // Not registered at the new place yet.
    let llm = ScriptedLlm::new(|_, _| text("done"));
    assert!(matches!(
        drive(&moved, &env.deps(llm.clone()), StopWhen::Idle).await,
        DriveResult::Unregistered
    ));
    let agent = env.agent();
    let lease = match moved.acquire(holder("m")).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    agent
        .sessions()
        .update_location(&lease, moved.sid(), &new_path.canonicalize().unwrap())
        .await
        .unwrap();
    drop(lease);
    assert!(drive(&moved, &env.deps(llm), StopWhen::Finished)
        .await
        .is_finished());
    // Nothing in the protocol files depends on the old location.
    let v = libopendan::read_session(agent.as_ref(), moved.sid(), true, true, 3)
        .await
        .unwrap();
    assert!(v.state.unwrap().is_finished());
}

#[tokio::test]
async fn registry_verify_marks_unreachable_without_deleting() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    std::fs::remove_dir_all(sd.path()).unwrap();
    let agent = env.agent();
    let marked = agent.sessions().verify().await.unwrap();
    assert_eq!(marked, vec![sd.sid().to_string()]);
    let e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert!(e.unreachable);
}

#[tokio::test]
async fn create_session_is_idempotent_per_creator_and_key() {
    let env = Env::new();
    let mut spec = work_spec("idem");
    spec.idempotency_key = Some("req-1".into());
    let a = env.create_work(spec.clone()).await;
    let b = env.create_work(spec.clone()).await;
    assert_eq!(a.sid(), b.sid());
    assert_eq!(std::fs::read_dir(&env.app_dir).unwrap().count(), 1);
    let agent = env.agent();
    let ch = env.channels();
    let other = libopendan::create_session(
        &env.app_dir,
        spec,
        agent.as_ref(),
        "app:app3@alice",
        ch.as_ref(),
    )
    .await
    .unwrap();
    assert_ne!(other.sid(), a.sid());
    // An explicit sid that exists with another identity conflicts.
    let mut clash = work_spec("clash");
    clash.session_id = Some(a.sid().to_string());
    let err =
        libopendan::create_session(&env.app_dir, clash, agent.as_ref(), APP, ch.as_ref()).await;
    assert!(matches!(
        err,
        Err(libopendan::OpenDanError::SessionIdConflict(_))
    ));
}

#[tokio::test]
async fn missing_tool_or_runtime_mismatch_fails_before_inference() {
    let env = Env::new();
    let mut spec = work_spec("x");
    spec.runtime.requirement.tools = vec!["definitely-not-a-tool-xyz".into()];
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|_, _| text("never"));
    match drive(&sd, &env.deps(llm.clone()), StopWhen::Finished).await {
        DriveResult::BindFailed { error } => assert_eq!(error["kind"], json!("bind_failed")),
        r => panic!("{r:?}"),
    }
    assert_eq!(llm.count(), 0);
    assert!(sd.state().unwrap().last_error.is_some());
    // A bound session refuses another runtime id.
    let sd2 = env.create_work(work_spec("y")).await;
    assert!(drive(
        &sd2,
        &env.deps(ScriptedLlm::new(|_, _| text("ok"))),
        StopWhen::Finished
    )
    .await
    .is_finished());
    let sd3 = env.create_work(work_spec("z")).await;
    let lease = match sd3.acquire(holder("b")).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    fsutil::publish_noreplace_json(
        &sd3.file(BINDING_FILE),
        &Binding {
            schema: "opendan.binding/3".into(),
            target: serde_json::Value::Null,
            runtime_id: "rt-other-host".into(),
            kind: "native".into(),
            workdir: sd3.path().display().to_string(),
            bound_at_ms: 0,
            bound_by: "x".into(),
        },
    )
    .unwrap();
    drop(lease);
    let llm = ScriptedLlm::new(|_, _| text("never"));
    match drive(&sd3, &env.deps(llm.clone()), StopWhen::Finished).await {
        DriveResult::BindFailed { error } => assert_eq!(error["kind"], json!("runtime_mismatch")),
        r => panic!("{r:?}"),
    }
    assert_eq!(llm.count(), 0);
}

#[tokio::test]
async fn tool_plan_tombstones_are_repaired_before_running() {
    let env = Env::new();
    // Agent tool + deny plan.
    let tools = env.agent_root.join("tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("rm-all"), "#!/bin/sh\necho boom\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tools.join("rm-all"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    std::fs::create_dir_all(env.agent_root.join("tool_plans")).unwrap();
    std::fs::write(
        env.agent_root.join("tool_plans").join("safe.toml"),
        "mode = \"deny\"\n[[deny]]\nname = \"rm-all\"\nreason = \"too dangerous\"\n",
    )
    .unwrap();
    let mut spec = work_spec("try the tool");
    spec.runtime.tool_plan = Some("safe".into());
    let sd = env.create_work(spec).await;
    let llm = ScriptedLlm::new(|req, _| match has_tool_result(req, "c1") {
        Some(r) => {
            assert!(r.contains("blocked by tool plan"), "{r}");
            text("blocked as expected")
        }
        None => tool_call("c1", "shell", json!({ "command": "rm-all" })),
    });
    // First drive prepares; then simulate a crash half-way through a later
    // preparation (tombstone gone, manifest stale).
    assert!(drive(&sd, &env.deps(llm.clone()), StopWhen::Finished)
        .await
        .is_finished());
    let bin = sd.runtime_bin_dir();
    assert!(bin.join("rm-all").exists());
    std::fs::remove_file(bin.join("rm-all")).unwrap();
    let plan =
        libopendan::runtime::bin_plan_for(&sd.config().unwrap(), Some(&env.agent_root), None, &[])
            .unwrap();
    assert!(libopendan::runtime::bin_overlay::verify(&bin, &plan).is_err());
    libopendan::runtime::bin_overlay::prepare(&bin, &plan).unwrap();
    libopendan::runtime::bin_overlay::verify(&bin, &plan).unwrap();
}

#[tokio::test]
async fn stale_running_session_is_flagged_in_the_activity_view() {
    let env = Env::new();
    let sd = env.create_work(work_spec("x")).await;
    let agent = env.agent();
    let lease = match sd.acquire(holder("s")).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    let mut e = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    e.status.rev = 5;
    e.status.run_state = RunState::Running;
    e.status.updated_at_ms = 1;
    e.status.activity.heartbeat_ms = 1;
    agent
        .sessions()
        .report_state(&lease, sd.sid(), e.status.clone())
        .await
        .unwrap();
    let list = agent.activity().active(None, 10).await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].possibly_interrupted);
    // Waiting sessions need no heartbeat.
    e.status.rev = 6;
    e.status.run_state = RunState::Waiting;
    agent
        .sessions()
        .report_state(&lease, sd.sid(), e.status.clone())
        .await
        .unwrap();
    assert!(!agent.activity().active(None, 10).await.unwrap()[0].possibly_interrupted);
    // Older revs are ignored.
    e.status.rev = 4;
    assert!(!agent
        .sessions()
        .report_state(&lease, sd.sid(), e.status)
        .await
        .unwrap());
}

#[test]
fn json_schemas_export() {
    let schemas = libopendan::protocol::json_schemas();
    assert!(schemas.len() >= 15);
    for (name, s) in schemas {
        assert!(s.is_object(), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kevent_waker_wakes_before_the_poll_timeout() {
    use libopendan::channel::{KEventWaker, Waker};
    let client = Arc::new(buckyos_api::KEventClient::new_local("test-node"));
    let waker = Arc::new(KEventWaker::new(client.clone()));
    let ev = libopendan::ids::wake_event("jarvis.alice", "work-x");
    let w = waker.clone();
    let ev2 = ev.clone();
    let started = Instant::now();
    let h = tokio::spawn(async move { w.wait(Some(&ev2), Duration::from_secs(10)).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    waker.notify(&ev, json!({ "sid": "work-x" })).await;
    h.await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "woken by kevent, not by timeout"
    );
}
