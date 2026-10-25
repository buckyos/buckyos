#![cfg(any(target_os = "linux", target_os = "macos", windows))]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buckyos_api::*;
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type Socket = WebSocketStream<TcpStream>;
const SECRET: &str = "testsecret012345678901234567890123456";
const TOKEN: &str = "test-attested-app-session";
const APP: &str = "jarvis.test.buckyos.io";
const OWNER: &str = "devtest";

fn config(work: &Path, generation: u64) -> ExecutorConfig {
    let app: AppInstanceId = format!("{APP}@{OWNER}").parse().unwrap();
    let agent = app.app_id().app_did().to_string();
    ExecutorConfig {
        enabled: true,
        revision: format!("sha256:lifecycle-{generation}"),
        generation,
        auth_valid_until_ms: node_executor::now_ms() + NODE_EXECUTOR_LEASE_MS,
        allowed_devices: Vec::new(),
        principal_bindings: BTreeMap::from([(
            agent.clone(),
            ExecutorPrincipalBinding {
                app_instance_id: app,
                agent_generation: 1,
                binding_generation: 1,
                enabled: true,
                app_installed: true,
                permission_mode: AgentPermissionMode::ReadWrite,
            },
        )]),
        profiles: BTreeMap::from([(
            "default".into(),
            ExecutorProfile {
                run_as: None,
                workdir_roots: vec![work.to_str().unwrap().into()],
                env: BTreeMap::new(),
                principals: vec![agent],
                ops: ExecutorOp::ALL.to_vec(),
            },
        )]),
        limits: ExecutorLimits::default(),
    }
}

async fn receive(socket: &mut Socket) -> AgentDaemonClientMessage {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(12), socket.next())
            .await
            .expect("management message timeout")
            .expect("management closed")
            .unwrap();
        match message {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
            other => panic!("unexpected management frame {other:?}"),
        }
    }
}

async fn send(socket: &mut Socket, message: AgentDaemonServerMessage) {
    socket
        .send(Message::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
}

async fn accept(listener: &TcpListener, path: &str, credential: &str) -> Socket {
    let (stream, _) = tokio::time::timeout(Duration::from_secs(12), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request_seen = Arc::new(Mutex::new(None));
    let seen = request_seen.clone();
    let socket = tokio_tungstenite::accept_hdr_async(
        stream,
        move |request: &Request, response: Response| {
            *seen.lock().unwrap() = Some((
                request.uri().to_string(),
                request
                    .headers()
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            ));
            Ok(response)
        },
    )
    .await
    .unwrap();
    let (uri, authorization) = request_seen.lock().unwrap().take().unwrap();
    assert_eq!(uri, path);
    assert_eq!(authorization, format!("Bearer {credential}"));
    socket
}

async fn start(dir: &Path, listener: &TcpListener) -> Child {
    let url = format!(
        "ws://{}{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor#{SECRET}",
        listener.local_addr().unwrap()
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_buckyos_agent_daemon"))
        .args([
            "--data-dir",
            dir.to_str().unwrap(),
            "run",
            "--url-stdin",
            "--allow-loopback-ws",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(url.as_bytes()).await.unwrap();
    input.shutdown().await.unwrap();
    child
}

async fn welcome(
    socket: &mut Socket,
    work: &Path,
    generation: u64,
    expiry: Option<u64>,
) -> (String, String, Option<String>) {
    let AgentDaemonClientMessage::Hello {
        protocol_version,
        executor_id,
        instance_id,
        resume_token,
    } = receive(socket).await
    else {
        panic!("expected hello");
    };
    assert_eq!(protocol_version, AGENT_DAEMON_PROTOCOL_VERSION);
    assert_eq!(executor_id, "test-executor");
    send(
        socket,
        AgentDaemonServerMessage::Welcome {
            protocol_version,
            executor_id,
            generation,
            resume_token: format!("resume-secret-{generation}"),
            server_time_ms: node_executor::now_ms(),
            expires_at_ms: expiry,
            heartbeat_interval_ms: AGENT_DAEMON_HEARTBEAT_MS,
            lease_timeout_ms: AGENT_DAEMON_LEASE_MS,
            config: config(work, generation),
        },
    )
    .await;
    loop {
        match receive(socket).await {
            AgentDaemonClientMessage::Ready {
                generation: actual,
                store_id,
                capabilities,
            } => {
                assert_eq!(actual, generation);
                #[cfg(target_os = "linux")]
                assert_eq!(capabilities["ui"]["state"], "unsupported");
                assert_eq!(capabilities["platform"], std::env::consts::OS);
                assert_eq!(capabilities["command"]["ready"], true);
                return (instance_id, store_id, resume_token);
            }
            AgentDaemonClientMessage::Heartbeat { .. } => {}
            other => panic!("expected readiness, got {other:?}"),
        }
    }
}

async fn open_stream(
    socket: &mut Socket,
    listener: &TcpListener,
    generation: u64,
    store_id: &str,
) -> DuplexStream {
    let stream_id = format!("stream-{generation}");
    let app_instance_id: AppInstanceId = format!("{APP}@{OWNER}").parse().unwrap();
    send(
        socket,
        AgentDaemonServerMessage::OpenStream {
            generation,
            stream_id: stream_id.clone(),
            ticket: "stream-ticket".into(),
            expires_at_ms: node_executor::now_ms() + 10_000,
            profile_id: "default".into(),
            session: AgentDaemonSession {
                subject: OWNER.into(),
                principal_kind: TokenPrincipalKind::App,
                token_use: TokenUse::Session,
                authorization_key: AuthTarget::App {
                    app_instance_id: app_instance_id.clone(),
                }
                .authorization_key(),
                app_instance_id: Some(app_instance_id),
                expires_at_ms: node_executor::now_ms() + 30_000,
            },
            session_token_sha256: hex::encode(Sha256::digest(TOKEN.as_bytes())),
        },
    )
    .await;
    let data = accept(
        listener,
        &format!("{AGENT_DAEMON_DATA_PATH}/{stream_id}"),
        "stream-ticket",
    )
    .await;
    let mut wire = agent_daemon_byte_stream(data);
    write_executor_control(
        &mut wire,
        &ExecutorClientMessage::Login(ExecutorLogin {
            v: NODE_EXECUTOR_PROTOCOL_VERSION,
            request_id: Some("login".into()),
            token: TOKEN.into(),
            profile: "default".into(),
            client: ExecutorClientInfo {
                runtime_id: "lifecycle".into(),
                run_id: "run1".into(),
            },
        }),
    )
    .await
    .unwrap();
    let ExecutorServerMessage::LoginOk(login) = reply(&mut wire, "login").await else {
        panic!("login rejected")
    };
    assert_eq!(login.store_id, store_id);
    assert!(login.caps.contains(&ExecutorOp::Exec));
    wire
}

async fn reply(wire: &mut DuplexStream, request_id: &str) -> ExecutorServerMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), read_executor_frame(wire))
            .await
            .expect("executor response timeout")
            .unwrap()
            .expect("executor closed");
        let ExecutorFrame::Control(bytes) = frame else {
            continue;
        };
        let msg: ExecutorServerMessage = serde_json::from_slice(&bytes).unwrap();
        let id = match &msg {
            ExecutorServerMessage::LoginOk(r) => r.request_id.as_deref(),
            ExecutorServerMessage::Accepted(r) | ExecutorServerMessage::Status(r) => {
                Some(r.request_id.as_str())
            }
            ExecutorServerMessage::QueryOk(r) => Some(r.request_id.as_str()),
            ExecutorServerMessage::Error(r) => r.request_id.as_deref(),
            _ => None,
        };
        if id == Some(request_id) {
            return msg;
        }
    }
}

async fn exec(wire: &mut DuplexStream, store_id: &str, op: &str, command: &str, work: &Path) {
    write_executor_control(
        wire,
        &ExecutorClientMessage::Exec(ExecutorExecRequest {
            request_id: op.into(),
            store_id: store_id.into(),
            op_id: op.into(),
            run_id: "run1".into(),
            call_id: op.into(),
            cmd: command.into(),
            cwd: work.to_str().unwrap().into(),
            env_overrides: BTreeMap::new(),
            execution_timeout_ms: Some(20_000),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply(wire, op).await,
        ExecutorServerMessage::Accepted(_)
    ));
}

async fn finished(wire: &mut DuplexStream, store_id: &str, op: &str) -> ExecutorOpStatus {
    for _ in 0..15 {
        write_executor_control(
            wire,
            &ExecutorClientMessage::Query(ExecutorQueryRequest {
                request_id: "query".into(),
                store_id: store_id.into(),
                op_id: op.into(),
                channel: None,
                offset: 0,
                max_bytes: None,
                wait_ms: Some(1000),
            }),
        )
        .await
        .unwrap();
        let ExecutorServerMessage::QueryOk(query) = reply(wire, "query").await else {
            panic!("query failed")
        };
        if query.op.state.is_terminal() {
            return query.op;
        }
    }
    panic!("operation did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_daemon_exec_file_reconnect_expiry_and_credential_redaction() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let data = temp.path().join("state");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child = start(&data, &listener).await;
    let mut management = accept(
        &listener,
        &format!("{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor"),
        SECRET,
    )
    .await;
    let expiry = Some(node_executor::now_ms() + if cfg!(windows) { 15_000 } else { 7_000 });
    let (instance, store_id, resume) = welcome(&mut management, &work, 1, expiry).await;
    assert!(resume.is_none());
    let mut wire = open_stream(&mut management, &listener, 1, &store_id).await;
    exec(
        &mut wire,
        &store_id,
        "exec",
        if cfg!(windows) {
            "[System.IO.File]::WriteAllText((Join-Path $PWD 'command.txt'), 'command')"
        } else {
            "printf command > command.txt"
        },
        &work,
    )
    .await;
    assert_eq!(
        finished(&mut wire, &store_id, "exec").await.state,
        ExecutorOpState::Succeeded
    );
    assert_eq!(std::fs::read(work.join("command.txt")).unwrap(), b"command");
    let path = work.join("file.txt").to_str().unwrap().to_owned();
    write_executor_control(
        &mut wire,
        &ExecutorClientMessage::Write(ExecutorWriteRequest {
            request_id: "write".into(),
            store_id: store_id.clone(),
            op_id: "write".into(),
            run_id: "run1".into(),
            call_id: "write".into(),
            path: path.clone(),
            mode: ExecutorWriteMode::Replace,
            expected_hash: None,
            expected_size: None,
            content_length: 5,
            content_hash: executor_content_hash(b"hello"),
        }),
    )
    .await
    .unwrap();
    write_executor_data(
        &mut wire,
        &ExecutorDataFrame {
            op_id: "write".into(),
            channel: ExecutorChannel::Upload,
            offset: 0,
            data: b"hello".to_vec(),
        },
    )
    .await
    .unwrap();
    write_executor_control(
        &mut wire,
        &ExecutorClientMessage::UploadEnd(ExecutorUploadEnd {
            request_id: "upload-end".into(),
            op_id: "write".into(),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply(&mut wire, "write").await,
        ExecutorServerMessage::Accepted(_)
    ));
    assert_eq!(
        finished(&mut wire, &store_id, "write").await.state,
        ExecutorOpState::Succeeded
    );
    write_executor_control(
        &mut wire,
        &ExecutorClientMessage::Edit(ExecutorEditRequest {
            request_id: "edit".into(),
            store_id: store_id.clone(),
            op_id: "edit".into(),
            run_id: "run1".into(),
            call_id: "edit".into(),
            path,
            expected_hash: None,
            edits: vec![ExecutorEdit {
                old_string: "hello".into(),
                new_string: "world".into(),
                replace_all: false,
            }],
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply(&mut wire, "edit").await,
        ExecutorServerMessage::Accepted(_)
    ));
    assert_eq!(
        finished(&mut wire, &store_id, "edit").await.state,
        ExecutorOpState::Succeeded
    );
    assert_eq!(std::fs::read(work.join("file.txt")).unwrap(), b"world");
    exec(
        &mut wire,
        &store_id,
        "detached",
        if cfg!(windows) { "Start-Sleep -Seconds 1; [System.IO.File]::WriteAllText((Join-Path $PWD 'recovered.txt'), 'recovered')" } else { "sleep 1; printf recovered > recovered.txt" },
        &work,
    )
    .await;
    send(
        &mut management,
        AgentDaemonServerMessage::Closing {
            reason: "authorization_unavailable".into(),
        },
    )
    .await;
    drop(management);
    drop(wire);
    let mut management = accept(
        &listener,
        &format!("{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor"),
        SECRET,
    )
    .await;
    let (new_instance, new_store, resume) = welcome(&mut management, &work, 2, expiry).await;
    assert_eq!(new_instance, instance);
    assert_eq!(new_store, store_id);
    assert_eq!(resume.as_deref(), Some("resume-secret-1"));
    let mut wire = open_stream(&mut management, &listener, 2, &store_id).await;
    assert_eq!(
        finished(&mut wire, &store_id, "detached").await.state,
        ExecutorOpState::Succeeded
    );
    assert_eq!(
        std::fs::read(work.join("recovered.txt")).unwrap(),
        b"recovered"
    );
    exec(
        &mut wire,
        &store_id,
        "expiry",
        if cfg!(windows) { "Start-Sleep -Seconds 60; [System.IO.File]::WriteAllText((Join-Path $PWD 'escaped.txt'), 'should-not-run')" } else { "sleep 60; printf should-not-run > escaped.txt" },
        &work,
    )
    .await;
    loop {
        match receive(&mut management).await {
            AgentDaemonClientMessage::Goodbye { generation } => {
                assert_eq!(generation, 2);
                break;
            }
            AgentDaemonClientMessage::Heartbeat { .. } => {}
            other => panic!("unexpected message {other:?}"),
        }
    }
    let output = tokio::time::timeout(Duration::from_secs(8), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("expired"));
    assert!(!stderr.contains(SECRET));
    assert!(!stderr.contains(TOKEN));
    assert!(!stderr.contains("resume-secret"));
    assert!(!work.join("escaped.txt").exists());
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.join("test-executor/status.json")).unwrap())
            .unwrap();
    assert_eq!(status["state"], "expired");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_daemon_duplicate_local_instance_and_normal_stop() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let data = temp.path().join("state");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child = start(&data, &listener).await;
    let mut management = accept(
        &listener,
        &format!("{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor"),
        SECRET,
    )
    .await;
    welcome(&mut management, &work, 1, None).await;
    let second = start(&data, &listener).await;
    let rejected = tokio::time::timeout(Duration::from_secs(5), second.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stderr).contains(SECRET));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("another instance"));
    let mut stop_command = Command::new(env!("CARGO_BIN_EXE_buckyos_agent_daemon"));
    stop_command.args([
        "--data-dir",
        data.to_str().unwrap(),
        "stop",
        "--executor",
        "test-executor",
    ]);
    let stopping = tokio::spawn(async move { stop_command.output().await.unwrap() });
    loop {
        match receive(&mut management).await {
            AgentDaemonClientMessage::Goodbye { generation } => {
                assert_eq!(generation, 1);
                break;
            }
            AgentDaemonClientMessage::Heartbeat { .. } => {}
            other => panic!("unexpected message {other:?}"),
        }
    }
    let output = tokio::time::timeout(Duration::from_secs(8), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    assert!(stopping.await.unwrap().status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_daemon_deadline_interrupts_reconnect_backoff_and_welcome() {
    for wait_for_hello in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let data = temp.path().join("state");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let child = start(&data, &listener).await;
        let mut management = accept(
            &listener,
            &format!("{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor"),
            SECRET,
        )
        .await;
        let lifetime = if wait_for_hello { 3_000 } else { 700 };
        welcome(
            &mut management,
            &work,
            1,
            Some(node_executor::now_ms() + lifetime),
        )
        .await;
        management.close(None).await.unwrap();
        drop(management);
        let mut stalled = None;
        if wait_for_hello {
            let mut socket = accept(
                &listener,
                &format!("{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor"),
                SECRET,
            )
            .await;
            assert!(matches!(
                receive(&mut socket).await,
                AgentDaemonClientMessage::Hello { .. }
            ));
            stalled = Some(socket);
        }
        let output = tokio::time::timeout(Duration::from_millis(1_800), child.wait_with_output())
            .await
            .expect("URL deadline must interrupt reconnect delay and welcome wait")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("expired"));
        drop(stalled);
    }
}

#[cfg(any(target_os = "macos", windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_pause_resume_is_reported_without_desktop_input() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let data = temp.path().join("state");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child = start(&data, &listener).await;
    let mut management = accept(
        &listener,
        &format!("{AGENT_DAEMON_MANAGE_PATH}?executor_id=test-executor"),
        SECRET,
    )
    .await;
    welcome(&mut management, &work, 1, None).await;
    for (command, paused) in [("pause", true), ("resume", false)] {
        let output = Command::new(env!("CARGO_BIN_EXE_buckyos_agent_daemon"))
            .args([
                "--data-dir",
                data.to_str().unwrap(),
                command,
                "--executor",
                "test-executor",
            ])
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let reported = match receive(&mut management).await {
                    AgentDaemonClientMessage::Ready { capabilities, .. } => {
                        assert_eq!(capabilities["command"]["ready"], true);
                        if capabilities["ui"]["state"] == "paused" {
                            assert_eq!(capabilities["input"]["ready"], false);
                        }
                        (capabilities["ui"]["state"] == "paused") == paused
                    }
                    AgentDaemonClientMessage::Heartbeat { .. } => false,
                    other => panic!("unexpected management message {other:?}"),
                };
                send(
                    &mut management,
                    AgentDaemonServerMessage::Snapshot {
                        generation: 1,
                        server_time_ms: node_executor::now_ms(),
                        expires_at_ms: None,
                        config: config(&work, 1),
                    },
                )
                .await;
                if reported {
                    break;
                }
            }
            loop {
                let output = Command::new(env!("CARGO_BIN_EXE_buckyos_agent_daemon"))
                    .args([
                        "--data-dir",
                        data.to_str().unwrap(),
                        "status",
                        "--executor",
                        "test-executor",
                        "--json",
                    ])
                    .output()
                    .await
                    .unwrap();
                assert!(output.status.success());
                let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                let instance = &status["instances"][0];
                assert_eq!(instance["state"], "online");
                if (instance["capabilities"]["ui"]["state"] == "paused") == paused {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("local control must be reported within one heartbeat");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_buckyos_agent_daemon"))
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "stop",
            "--executor",
            "test-executor",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(child.wait_with_output().await.unwrap().status.success());
}
