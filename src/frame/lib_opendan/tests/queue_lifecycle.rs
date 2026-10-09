//! The life of a session's input queue: a producer / consumer queue that is
//! empty while the session keeps up, given back once the session takes no
//! more input, and recreated by the driver under its fixed name when kmsg
//! lost it (the consumption progress restarts with the new numbering).

mod common;

use common::*;
use libopendan::channel::DirMsgQueue;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, StopWhen};
use libopendan::{OpenDanError, SessionTemplate};
use serde_json::json;

fn ui_spec() -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some(format!("{AGENT}/dm%3A{USER}"));
    spec
}

#[tokio::test]
async fn a_lost_queue_is_recreated_by_the_driver_and_read_from_the_start() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    let agent = env.agent();
    let llm = ScriptedLlm::new(|req, _| text(&format!("seen {}", user_texts(req).join(" | "))));
    let deps = env.deps(llm.clone());
    for t in ["one", "two"] {
        libopendan::post_input(agent.as_ref(), sd.sid(), &msg(t)).await.unwrap();
        let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
        assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    }
    assert_eq!(sd.state().unwrap().source("q").acked_index, 2);

    // kmsg loses its data; the session directory survives.
    DirMsgQueue::new(&env.queue_dir).unwrap().wipe().unwrap();
    let err = libopendan::post_input(agent.as_ref(), sd.sid(), &msg("lost"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, OpenDanError::QueueMissing { session_id } if session_id == sd.sid()),
        "{err}"
    );
    assert_eq!(err.to_json()["kind"], "queue_missing");

    // The driver resets the progress, then creates the same queue again.
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.source("q"), SourceProgress::default());
    let wl = read_worklog(&sd);
    let reset = wl
        .iter()
        .find_map(|e| match &e.body {
            WorklogBody::ControlApplied { command, detail, .. } if command == "input_source_reset" => {
                Some(detail.clone())
            }
            _ => None,
        })
        .expect("input_source_reset logged");
    assert_eq!(reset["src"], "q");
    assert_eq!(reset["lost_acked_index"], 2);
    let ch = env.channels();
    let stats = ch.client().get_queue_stats(&queue_of(&sd)).await.unwrap();
    assert_eq!(stats.last_index, 0);
    let sub = sd.config().unwrap().channels.kmsg().unwrap().2.to_string();
    assert!(DirMsgQueue::new(&env.queue_dir).unwrap().cursor(&sub).is_some(), "subscribed again");

    // New deliveries are numbered from 1 again and none is skipped, even
    // when there are more of them than the old acked position.
    for t in ["three", "four", "five"] {
        libopendan::post_input(agent.as_ref(), sd.sid(), &msg(t)).await.unwrap();
    }
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    let last = llm.transcript(llm.count() - 1);
    for t in ["three", "four", "five"] {
        assert!(last.contains(t), "{t} reached the LLM: {last}");
    }
    assert_eq!(sd.state().unwrap().source("q").acked_index, 3);
    // A healthy queue is left alone.
    let n = read_worklog(&sd).len();
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(read_worklog(&sd).len(), n);
}

async fn stats(env: &Env, sd: &libopendan::SessionDir) -> Option<(u64, u64, u64)> {
    env.channels()
        .client()
        .get_queue_stats(&queue_of(sd))
        .await
        .ok()
        .map(|s| (s.message_count, s.first_index, s.last_index))
}

#[tokio::test]
async fn consumed_records_do_not_stay_in_the_queue() {
    let env = Env::new();
    let sd = env.create_work(ui_spec()).await;
    let agent = env.agent();
    let deps = env.deps(ScriptedLlm::new(|_, _| text("ok")));
    for t in ["one", "two"] {
        libopendan::post_input(agent.as_ref(), sd.sid(), &msg(t)).await.unwrap();
    }
    assert_eq!(stats(&env, &sd).await, Some((2, 1, 2)));
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { .. }), "{r:?}");
    // Committed to state.json, acknowledged, dropped by kmsg.
    assert_eq!(sd.state().unwrap().source("q").acked_index, 2);
    assert_eq!(stats(&env, &sd).await, Some((0, 0, 2)));
    // Numbering goes on.
    let index = libopendan::post_input(agent.as_ref(), sd.sid(), &msg("three")).await.unwrap();
    assert_eq!(index, 3);
}

#[tokio::test]
async fn a_finished_session_gives_its_queue_back() {
    let env = Env::new();
    let agent = env.agent();
    let deps = env.deps(ScriptedLlm::new(|_, _| text("ok")));
    // A chat session stopped for good: nothing can reach it any more.
    let ui = env.create_work(ui_spec()).await;
    libopendan::post_input(agent.as_ref(), ui.sid(), &msg("hi")).await.unwrap();
    drive(&ui, &deps, StopWhen::TurnClosed).await;
    libopendan::post_input(
        agent.as_ref(),
        ui.sid(),
        &PostedInput::control(APP, "stop-1", ControlCommand::Stop { reason: None }),
    )
    .await
    .unwrap();
    assert!(drive(&ui, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(stats(&env, &ui).await, None, "queue released");
    let err = libopendan::post_input(agent.as_ref(), ui.sid(), &msg("late")).await.unwrap_err();
    assert!(matches!(err, OpenDanError::SessionFinished(_)), "{err}");
    // Driving it again does not touch kmsg (no error, nothing recreated).
    assert!(drive(&ui, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(stats(&env, &ui).await, None);

    // A finished work session keeps its queue while it can take a decision.
    let work = env.create_work(work_spec("x")).await;
    assert!(drive(&work, &deps, StopWhen::Finished).await.is_finished());
    assert_eq!(work.state().unwrap().acceptance, Acceptance::Pending);
    assert!(stats(&env, &work).await.is_some());
    let decide = |d: &str| {
        PostedInput::control(APP, format!("decide-{d}"), ControlCommand::Decide {
            decision: d.into(),
            by: USER.into(),
            note: None,
        })
    };
    libopendan::post_input(agent.as_ref(), work.sid(), &decide("accept")).await.unwrap();
    assert!(drive(&work, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(work.state().unwrap().acceptance, Acceptance::Accepted);
    // Accepted can still be discarded.
    assert_eq!(stats(&env, &work).await, Some((0, 0, 1)));
    libopendan::post_input(agent.as_ref(), work.sid(), &decide("discard")).await.unwrap();
    assert!(drive(&work, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(work.state().unwrap().acceptance, Acceptance::Discarded);
    assert_eq!(stats(&env, &work).await, None, "queue released");
}
