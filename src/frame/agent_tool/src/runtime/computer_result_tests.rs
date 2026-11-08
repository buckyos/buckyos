use super::*;
use base64::Engine;
use buckyos_api::*;
use llm_context::{Observation, ToolCallCtx, ToolManager};
use std::collections::HashMap;
use std::path::Path;
use tokio::io::AsyncWriteExt;

use crate::llm_bash::RunBinding;
use crate::xllm::{LoopModel, XllmToolManager};

const BINDING_ID: &str = "agent-test:display:1";
const OBSERVATION_ID: &str = "observation-native-test";
const EXPIRES_AT_MS: u64 = 4_000_000_060_000;
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAUAAAAC0AQAAAADNn8i8AAAAHklEQVR4nO3BMQEAAADCoPVPbQ0PoAAAAAAAAAB+DRzUAAFaKJ4EAAAAAElFTkSuQmCC";
const SCREENSHOT_ERROR: &str = "Screenshot content is no longer available";

struct TokensOnly;

#[async_trait]
impl NodeExecutorConnector for TokensOnly {
    async fn connect(&self, _: &str) -> std::io::Result<Box<dyn NodeIo>> {
        panic!("native computer must never use a network connector")
    }

    async fn session_token(&self) -> Result<String, String> {
        Ok("agent-token".into())
    }
}

#[derive(Clone, Copy)]
enum Reply {
    Inspect,
    Observe,
    FailedClick,
    ClickWithScreenshotFailure,
}

impl Reply {
    fn request(self) -> Value {
        match self {
            Self::Inspect => json!({"command": "inspect"}),
            Self::Observe => json!({"command": "observe", "mode": "visual"}),
            Self::FailedClick | Self::ClickWithScreenshotFailure => json!({
                "command": "click",
                "target": {
                    "kind": "point", "observation_id": OBSERVATION_ID,
                    "x": 120, "y": 40, "space": "screenshot"
                }
            }),
        }
    }

    fn status(self, request: &ExecutorComputerRequest) -> ExecutorOpStatus {
        let (state, kind, mut computer) = match self {
            Self::Inspect | Self::Observe | Self::ClickWithScreenshotFailure => (
                "succeeded",
                match self {
                    Self::Inspect => "computer_inspect",
                    Self::Observe => "computer_observe",
                    _ => "computer_act",
                },
                serde_json::to_value(ComputerResult::new(
                    &request.op_id,
                    BINDING_ID,
                    ComputerExecutionState::Completed,
                ))
                .unwrap(),
            ),
            Self::FailedClick => (
                "failed",
                "computer_act",
                serde_json::to_value(ComputerResult::failed(
                    &request.op_id,
                    BINDING_ID,
                    ComputerExecutionState::NotStarted,
                    "stale_observation",
                    "The desktop changed; observe again before clicking",
                ))
                .unwrap(),
            ),
        };
        match self {
            Self::Inspect => {
                computer["capabilities"] = json!({
                    "capture": {"supported": true, "ready": true},
                    "input": {"supported": true, "ready": true},
                    "accessibility": {"supported": false, "ready": false},
                    "observation_modes": ["visual"],
                    "commands": ["inspect", "observe", "click"], "focus_guard": "window"
                });
                computer["binding"] = json!({
                    "binding_id": BINDING_ID, "executor_id": "did:web:ood1",
                    "profile_id": "agent-test", "store_id": "ledger", "origin_generation": 1,
                    "authorized_principal": "agent", "interactive_user": "test-user",
                    "interactive_session_id": "gui-test", "desktop_epoch": "desktop-test",
                    "computer_session_id": "computer-test", "surface_id": "display:1",
                    "surface_epoch": 1, "interaction_resource_id": "desktop-test",
                    "expires_at_ms": EXPIRES_AT_MS
                });
            }
            Self::Observe | Self::ClickWithScreenshotFailure => {
                computer["feedback"]["state"] = json!("included");
                computer["observation"] = json!({
                    "observation_id": OBSERVATION_ID, "binding_id": BINDING_ID,
                    "surface_id": "display:1", "surface_epoch": 1, "interaction_revision": 7,
                    "requested_mode": "visual", "effective_mode": "visual",
                    "selection_reason": "requested visual mode",
                    "capture_started_at_ms": 4_000_000_000_000_u64,
                    "capture_completed_at_ms": 4_000_000_000_010_u64,
                    "consistency": "stable", "focus_guard": "window",
                    "expires_at_ms": EXPIRES_AT_MS, "warnings": [],
                    "screenshot": {
                        "artifact_ref": "content", "mime_type": "image/png",
                        "width": 320, "height": 180, "space": "screenshot", "interactive": true
                    }
                });
            }
            Self::FailedClick => {}
        }
        let mut result = json!({
            "state": state, "computer": computer, "finished_at_ms": crate::now_ms()
        });
        if matches!(self, Self::Observe | Self::ClickWithScreenshotFailure) {
            let size = png().len();
            result["content"] =
                json!({"total_bytes": size, "retained_bytes": size, "truncated": false});
        }
        serde_json::from_value(json!({
            "op_id": request.op_id, "kind": kind, "state": state,
            "accepted_at_ms": crate::now_ms(), "result": result
        }))
        .unwrap()
    }
}

fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(PNG_B64)
        .unwrap()
}

async fn receive(stream: &mut Box<dyn node_executor_ipc::ExecutorIo>) -> ExecutorClientMessage {
    let frame = tokio::time::timeout(Duration::from_secs(5), read_executor_frame(stream))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ExecutorFrame::Control(payload) = frame else {
        panic!("expected control frame")
    };
    serde_json::from_slice(&payload).unwrap()
}

async fn send(stream: &mut Box<dyn node_executor_ipc::ExecutorIo>, reply: Value) {
    let _: ExecutorServerMessage = serde_json::from_value(reply.clone()).unwrap();
    stream
        .write_all(&encode_executor_control(&reply).unwrap())
        .await
        .unwrap();
}

struct Fixture {
    dir: tempfile::TempDir,
    run: RunBindingSlot,
    config: NativeComputerConfig,
    server: tokio::task::JoinHandle<()>,
    reply: Reply,
}

impl Fixture {
    async fn new(reply: Reply) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = if cfg!(windows) {
            node_executor_ipc::local_executor_endpoint(dir.path())
        } else {
            dir.path().join("computer.sock").display().to_string()
        };
        let mut listener = node_executor_ipc::LocalExecutorListener::bind(&endpoint)
            .await
            .unwrap();
        let server = tokio::spawn(async move {
            let mut stream = listener.accept().await.unwrap();
            let ExecutorClientMessage::Login(login) = receive(&mut stream).await else {
                panic!("expected login")
            };
            assert_eq!(login.token, "agent-token");
            assert_eq!(login.profile, "agent-test");
            send(
                    &mut stream,
                    json!({
                        "type": "login_ok", "request_id": login.request_id,
                        "posture": "normal", "principal": "agent", "app_instance_id": "agent@owner",
                "caps": ["describe", "computer_inspect", "computer_observe", "computer_act", "query"],
                        "limits": ExecutorLimits::default(), "revision": "one", "generation": 1,
                        "auth_valid_until_ms": crate::now_ms() + 60_000,
                        "token_expires_at_ms": crate::now_ms() + 60_000,
                        "store_id": "ledger", "server_epoch": "epoch"
                    }),
                )
                .await;
            let ExecutorClientMessage::Describe(describe) = receive(&mut stream).await else {
                panic!("expected describe for the omitted binding_id")
            };
            send(
                &mut stream,
                json!({
                    "type": "describe_ok", "request_id": describe.request_id,
                    "target": {
                        "node_did": "did:web:ood1", "profile_id": "agent-test", "workdir_roots": [],
                        "store_id": "ledger", "protocol_version": NODE_EXECUTOR_PROTOCOL_VERSION
                    },
                    "info": {
                        "os": "macos", "arch": "aarch64", "hostname": "desktop", "shell": "bash",
                        "time_ms": crate::now_ms(),
                        "desktop": {"ui": {"supported": true, "ready": true, "display_id": 1}}
                    }
                }),
            )
            .await;
            let ExecutorClientMessage::Computer(request) = receive(&mut stream).await else {
                panic!("expected computer operation")
            };
            let mut expected = reply.request();
            expected["binding_id"] = json!(BINDING_ID);
            assert_eq!(
                request.request,
                serde_json::from_value::<ComputerRequest>(expected).unwrap()
            );
            assert_eq!(request.store_id, "ledger");
            let status = reply.status(&request);
            send(
                &mut stream,
                json!({
                    "type": "accepted", "request_id": request.request_id, "op": status
                }),
            )
            .await;
            if matches!(reply, Reply::Observe | Reply::ClickWithScreenshotFailure) {
                let ExecutorClientMessage::Query(query) = receive(&mut stream).await else {
                    panic!("expected PNG download")
                };
                assert_eq!(query.op_id, request.op_id);
                assert_eq!(query.channel, Some(ExecutorChannel::Content));
                assert_eq!(query.offset, 0);
                if matches!(reply, Reply::ClickWithScreenshotFailure) {
                    send(
                        &mut stream,
                        json!({
                            "type": "error", "request_id": query.request_id,
                            "error": {"code": "result_expired", "message": SCREENSHOT_ERROR}
                        }),
                    )
                    .await;
                    match tokio::time::timeout(Duration::from_secs(5), read_executor_frame(&mut stream)).await.unwrap() {
                        Ok(None) => {},
                        Err(error) if matches!(error.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset) => {},
                        other => panic!("completed click must not be reissued after a screenshot error: {other:?}"),
                    }
                    return;
                }
                send(
                    &mut stream,
                    json!({
                        "type": "query_ok", "request_id": query.request_id, "op": status,
                        "chunk": {
                            "channel": "content", "offset": 0, "next_offset": png().len(),
                            "retained_from": 0, "truncated": false, "eof": true, "data_b64": PNG_B64
                        }
                    }),
                )
                .await;
            }
        });
        let run = new_run_binding_slot();
        *run.lock().unwrap() = Some(RunBinding {
            run_id: "computer-result-test".into(),
            run_dir: Some(dir.path().join("run")),
        });
        Self {
            dir,
            run,
            server,
            reply,
            config: NativeComputerConfig {
                endpoint,
                node_did: "did:web:ood1".into(),
                profile_id: "agent-test".into(),
                access: ComputerAccess::Control,
            },
        }
    }

    fn tool(&self) -> Arc<dyn AgentTool> {
        tool(
            self.config.clone(),
            self.run.clone(),
            Some(Arc::new(TokensOnly)),
        )
    }

    async fn call(&self) -> AgentToolResult {
        let ctx = SessionRuntimeContext {
            trace_id: "computer-result-test".into(),
            agent_name: "agent".into(),
            behavior: "test".into(),
            tool_call_index: 0,
            wakeup_id: "wakeup".into(),
            session_id: "session".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        };
        tokio::time::timeout(
            Duration::from_secs(5),
            super::super::CURRENT_TOOL_CALL.scope(
                "computer-call".into(),
                self.tool()
                    .call(&ctx, json!({"request": self.reply.request()})),
            ),
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn dispatch(&self) -> Observation {
        let mut manager = XllmToolManager::new(
            self.dir.path().into(),
            "computer-result-test",
            LoopModel::FunctionCall,
        );
        manager.register(self.tool(), "native").unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            manager.call_tool(
                AiToolCall {
                    name: "computer".into(),
                    call_id: "computer-dispatch".into(),
                    args: HashMap::from([("request".into(), self.reply.request())]),
                },
                ToolCallCtx::noop(),
            ),
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn finish(self) {
        tokio::time::timeout(Duration::from_secs(5), self.server)
            .await
            .unwrap()
            .unwrap();
    }
}

fn assert_observation(text: &str) {
    let details: Value =
        serde_json::from_str(text).expect("the model must receive computer JSON, not just ok");
    let observation = &details["status"]["result"]["computer"]["observation"];
    assert_eq!(observation["observation_id"], OBSERVATION_ID);
    assert_eq!(observation["binding_id"], BINDING_ID);
    assert_eq!(observation["expires_at_ms"], EXPIRES_AT_MS);
    assert_eq!(observation["screenshot"]["width"], 320);
    assert_eq!(observation["screenshot"]["height"], 180);
    assert_eq!(observation["screenshot"]["space"], "screenshot");
    let path = Path::new(
        details["screenshot"]
            .as_str()
            .expect("local screenshot path"),
    );
    assert!(path.is_absolute());
    let bytes = std::fs::read(path).expect("downloaded screenshot must remain readable");
    assert_eq!(bytes, png());
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 320);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 180);
}

fn assert_failure(text: &str) {
    assert!(text.contains("stale_observation"));
    assert!(text.contains("The desktop changed; observe again before clicking"));
    assert!(text.contains("not_started"));
    assert!(text.contains("operation_id"));
}

#[tokio::test]
async fn inspect_exposes_native_driver_capabilities_and_binding() {
    let fixture = Fixture::new(Reply::Inspect).await;
    let result = fixture.call().await;
    assert_eq!(result.status, AgentToolStatus::Success);
    let details: Value =
        serde_json::from_str(&result.observation_text()).expect("inspect must expose its details");
    let computer = &details["status"]["result"]["computer"];
    assert_eq!(computer["binding"]["binding_id"], BINDING_ID);
    assert_eq!(computer["binding"]["expires_at_ms"], EXPIRES_AT_MS);
    assert_eq!(computer["capabilities"]["capture"]["ready"], true);
    assert_eq!(
        computer["capabilities"]["observation_modes"],
        json!(["visual"])
    );
    fixture.finish().await;
}

#[tokio::test]
async fn observe_exposes_downloaded_png_and_original_target_coordinates() {
    let fixture = Fixture::new(Reply::Observe).await;
    let result = fixture.call().await;
    assert_eq!(result.status, AgentToolStatus::Success);
    assert_observation(&result.observation_text());
    fixture.finish().await;
}

#[tokio::test]
async fn failed_native_click_is_an_error_with_execution_details() {
    let fixture = Fixture::new(Reply::FailedClick).await;
    let result = fixture.call().await;
    assert_eq!(result.status, AgentToolStatus::Error);
    assert_failure(&result.observation_text());
    fixture.finish().await;
}

#[tokio::test]
async fn completed_click_preserves_success_when_screenshot_download_fails() {
    let fixture = Fixture::new(Reply::ClickWithScreenshotFailure).await;
    let result = fixture.call().await;
    assert_eq!(result.status, AgentToolStatus::Success);
    let details: Value = serde_json::from_str(&result.observation_text()).unwrap();
    assert_eq!(details["status"]["state"], "succeeded");
    assert_eq!(
        details["status"]["result"]["computer"]["execution"]["state"],
        "completed"
    );
    assert_eq!(details["screenshot_error"], SCREENSHOT_ERROR);
    assert!(details.get("screenshot").is_none());
    assert!(!fixture
        .dir
        .path()
        .join("run/computer/computer-call.png")
        .exists());
    fixture.finish().await;
}

#[tokio::test]
async fn xllm_dispatch_preserves_native_observation_in_model_content() {
    let fixture = Fixture::new(Reply::Observe).await;
    let observation = fixture.dispatch().await;
    let Observation::Success { content, .. } = observation else {
        panic!("expected a successful observation")
    };
    assert_observation(content.as_str().expect("model content"));
    fixture.finish().await;
}

#[tokio::test]
async fn xllm_dispatch_preserves_failed_native_action_in_model_error() {
    let fixture = Fixture::new(Reply::FailedClick).await;
    let observation = fixture.dispatch().await;
    let Observation::Error { message, .. } = observation else {
        panic!("expected an error observation")
    };
    assert_failure(&message);
    fixture.finish().await;
}
