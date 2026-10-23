mod common;

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{BuckyOSRuntime, BuckyOSRuntimeType, RuntimeAuthState};
use common::*;
use libopendan::bridge::OutboundRecord;
use libopendan::protocol::*;
use libopendan::runner::{drive, DriveResult, OutboundSink, SendResult, StopWhen};
use libopendan::{SessionDir, SessionTemplate};
use serde_json::json;

const CHILD_ROOT: &str = "LIBOPENDAN_AUTH_TEST_ROOT";
const CHILD_SESSION: &str = "LIBOPENDAN_AUTH_TEST_SESSION";

#[derive(Default)]
struct Sink {
    unavailable: bool,
    sent: Mutex<Vec<OutboundRecord>>,
}

#[async_trait]
impl OutboundSink for Sink {
    async fn send(&self, _sid: &str, record: &OutboundRecord) -> SendResult {
        self.sent.lock().unwrap().push(record.clone());
        if self.unavailable {
            SendResult::Retry { error: "authentication unavailable".into() }
        } else {
            SendResult::Sent { msg_id: Some(msg_key(&record.msg)), deliveries: vec![] }
        }
    }
}

async fn register_runtime(exp: u64) -> String {
    let runtime = BuckyOSRuntime::new("app2", Some("alice".into()), BuckyOSRuntimeType::AppService);
    runtime.set_all_background_tasks_enabled(false).await;
    let token = json!({ "iss": "verify-hub", "sub": "alice", "appid": "app2", "exp": exp }).to_string();
    *runtime.session_token.write().await = token.clone();
    buckyos_api::set_buckyos_api_runtime(runtime).unwrap();
    token
}

fn input(text: &str) -> PostedInput {
    let msg = text_msg(&parse_did("did:bns:alice").unwrap(), &parse_did(AGENT).unwrap(), text);
    PostedInput::msg(APP, msg, MsgDelivery::default()).unwrap()
}

#[tokio::test]
async fn authentication_failure_keeps_inputs_and_outbox_until_new_runtime_recovers() {
    if let Ok(root) = std::env::var(CHILD_ROOT) {
        let env = Env::at(Path::new(&root));
        let sid = std::env::var(CHILD_SESSION).unwrap();
        let sd = SessionDir::open(env.app_dir.join(sid)).unwrap();
        let before = sd.state().unwrap();
        register_runtime(1).await;
        let sink = Arc::new(Sink::default());
        let llm = ScriptedLlm::new(|_, _| panic!("inference while authentication is unavailable"));
        let mut deps = env.deps(llm.clone());
        deps.outbound = Some(sink.clone());
        for _ in 0..2 {
            let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
            assert!(matches!(result, DriveResult::BindFailed { .. }), "{result:?}");
            let state = sd.state().unwrap();
            assert_eq!(state.inputs, before.inputs);
            assert_eq!(state.outbox, before.outbox);
            assert_eq!(state.live_run, before.live_run);
            assert!(!state.is_finished());
            assert_eq!(state.last_error.as_ref().unwrap()["kind"], "bind_failed");
        }
        let runtime = buckyos_api::get_buckyos_api_runtime().unwrap();
        assert_eq!(runtime.get_authentication_status().await.state, RuntimeAuthState::Unavailable);
        let mut tool_env = vec![("BUCKYOS_APPCLIENT_SESSION_TOKEN".into(), "stale-token".into())];
        assert!(agent_tool::runtime_context::refresh_appclient_session_env(&mut tool_env).await.is_err());
        assert_eq!(llm.count(), 0);
        assert!(sink.sent.lock().unwrap().is_empty());
        return;
    }

    let env = Env::new();
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut spec = SessionTemplate::load("ui", None).unwrap().spec("auth-recovery");
    spec.prompt.llm_context = json!({ "tools": { "enabled": true } });
    spec.workspace = Some(env.workspace(&workspace).await);
    spec.runtime.requirement.tools = vec!["sh".into()];
    let sd = env.create_work(spec).await;
    let agent = env.agent();
    libopendan::post_input(agent.as_ref(), sd.sid(), &input("first message")).await.unwrap();
    let failed_sink = Arc::new(Sink { unavailable: true, ..Default::default() });
    let mut deps = env.deps(ScriptedLlm::new(|_, _| text("first reply")));
    deps.outbound = Some(failed_sink);
    let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { .. }), "{result:?}");
    let before = sd.state().unwrap();
    assert_eq!(before.outbox[0].status, OutboxStatus::Pending);
    let pending = before.outbox[0].clone();
    let backlog = input("message during authentication failure");
    libopendan::post_input(agent.as_ref(), sd.sid(), &backlog).await.unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("authentication_failure_keeps_inputs_and_outbox_until_new_runtime_recovers")
        .arg("--nocapture")
        .env(CHILD_ROOT, &env.root)
        .env(CHILD_SESSION, sd.sid())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(sd.state().unwrap().inputs, before.inputs);
    assert!(sd.state().unwrap().last_error.is_some());

    let token = register_runtime(u64::MAX).await;
    let mut tool_env = vec![
        ("BUCKYOS_APPCLIENT_SESSION_TOKEN".into(), "stale-token".into()),
        ("BUCKYOS_APPCLIENT_SESSION_TOKEN".into(), "another-stale-token".into()),
    ];
    agent_tool::runtime_context::refresh_appclient_session_env(&mut tool_env).await.unwrap();
    assert_eq!(tool_env, vec![("BUCKYOS_APPCLIENT_SESSION_TOKEN".into(), token)]);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    let sink = Arc::new(Sink::default());
    let llm = ScriptedLlm::new(|_, _| text("recovered reply"));
    let mut deps = env.deps(llm.clone());
    deps.outbound = Some(sink.clone());
    let result = drive(&sd, &deps, StopWhen::TurnClosed).await;
    assert!(matches!(result, DriveResult::TurnClosed { turn: 2, .. }), "{result:?}");
    let state = sd.state().unwrap();
    assert_eq!(state.source("q").acked_index, 2);
    assert!(state.last_error.is_none());
    assert!(state.outbox.iter().all(|entry| entry.status == OutboxStatus::Sent));
    let sent = sink.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].key, pending.key);
    assert_eq!(sent[0].msg, pending.msg);
    assert_eq!(sent[1].msg.content.content, "recovered reply");
    assert_eq!(llm.count(), 1);
    drive(&sd, &deps, StopWhen::Idle).await;
    assert_eq!(sink.sent.lock().unwrap().len(), 2);
    assert_eq!(llm.count(), 1);
}
