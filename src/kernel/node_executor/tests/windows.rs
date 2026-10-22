//! End-to-end Command Stream tests on Windows: PowerShell and file workers
//! run as the test's own user over in-memory streams.

#![cfg(windows)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use buckyos_api::*;
use node_executor::{ExecutorHost, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession, WorkerLauncher};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

const OWNER: &str = "devtest";
const APP: &str = "jarvis.test.buckyos.io";

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
            expires_at_ms: node_executor::now_ms() + 600_000,
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

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    work: PathBuf,
    exec: NodeExecutor,
}

/// `C:\...` without the `\\?\` prefix that canonicalize adds.
fn plain(path: &Path) -> String {
    let real = path.canonicalize().unwrap().to_string_lossy().to_string();
    real.strip_prefix(r"\\?\").unwrap_or(&real).to_string()
}

async fn setup() -> Env {
    let dir = tempfile::Builder::new().prefix("nx-win").tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    let exec = NodeExecutor::open(ExecutorHost {
        node_id: "ood1".into(),
        node_did: "did:web:ood1.test.buckyos.io".into(),
        store_dir: root.join("ledger"),
        worker: WorkerLauncher {
            program: env!("CARGO_BIN_EXE_node_executor_worker").into(),
            args: vec![],
        },
        shell: node_executor::default_shell(),
        verifier: Arc::new(Verifier),
        rbac: Arc::new(AllowAll),
    })
    .unwrap();
    let agent = AppId::parse(APP).unwrap().app_did().to_string();
    let config = ExecutorConfig {
        enabled: true,
        revision: "sha256:win".into(),
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
                run_as: None,
                workdir_roots: vec![work.to_string_lossy().to_string()],
                env: BTreeMap::from([("NX_PROFILE".into(), "from-profile".into())]),
                principals: vec![agent],
                ops: ExecutorOp::ALL.to_vec(),
            },
        )]),
        limits: ExecutorLimits {
            max_concurrency: 4,
            ..Default::default()
        },
    };
    exec.apply_config(Some(config)).await;
    Env {
        _dir: dir,
        root,
        work,
        exec,
    }
}

struct Client {
    rd: ReadHalf<DuplexStream>,
    wr: WriteHalf<DuplexStream>,
    next: u64,
    store_id: String,
}

impl Client {
    async fn connect(env: &Env) -> Client {
        let (a, b) = tokio::io::duplex(1 << 20);
        let exec = env.exec.clone();
        tokio::spawn(async move { exec.serve(b).await });
        let (rd, wr) = tokio::io::split(a);
        let mut c = Client {
            rd,
            wr,
            next: 1,
            store_id: env.exec.store_id(),
        };
        let rid = c.rid();
        let msg = ExecutorClientMessage::Login(ExecutorLogin {
            v: NODE_EXECUTOR_PROTOCOL_VERSION,
            request_id: Some(rid.clone()),
            token: "t".into(),
            profile: "dev".into(),
            client: ExecutorClientInfo {
                runtime_id: "node-dev".into(),
                run_id: "run1".into(),
            },
        });
        match c.call(msg, &rid).await {
            ExecutorServerMessage::LoginOk(ok) => assert!(ok.caps.contains(&ExecutorOp::Exec)),
            other => panic!("login failed: {other:?}"),
        }
        c
    }

    fn rid(&mut self) -> String {
        self.next += 1;
        format!("r{}", self.next)
    }

    async fn call(&mut self, msg: ExecutorClientMessage, rid: &str) -> ExecutorServerMessage {
        write_executor_control(&mut self.wr, &msg).await.unwrap();
        self.reply(rid).await
    }

    async fn reply(&mut self, rid: &str) -> ExecutorServerMessage {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(30), read_executor_frame(&mut self.rd))
                .await
                .expect("recv timeout")
                .unwrap()
                .expect("stream closed");
            let ExecutorFrame::Control(p) = frame else { continue };
            let msg: ExecutorServerMessage = serde_json::from_slice(&p).unwrap();
            let id = match &msg {
                ExecutorServerMessage::LoginOk(m) => m.request_id.clone(),
                ExecutorServerMessage::DescribeOk(m) => Some(m.request_id.clone()),
                ExecutorServerMessage::Accepted(m) | ExecutorServerMessage::Status(m) => Some(m.request_id.clone()),
                ExecutorServerMessage::QueryOk(m) => Some(m.request_id.clone()),
                ExecutorServerMessage::Error(m) => m.request_id.clone(),
                _ => None,
            };
            if id.as_deref() == Some(rid) {
                return msg;
            }
        }
    }

    async fn exec(&mut self, op_id: &str, cmd: &str, cwd: &Path, timeout_ms: u64) -> ExecutorServerMessage {
        let rid = self.rid();
        let msg = ExecutorClientMessage::Exec(ExecutorExecRequest {
            request_id: rid.clone(),
            store_id: self.store_id.clone(),
            op_id: op_id.into(),
            run_id: "run1".into(),
            call_id: format!("c-{op_id}"),
            cmd: cmd.into(),
            cwd: cwd.to_string_lossy().to_string(),
            env_overrides: BTreeMap::from([("NX_OVERRIDE".into(), "from-request".into())]),
            execution_timeout_ms: Some(timeout_ms),
        });
        self.call(msg, &rid).await
    }

    async fn query(&mut self, op_id: &str, channel: Option<ExecutorChannel>, wait_ms: u64) -> ExecutorServerMessage {
        let rid = self.rid();
        let msg = ExecutorClientMessage::Query(ExecutorQueryRequest {
            request_id: rid.clone(),
            store_id: self.store_id.clone(),
            op_id: op_id.into(),
            channel,
            offset: 0,
            max_bytes: None,
            wait_ms: Some(wait_ms),
        });
        self.call(msg, &rid).await
    }

    async fn channel(&mut self, op_id: &str, channel: ExecutorChannel) -> Vec<u8> {
        let ExecutorServerMessage::QueryOk(q) = self.query(op_id, Some(channel), 0).await else {
            panic!("operation {op_id}: channel query failed");
        };
        base64::engine::general_purpose::STANDARD
            .decode(q.chunk.unwrap().data_b64)
            .unwrap()
    }

    /// Terminal status plus stdout.
    async fn wait_done(&mut self, op_id: &str) -> (ExecutorOpStatus, Vec<u8>) {
        for _ in 0..60 {
            if let ExecutorServerMessage::QueryOk(q) = self.query(op_id, None, 1000).await {
                if q.op.state.is_terminal() {
                    let out = self.channel(op_id, ExecutorChannel::Stdout).await;
                    return (q.op, out);
                }
            }
        }
        panic!("operation {op_id} did not finish");
    }

    async fn cancel(&mut self, op_id: &str) -> ExecutorServerMessage {
        let rid = self.rid();
        let msg = ExecutorClientMessage::Cancel(ExecutorOpRef {
            request_id: rid.clone(),
            store_id: self.store_id.clone(),
            op_id: op_id.into(),
        });
        self.call(msg, &rid).await
    }

    async fn write_file(&mut self, op_id: &str, path: &Path, content: &[u8], expected: Option<String>) -> ExecutorServerMessage {
        let rid = self.rid();
        let msg = ExecutorClientMessage::Write(ExecutorWriteRequest {
            request_id: rid.clone(),
            store_id: self.store_id.clone(),
            op_id: op_id.into(),
            run_id: "run1".into(),
            call_id: format!("c-{op_id}"),
            path: path.to_string_lossy().to_string(),
            mode: ExecutorWriteMode::Replace,
            expected_hash: expected,
            content_length: content.len() as u64,
            content_hash: executor_content_hash(content),
        });
        write_executor_control(&mut self.wr, &msg).await.unwrap();
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
        let end = ExecutorClientMessage::UploadEnd(ExecutorUploadEnd {
            request_id: self.rid(),
            op_id: op_id.into(),
        });
        write_executor_control(&mut self.wr, &end).await.unwrap();
        self.reply(&rid).await
    }

    async fn read_file(&mut self, op_id: &str, path: &Path) -> ExecutorServerMessage {
        let rid = self.rid();
        let msg = ExecutorClientMessage::Read(ExecutorReadRequest {
            request_id: rid.clone(),
            store_id: self.store_id.clone(),
            op_id: op_id.into(),
            run_id: "run1".into(),
            call_id: format!("c-{op_id}"),
            path: path.to_string_lossy().to_string(),
            offset: 0,
            length: None,
        });
        self.call(msg, &rid).await
    }

    async fn edit_file(&mut self, op_id: &str, path: &Path, old: &str, new: &str) -> ExecutorServerMessage {
        let rid = self.rid();
        let msg = ExecutorClientMessage::Edit(ExecutorEditRequest {
            request_id: rid.clone(),
            store_id: self.store_id.clone(),
            op_id: op_id.into(),
            run_id: "run1".into(),
            call_id: format!("c-{op_id}"),
            path: path.to_string_lossy().to_string(),
            edits: vec![ExecutorEdit {
                old_string: old.into(),
                new_string: new.into(),
                replace_all: false,
            }],
            expected_hash: None,
        });
        self.call(msg, &rid).await
    }
}

fn accepted(msg: &ExecutorServerMessage) -> &ExecutorOpReply {
    match msg {
        ExecutorServerMessage::Accepted(r) => r,
        other => panic!("expected accepted, got {other:?}"),
    }
}

fn failure(status: &ExecutorOpStatus) -> ExecutorErrorCode {
    status.result.as_ref().unwrap().error.as_ref().unwrap().code
}

fn junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn powershell_exec_with_host_env_cwd_and_exit_code() {
    std::env::set_var("BUCKYOS_NX_SECRET", "leak");
    let env = setup().await;
    let mut c = Client::connect(&env).await;

    let rid = c.rid();
    let msg = ExecutorClientMessage::Describe(ExecutorDescribeRequest {
        request_id: rid.clone(),
        workdir: Some(env.work.to_string_lossy().to_string()),
    });
    let ExecutorServerMessage::DescribeOk(d) = c.call(msg, &rid).await else {
        panic!("describe failed");
    };
    assert_eq!(d.target.workdir.as_deref(), Some(plain(&env.work).as_str()));
    assert_eq!((d.target.uid, d.target.gid), (None, None));
    assert_eq!(d.info.os, "windows");

    let cmd = r#"
(Get-Location).ProviderPath
$env:NX_PROFILE
$env:NX_OVERRIDE
"secret=$env:BUCKYOS_NX_SECRET"
"root=$([bool]$env:SystemRoot)"
Write-Output "中文 ok"
[Console]::Error.WriteLine('oops')
cmd /c exit 3
"#;
    accepted(&c.exec("e1", cmd, &env.work, 20_000).await);
    let (status, out) = c.wait_done("e1").await;
    let text = String::from_utf8(out).unwrap();
    assert_eq!(status.state, ExecutorOpState::Failed, "{text}");
    let result = status.result.unwrap();
    assert_eq!(result.exit_code, Some(3), "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            plain(&env.work).as_str(),
            "from-profile",
            "from-request",
            "secret=",
            "root=True",
            "中文 ok"
        ],
        "{text}"
    );
    assert_eq!(c.channel("e1", ExecutorChannel::Stderr).await, b"oops\r\n");

    accepted(&c.exec("e2", "exit 7", &env.work, 20_000).await);
    assert_eq!(c.wait_done("e2").await.0.result.unwrap().exit_code, Some(7));
    accepted(&c.exec("e3", "Get-Item C:\\no-such-path-for-nx", &env.work, 20_000).await);
    let (status, _) = c.wait_done("e3").await;
    assert_eq!(status.result.unwrap().exit_code, Some(1));
    assert!(!c.channel("e3", ExecutorChannel::Stderr).await.is_empty());

    let outside = c.exec("e4", "exit 0", &env.root, 20_000).await;
    accepted(&outside);
    let (status, _) = c.wait_done("e4").await;
    assert_eq!(failure(&status), ExecutorErrorCode::Forbidden);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_and_timeout_end_the_whole_process_tree() {
    let env = setup().await;
    let mut c = Client::connect(&env).await;
    let child = |marker: &str| {
        format!(
            "Write-Output start; $p = Start-Process -FilePath powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 4; Set-Content -Path {marker} -Value yes' -WorkingDirectory '{}' -NoNewWindow -PassThru; $p.WaitForExit()",
            env.work.display()
        )
    };
    accepted(&c.exec("long", &child("cancelled.txt"), &env.work, 60_000).await);
    accepted(&c.exec("short", &child("timed-out.txt"), &env.work, 1500).await);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    match c.cancel("long").await {
        ExecutorServerMessage::Status(s) => assert!(s.op.cancel_requested),
        other => panic!("{other:?}"),
    }
    let (status, out) = c.wait_done("long").await;
    assert_eq!(status.state, ExecutorOpState::Cancelled);
    assert_eq!(out, b"start\r\n");
    let (status, _) = c.wait_done("short").await;
    assert_eq!(status.state, ExecutorOpState::TimedOut);
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!env.work.join("cancelled.txt").exists());
    assert!(!env.work.join("timed-out.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_tools_stay_inside_the_roots() {
    let env = setup().await;
    let mut c = Client::connect(&env).await;
    let file = env.work.join("notes.txt");

    accepted(&c.write_file("w1", &file, b"alpha\nbeta\ngamma\n", None).await);
    let (status, _) = c.wait_done("w1").await;
    assert_eq!(status.state, ExecutorOpState::Succeeded, "{status:?}");
    let written = status.result.unwrap().file.unwrap();
    assert!(written.created);
    assert_eq!(written.path, plain(&file));
    accepted(&c.write_file("w2", &file, b"stale", Some("sha256:00".into())).await);
    assert_eq!(failure(&c.wait_done("w2").await.0), ExecutorErrorCode::Conflict);

    accepted(&c.write_file("w3", &env.work.join("sub").join("dir").join("new.txt"), b"nested", None).await);
    assert_eq!(c.wait_done("w3").await.0.state, ExecutorOpState::Succeeded);
    assert_eq!(std::fs::read(env.work.join("sub/dir/new.txt")).unwrap(), b"nested");
    accepted(&c.write_file("w4", &env.root.join("outside").join("x.txt"), b"x", None).await);
    assert_eq!(failure(&c.wait_done("w4").await.0), ExecutorErrorCode::Forbidden);
    assert!(!env.root.join("outside").exists());

    accepted(&c.edit_file("e1", &file, "beta", "BETA").await);
    let (status, _) = c.wait_done("e1").await;
    assert_eq!(status.result.unwrap().file.unwrap().line, Some(2));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\nBETA\ngamma\n");

    accepted(&c.read_file("r1", &file).await);
    c.wait_done("r1").await;
    std::fs::write(&file, b"changed later").unwrap();
    assert_eq!(c.channel("r1", ExecutorChannel::Content).await, b"alpha\nBETA\ngamma\n");

    // A junction pointing outside the roots is refused for reads and writes.
    let secret_dir = env.root.join("secret");
    std::fs::create_dir(&secret_dir).unwrap();
    std::fs::write(secret_dir.join("s.txt"), b"top secret").unwrap();
    junction(&env.work.join("link"), &secret_dir);
    accepted(&c.read_file("r2", &env.work.join("link").join("s.txt")).await);
    assert_eq!(failure(&c.wait_done("r2").await.0), ExecutorErrorCode::Forbidden);
    accepted(&c.write_file("w5", &env.work.join("link").join("s.txt"), b"pwned", None).await);
    assert_eq!(failure(&c.wait_done("w5").await.0), ExecutorErrorCode::Forbidden);
    assert_eq!(std::fs::read(secret_dir.join("s.txt")).unwrap(), b"top secret");
}
