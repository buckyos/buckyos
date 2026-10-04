//! The reply path: a Turn's reply is committed with the Turn, handed to the
//! outbound sink afterwards and re-sent unchanged until the sink answers.

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, OutboundSink, SendResult, StopWhen};
use libopendan::{AgentStateClient, SessionTemplate};
use serde_json::json;

const BOB: &str = "did:bns:bob";

#[derive(Default)]
struct Sink {
    /// Sends that fail before the sink starts answering.
    fail: Mutex<u32>,
    reject: bool,
    sent: Mutex<Vec<OutboundRecord>>,
}

#[async_trait]
impl OutboundSink for Sink {
    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        self.sent.lock().unwrap().push(record.clone());
        let mut fail = self.fail.lock().unwrap();
        if *fail > 0 {
            *fail -= 1;
            return SendResult::Retry {
                error: "msg-center unreachable".into(),
            };
        }
        if self.reject {
            return SendResult::Rejected {
                reason: "blocked_author".into(),
            };
        }
        SendResult::Sent {
            msg_id: Some(msg_key(&record.msg)),
            deliveries: vec!["d-1".into()],
        }
    }
}

fn ui_spec(binding: Option<OutboundBinding>) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some(format!("{AGENT}/dm%3A{BOB}"));
    spec.outbound = binding;
    spec
}

fn from_bob(t: &str) -> PostedInput {
    let m = text_msg(&parse_did(BOB).unwrap(), &parse_did(AGENT).unwrap(), t);
    PostedInput::msg(APP, m, MsgDelivery::default()).unwrap()
}

fn bob_binding() -> OutboundBinding {
    OutboundBinding {
        to: BOB.into(),
        to_session: None,
        kind: "chat".into(),
    }
}

#[tokio::test]
async fn a_reply_is_committed_then_sent_along_the_way_the_message_came() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(Some(bob_binding()))).await;
    assert_eq!(sd.config().unwrap().session.kind, SessionKind::Ui);
    let agent = env.agent();
    let entry = agent.sessions().lookup(sd.sid()).await.unwrap().unwrap();
    assert_eq!(entry.route_key, sd.config().unwrap().session.route_key);
    let input = from_bob("hello");
    libopendan::post_input(agent.as_ref(), sd.sid(), &input).await.unwrap();
    let sink = Arc::new(Sink::default());
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi bob")));
    deps.outbound = Some(sink.clone());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(r, DriveResult::TurnClosed { turn: 1, .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outbox.len(), 1);
    let e = &st.outbox[0];
    assert_eq!(e.status, OutboxStatus::Sent);
    assert_eq!(e.deliveries, vec!["d-1".to_string()]);
    assert_eq!(e.msg.content.content, "hi bob");
    assert_eq!(e.msg.to, vec![parse_did(BOB).unwrap()]);
    assert_eq!(e.msg.from, parse_did(AGENT).unwrap());
    assert_eq!(e.msg.thread.reply_to.as_ref().unwrap().to_string(), input.key);
    assert!(e.key.starts_with(&format!("{}:1:", sd.sid())), "{}", e.key);
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
    // Nothing is sent twice.
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unreachable_sink_gets_the_same_message_again() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(Some(bob_binding()))).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let sink = Arc::new(Sink {
        fail: Mutex::new(1),
        ..Default::default()
    });
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi bob")));
    deps.outbound = Some(sink.clone());
    let r = drive(&sd, &deps, StopWhen::TurnClosed).await;
    // A failed send does not fail the Turn.
    assert!(
        matches!(r, DriveResult::TurnClosed { status: TurnStatus::Completed, .. }),
        "{r:?}"
    );
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Pending);
    assert_eq!(st.outbox[0].attempts, 1);
    assert!(libopendan::runner::has_pending_outbound(&st));
    // A later drive (another process after a restart, as far as the session
    // can tell) hands over the stored message, not a new one.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let mut deps = env.deps(ScriptedLlm::new(|_, _| panic!("no inference")));
    deps.outbound = Some(sink.clone());
    let r = drive(&sd, &deps, StopWhen::Idle).await;
    assert!(matches!(r, DriveResult::Idle { .. }), "{r:?}");
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Sent);
    let sent = sink.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], sent[1], "same key, same MsgObject (same ObjId)");
}

#[tokio::test]
async fn rejected_and_misrouted_replies_are_recorded_not_retried() {
    let env = Env::new();
    // Bound to another peer than the one the message came from.
    let mut binding = bob_binding();
    binding.to = "did:bns:carol".into();
    let sd = env.create_work(ui_spec(Some(binding))).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let sink = Arc::new(Sink::default());
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi")));
    deps.outbound = Some(sink.clone());
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Failed);
    assert!(st.outbox[0].error.as_deref().unwrap().starts_with("route_mismatch"));
    assert!(sink.sent.lock().unwrap().is_empty());

    let env = Env::new();
    let sd = env.create_work(ui_spec(None)).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let sink = Arc::new(Sink {
        reject: true,
        ..Default::default()
    });
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("hi")));
    deps.outbound = Some(sink.clone());
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    drive(&sd, &deps, StopWhen::Idle).await;
    let st = sd.state().unwrap();
    assert_eq!(st.outbox[0].status, OutboxStatus::Failed);
    assert_eq!(st.outbox[0].error.as_deref(), Some("blocked_author"));
    assert_eq!(sink.sent.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn without_a_sink_no_outbox_is_kept() {
    let env = Env::new();
    let sd = env.create_work(ui_spec(None)).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &from_bob("hello")).await.unwrap();
    let deps = env.deps(ScriptedLlm::new(|_, _| text("hi")));
    drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(sd.state().unwrap().outbox.is_empty());
}
