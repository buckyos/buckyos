//! L4: perception + self-improve consolidation.

mod common;

use common::*;
use libopendan::lock::Acquire;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::state::AgentStateClient;

#[tokio::test]
async fn self_improve_consolidates_the_backlog_once() {
    let env = Env::new();
    let agent = env.agent();
    // Two finished work sessions leave run digests + outcomes.
    for obj in ["A", "B"] {
        let sd = env.create_work(work_spec(obj)).await;
        assert!(drive(&sd, &env.deps(ScriptedLlm::new(|_, _| text("ok"))), StopWhen::Finished)
            .await
            .is_finished());
    }
    let cur0 = agent.perception().cursor().await.unwrap();
    let backlog = agent.perception().backlog(&cur0).await.unwrap();
    assert_eq!(backlog.items.len(), 2);
    let ch = env.channels();
    let si = libopendan::create_self_improve_session(
        &env.app_dir,
        agent.as_ref(),
        APP,
        ch.as_ref(),
        PromptSection {
            llm_context: serde_json::json!({ "tools": { "enabled": true } }),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .expect("backlog not empty");
    // Same window → same session (idempotent).
    let again = libopendan::create_self_improve_session(
        &env.app_dir,
        agent.as_ref(),
        APP,
        ch.as_ref(),
        PromptSection::default(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(again.sid(), si.sid());
    // While another holder runs self-improve, this one is busy.
    let other = match agent.locks().acquire("self_improve", env.deps(ScriptedLlm::new(|_, _| text("x"))).holder()).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    let llm = ScriptedLlm::new(|req, _| {
        let u = last_user_text(req);
        assert!(u.contains("<perceptions>") && u.contains("run_digest"), "{u}");
        text("kept 2 facts")
    });
    assert!(matches!(
        drive(&si, &env.deps(llm.clone()), StopWhen::Finished).await,
        DriveResult::Busy { .. }
    ));
    assert_eq!(llm.count(), 0);
    drop(other);
    let r = drive(&si, &env.deps(llm.clone()), StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    let cur = agent.perception().cursor().await.unwrap();
    for item in &backlog.items {
        assert_eq!(cur.offsets[&item.session_id], item.to_offset);
    }
    // Its own perceptions do not feed the watermark (no self echo).
    let b2 = agent.perception().backlog(&cur).await.unwrap();
    assert!(b2.is_empty(), "{b2:?}");
    assert!(agent.perception().last_seq(si.sid()).await.unwrap() >= 1);
}

#[tokio::test]
async fn failed_self_improve_does_not_advance_the_cursor() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(work_spec("A")).await;
    assert!(drive(&sd, &env.deps(ScriptedLlm::new(|_, _| text("ok"))), StopWhen::Finished)
        .await
        .is_finished());
    let ch = env.channels();
    let si = libopendan::create_self_improve_session(
        &env.app_dir,
        agent.as_ref(),
        APP,
        ch.as_ref(),
        PromptSection::default(),
    )
    .await
    .unwrap()
    .unwrap();
    // Stopped instead of succeeded.
    libopendan::post_input(agent.as_ref(), si.sid(), &Input::control("s", &ControlCommand::Stop { reason: None }), APP)
        .await
        .unwrap();
    let r = drive(&si, &env.deps(ScriptedLlm::new(|_, _| text("never"))), StopWhen::Finished).await;
    assert!(r.is_finished());
    assert_eq!(si.state().unwrap().outcome, Some(Outcome::Stopped));
    assert!(agent.perception().cursor().await.unwrap().offsets.is_empty());
}

#[tokio::test]
async fn perception_append_is_idempotent_and_monotonic() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(work_spec("A")).await;
    let lease = match sd.acquire(env.deps(ScriptedLlm::new(|_, _| text("x"))).holder()).unwrap() {
        Acquire::Acquired(l) => l,
        _ => panic!(),
    };
    let rec = |seq: u64| PerceptionRecord {
        seq,
        at_ms: 1,
        session_id: sd.sid().to_string(),
        kind: "observation".into(),
        source: "session".into(),
        tags: vec![],
        objects: vec![],
        summary: format!("r{seq}"),
        payload: serde_json::Value::Null,
        refs: serde_json::Value::Null,
    };
    let p = agent.perception();
    assert_eq!(p.append(&lease, sd.sid(), vec![rec(1), rec(2)]).await.unwrap(), 2);
    // Retry of the same batch + one new record: only the new one lands.
    assert_eq!(p.append(&lease, sd.sid(), vec![rec(1), rec(2), rec(3)]).await.unwrap(), 3);
    let b = p.backlog(&PerceptionCursor::default()).await.unwrap();
    let all = p.read(&b.items[0]).await.unwrap();
    assert_eq!(all.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
    // The cursor only moves under the self_improve lease.
    let c = b.advanced(&PerceptionCursor::default());
    assert!(p.commit_cursor(&lease, &c).await.is_err());
}
