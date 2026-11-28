//! The AICC routing group of a session (AICC user-centric routing TODO
//! §3.5, §4.6) and the fallbacks its answers report (§3.6): the group is
//! looked up along `origin.parent_session` at every call, the nearest one
//! wins; a fallback is noted once per usecase and model.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::AiResponse;
use common::*;
use libopendan::api::{create_sub_session, route_group_of, RouteGroupRef, SubSessionSpec};
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use libopendan::state::AgentStateClient;
use libopendan::{SessionDir, SessionTemplate};
use llm_context::deps::{LlmClient, LlmInference, LlmInferenceRequest};
use llm_context::error::LLMComputeError;
use serde_json::json;

fn work(objective: &str) -> libopendan::api::SessionSpec {
    let mut spec = SessionTemplate::load("work", None).unwrap().spec(objective);
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.prompt.initial_inputs = vec![PostedInput::text(APP, AGENT, "go").unwrap()];
    spec
}

fn sub(title: &str) -> SubSessionSpec {
    SubSessionSpec {
        objective: format!("do {title}"),
        title: Some(title.to_string()),
        msgs: vec!["go".into()],
        key: Some(title.to_string()),
        ..Default::default()
    }
}

fn set_group(sd: &SessionDir, group: Option<&str>) {
    let mut cfg = sd.config().unwrap();
    cfg.route_group = group.map(str::to_string);
    libopendan::fsutil::atomic_replace_json(&sd.file(SESSION_CONFIG_FILE), &cfg).unwrap();
}

fn group(group_id: &str, sd: &SessionDir) -> Option<RouteGroupRef> {
    Some(RouteGroupRef {
        group_id: group_id.into(),
        session_id: sd.sid().into(),
    })
}

#[tokio::test]
async fn the_nearest_group_along_the_parent_chain_wins() {
    let env = Env::new();
    let agent = env.agent();
    let ch = env.channels();
    let root = env.create_work(work("the plan")).await;
    let child = create_sub_session(agent.as_ref(), APP, root.sid(), sub("a"), ch.as_ref()).await.unwrap();
    let grandchild = create_sub_session(agent.as_ref(), APP, child.sid(), sub("b"), ch.as_ref()).await.unwrap();

    for sd in [&root, &child, &grandchild] {
        assert_eq!(route_group_of(agent.as_ref(), sd.sid()).await.unwrap(), None, "no group anywhere");
    }

    // Set on the root after the sub sessions exist: they follow at once.
    set_group(&root, Some("g-root"));
    for sd in [&root, &child, &grandchild] {
        assert_eq!(route_group_of(agent.as_ref(), sd.sid()).await.unwrap(), group("g-root", &root));
    }

    // A session's own group wins over its parent's, for it and below it.
    set_group(&child, Some("g-child"));
    assert_eq!(route_group_of(agent.as_ref(), root.sid()).await.unwrap(), group("g-root", &root));
    assert_eq!(route_group_of(agent.as_ref(), child.sid()).await.unwrap(), group("g-child", &child));
    assert_eq!(route_group_of(agent.as_ref(), grandchild.sid()).await.unwrap(), group("g-child", &child));

    set_group(&grandchild, Some("g-own"));
    assert_eq!(route_group_of(agent.as_ref(), grandchild.sid()).await.unwrap(), group("g-own", &grandchild));

    // Cleared again: the next one up.
    set_group(&child, None);
    set_group(&grandchild, None);
    assert_eq!(route_group_of(agent.as_ref(), grandchild.sid()).await.unwrap(), group("g-root", &root));
}

/// Answers as AICC does when the fallback LLM serves `llm.plan`, recording
/// the routing group of every request.
struct FallbackLlm {
    groups: Mutex<Vec<Option<String>>>,
    calls: AtomicUsize,
}

#[async_trait]
impl LlmClient for FallbackLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.infer_routed(req).await.map(|r| r.response)
    }

    async fn infer_routed(&self, req: LlmInferenceRequest) -> Result<LlmInference, LLMComputeError> {
        self.groups.lock().unwrap().push(req.route_group.clone());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmInference {
            response: text("hi"),
            route_trace: Some(
                serde_json::from_value(json!({
                    "attempts": [],
                    "final_model": "small-1@local",
                    "fallback": { "kind": "coverage", "from": "llm.plan", "to": "llm.fallback" }
                }))
                .unwrap(),
            ),
        })
    }
}

fn ui() -> libopendan::api::SessionSpec {
    let template = SessionTemplate::load("ui", None).unwrap();
    let mut spec = template.spec(template.objective_or_default(false));
    spec.route_key = Some(format!("{AGENT}/dm%3Adid%3Abns%3Aalice"));
    spec.prompt.llm_context = json!({ "tools": { "enabled": true, "tools": [{ "groupname": "bash_core" }] } });
    spec
}

#[tokio::test]
async fn a_conversation_routes_by_its_group_and_notes_a_fallback_once() {
    let env = Env::new();
    let agent = env.agent();
    let sd = env.create_work(ui()).await;
    let llm = Arc::new(FallbackLlm {
        groups: Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });

    agent.sessions().post_input(sd.sid(), &msg("hello")).await.unwrap();
    drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;

    // Set by the owner between two messages; the next inference has it.
    agent
        .sessions()
        .post_input(
            sd.sid(),
            &PostedInput::control(APP, "rg-1", ControlCommand::RouteGroup { route_group: Some("g-chat".into()) }),
        )
        .await
        .unwrap();
    agent.sessions().post_input(sd.sid(), &msg("again")).await.unwrap();
    drive(&sd, &env.deps(llm.clone()), StopWhen::Idle).await;
    assert_eq!(sd.config().unwrap().route_group.as_deref(), Some("g-chat"));

    let groups = llm.groups.lock().unwrap().clone();
    assert!(groups.len() >= 2, "{groups:?}");
    assert_eq!(groups[0], None);
    assert_eq!(groups.last().unwrap().as_deref(), Some("g-chat"));

    // Every answer came from the fallback LLM: one note for the session.
    let notes = sd.state().unwrap().route_fallbacks;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].kind, RouteFallbackKind::Coverage);
    assert_eq!(notes[0].usecase, "llm.plan");
    assert_eq!(notes[0].model, "small-1@local");
    assert_eq!(notes[0].turn, Some(1));
}

const BOB: &str = "did:bns:bob";

/// Takes every message; tells about a fallback in one line.
#[derive(Default)]
struct NoticeSink {
    sent: Mutex<Vec<libopendan::bridge::OutboundRecord>>,
}

#[async_trait]
impl libopendan::runner::OutboundSink for NoticeSink {
    async fn notice(
        &self,
        _cfg: &SessionConfig,
        mut base: ndn_lib::MsgObject,
        note: &RouteFallbackNote,
    ) -> Option<ndn_lib::MsgObject> {
        base.content.content = format!("fallback {} {}", note.usecase, note.model);
        Some(base)
    }

    async fn send(&self, _sid: &str, record: &libopendan::bridge::OutboundRecord) -> libopendan::runner::SendResult {
        self.sent.lock().unwrap().push(record.clone());
        libopendan::runner::SendResult::Sent {
            msg_id: Some(msg_key(&record.msg)),
            deliveries: Vec::new(),
        }
    }
}

#[tokio::test]
async fn the_other_side_is_told_once_before_the_reply() {
    let env = Env::new();
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("chat");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.route_key = Some(format!("{AGENT}/dm%3A{BOB}"));
    spec.outbound = Some(OutboundBinding {
        to: BOB.into(),
        to_session: None,
        kind: "chat".into(),
    });
    let sd = env.create_work(spec).await;
    let agent = env.agent();
    let sink = Arc::new(NoticeSink::default());
    let llm = Arc::new(FallbackLlm {
        groups: Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let mut deps = env.deps(llm);
    deps.outbound = Some(sink.clone());
    for t in ["hello", "again"] {
        let m = text_msg(&parse_did(BOB).unwrap(), &parse_did(AGENT).unwrap(), t);
        let input = PostedInput::msg(APP, m, MsgDelivery::default()).unwrap();
        libopendan::post_input(agent.as_ref(), sd.sid(), &input).await.unwrap();
        drive(&sd, &deps, StopWhen::Idle).await;
    }
    let sent: Vec<String> = sink.sent.lock().unwrap().iter().map(|r| r.msg.content.content.clone()).collect();
    assert_eq!(sent, vec!["fallback llm.plan small-1@local", "hi", "hi"]);
    let st = sd.state().unwrap();
    assert_eq!(st.outbox.iter().filter(|e| e.purpose == OutboxPurpose::Notice).count(), 1);
}
