//! Manual DV harness: the real desktop Driver behind a real NodeExecutor
//! (operation ledger, content channel, semantic limits), driven one request
//! at a time over `127.0.0.1:<port>`. Each connection sends one JSON line —
//! a Computer request (`binding_id` may be omitted) — and gets back what the
//! agent `computer` tool shows the model: the result JSON, then the semantic
//! outline as plain lines; screenshots are saved as `<out>/<op_id>.png`.
//!
//! cargo run -p computer_driver --example computer_dv -- <port> <out_dir>

#[cfg(any(target_os = "macos", windows))]
mod harness {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use buckyos_api::*;
    use node_executor::*;
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const APP: &str = "computer-dv.test.buckyos.io";
    const OWNER: &str = "owner";
    const PROFILE: &str = "desktop";

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
                expires_at_ms: now_ms() + 3_600_000,
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

    async fn exchange(
        client: &mut tokio::io::DuplexStream,
        message: ExecutorClientMessage,
    ) -> Result<ExecutorServerMessage, String> {
        let id = message.request_id().map(str::to_owned);
        write_executor_control(client, &message)
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(60), read_executor_frame(client))
                .await
                .map_err(|_| "executor reply timed out".to_string())?
                .map_err(|e| e.to_string())?
                .ok_or("executor closed")?;
            let ExecutorFrame::Control(bytes) = frame else {
                continue;
            };
            let reply: ExecutorServerMessage =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
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
                return Ok(reply);
            }
        }
    }

    async fn connect(executor: &NodeExecutor) -> Result<tokio::io::DuplexStream, String> {
        let (mut client, server) = tokio::io::duplex(1 << 20);
        let serving = executor.clone();
        tokio::spawn(async move { serving.serve(server).await });
        let login = exchange(
            &mut client,
            ExecutorClientMessage::Login(ExecutorLogin {
                v: NODE_EXECUTOR_PROTOCOL_VERSION,
                request_id: Some("login".into()),
                token: "dv".into(),
                profile: PROFILE.into(),
                client: ExecutorClientInfo {
                    runtime_id: "dv".into(),
                    run_id: "dv".into(),
                },
            }),
        )
        .await?;
        match login {
            ExecutorServerMessage::LoginOk(_) => Ok(client),
            other => Err(format!("{other:?}")),
        }
    }

    struct Session {
        executor: NodeExecutor,
        config: ExecutorConfig,
        client: tokio::io::DuplexStream,
        store: String,
        out: PathBuf,
        next: u64,
    }

    impl Session {
        async fn run(&mut self, line: &str) -> Result<String, String> {
            let mut request: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
            if request.get("binding_id").is_none() {
                request["binding_id"] = json!(format!("{PROFILE}:display:1"));
            }
            let request: ComputerRequest =
                serde_json::from_value(request).map_err(|e| e.to_string())?;
            self.config.auth_valid_until_ms = now_ms() + 3_600_000;
            self.executor.apply_config(Some(self.config.clone())).await;
            self.client = connect(&self.executor).await?;
            self.next += 1;
            let op_id = format!("dv-{}-{}", std::process::id(), self.next);
            let accepted = exchange(
                &mut self.client,
                ExecutorClientMessage::Computer(ExecutorComputerRequest {
                    request_id: format!("submit-{op_id}"),
                    store_id: self.store.clone(),
                    op_id: op_id.clone(),
                    run_id: "dv".into(),
                    call_id: op_id.clone(),
                    request,
                }),
            )
            .await?;
            let mut status = match accepted {
                ExecutorServerMessage::Accepted(reply) => reply.op,
                other => return Err(format!("{other:?}")),
            };
            while !status.state.is_terminal() {
                let reply = exchange(
                    &mut self.client,
                    ExecutorClientMessage::Query(ExecutorQueryRequest {
                        request_id: format!("query-{op_id}"),
                        store_id: self.store.clone(),
                        op_id: op_id.clone(),
                        channel: None,
                        offset: 0,
                        max_bytes: None,
                        wait_ms: Some(1_000),
                    }),
                )
                .await?;
                match reply {
                    ExecutorServerMessage::QueryOk(reply) => status = reply.op,
                    other => return Err(format!("{other:?}")),
                }
            }
            let mut details = json!({"status": status, "operation_id": op_id});
            let has_image = status
                .result
                .as_ref()
                .and_then(|r| r.content.as_ref())
                .is_some_and(|c| c.retained_bytes > 0);
            if has_image {
                let mut png = Vec::new();
                loop {
                    let reply = exchange(
                        &mut self.client,
                        ExecutorClientMessage::Query(ExecutorQueryRequest {
                            request_id: format!("content-{op_id}"),
                            store_id: self.store.clone(),
                            op_id: op_id.clone(),
                            channel: Some(ExecutorChannel::Content),
                            offset: png.len() as u64,
                            max_bytes: None,
                            wait_ms: None,
                        }),
                    )
                    .await?;
                    let ExecutorServerMessage::QueryOk(reply) = reply else {
                        return Err(format!("{reply:?}"));
                    };
                    let chunk = reply.chunk.ok_or("no content chunk")?;
                    use base64::Engine;
                    png.extend(
                        base64::engine::general_purpose::STANDARD
                            .decode(&chunk.data_b64)
                            .map_err(|e| e.to_string())?,
                    );
                    if chunk.eof {
                        break;
                    }
                }
                let path = self.out.join(format!("{op_id}.png"));
                std::fs::write(&path, png).map_err(|e| e.to_string())?;
                details["screenshot"] = json!(path);
            }
            const OBSERVATION: &str = "/status/result/computer/observation";
            let outline = details
                .pointer(&format!("{OBSERVATION}/semantic/text"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let Some(outline) = outline else {
                return Ok(details.to_string());
            };
            if let Some(text) = details.pointer_mut(&format!("{OBSERVATION}/semantic/text")) {
                *text = json!("(outline below)");
            }
            Ok(format!("{details}\n\n{outline}"))
        }
    }

    pub fn main() {
        let mut args = std::env::args().skip(1);
        let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(47_800);
        let out = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
        std::fs::create_dir_all(&out).unwrap();
        #[cfg(target_os = "macos")]
        computer_driver::initialize_desktop();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let ledger = tempfile::tempdir().unwrap();
            let executor = NodeExecutor::open_with_computer(
                ExecutorHost {
                    node_id: "computer-dv".into(),
                    node_did: "computer-dv".into(),
                    store_dir: ledger.path().join("ledger"),
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
                    path_prefix: Vec::new(),
                    buckyos_root: None,
                },
                computer_driver::driver(),
            )
            .unwrap();
            let agent = AppId::parse(APP).unwrap().app_did().to_string();
            let config = ExecutorConfig {
                enabled: true,
                revision: "sha256:dv".into(),
                generation: 1,
                auth_valid_until_ms: now_ms() + 3_600_000,
                allowed_devices: vec![],
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
                    PROFILE.into(),
                    ExecutorProfile {
                        baseline: None,
                        run_as: None,
                        workdir_roots: vec![ledger.path().display().to_string()],
                        env: BTreeMap::new(),
                        principals: vec![agent],
                        ops: ExecutorOp::ALL.to_vec(),
                    },
                )]),
                limits: ExecutorLimits::default(),
            };
            executor.apply_config(Some(config.clone())).await;
            let mut session = Session {
                config,
                client: connect(&executor).await.unwrap(),
                executor: executor.clone(),
                store: executor.store_id(),
                out,
                next: 0,
            };
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .unwrap();
            println!("computer DV harness listening on 127.0.0.1:{port}");
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (read, mut write) = stream.into_split();
                let mut line = String::new();
                if BufReader::new(read).read_line(&mut line).await.is_err() {
                    continue;
                }
                if line.trim() == "quit" {
                    break;
                }
                let reply = session.run(line.trim()).await.unwrap_or_else(|e| format!("harness error: {e}"));
                let _ = write.write_all(reply.as_bytes()).await;
            }
            drop(ledger);
        });
    }
}

fn main() {
    #[cfg(any(target_os = "macos", windows))]
    harness::main();
    #[cfg(not(any(target_os = "macos", windows)))]
    eprintln!("this platform has no desktop Driver");
}
