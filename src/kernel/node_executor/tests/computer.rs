use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::*;
use node_executor::*;
use tokio::io::DuplexStream;

const APP: &str = "computer.test.buckyos.io";
const OWNER: &str = "owner";

struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, token: &str) -> Result<VerifiedSession, String> {
        let app = if token == "other" {
            "other.test.buckyos.io"
        } else {
            APP
        };
        Ok(VerifiedSession {
            subject: OWNER.into(),
            principal_kind: TokenPrincipalKind::App,
            token_use: TokenUse::Session,
            app_instance_id: Some(format!("{app}@{OWNER}").parse().unwrap()),
            authorization_key: format!("app:{app}"),
            expires_at_ms: now_ms() + 60_000,
        })
    }
}
struct AllowAll;
#[async_trait]
impl RbacChecker for AllowAll {
    async fn allowed(&self, _: &str, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    Complete,
    FeedbackFailure,
    Cancel,
    Unknown,
    Image,
    PersistFailure,
    Timeout,
}
struct FakeDriver {
    calls: AtomicUsize,
    paused: AtomicBool,
    behavior: Behavior,
    storage: std::sync::Mutex<Option<std::path::PathBuf>>,
}
impl FakeDriver {
    fn new(behavior: Behavior) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            paused: AtomicBool::new(false),
            behavior,
            storage: std::sync::Mutex::new(None),
        })
    }
}
#[async_trait]
impl ComputerDriver for FakeDriver {
    async fn execute(&self, context: ComputerContext, request: ComputerRequest) -> ComputerOutput {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut result = ComputerResult::new(
            &context.op_id,
            request.binding_id(),
            ComputerExecutionState::Completed,
        );
        let mut png = None;
        match self.behavior {
            Behavior::Complete => {}
            Behavior::PersistFailure => {
                let root = self.storage.lock().unwrap().clone().unwrap();
                for entry in std::fs::read_dir(root.join("operations")).unwrap() {
                    let path = entry.unwrap().path();
                    let record: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(path.join("request.json")).unwrap())
                            .unwrap();
                    if record["identity"]["op_id"] == context.op_id {
                        std::fs::create_dir(path.join("result.json")).unwrap();
                    }
                }
            }
            Behavior::Timeout => {
                result.execution.state = ComputerExecutionState::NotStarted;
                result.error = Some(ComputerError {
                    code: "desktop_busy_timeout".into(),
                    message: "desktop resource remained busy".into(),
                });
            }
            Behavior::FeedbackFailure => {
                result.feedback.state = ComputerFeedbackState::Failed;
                result.error = Some(ComputerError {
                    code: "capture_failed".into(),
                    message: "input completed; capture failed".into(),
                });
            }
            Behavior::Cancel => {
                while !context.cancellation.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                result.execution.state = ComputerExecutionState::Partial;
                result.error = Some(ComputerError {
                    code: "cancelled".into(),
                    message: "input already delivered before cancellation".into(),
                });
            }
            Behavior::Unknown => {
                result.execution.state = ComputerExecutionState::Unknown;
                result.error = Some(ComputerError {
                    code: "helper_lost".into(),
                    message: "no dispatch confirmation".into(),
                });
            }
            Behavior::Image => {
                use base64::Engine;
                png = Some(base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aZ1sAAAAASUVORK5CYII=").unwrap());
                result.feedback.state = ComputerFeedbackState::Included;
                result.observation = Some(ComputerObservation {
                    observation_id: "observation".into(),
                    binding_id: request.binding_id().into(),
                    surface_id: "display:1".into(),
                    surface_epoch: 1,
                    interaction_revision: 1,
                    requested_mode: ComputerObservationMode::Visual,
                    effective_mode: ComputerObservationMode::Visual,
                    selection_reason: "explicit".into(),
                    capture_started_at_ms: now_ms(),
                    capture_completed_at_ms: now_ms(),
                    consistency: "best_effort".into(),
                    screenshot: ComputerScreenshot {
                        artifact_ref: String::new(),
                        mime_type: "image/png".into(),
                        width: 1,
                        height: 1,
                        space: ComputerCoordinateSpace::Screenshot,
                        interactive: true,
                    },
                    focus_guard: "surface".into(),
                    expires_at_ms: now_ms() + 30_000,
                    warnings: vec![],
                });
            }
        }
        ComputerOutput { result, png }
    }
    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }
}

struct Fixture {
    executor: NodeExecutor,
    client: DuplexStream,
    _dir: tempfile::TempDir,
    driver: Arc<FakeDriver>,
    store: String,
    config: ExecutorConfig,
}
impl Fixture {
    async fn new(behavior: Behavior, ops: Vec<ExecutorOp>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let driver = FakeDriver::new(behavior);
        *driver.storage.lock().unwrap() = Some(dir.path().join("ledger"));
        let executor = NodeExecutor::open_with_computer(
            ExecutorHost {
                node_id: "computer-test".into(),
                node_did: "computer-test".into(),
                store_dir: dir.path().join("ledger"),
                worker: WorkerLauncher {
                    program: std::env::current_exe().unwrap(),
                    args: vec![],
                },
                shell: std::env::current_exe().unwrap(),
                verifier: Arc::new(Verifier),
                rbac: Arc::new(AllowAll),
            },
            ExecutorOptions {
                identity_mode: ExecutorIdentityMode::CurrentUser,
                admission: ExecutorAdmission::Wss,
            },
            driver.clone(),
        )
        .unwrap();
        let mut bindings = BTreeMap::new();
        let mut principals = Vec::new();
        for app in [APP, "other.test.buckyos.io"] {
            let agent = AppId::parse(app).unwrap().app_did().to_string();
            principals.push(agent.clone());
            bindings.insert(
                agent,
                ExecutorPrincipalBinding {
                    app_instance_id: format!("{app}@{OWNER}").parse().unwrap(),
                    agent_generation: 1,
                    binding_generation: 1,
                    enabled: true,
                    app_installed: true,
                    permission_mode: AgentPermissionMode::ReadWrite,
                },
            );
        }
        let config = ExecutorConfig {
            enabled: true,
            revision: "sha256:test".into(),
            generation: 1,
            auth_valid_until_ms: now_ms() + 30_000,
            allowed_devices: vec![],
            principal_bindings: bindings,
            profiles: BTreeMap::from([(
                "desktop".into(),
                ExecutorProfile {
                    run_as: None,
                    workdir_roots: vec![dir.path().display().to_string()],
                    env: BTreeMap::new(),
                    principals,
                    ops,
                },
            )]),
            limits: ExecutorLimits::default(),
        };
        assert_eq!(
            executor.apply_config(Some(config.clone())).await,
            ApplyResult::Replaced
        );
        let mut client = connect(&executor, "test").await;
        let store = executor.store_id();
        let _ = &mut client;
        Self {
            executor,
            client,
            _dir: dir,
            driver,
            store,
            config,
        }
    }
    async fn submit(&mut self, id: &str, request: ComputerRequest) -> ExecutorServerMessage {
        exchange(
            &mut self.client,
            ExecutorClientMessage::Computer(ExecutorComputerRequest {
                request_id: format!("submit-{id}"),
                store_id: self.store.clone(),
                op_id: id.into(),
                run_id: "run".into(),
                call_id: id.into(),
                request,
            }),
        )
        .await
    }
    async fn result(&mut self, id: &str) -> ExecutorOpStatus {
        loop {
            let reply = exchange(
                &mut self.client,
                ExecutorClientMessage::Query(ExecutorQueryRequest {
                    request_id: "query".into(),
                    store_id: self.store.clone(),
                    op_id: id.into(),
                    channel: None,
                    offset: 0,
                    max_bytes: None,
                    wait_ms: Some(100),
                }),
            )
            .await;
            let ExecutorServerMessage::QueryOk(reply) = reply else {
                panic!("{reply:?}")
            };
            if reply.op.state.is_terminal() {
                return reply.op;
            }
        }
    }
}
async fn connect(executor: &NodeExecutor, token: &str) -> DuplexStream {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let executor = executor.clone();
    tokio::spawn(async move { executor.serve(server).await });
    let reply = exchange(
        &mut client,
        ExecutorClientMessage::Login(ExecutorLogin {
            v: NODE_EXECUTOR_PROTOCOL_VERSION,
            request_id: Some("login".into()),
            token: token.into(),
            profile: "desktop".into(),
            client: ExecutorClientInfo {
                runtime_id: "runtime".into(),
                run_id: "run".into(),
            },
        }),
    )
    .await;
    assert!(
        matches!(reply, ExecutorServerMessage::LoginOk(_)),
        "{reply:?}"
    );
    client
}
async fn exchange(
    client: &mut DuplexStream,
    message: ExecutorClientMessage,
) -> ExecutorServerMessage {
    let id = message.request_id().map(str::to_owned);
    write_executor_control(client, &message).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = read_executor_frame(client).await.unwrap().unwrap();
            let ExecutorFrame::Control(bytes) = frame else {
                continue;
            };
            let reply: ExecutorServerMessage = serde_json::from_slice(&bytes).unwrap();
            let request_id = match &reply {
                ExecutorServerMessage::LoginOk(r) => r.request_id.clone(),
                ExecutorServerMessage::Accepted(r) | ExecutorServerMessage::Status(r) => {
                    Some(r.request_id.clone())
                }
                ExecutorServerMessage::QueryOk(r) => Some(r.request_id.clone()),
                ExecutorServerMessage::Error(r) => r.request_id.clone(),
                _ => continue,
            };
            if request_id == id {
                return reply;
            }
        }
    })
    .await
    .unwrap()
}
fn click(feedback: ComputerFeedback) -> ComputerRequest {
    ComputerRequest::Click {
        binding_id: "desktop:display:1".into(),
        target: ComputerTarget::Point {
            observation_id: "observation".into(),
            x: 0,
            y: 0,
            space: ComputerCoordinateSpace::Screenshot,
        },
        button: ComputerButton::Left,
        count: 1,
        modifiers: vec![],
        feedback,
        feedback_mode: None,
        execution_timeout_ms: 5_000,
    }
}

#[tokio::test]
async fn duplicate_id_does_not_dispatch_twice_and_changed_request_conflicts() {
    let mut fixture = Fixture::new(Behavior::Complete, ExecutorOp::ALL.to_vec()).await;
    assert!(matches!(
        fixture.submit("click", click(ComputerFeedback::None)).await,
        ExecutorServerMessage::Accepted(_)
    ));
    assert_eq!(
        fixture.result("click").await.state,
        ExecutorOpState::Succeeded
    );
    let ExecutorServerMessage::Accepted(repeated) =
        fixture.submit("click", click(ComputerFeedback::None)).await
    else {
        panic!()
    };
    assert!(repeated.duplicate);
    assert_eq!(fixture.driver.calls.load(Ordering::SeqCst), 1);
    let mut changed = click(ComputerFeedback::None);
    if let ComputerRequest::Click { count, .. } = &mut changed {
        *count = 2;
    }
    let ExecutorServerMessage::Error(conflict) = fixture.submit("click", changed).await else {
        panic!()
    };
    assert_eq!(conflict.error.code, ExecutorErrorCode::Conflict);
    assert_eq!(fixture.driver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn action_feedback_requires_capture_grant_before_dispatch() {
    let ops = ExecutorOp::ALL
        .into_iter()
        .filter(|op| *op != ExecutorOp::ComputerObserve)
        .collect();
    let mut fixture = Fixture::new(Behavior::Complete, ops).await;
    assert!(matches!(
        fixture
            .submit("denied", click(ComputerFeedback::After))
            .await,
        ExecutorServerMessage::Error(_)
    ));
    assert_eq!(fixture.driver.calls.load(Ordering::SeqCst), 0);
    fixture
        .submit("allowed", click(ComputerFeedback::None))
        .await;
    assert_eq!(
        fixture.result("allowed").await.state,
        ExecutorOpState::Succeeded
    );
}

#[tokio::test]
async fn completed_input_is_preserved_when_feedback_fails() {
    let mut fixture = Fixture::new(Behavior::FeedbackFailure, ExecutorOp::ALL.to_vec()).await;
    fixture
        .submit("click", click(ComputerFeedback::After))
        .await;
    let result = fixture.result("click").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::Failed);
    let computer = result.computer.unwrap();
    assert_eq!(computer.execution.state, ComputerExecutionState::Completed);
    assert_eq!(computer.feedback.state, ComputerFeedbackState::Failed);
}

#[tokio::test]
async fn cancellation_does_not_erase_partial_input() {
    let mut fixture = Fixture::new(Behavior::Cancel, ExecutorOp::ALL.to_vec()).await;
    fixture.submit("click", click(ComputerFeedback::None)).await;
    while fixture.driver.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    exchange(
        &mut fixture.client,
        ExecutorClientMessage::Cancel(ExecutorOpRef {
            request_id: "cancel".into(),
            store_id: fixture.store.clone(),
            op_id: "click".into(),
        }),
    )
    .await;
    let result = fixture.result("click").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::Cancelled);
    assert_eq!(
        result.computer.unwrap().execution.state,
        ComputerExecutionState::Partial
    );
}

#[tokio::test]
async fn unknown_input_pauses_the_driver_and_is_never_replayed() {
    let mut fixture = Fixture::new(Behavior::Unknown, ExecutorOp::ALL.to_vec()).await;
    fixture.submit("click", click(ComputerFeedback::None)).await;
    let result = fixture.result("click").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::Unknown);
    assert!(result.error.unwrap().effect_unknown);
    assert!(fixture.driver.paused.load(Ordering::SeqCst));
    fixture.submit("click", click(ComputerFeedback::None)).await;
    assert_eq!(fixture.driver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn screenshots_use_ledger_content_and_original_actor_acl() {
    let mut fixture = Fixture::new(Behavior::Image, ExecutorOp::ALL.to_vec()).await;
    fixture
        .submit(
            "observe",
            ComputerRequest::Observe {
                binding_id: "desktop:display:1".into(),
                mode: ComputerObservationMode::Visual,
            },
        )
        .await;
    let result = fixture.result("observe").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::Succeeded);
    assert!(result
        .computer
        .unwrap()
        .observation
        .unwrap()
        .screenshot
        .artifact_ref
        .starts_with("executor-content:"));
    let query = ExecutorClientMessage::Query(ExecutorQueryRequest {
        request_id: "image".into(),
        store_id: fixture.store.clone(),
        op_id: "observe".into(),
        channel: Some(ExecutorChannel::Content),
        offset: 0,
        max_bytes: None,
        wait_ms: None,
    });
    let ExecutorServerMessage::QueryOk(image) = exchange(&mut fixture.client, query.clone()).await
    else {
        panic!()
    };
    assert!(image.chunk.unwrap().data_b64.starts_with("iVBOR"));
    let mut other = connect(&fixture.executor, "other").await;
    let ExecutorServerMessage::Error(denied) = exchange(&mut other, query).await else {
        panic!()
    };
    assert_eq!(denied.error.code, ExecutorErrorCode::NotFound);
}

#[tokio::test]
async fn failed_completion_persistence_is_queryable_as_unknown_without_restart() {
    let mut fixture = Fixture::new(Behavior::PersistFailure, ExecutorOp::ALL.to_vec()).await;
    fixture.submit("click", click(ComputerFeedback::None)).await;
    while fixture.executor.active_operations() > 0 {
        tokio::task::yield_now().await;
    }
    let result = fixture.result("click").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::Unknown);
    assert_eq!(
        result.computer.unwrap().execution.state,
        ComputerExecutionState::Unknown
    );
    assert!(fixture.driver.paused.load(Ordering::SeqCst));
    for entry in std::fs::read_dir(fixture._dir.path().join("ledger/operations")).unwrap() {
        std::fs::remove_dir(entry.unwrap().path().join("result.json")).unwrap();
    }
    assert_eq!(
        fixture.result("click").await.state,
        ExecutorOpState::Unknown
    );
    fixture.submit("click", click(ComputerFeedback::None)).await;
    assert_eq!(fixture.driver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn driver_timeout_codes_keep_timeout_lifecycle_and_not_started_fact() {
    let mut fixture = Fixture::new(Behavior::Timeout, ExecutorOp::ALL.to_vec()).await;
    fixture.submit("click", click(ComputerFeedback::None)).await;
    let result = fixture.result("click").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::TimedOut);
    assert_eq!(
        result.computer.unwrap().execution.state,
        ComputerExecutionState::NotStarted
    );
}

#[tokio::test]
async fn revoking_capture_cancels_an_action_with_pending_feedback() {
    let mut fixture = Fixture::new(Behavior::Cancel, ExecutorOp::ALL.to_vec()).await;
    fixture
        .submit("click", click(ComputerFeedback::After))
        .await;
    while fixture.driver.calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let mut config = fixture.config.clone();
    config.generation += 1;
    config.revision = "sha256:revoked".into();
    config
        .profiles
        .get_mut("desktop")
        .unwrap()
        .ops
        .retain(|op| *op != ExecutorOp::ComputerObserve);
    assert_eq!(
        fixture.executor.apply_config(Some(config)).await,
        ApplyResult::Replaced
    );
    fixture.client = connect(&fixture.executor, "test").await;
    let result = fixture.result("click").await.result.unwrap();
    assert_eq!(result.state, ExecutorOpState::Cancelled);
    assert_eq!(
        result.computer.unwrap().execution.state,
        ComputerExecutionState::Partial
    );
}
