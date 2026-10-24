use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::*;
use node_executor::{
    ApplyResult, ExecutorHost, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession,
    WorkerLauncher,
};
use tokio::io::DuplexStream;

const APP: &str = "jarvis.test.buckyos.io";
const OWNER: &str = "devtest";
const NODE: &str = "did:web:ood1.test.buckyos.io";
#[cfg(unix)]
const TEST_ROOT: &str = "/executor-test";
#[cfg(windows)]
const TEST_ROOT: &str = r"C:\executor-test";

struct Verifier;

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, _: &str) -> Result<VerifiedSession, String> {
        Ok(VerifiedSession {
            subject: OWNER.into(),
            principal_kind: TokenPrincipalKind::App,
            token_use: TokenUse::Session,
            app_instance_id: Some(format!("{APP}@{OWNER}").parse().unwrap()),
            authorization_key: format!("app:{APP}"),
            expires_at_ms: node_executor::now_ms() + 60_000,
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

struct Fixture {
    executor: NodeExecutor,
    client: DuplexStream,
    server: tokio::task::JoinHandle<()>,
    dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn open() -> Self {
        Self::open_with(host_run_as()).await
    }

    async fn open_with(run_as: Option<ExecutorRunAs>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let executor = NodeExecutor::open(ExecutorHost {
            node_id: "ood1".into(),
            node_did: NODE.into(),
            store_dir: dir.path().join("ledger"),
            worker: WorkerLauncher {
                program: dir.path().join("worker-must-not-be-started"),
                args: Vec::new(),
            },
            shell: dir.path().join("shell-must-not-be-started"),
            verifier: Arc::new(Verifier),
            rbac: Arc::new(AllowAll),
        })
        .unwrap();
        let agent = AppId::parse(APP).unwrap().app_did().to_string();
        let config = ExecutorConfig {
            enabled: true,
            revision: "sha256:platform-test".into(),
            generation: 1,
            auth_valid_until_ms: node_executor::now_ms() + 30_000,
            allowed_devices: vec!["did:dev:peer".into()],
            principal_bindings: BTreeMap::from([(
                agent.clone(),
                ExecutorPrincipalBinding {
                    app_instance_id: format!("{APP}@{OWNER}").parse().unwrap(),
                    agent_generation: 1,
                    binding_generation: 1,
                    enabled: true,
                    app_installed: true,
                    permission_mode: AgentPermissionMode::ReadWrite,
                },
            )]),
            profiles: BTreeMap::from([(
                "dev".into(),
                ExecutorProfile {
                    run_as,
                    workdir_roots: vec![TEST_ROOT.into()],
                    env: BTreeMap::new(),
                    principals: vec![agent],
                    ops: ExecutorOp::ALL.to_vec(),
                },
            )]),
            limits: ExecutorLimits::default(),
        };
        assert_eq!(
            executor.apply_config(Some(config)).await,
            ApplyResult::Replaced
        );
        let (client, stream) = tokio::io::duplex(64 * 1024);
        let serving = executor.clone();
        let server = tokio::spawn(async move { serving.serve(stream).await });
        Self {
            executor,
            client,
            server,
            dir,
        }
    }

    async fn request(&mut self, request: ExecutorClientMessage) -> ExecutorServerMessage {
        write_executor_control(&mut self.client, &request)
            .await
            .unwrap();
        let frame = tokio::time::timeout(
            Duration::from_secs(5),
            read_executor_frame(&mut self.client),
        )
        .await
        .expect("executor reply timed out")
        .unwrap()
        .expect("executor closed the stream");
        let ExecutorFrame::Control(bytes) = frame else {
            panic!("unexpected data frame");
        };
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn login(&mut self) -> ExecutorLoginOk {
        let response = self
            .request(ExecutorClientMessage::Login(ExecutorLogin {
                v: NODE_EXECUTOR_PROTOCOL_VERSION,
                request_id: Some("login".into()),
                token: "test-token".into(),
                profile: "dev".into(),
                client: ExecutorClientInfo {
                    runtime_id: "platform-test".into(),
                    run_id: "run1".into(),
                },
            }))
            .await;
        let ExecutorServerMessage::LoginOk(login) = response else {
            panic!("login failed: {response:?}");
        };
        login
    }
}

/// What a profile carries on this host: a Windows host runs as its own user.
fn host_run_as() -> Option<ExecutorRunAs> {
    (!cfg!(windows)).then_some(ExecutorRunAs {
        uid: 65534,
        gid: 65534,
    })
}

fn foreign_run_as() -> Option<ExecutorRunAs> {
    match host_run_as() {
        Some(_) => None,
        None => Some(ExecutorRunAs {
            uid: 65534,
            gid: 65534,
        }),
    }
}

fn supports_execution() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)]
    {
        true
    }
}

#[tokio::test]
async fn control_stream_advertises_host_capabilities_without_starting_a_worker() {
    let mut fixture = Fixture::open().await;
    let mut expected = BTreeSet::from([
        ExecutorOp::OpenStream,
        ExecutorOp::Describe,
        ExecutorOp::Query,
        ExecutorOp::Cancel,
    ]);
    if supports_execution() {
        expected.extend([
            ExecutorOp::Exec,
            ExecutorOp::Read,
            ExecutorOp::Write,
            ExecutorOp::Edit,
            ExecutorOp::Mkdir,
            ExecutorOp::Stat,
        ]);
    }
    assert_eq!(fixture.executor.host_capabilities(), expected);
    let login = fixture.login().await;
    assert_eq!(login.caps.into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(login.store_id, fixture.executor.store_id());
    let response = fixture
        .request(ExecutorClientMessage::Describe(ExecutorDescribeRequest {
            request_id: "describe".into(),
            workdir: None,
        }))
        .await;
    let ExecutorServerMessage::DescribeOk(describe) = response else {
        panic!("describe failed: {response:?}");
    };
    assert_eq!(describe.target.node_did, NODE);
    assert_eq!(describe.target.store_id, login.store_id);
    assert_eq!(describe.target.workdir, None);
    assert_eq!(describe.info.os, std::env::consts::OS);
    assert_eq!(describe.info.arch, std::env::consts::ARCH);
    assert!(fixture.dir.path().join("ledger/meta.json").is_file());
}

fn worker_requests(store_id: &str) -> Vec<ExecutorClientMessage> {
    let file = format!("{TEST_ROOT}/file");
    let mut requests = vec![serde_json::json!({
        "type": "describe", "request_id": "describe", "workdir": TEST_ROOT,
    })];
    for kind in ["exec", "read", "write", "edit"] {
        let mut request = serde_json::json!({
            "type": kind, "request_id": kind, "store_id": store_id,
            "op_id": kind, "run_id": "run1", "call_id": kind,
        });
        let fields = match kind {
            "exec" => serde_json::json!({"cmd": "exit 0", "cwd": TEST_ROOT}),
            "read" => serde_json::json!({"path": file}),
            "write" => serde_json::json!({
                "path": file, "mode": "replace",
                "content_length": 1, "content_hash": executor_content_hash(b"x"),
            }),
            "edit" => serde_json::json!({
                "path": file,
                "edits": [{"old_string": "old", "new_string": "new"}],
            }),
            _ => unreachable!(),
        };
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        requests.push(request);
    }
    requests
        .into_iter()
        .map(|r| serde_json::from_value(r).unwrap())
        .collect()
}

async fn assert_capability_without_records(fixture: &mut Fixture) {
    let store_id = fixture.executor.store_id();
    for request in worker_requests(&store_id) {
        let request_id = request.request_id().unwrap().to_string();
        let response = fixture.request(request).await;
        let ExecutorServerMessage::Error(reply) = response else {
            panic!("unsupported operation was accepted: {response:?}");
        };
        assert_eq!(reply.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(reply.error.code, ExecutorErrorCode::Capability);
        assert!(!reply.error.effect_unknown);
    }
    for name in ["operations", "tmp"] {
        assert_eq!(
            std::fs::read_dir(fixture.dir.path().join("ledger").join(name))
                .unwrap()
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn unsupported_hosts_reject_operations_before_creating_records_or_uploads() {
    if supports_execution() {
        return;
    }
    let mut fixture = Fixture::open().await;
    fixture.login().await;
    assert_capability_without_records(&mut fixture).await;
}

/// A root Unix daemon needs `run_as`; a Windows host cannot switch users.
#[tokio::test]
async fn profiles_whose_run_as_does_not_fit_the_host_keep_only_control_ops() {
    if !supports_execution() {
        return;
    }
    let mut fixture = Fixture::open_with(foreign_run_as()).await;
    let login = fixture.login().await;
    assert_eq!(
        login.caps.into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            ExecutorOp::OpenStream,
            ExecutorOp::Describe,
            ExecutorOp::Query,
            ExecutorOp::Cancel,
        ])
    );
    assert_capability_without_records(&mut fixture).await;
}
