//! End-to-end Command Stream tests over in-memory streams with real worker
//! processes. Switch-user cases need root; current-user cases run unprivileged.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use buckyos_api::*;
use node_executor::{ExecutorHost, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession, WorkerLauncher};
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

const NOBODY: u32 = 65534;
const OWNER: &str = "devtest";
const APP: &str = "jarvis.test.buckyos.io";

fn is_root() -> bool {
    unsafe { libc_geteuid() == 0 }
}

extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
    #[link_name = "dup"]
    fn libc_dup(fd: i32) -> i32;
    #[link_name = "close"]
    fn libc_close(fd: i32) -> i32;
}

fn agent_did(app: &str) -> String {
    AppId::parse(app).unwrap().app_did().to_string()
}

fn instance(app: &str, owner: &str) -> AppInstanceId {
    format!("{app}@{owner}").parse().unwrap()
}

/// Token text: `app|<instance>|<exp_ms>` or `user|<sub>|<exp_ms>`.
struct FakeVerifier;

#[async_trait]
impl TokenVerifier for FakeVerifier {
    async fn verify(&self, token: &str) -> Result<VerifiedSession, String> {
        let parts: Vec<&str> = token.split('|').collect();
        let exp: u64 = parts.get(2).and_then(|v| v.parse().ok()).ok_or("bad token")?;
        match parts[0] {
            "app" => {
                let inst: AppInstanceId = parts[1].parse().map_err(|_| "bad instance")?;
                let sub = parts.get(3).map(|s| s.to_string()).unwrap_or_else(|| inst.owner_user_id().to_string());
                Ok(VerifiedSession {
                    subject: sub,
                    principal_kind: TokenPrincipalKind::App,
                    token_use: TokenUse::Session,
                    authorization_key: format!("app:{}", inst.app_id()),
                    app_instance_id: Some(inst),
                    expires_at_ms: exp,
                })
            }
            "user" => Ok(VerifiedSession {
                subject: parts[1].to_string(),
                principal_kind: TokenPrincipalKind::User,
                token_use: TokenUse::Session,
                app_instance_id: None,
                authorization_key: "app:control-panel".into(),
                expires_at_ms: exp,
            }),
            _ => Err("unknown".into()),
        }
    }
}

#[derive(Default)]
struct FakeRbac {
    denied: Mutex<Vec<String>>,
}

#[async_trait]
impl RbacChecker for FakeRbac {
    async fn allowed(&self, _user: &str, _app: &str, _resource: &str, action: &str) -> bool {
        !self.denied.lock().unwrap().iter().any(|a| a == action)
    }
}

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    work: PathBuf,
    exec: NodeExecutor,
    rbac: Arc<FakeRbac>,
}

fn worker_copy(dir: &Path) -> PathBuf {
    let target = dir.join("node_executor_worker");
    let worker = std::env::var_os("NODE_EXECUTOR_TEST_WORKER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_node_executor_worker")));
    std::fs::copy(worker, &target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    target
}

fn chown(path: &Path, uid: u32) {
    std::os::unix::fs::chown(path, Some(uid), Some(uid)).unwrap();
}

fn profile(work: &Path, principals: Vec<String>, ops: &[ExecutorOp]) -> ExecutorProfile {
    ExecutorProfile {
        run_as: Some(ExecutorRunAs { uid: NOBODY, gid: NOBODY }),
        workdir_roots: vec![work.to_string_lossy().to_string()],
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        principals,
        ops: ops.to_vec(),
    }
}

fn config(work: &Path, generation: u64, mode: AgentPermissionMode, valid_ms: u64) -> ExecutorConfig {
    let agent = agent_did(APP);
    ExecutorConfig {
        enabled: true,
        revision: format!("sha256:rev{generation}"),
        generation,
        auth_valid_until_ms: node_executor::now_ms() + valid_ms,
        allowed_devices: vec!["did:dev:peer".into()],
        principal_bindings: BTreeMap::from([(
            agent.clone(),
            ExecutorPrincipalBinding {
                app_instance_id: instance(APP, OWNER),
                agent_generation: 1,
                binding_generation: 1,
                enabled: true,
                app_installed: true,
                permission_mode: mode,
            },
        )]),
        profiles: BTreeMap::from([("dev".into(), profile(work, vec![agent], &ExecutorOp::ALL))]),
        limits: ExecutorLimits {
            max_concurrency: 2,
            max_output_bytes_per_stream: 64 * 1024,
            ..Default::default()
        },
    }
}

async fn setup() -> Option<Env> {
    if !is_root() {
        eprintln!("skipping: node_executor tests need root");
        return None;
    }
    let dir = tempfile::Builder::new().prefix("nx-test").tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let root = dir.path().to_path_buf();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    chown(&work, NOBODY);
    let worker = worker_copy(&root);
    let rbac = Arc::new(FakeRbac::default());
    let exec = NodeExecutor::open(ExecutorHost {
        node_id: "ood1".into(),
        node_did: "did:web:ood1.test.buckyos.io".into(),
        store_dir: root.join("ledger"),
        worker: WorkerLauncher {
            program: worker,
            args: vec![],
        },
        shell: node_executor::default_shell(),
        verifier: Arc::new(FakeVerifier),
        rbac: rbac.clone(),
    })
    .unwrap();
    exec.apply_config(Some(config(&work, 1, AgentPermissionMode::ReadWrite, 30_000)))
        .await;
    Some(Env {
        _dir: dir,
        root,
        work,
        exec,
        rbac,
    })
}

fn app_token(ms: u64) -> String {
    format!("app|{}|{}", instance(APP, OWNER), node_executor::now_ms() + ms)
}

struct Client {
    rd: ReadHalf<DuplexStream>,
    wr: WriteHalf<DuplexStream>,
    next: u64,
    events: Vec<ExecutorServerMessage>,
    data: Vec<ExecutorDataFrame>,
    store_id: String,
}

impl Client {
    async fn connect(env: &Env) -> Client {
        let (a, b) = tokio::io::duplex(1 << 20);
        let exec = env.exec.clone();
        tokio::spawn(async move { exec.serve(b).await });
        let (rd, wr) = tokio::io::split(a);
        Client {
            rd,
            wr,
            next: 1,
            events: Vec::new(),
            data: Vec::new(),
            store_id: env.exec.store_id(),
        }
    }

    fn rid(&mut self) -> String {
        self.next += 1;
        format!("r{}", self.next)
    }

    async fn send(&mut self, msg: &ExecutorClientMessage) {
        write_executor_control(&mut self.wr, msg).await.unwrap();
    }

    async fn recv(&mut self) -> Option<ExecutorServerMessage> {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(20), read_executor_frame(&mut self.rd))
                .await
                .expect("recv timeout")
                .ok()??;
            match frame {
                ExecutorFrame::Control(p) => return Some(serde_json::from_slice(&p).unwrap()),
                ExecutorFrame::Data(d) => self.data.push(d),
            }
        }
    }

    /// Next message for `rid`, keeping unrelated events.
    async fn reply(&mut self, rid: &str) -> ExecutorServerMessage {
        loop {
            let msg = self.recv().await.expect("stream closed");
            let id = match &msg {
                ExecutorServerMessage::LoginOk(m) => m.request_id.clone(),
                ExecutorServerMessage::DescribeOk(m) => Some(m.request_id.clone()),
                ExecutorServerMessage::Accepted(m) | ExecutorServerMessage::Status(m) => Some(m.request_id.clone()),
                ExecutorServerMessage::QueryOk(m) => Some(m.request_id.clone()),
                ExecutorServerMessage::Error(m) => m.request_id.clone(),
                ExecutorServerMessage::Pong(m) => Some(m.request_id.clone()),
                _ => None,
            };
            if id.as_deref() == Some(rid) {
                return msg;
            }
            self.events.push(msg);
        }
    }

    async fn login_with(&mut self, token: String, run_id: &str) -> ExecutorServerMessage {
        let rid = self.rid();
        self.send(&ExecutorClientMessage::Login(ExecutorLogin {
            v: NODE_EXECUTOR_PROTOCOL_VERSION,
            request_id: Some(rid.clone()),
            token,
            profile: "dev".into(),
            client: ExecutorClientInfo {
                runtime_id: "node-dev".into(),
                run_id: run_id.into(),
            },
        }))
        .await;
        self.reply(&rid).await
    }

    async fn login(&mut self) {
        match self.login_with(app_token(60_000), "run1").await {
            ExecutorServerMessage::LoginOk(ok) => assert_eq!(ok.store_id, self.store_id),
            other => panic!("login failed: {other:?}"),
        }
    }

    async fn exec(&mut self, op_id: &str, call_id: &str, cmd: &str, cwd: &Path) -> ExecutorServerMessage {
        let rid = self.rid();
        let store_id = self.store_id.clone();
        self.send(&ExecutorClientMessage::Exec(ExecutorExecRequest {
            request_id: rid.clone(),
            store_id,
            op_id: op_id.into(),
            run_id: "run1".into(),
            call_id: call_id.into(),
            cmd: cmd.into(),
            cwd: cwd.to_string_lossy().to_string(),
            env_overrides: BTreeMap::new(),
            execution_timeout_ms: Some(20_000),
        }))
        .await;
        self.reply(&rid).await
    }

    async fn query(&mut self, op_id: &str, channel: Option<ExecutorChannel>, wait_ms: u64) -> ExecutorServerMessage {
        let rid = self.rid();
        let store_id = self.store_id.clone();
        self.send(&ExecutorClientMessage::Query(ExecutorQueryRequest {
            request_id: rid.clone(),
            store_id,
            op_id: op_id.into(),
            channel,
            offset: 0,
            max_bytes: None,
            wait_ms: Some(wait_ms),
        }))
        .await;
        self.reply(&rid).await
    }

    /// Wait for a terminal state; returns status and the stdout chunk. Polls
    /// without a channel: with one, any retained output at offset 0 ends the
    /// wait at once.
    async fn wait_done(&mut self, op_id: &str) -> (ExecutorOpStatus, Vec<u8>) {
        for _ in 0..40 {
            if let ExecutorServerMessage::QueryOk(q) = self.query(op_id, None, 1000).await {
                if q.op.state.is_terminal() {
                    let ExecutorServerMessage::QueryOk(q) = self.query(op_id, Some(ExecutorChannel::Stdout), 0).await else {
                        panic!("operation {op_id}: stdout query failed");
                    };
                    let data = base64::engine::general_purpose::STANDARD
                        .decode(q.chunk.unwrap().data_b64)
                        .unwrap();
                    return (q.op, data);
                }
            }
        }
        panic!("operation {op_id} did not finish");
    }

    async fn cancel(&mut self, op_id: &str) -> ExecutorServerMessage {
        let rid = self.rid();
        let store_id = self.store_id.clone();
        self.send(&ExecutorClientMessage::Cancel(ExecutorOpRef {
            request_id: rid.clone(),
            store_id,
            op_id: op_id.into(),
        }))
        .await;
        self.reply(&rid).await
    }

    async fn write_file(&mut self, op_id: &str, call_id: &str, path: &Path, content: &[u8], expected: Option<String>) -> ExecutorServerMessage {
        let rid = self.rid();
        let store_id = self.store_id.clone();
        self.send(&ExecutorClientMessage::Write(ExecutorWriteRequest {
            request_id: rid.clone(),
            store_id,
            op_id: op_id.into(),
            run_id: "run1".into(),
            call_id: call_id.into(),
            path: path.to_string_lossy().to_string(),
            mode: ExecutorWriteMode::Replace,
            expected_hash: expected,
            content_length: content.len() as u64,
            content_hash: executor_content_hash(content),
        }))
        .await;
        for (i, chunk) in content.chunks(EXECUTOR_MAX_DATA_BYTES).enumerate() {
            write_executor_data(
                &mut self.wr,
                &ExecutorDataFrame {
                    op_id: op_id.into(),
                    channel: ExecutorChannel::Upload,
                    offset: (i * EXECUTOR_MAX_DATA_BYTES) as u64,
                    data: chunk.to_vec(),
                },
            )
            .await
            .unwrap();
        }
        let end = self.rid();
        self.send(&ExecutorClientMessage::UploadEnd(ExecutorUploadEnd {
            request_id: end,
            op_id: op_id.into(),
        }))
        .await;
        self.reply(&rid).await
    }

    async fn simple(&mut self, msg: ExecutorClientMessage, rid: String) -> ExecutorServerMessage {
        self.send(&msg).await;
        self.reply(&rid).await
    }

    async fn close(mut self) {
        let _ = self.wr.shutdown().await;
    }
}

fn error_code(msg: &ExecutorServerMessage) -> Option<ExecutorErrorCode> {
    match msg {
        ExecutorServerMessage::Error(e) => Some(e.error.code),
        _ => None,
    }
}

fn accepted(msg: &ExecutorServerMessage) -> &ExecutorOpReply {
    match msg {
        ExecutorServerMessage::Accepted(r) => r,
        other => panic!("expected accepted, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn login_requires_a_one_to_one_enabled_agent_binding() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    let r = c.login_with(format!("user|{OWNER}|{}", node_executor::now_ms() + 60_000), "run1").await;
    assert_eq!(error_code(&r), Some(ExecutorErrorCode::Unauthenticated));

    let mut c = Client::connect(&env).await;
    let r = c
        .login_with(format!("app|{}|{}|mallory", instance(APP, OWNER), node_executor::now_ms() + 60_000), "run1")
        .await;
    assert_eq!(error_code(&r), Some(ExecutorErrorCode::Unauthenticated), "sub must be the owner");

    let mut c = Client::connect(&env).await;
    let r = c
        .login_with(format!("app|{}|{}", instance("other.test.buckyos.io", OWNER), node_executor::now_ms() + 60_000), "run1")
        .await;
    assert_eq!(error_code(&r), Some(ExecutorErrorCode::Forbidden));

    // A second Agent (even disabled) bound to the same runtime disables the mapping.
    let mut shared = config(&env.work, 2, AgentPermissionMode::ReadWrite, 30_000);
    shared.principal_bindings.insert(
        agent_did("helper.test.buckyos.io"),
        ExecutorPrincipalBinding {
            app_instance_id: instance(APP, OWNER),
            agent_generation: 1,
            binding_generation: 1,
            enabled: false,
            app_installed: true,
            permission_mode: AgentPermissionMode::ReadWrite,
        },
    );
    env.exec.apply_config(Some(shared)).await;
    let mut c = Client::connect(&env).await;
    let r = c.login_with(app_token(60_000), "run1").await;
    assert_eq!(error_code(&r), Some(ExecutorErrorCode::Forbidden));

    let mut disabled = config(&env.work, 3, AgentPermissionMode::ReadWrite, 30_000);
    disabled.principal_bindings.values_mut().for_each(|b| b.enabled = false);
    env.exec.apply_config(Some(disabled)).await;
    let mut c = Client::connect(&env).await;
    assert_eq!(error_code(&c.login_with(app_token(60_000), "run1").await), Some(ExecutorErrorCode::Forbidden));

    env.exec
        .apply_config(Some(config(&env.work, 4, AgentPermissionMode::ReadWrite, 30_000)))
        .await;
    env.rbac.denied.lock().unwrap().push("executor.open_stream".into());
    let mut c = Client::connect(&env).await;
    assert_eq!(error_code(&c.login_with(app_token(60_000), "run1").await), Some(ExecutorErrorCode::Forbidden));
    env.rbac.denied.lock().unwrap().clear();
    let mut c = Client::connect(&env).await;
    c.login().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exec_runs_as_profile_user_with_clean_env_and_dedupes() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    c.login().await;
    let leaked = unsafe { libc_dup(2) };
    assert!(leaked > 2);
    let cmd = format!(
        "id -u; id -g; pwd; [ -e /dev/fd/{leaked} ] && echo leaked-fd; env | sort; echo oops >&2; exit 3"
    );
    let cmd = cmd.as_str();
    let r = c.exec("op-1", "call-1", cmd, &env.work).await;
    assert!(!accepted(&r).duplicate);
    let (status, stdout) = c.wait_done("op-1").await;
    unsafe { libc_close(leaked) };
    assert_eq!(status.state, ExecutorOpState::Failed);
    let result = status.result.unwrap();
    assert_eq!(result.exit_code, Some(3));
    let text = String::from_utf8(stdout).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(NOBODY.to_string().as_str()), "{text}");
    assert_eq!(lines.next(), Some(NOBODY.to_string().as_str()), "egid: {text}");
    assert_eq!(lines.next(), Some(env.work.canonicalize().unwrap().to_str().unwrap()), "{text}");
    assert!(!text.contains("leaked-fd"), "{text}");
    assert!(!text.contains("BUCKYOS_"), "{text}");
    assert!(text.contains("PATH=/usr/bin:/bin"));
    assert_eq!(result.stderr.unwrap().total_bytes, 5);

    let again = c.exec("op-1", "call-1", cmd, &env.work).await;
    assert!(accepted(&again).duplicate);
    let conflict = c.exec("op-1", "call-1", "echo different", &env.work).await;
    assert_eq!(error_code(&conflict), Some(ExecutorErrorCode::Conflict));
    let same_call = c.exec("op-2", "call-1", cmd, &env.work).await;
    assert_eq!(error_code(&same_call), Some(ExecutorErrorCode::Conflict));

    let outside = c.exec("op-3", "call-3", "true", Path::new("/etc")).await;
    accepted(&outside);
    let (status, _) = c.wait_done("op-3").await;
    assert_eq!(status.state, ExecutorOpState::Failed);
    assert_eq!(status.result.unwrap().error.unwrap().code, ExecutorErrorCode::Forbidden);

    let rid = c.rid();
    let wrong_store = c
        .simple(
            ExecutorClientMessage::Query(ExecutorQueryRequest {
                request_id: rid.clone(),
                store_id: "st-other".into(),
                op_id: "op-1".into(),
                channel: None,
                offset: 0,
                max_bytes: None,
                wait_ms: None,
            }),
            rid,
        )
        .await;
    assert_eq!(error_code(&wrong_store), Some(ExecutorErrorCode::StoreMismatch));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn operations_survive_disconnect_and_cancel_from_a_new_stream() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("slow", "c-slow", "sleep 1; echo finished", &env.work).await);
    accepted(&c.exec("long", "c-long", "echo start; sleep 60; echo never", &env.work).await);
    c.close().await;

    let mut c = Client::connect(&env).await;
    c.login().await;
    let (status, out) = c.wait_done("slow").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded);
    assert_eq!(out, b"finished\n");

    match c.cancel("long").await {
        ExecutorServerMessage::Status(s) => assert!(s.op.cancel_requested),
        other => panic!("{other:?}"),
    }
    let (status, out) = c.wait_done("long").await;
    assert_eq!(status.state, ExecutorOpState::Cancelled);
    assert_eq!(out, b"start\n");

    // Another Agent cannot see the operation.
    let mut other = config(&env.work, 2, AgentPermissionMode::ReadWrite, 30_000);
    let helper = agent_did("helper.test.buckyos.io");
    other.principal_bindings.insert(
        helper.clone(),
        ExecutorPrincipalBinding {
            app_instance_id: instance("helper.test.buckyos.io", OWNER),
            agent_generation: 1,
            binding_generation: 1,
            enabled: true,
            app_installed: true,
            permission_mode: AgentPermissionMode::ReadWrite,
        },
    );
    other.profiles.get_mut("dev").unwrap().principals.push(helper);
    env.exec.apply_config(Some(other)).await;
    let mut h = Client::connect(&env).await;
    let token = format!("app|{}|{}", instance("helper.test.buckyos.io", OWNER), node_executor::now_ms() + 60_000);
    assert!(matches!(h.login_with(token, "run1").await, ExecutorServerMessage::LoginOk(_)));
    assert_eq!(error_code(&h.query("slow", None, 0).await), Some(ExecutorErrorCode::NotFound));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_tools_run_as_worker_with_atomic_replace_and_hash_checks() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    c.login().await;
    let file = env.work.join("notes.txt");
    let big: Vec<u8> = (0..40_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let r = c.write_file("w1", "cw1", &file, &big, None).await;
    accepted(&r);
    let (status, _) = c.wait_done("w1").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded, "{status:?}");
    let written = status.result.unwrap().file.unwrap();
    assert!(written.created);
    assert_eq!(std::fs::read(&file).unwrap(), big);
    use std::os::unix::fs::MetadataExt;
    assert_eq!(std::fs::metadata(&file).unwrap().uid(), NOBODY);

    accepted(&c.write_file("wd", "cwd", &env.work.join("sub/dir/new.txt"), b"nested", None).await);
    assert_eq!(c.wait_done("wd").await.0.state, ExecutorOpState::Succeeded);
    assert_eq!(std::fs::read(env.work.join("sub/dir/new.txt")).unwrap(), b"nested");
    accepted(&c.write_file("wo", "cwo", &env.root.join("outside/x.txt"), b"x", None).await);
    let (outside, _) = c.wait_done("wo").await;
    assert_eq!(outside.result.unwrap().error.unwrap().code, ExecutorErrorCode::Forbidden);
    assert!(!env.root.join("outside").exists());

    let small = b"alpha\nbeta\ngamma\n".to_vec();
    accepted(&c.write_file("w2", "cw2", &file, &small, Some(written.hash.clone())).await);
    assert_eq!(c.wait_done("w2").await.0.state, ExecutorOpState::Succeeded);
    accepted(&c.write_file("w3", "cw3", &file, b"stale", Some(written.hash)).await);
    let (stale, _) = c.wait_done("w3").await;
    assert_eq!(stale.state, ExecutorOpState::Failed);
    assert_eq!(stale.result.unwrap().error.unwrap().code, ExecutorErrorCode::Conflict);

    let rid = c.rid();
    let store_id = c.store_id.clone();
    let r = c
        .simple(
            ExecutorClientMessage::Edit(ExecutorEditRequest {
                request_id: rid.clone(),
                store_id: store_id.clone(),
                op_id: "e1".into(),
                run_id: "run1".into(),
                call_id: "ce1".into(),
                path: file.to_string_lossy().to_string(),
                edits: vec![ExecutorEdit {
                    old_string: "beta".into(),
                    new_string: "BETA".into(),
                    replace_all: false,
                }],
                expected_hash: None,
            }),
            rid,
        )
        .await;
    accepted(&r);
    let (edit, _) = c.wait_done("e1").await;
    let edit_file = edit.result.clone().unwrap().file.unwrap();
    assert_eq!(edit_file.line, Some(2));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\nBETA\ngamma\n");

    let rid = c.rid();
    let r = c
        .simple(
            ExecutorClientMessage::Read(ExecutorReadRequest {
                request_id: rid.clone(),
                store_id: store_id.clone(),
                op_id: "r1".into(),
                run_id: "run1".into(),
                call_id: "cr1".into(),
                path: file.to_string_lossy().to_string(),
                offset: 0,
                length: None,
            }),
            rid,
        )
        .await;
    accepted(&r);
    c.wait_done("r1").await;
    // The persisted read result does not change when the file changes.
    std::fs::write(&file, b"changed later").unwrap();
    match c.query("r1", Some(ExecutorChannel::Content), 0).await {
        ExecutorServerMessage::QueryOk(q) => {
            let data = base64::engine::general_purpose::STANDARD.decode(q.chunk.unwrap().data_b64).unwrap();
            assert_eq!(data, b"alpha\nBETA\ngamma\n");
        }
        other => panic!("{other:?}"),
    }

    // A symlink pointing outside the roots is refused.
    let secret = env.root.join("secret.txt");
    std::fs::write(&secret, b"top secret").unwrap();
    std::os::unix::fs::symlink(&secret, env.work.join("link")).unwrap();
    let rid = c.rid();
    accepted(
        &c.simple(
            ExecutorClientMessage::Read(ExecutorReadRequest {
                request_id: rid.clone(),
                store_id,
                op_id: "r2".into(),
                run_id: "run1".into(),
                call_id: "cr2".into(),
                path: env.work.join("link").to_string_lossy().to_string(),
                offset: 0,
                length: None,
            }),
            rid,
        )
        .await,
    );
    let (escaped, _) = c.wait_done("r2").await;
    assert_eq!(escaped.state, ExecutorOpState::Failed);
    assert_eq!(escaped.result.unwrap().error.unwrap().code, ExecutorErrorCode::Forbidden);

    // The worker keeps no group 0: a root:0 0640 file inside the roots stays unreadable.
    let guarded = env.work.join("root-group.txt");
    std::fs::write(&guarded, b"group 0 only").unwrap();
    std::os::unix::fs::chown(&guarded, Some(0), Some(0)).unwrap();
    std::fs::set_permissions(&guarded, std::fs::Permissions::from_mode(0o640)).unwrap();
    let rid = c.rid();
    let store_id = c.store_id.clone();
    accepted(
        &c.simple(
            ExecutorClientMessage::Read(ExecutorReadRequest {
                request_id: rid.clone(),
                store_id,
                op_id: "r3".into(),
                run_id: "run1".into(),
                call_id: "cr3".into(),
                path: guarded.to_string_lossy().to_string(),
                offset: 0,
                length: None,
            }),
            rid,
        )
        .await,
    );
    let (guarded_read, _) = c.wait_done("r3").await;
    assert_eq!(guarded_read.state, ExecutorOpState::Failed);
    assert_eq!(guarded_read.result.unwrap().error.unwrap().code, ExecutorErrorCode::Forbidden);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_only_agents_cannot_mutate_and_slots_are_bounded() {
    let Some(env) = setup().await else { return };
    env.exec
        .apply_config(Some(config(&env.work, 2, AgentPermissionMode::ReadOnly, 30_000)))
        .await;
    let mut c = Client::connect(&env).await;
    match c.login_with(app_token(60_000), "run1").await {
        ExecutorServerMessage::LoginOk(ok) => {
            assert!(!ok.caps.contains(&ExecutorOp::Exec));
            assert!(ok.caps.contains(&ExecutorOp::Read));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        error_code(&c.exec("x", "cx", "true", &env.work).await),
        Some(ExecutorErrorCode::Forbidden)
    );
    assert_eq!(
        error_code(&c.write_file("w", "cw", &env.work.join("f"), b"x", None).await),
        Some(ExecutorErrorCode::Forbidden)
    );

    env.exec
        .apply_config(Some(config(&env.work, 3, AgentPermissionMode::ReadWrite, 30_000)))
        .await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("s1", "c1", "sleep 30", &env.work).await);
    accepted(&c.exec("s2", "c2", "sleep 30", &env.work).await);
    assert_eq!(
        error_code(&c.exec("s3", "c3", "sleep 30", &env.work).await),
        Some(ExecutorErrorCode::Busy)
    );
    // Queries still answer while the slots are full.
    assert!(matches!(c.query("s1", None, 0).await, ExecutorServerMessage::QueryOk(_)));
    c.cancel("s1").await;
    c.cancel("s2").await;
    c.wait_done("s1").await;
    c.wait_done("s2").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revocation_closes_streams_and_cancels_active_operations() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("victim", "cv", "sleep 60", &env.work).await);

    let mut revoked = config(&env.work, 2, AgentPermissionMode::ReadWrite, 30_000);
    revoked.profiles.get_mut("dev").unwrap().principals = vec![agent_did("helper.test.buckyos.io")];
    env.exec.apply_config(Some(revoked)).await;
    let mut closing = None;
    while let Some(msg) = c.recv().await {
        if let ExecutorServerMessage::Closing(cl) = msg {
            closing = Some(cl.reason);
        }
    }
    assert_eq!(closing, Some(ExecutorCloseReason::Revoked));

    env.exec
        .apply_config(Some(config(&env.work, 3, AgentPermissionMode::ReadWrite, 30_000)))
        .await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    let (status, _) = c.wait_done("victim").await;
    assert_eq!(status.state, ExecutorOpState::Cancelled);

    // A stale generation cannot undo the change.
    assert_eq!(
        env.exec
            .apply_config(Some(config(&env.work, 2, AgentPermissionMode::ReadWrite, 30_000)))
            .await,
        node_executor::ApplyResult::Ignored
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_snapshot_closes_streams_and_token_expiry_only_closes_the_stream() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    assert!(matches!(c.login_with(app_token(1500), "run1").await, ExecutorServerMessage::LoginOk(_)));
    accepted(&c.exec("t1", "ct1", "sleep 3; echo survived", &env.work).await);
    let mut reason = None;
    while let Some(msg) = c.recv().await {
        if let ExecutorServerMessage::Closing(cl) = msg {
            reason = Some(cl.reason);
        }
    }
    assert_eq!(reason, Some(ExecutorCloseReason::TokenExpired));
    let mut c = Client::connect(&env).await;
    c.login().await;
    let (status, out) = c.wait_done("t1").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded);
    assert_eq!(out, b"survived\n");

    env.exec
        .apply_config(Some(config(&env.work, 2, AgentPermissionMode::ReadWrite, 1500)))
        .await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("t2", "ct2", "sleep 60", &env.work).await);
    let mut reason = None;
    while let Some(msg) = c.recv().await {
        if let ExecutorServerMessage::Closing(cl) = msg {
            reason = Some(cl.reason);
        }
    }
    assert_eq!(reason, Some(ExecutorCloseReason::AuthExpired));
    let mut c = Client::connect(&env).await;
    assert_eq!(error_code(&c.login_with(app_token(60_000), "run1").await), Some(ExecutorErrorCode::Unavailable));

    // A renewal of the same generation with the same deadline does not extend it.
    env.exec
        .apply_config(Some(config(&env.work, 3, AgentPermissionMode::ReadWrite, 30_000)))
        .await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    let (status, _) = c.wait_done("t2").await;
    assert_eq!(status.state, ExecutorOpState::Cancelled);
    let _ = HashMap::<u8, u8>::new();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interrupted_upload_never_touches_the_target() {
    let Some(env) = setup().await else { return };
    let mut c = Client::connect(&env).await;
    c.login().await;
    let target = env.work.join("partial.txt");
    let content = vec![b'x'; 20_000];
    let store_id = c.store_id.clone();
    c.send(&ExecutorClientMessage::Write(ExecutorWriteRequest {
        request_id: "w".into(),
        store_id,
        op_id: "wp".into(),
        run_id: "run1".into(),
        call_id: "cwp".into(),
        path: target.to_string_lossy().to_string(),
        mode: ExecutorWriteMode::Replace,
        expected_hash: None,
        content_length: content.len() as u64,
        content_hash: executor_content_hash(&content),
    }))
    .await;
    write_executor_data(
        &mut c.wr,
        &ExecutorDataFrame {
            op_id: "wp".into(),
            channel: ExecutorChannel::Upload,
            offset: 0,
            data: content[..EXECUTOR_MAX_DATA_BYTES].to_vec(),
        },
    )
    .await
    .unwrap();
    c.close().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!target.exists());
    let mut c = Client::connect(&env).await;
    c.login().await;
    assert_eq!(error_code(&c.query("wp", None, 0).await), Some(ExecutorErrorCode::NotFound));
    // The retry with the same identity is a normal first submission.
    accepted(&c.write_file("wp", "cwp", &target, &content, None).await);
    assert_eq!(c.wait_done("wp").await.0.state, ExecutorOpState::Succeeded);
    assert_eq!(std::fs::read(&target).unwrap(), content);
}

async fn setup_current_user() -> Env {
    let dir = tempfile::Builder::new().prefix("nx-current-user").tempdir_in("/tmp").unwrap();
    let root = dir.path().to_path_buf();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    let worker = worker_copy(&root);
    let rbac = Arc::new(FakeRbac::default());
    let exec = NodeExecutor::open_with_options(ExecutorHost {
        node_id: "executor-current-user".into(),
        node_did: "executor-current-user".into(),
        store_dir: root.join("ledger"),
        worker: WorkerLauncher { program: worker, args: vec![] },
        shell: node_executor::default_shell(),
        verifier: Arc::new(FakeVerifier),
        rbac: rbac.clone(),
    }, node_executor::ExecutorOptions {
        identity_mode: node_executor::ExecutorIdentityMode::CurrentUser,
        admission: node_executor::ExecutorAdmission::Wss,
    }).unwrap();
    let mut authorization = config(&work, 1, AgentPermissionMode::ReadWrite, 30_000);
    authorization.allowed_devices.clear();
    authorization.profiles.get_mut("dev").unwrap().run_as = None;
    assert!(authorization.validate().is_err());
    authorization.validate_execution().unwrap();
    assert_eq!(exec.apply_config(Some(authorization)).await, node_executor::ApplyResult::Replaced);
    assert!(exec.gateway_acl().is_none());
    Env { _dir: dir, root, work, exec, rbac }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_user_exec_preserves_identity_groups_and_clean_environment() {
    let env = setup_current_user().await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("identity", "identity-call", "id -u; id -g; id -G; pwd; printf '%s\n' \"$HOME\"; env | sort", &env.work).await);
    let (status, output) = c.wait_done("identity").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded);
    let output = String::from_utf8(output).unwrap();
    let mut lines = output.lines();
    assert_eq!(lines.next().unwrap(), unsafe { libc::getuid() }.to_string());
    assert_eq!(lines.next().unwrap(), unsafe { libc::getgid() }.to_string());
    let expected_groups = std::process::Command::new("id").arg("-G").output().unwrap();
    assert_eq!(lines.next().unwrap(), String::from_utf8(expected_groups.stdout).unwrap().trim());
    assert_eq!(Path::new(lines.next().unwrap()).canonicalize().unwrap(), env.work.canonicalize().unwrap());
    assert!(lines.next().unwrap().starts_with('/'));
    assert!(!output.contains("BUCKYOS_"));
    accepted(&c.exec("outside", "outside-call", "true", Path::new("/etc")).await);
    let (outside, _) = c.wait_done("outside").await;
    assert_eq!(outside.result.unwrap().error.unwrap().code, ExecutorErrorCode::Forbidden);
    let file = env.work.join("current.txt");
    accepted(&c.write_file("write-current", "write-current-call", &file, b"current user content", None).await);
    let (status, _) = c.wait_done("write-current").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded);
    assert_eq!(std::fs::read(&file).unwrap(), b"current user content");
    let read = ExecutorClientMessage::Read(ExecutorReadRequest {
        request_id: "read-current".into(), store_id: c.store_id.clone(), op_id: "read-current".into(),
        run_id: "run1".into(), call_id: "read-current-call".into(),
        path: file.to_str().unwrap().into(), offset: 0, length: None,
    });
    accepted(&c.simple(read, "read-current".into()).await);
    let (status, _) = c.wait_done("read-current").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded);
    assert_eq!(status.result.unwrap().content.unwrap().hash, Some(executor_content_hash(b"current user content")));
    let edit = ExecutorClientMessage::Edit(ExecutorEditRequest {
        request_id: "edit-current".into(), store_id: c.store_id.clone(), op_id: "edit-current".into(),
        run_id: "run1".into(), call_id: "edit-current-call".into(),
        path: file.to_str().unwrap().into(), expected_hash: None,
        edits: vec![ExecutorEdit { old_string: "current".into(), new_string: "edited".into(), replace_all: false }],
    });
    accepted(&c.simple(edit, "edit-current".into()).await);
    assert_eq!(c.wait_done("edit-current").await.0.state, ExecutorOpState::Succeeded);
    assert_eq!(std::fs::read(file).unwrap(), b"edited user content");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_user_rejects_remote_identity_and_ui_operations() {
    let env = setup_current_user().await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    let describe = c.simple(ExecutorClientMessage::Describe(ExecutorDescribeRequest {
        request_id: "target".into(), workdir: None,
    }), "target".into()).await;
    let ExecutorServerMessage::DescribeOk(describe) = describe else { panic!("{describe:?}") };
    assert_eq!(describe.target.executor_id.as_deref(), Some("executor-current-user"));
    assert!(describe.target.node_did.is_empty());
    write_executor_control(&mut c.wr, &serde_json::json!({
        "type": "ui.inspect", "request_id": "unsupported-ui"
    })).await.unwrap();
    let response = c.reply("unsupported-ui").await;
    assert_eq!(error_code(&response), Some(ExecutorErrorCode::Capability));
    let ExecutorServerMessage::Error(error) = response else { unreachable!() };
    assert!(error.error.message.contains("unsupported"));
    let mut authorization = config(&env.work, 2, AgentPermissionMode::ReadWrite, 30_000);
    authorization.allowed_devices.clear();
    env.exec.apply_config(Some(authorization)).await;
    let mut c = Client::connect(&env).await;
    let login = c.login_with(app_token(60_000), "run1").await;
    let ExecutorServerMessage::LoginOk(login) = login else { panic!("{login:?}") };
    assert!(!login.caps.contains(&ExecutorOp::Exec));
    assert_eq!(error_code(&c.exec("forbidden-user", "forbidden-call", "true", &env.work).await), Some(ExecutorErrorCode::Capability));
    assert_eq!(std::fs::read_dir(env.root.join("ledger/operations")).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_user_pause_resume_and_shutdown_preserve_queryable_results() {
    let env = setup_current_user().await;
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("completed", "completed-call", "echo completed", &env.work).await);
    assert_eq!(c.wait_done("completed").await.0.state, ExecutorOpState::Succeeded);
    env.exec.pause();
    assert!(!env.exec.is_open());
    assert_eq!(error_code(&c.exec("paused", "paused-call", "true", &env.work).await), Some(ExecutorErrorCode::Unavailable));
    let mut paused = Client::connect(&env).await;
    assert_eq!(error_code(&paused.login_with(app_token(60_000), "run1").await), Some(ExecutorErrorCode::Unavailable));
    env.exec.resume();
    accepted(&c.exec("long-current", "long-current-call", "(trap '' TERM; sleep 60) & echo $! > child.pid; wait", &env.work).await);
    let child_file = env.work.join("child.pid");
    for _ in 0..100 {
        if child_file.is_file() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let child_pid = std::fs::read_to_string(child_file).unwrap().trim().to_owned();
    env.exec.shutdown(ExecutorCloseReason::Shutdown, "test shutdown").await;
    assert!(!env.exec.is_open());
    assert_eq!(env.exec.active_operations(), 0);
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{child_pid}/stat")) {
        assert_eq!(stat.rsplit_once(')').unwrap().1.split_whitespace().next(), Some("Z"), "cancelled process group still has a live child");
    }
    env.exec.resume();
    let mut resumed = Client::connect(&env).await;
    resumed.login().await;
    assert_eq!(resumed.wait_done("completed").await.0.state, ExecutorOpState::Succeeded);
    assert_eq!(resumed.wait_done("long-current").await.0.state, ExecutorOpState::Cancelled);
    accepted(&resumed.exec("new-current", "new-current-call", "echo resumed", &env.work).await);
    assert_eq!(resumed.wait_done("new-current").await.1, b"resumed\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_user_aborted_streams_release_admission_slots() {
    let env = setup_current_user().await;
    for _ in 0..80 {
        let (client, server) = tokio::io::duplex(1 << 16);
        let serving = env.exec.clone();
        let task = tokio::spawn(async move { serving.serve(server).await });
        let (rd, wr) = tokio::io::split(client);
        let mut c = Client { rd, wr, next: 1, events: Vec::new(), data: Vec::new(), store_id: env.exec.store_id() };
        c.login().await;
        c.send(&ExecutorClientMessage::Write(ExecutorWriteRequest {
            request_id: "partial".into(), store_id: c.store_id.clone(), op_id: "partial".into(),
            run_id: "run1".into(), call_id: "partial".into(), path: env.work.join("never-written").to_str().unwrap().into(),
            mode: ExecutorWriteMode::Replace, expected_hash: None, content_length: 1024,
            content_hash: executor_content_hash(&vec![0; 1024]),
        })).await;
        c.simple(ExecutorClientMessage::Ping(ExecutorPing { request_id: "barrier".into() }), "barrier".into()).await;
        task.abort();
        let _ = task.await;
        assert_eq!(std::fs::read_dir(env.root.join("ledger/tmp")).unwrap().count(), 0);
        assert!(!env.work.join("never-written").exists());
    }
    let mut c = Client::connect(&env).await;
    c.login().await;
    accepted(&c.exec("still-available", "still-available-call", "true", &env.work).await);
    assert_eq!(c.wait_done("still-available").await.0.state, ExecutorOpState::Succeeded);
}
