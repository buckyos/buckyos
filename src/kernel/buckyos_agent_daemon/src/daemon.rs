use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::*;
use futures_util::{SinkExt, StreamExt};
use node_executor::{
    ExecutorAdmission, ExecutorHost, ExecutorIdentityMode, ExecutorOptions, NodeExecutor,
    RbacChecker, TokenVerifier, VerifiedSession, WorkerLauncher,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, protocol::WebSocketConfig, Message,
};
use url::Url;

use crate::access::{boot_ms, AccessUrl, Deadline};
use crate::local::{private_dir, Status};

struct Authorization {
    executor_id: String,
    sessions: Mutex<HashMap<String, VerifiedSession>>,
    config: RwLock<Option<ExecutorConfig>>,
    gate: Arc<AtomicU64>,
}

#[async_trait]
impl TokenVerifier for Authorization {
    async fn verify(&self, token: &str) -> Result<VerifiedSession, String> {
        if boot_ms() >= self.gate.load(Ordering::Acquire) {
            return Err("management lease is unavailable".into());
        }
        let hash = hex::encode(Sha256::digest(token.as_bytes()));
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(&hash)
            .filter(|s| s.expires_at_ms > node_executor::now_ms())
            .cloned()
            .ok_or_else(|| "App session was not attested for this connection".into())
    }
}

#[async_trait]
impl RbacChecker for Authorization {
    async fn allowed(&self, user: &str, app: &str, resource: &str, action: &str) -> bool {
        if boot_ms() >= self.gate.load(Ordering::Acquire) {
            return false;
        }
        let guard = self.config.read().unwrap();
        let Some(config) = guard.as_ref() else {
            return false;
        };
        config.profiles.iter().any(|(profile_id, profile)| {
            resource == agent_executor_resource(&self.executor_id, profile_id)
                && config.principal_bindings.iter().any(|(agent, binding)| {
                    binding.enabled
                        && binding.app_installed
                        && binding.app_instance_id.owner_user_id() == user
                        && AuthTarget::App {
                            app_instance_id: binding.app_instance_id.clone(),
                        }
                        .authorization_key()
                            == app
                        && profile.principals.contains(agent)
                        && config
                            .granted_ops(profile_id, agent)
                            .iter()
                            .any(|op| op.rbac_action() == action)
                })
        })
    }
}

pub struct DaemonOptions {
    pub access: AccessUrl,
    pub data_dir: PathBuf,
    pub shell: PathBuf,
}

#[cfg(any(target_os = "macos", windows))]
struct ComputerSignals(tokio::task::JoinHandle<()>);

#[cfg(any(target_os = "macos", windows))]
impl Drop for ComputerSignals {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn capabilities(shell: &std::path::Path, executor: &NodeExecutor) -> serde_json::Value {
    let mut value = crate::local::capabilities(shell);
    if executor.computer_paused() {
        value["ui"]["ready"] = false.into();
        value["ui"]["state"] = "paused".into();
        value["input"]["ready"] = false.into();
        value["input"]["reason"] = "paused".into();
        value["capture"]["ready"] = false.into();
        value["capture"]["reason"] = "paused".into();
    }
    value
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(endpoint: &Url, secret: &str) -> Result<Socket, String> {
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| "invalid WSS endpoint")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {secret}")
            .parse()
            .map_err(|_| "invalid credential")?,
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| "WSS connection timed out")?;
    match result {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response))
            if [401, 403, 409, 410].contains(&response.status().as_u16()) =>
        {
            Err(format!("access rejected ({})", response.status().as_u16()))
        }
        Err(_) => Err("WSS connection failed (check endpoint, network and TLS trust)".into()),
    }
}

async fn send(socket: &mut Socket, message: AgentDaemonClientMessage) -> Result<(), String> {
    let text = serde_json::to_string(&message).map_err(|_| "cannot encode management message")?;
    tokio::time::timeout(
        Duration::from_secs(3),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| "management write timed out")?
    .map_err(|_| "management connection closed".into())
}

async fn receive(socket: &mut Socket) -> Result<AgentDaemonServerMessage, String> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|_| "invalid management message".into())
            }
            Some(Ok(Message::Ping(data))) => socket
                .send(Message::Pong(data))
                .await
                .map_err(|_| "management connection closed")?,
            Some(Ok(Message::Pong(_))) => {}
            _ => return Err("management connection closed".into()),
        }
    }
}

fn local_config(
    mut config: ExecutorConfig,
    server_time: u64,
    expires: Option<u64>,
) -> Result<ExecutorConfig, String> {
    config
        .validate_execution()
        .map_err(|_| "invalid executor authorization snapshot")?;
    if !config.enabled || config.profiles.values().any(|p| p.run_as.is_some()) {
        return Err("current_user mode requires enabled profiles without run_as".into());
    }
    let end = expires.map_or(config.auth_valid_until_ms, |expiry| {
        expiry.min(config.auth_valid_until_ms)
    });
    let remaining = end.saturating_sub(server_time).min(NODE_EXECUTOR_LEASE_MS);
    if remaining == 0 {
        return Err("executor authorization expired".into());
    }
    config.auth_valid_until_ms = node_executor::now_ms().saturating_add(remaining);
    Ok(config)
}

async fn serve_data(
    endpoint: Url,
    ticket: String,
    profile: String,
    token_hash: String,
    session: AgentDaemonSession,
    server_time: u64,
    executor: NodeExecutor,
    auth: Arc<Authorization>,
) -> Result<(), String> {
    let requested_at = boot_ms();
    let socket = connect(&endpoint, &ticket).await?;
    let mut wire = agent_daemon_byte_stream(socket);
    let frame = tokio::time::timeout(Duration::from_secs(10), read_executor_frame(&mut wire))
        .await
        .map_err(|_| "Command Stream login timed out")?
        .map_err(|_| "invalid Command Stream login")?;
    let Some(ExecutorFrame::Control(payload)) = frame else {
        return Err("Command Stream requires login".into());
    };
    let message: ExecutorClientMessage =
        serde_json::from_slice(&payload).map_err(|_| "invalid Command Stream login")?;
    let ExecutorClientMessage::Login(login) = &message else {
        return Err("Command Stream requires login".into());
    };
    if login.profile != profile
        || hex::encode(Sha256::digest(login.token.as_bytes())) != token_hash
        || boot_ms() >= auth.gate.load(Ordering::Acquire)
    {
        return Err("Command Stream does not match its authorization".into());
    }
    let server_now = server_time.saturating_add(boot_ms().saturating_sub(requested_at));
    if session.expires_at_ms <= server_now {
        return Err("App session expired".into());
    }
    let expires_at_ms =
        node_executor::now_ms().saturating_add(session.expires_at_ms.saturating_sub(server_now));
    {
        let mut sessions = auth.sessions.lock().unwrap();
        sessions.retain(|_, session| session.expires_at_ms > node_executor::now_ms());
        if sessions.len() >= 256 && !sessions.contains_key(&token_hash) {
            return Err("too many active App sessions".into());
        }
        sessions.insert(
            token_hash,
            VerifiedSession {
                subject: session.subject,
                principal_kind: session.principal_kind,
                token_use: session.token_use,
                app_instance_id: session.app_instance_id,
                authorization_key: session.authorization_key,
                expires_at_ms,
            },
        );
    }
    let (core, mut bridge) = tokio::io::duplex(128 * 1024);
    let first = encode_executor_control(&message).map_err(|_| "invalid login frame")?;
    let serve = executor.serve(core);
    let forward = async {
        bridge.write_all(&first).await?;
        let (mut read_wire, mut write_wire) = tokio::io::split(wire);
        let (mut read_core, mut write_core) = tokio::io::split(bridge);
        let incoming = async {
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let n = read_wire.read(&mut buffer).await?;
                if n == 0 || boot_ms() >= auth.gate.load(Ordering::Acquire) {
                    break;
                }
                write_core.write_all(&buffer[..n]).await?;
            }
            write_core.shutdown().await
        };
        let outgoing = tokio::io::copy(&mut read_core, &mut write_wire);
        tokio::select! { result = incoming => result, result = outgoing => result.map(|_| ()) }
    };
    tokio::select! { _ = serve => {}, _ = forward => {} }
    Ok(())
}

async fn wait_deadline(deadline: &Deadline) {
    loop {
        if deadline.expired() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn retry_wait(retry: u32, stop: &mut watch::Receiver<bool>, deadline: &Deadline) -> bool {
    tokio::select! {
        _ = stop.changed() => false,
        _ = wait_deadline(deadline) => false,
        _ = tokio::time::sleep(Duration::from_secs(1u64 << retry.min(4))) => true,
    }
}

pub async fn run(options: DaemonOptions, mut stop: watch::Receiver<bool>) -> Result<(), String> {
    if !cfg!(any(target_os = "linux", target_os = "macos", windows)) {
        return Err("this build supports Linux, macOS and Windows hosts".into());
    }
    if !options.shell.is_absolute() || !options.shell.is_file() {
        return Err("an installed absolute platform shell path is required".into());
    }
    private_dir(&options.data_dir)?;
    let dir = options.data_dir.join(&options.access.executor_id);
    private_dir(&dir)?;
    #[cfg(windows)]
    let _instance_lock = crate::local::windows::InstanceLock::acquire(&dir.join("daemon.lock"))?;
    #[cfg(windows)]
    private_dir(&dir.join("ledger"))?;
    let gate = Arc::new(AtomicU64::new(0));
    let auth = Arc::new(Authorization {
        executor_id: options.access.executor_id.clone(),
        sessions: Mutex::new(HashMap::new()),
        config: RwLock::new(None),
        gate: gate.clone(),
    });
    let host = ExecutorHost {
        node_id: options.access.executor_id.clone(),
        node_did: options.access.executor_id.clone(),
        store_dir: dir.join("ledger"),
        shell: options.shell.clone(),
        worker: WorkerLauncher {
            program: std::env::current_exe().map_err(|_| "cannot locate daemon worker")?,
            args: vec![node_executor::worker::WORKER_ARG.into()],
        },
        verifier: auth.clone(),
        rbac: auth.clone(),
    };
    let executor_options = ExecutorOptions {
        identity_mode: ExecutorIdentityMode::CurrentUser,
        admission: ExecutorAdmission::Wss,
    };
    #[cfg(any(target_os = "macos", windows))]
    let opened =
        NodeExecutor::open_with_computer(host, executor_options, crate::computer::driver());
    #[cfg(not(any(target_os = "macos", windows)))]
    let opened = NodeExecutor::open_with_options(host, executor_options);
    let executor =
        opened.map_err(|_| "cannot open executor ledger (another instance may be running)")?;
    #[cfg(target_os = "macos")]
    let computer_signals = {
        let mut pause =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())
                .map_err(|_| "cannot install desktop pause signal")?;
        let mut resume =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2())
                .map_err(|_| "cannot install desktop resume signal")?;
        let executor = executor.clone();
        ComputerSignals(tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = pause.recv() => {
                        executor.set_computer_paused(true);
                        eprintln!("desktop input paused locally; resume requires a new observation");
                    }
                    _ = resume.recv() => {
                        executor.set_computer_paused(false);
                        eprintln!("desktop resume requested locally; inspect readiness and take a new observation before acting");
                    }
                }
            }
        }))
    };
    executor.pause();
    let mut status = Status::new(&options.access.executor_id, &options.shell);
    status.store_id = Some(executor.store_id());
    #[cfg(windows)]
    let computer_signals = {
        if !status.running() {
            return Err("cannot identify current Windows process".into());
        }
        let controls = crate::local::windows::Controls::create(
            status.pid,
            &status.process_start,
            &status.user_sid,
        )?;
        let (tx, rx) = watch::channel(*stop.borrow());
        let mut external_stop = std::mem::replace(&mut stop, rx);
        let executor = executor.clone();
        ComputerSignals(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(25));
            loop {
                tokio::select! {
                    _ = external_stop.changed() => { let _ = tx.send(true); break; }
                    _ = tick.tick() => match controls.poll() {
                        Ok(Some(0)) | Err(_) => { let _ = tx.send(true); break; }
                        Ok(Some(1)) => executor.set_computer_paused(true),
                        Ok(Some(2)) => executor.set_computer_paused(false),
                        _ => {}
                    }
                }
            }
        }))
    };
    status.write(&dir)?;
    let instance = uuid::Uuid::new_v4().to_string();
    let mut resume_token = None;
    let mut deadline = Deadline::default();
    let mut retry = 0u32;
    let mut final_error = None;
    let mut management_endpoint = options.access.endpoint.clone();
    management_endpoint
        .query_pairs_mut()
        .append_pair("executor_id", &options.access.executor_id);
    loop {
        if *stop.borrow() || deadline.expired() {
            break;
        }
        let started = boot_ms();
        let connection = tokio::select! {
            _ = stop.changed() => break,
            _ = wait_deadline(&deadline) => break,
            result = connect(&management_endpoint, &options.access.secret) => result,
        };
        let mut socket = match connection {
            Ok(socket) => socket,
            Err(error) => {
                if error.starts_with("access rejected") || retry >= 8 {
                    final_error = Some(error);
                    break;
                }
                retry += 1;
                eprintln!("{error}; reconnecting ({retry}/8)");
                if !retry_wait(retry, &mut stop, &deadline).await {
                    break;
                }
                continue;
            }
        };
        let result = connection_loop(
            &options,
            &executor,
            &auth,
            &mut socket,
            &instance,
            &mut resume_token,
            &mut deadline,
            started,
            &mut status,
            &dir,
            &mut stop,
        )
        .await;
        gate.store(0, Ordering::Release);
        executor.invalidate_computer();
        executor
            .close_streams(
                ExecutorCloseReason::Shutdown,
                "management connection unavailable",
            )
            .await;
        auth.sessions.lock().unwrap().clear();
        status.state = "offline".into();
        let _ = status.write(&dir);
        match result {
            Ok(()) => break,
            Err(error) if error.starts_with("terminal:") => {
                final_error = Some(error.trim_start_matches("terminal:").into());
                break;
            }
            Err(error) => {
                retry += 1;
                if retry >= 8 || deadline.expired() {
                    final_error = Some(error);
                    break;
                }
                eprintln!("management connection lost; reconnecting ({retry}/8)");
            }
        }
        if !retry_wait(retry, &mut stop, &deadline).await {
            break;
        }
    }
    gate.store(0, Ordering::Release);
    executor.invalidate_computer();
    #[cfg(any(target_os = "macos", windows))]
    drop(computer_signals);
    executor
        .shutdown(
            if deadline.expired() {
                ExecutorCloseReason::AuthExpired
            } else {
                ExecutorCloseReason::Shutdown
            },
            "daemon stopped",
        )
        .await;
    status.state = if deadline.expired() {
        "expired"
    } else {
        "stopped"
    }
    .into();
    status.write(&dir)?;
    if deadline.expired() {
        eprintln!("access URL expired; daemon stopped");
    }
    match final_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn connection_loop(
    options: &DaemonOptions,
    executor: &NodeExecutor,
    auth: &Arc<Authorization>,
    socket: &mut Socket,
    instance: &str,
    resume: &mut Option<String>,
    deadline: &mut Deadline,
    started: u64,
    status: &mut Status,
    dir: &std::path::Path,
    stop: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let handshake = async {
        send(
            socket,
            AgentDaemonClientMessage::Hello {
                protocol_version: AGENT_DAEMON_PROTOCOL_VERSION,
                executor_id: options.access.executor_id.clone(),
                instance_id: instance.into(),
                resume_token: resume.clone(),
            },
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(10), receive(socket))
            .await
            .map_err(|_| "welcome timed out")?
    };
    let welcome = tokio::select! {
        _ = stop.changed() => return Ok(()),
        _ = wait_deadline(deadline) => return Ok(()),
        welcome = handshake => welcome?,
    };
    let AgentDaemonServerMessage::Welcome {
        protocol_version,
        executor_id,
        generation,
        resume_token,
        server_time_ms,
        expires_at_ms,
        heartbeat_interval_ms,
        lease_timeout_ms,
        config,
    } = welcome
    else {
        return Err("terminal:access refused before readiness".into());
    };
    if protocol_version != AGENT_DAEMON_PROTOCOL_VERSION
        || executor_id != options.access.executor_id
        || heartbeat_interval_ms != AGENT_DAEMON_HEARTBEAT_MS
        || lease_timeout_ms != AGENT_DAEMON_LEASE_MS
    {
        return Err("terminal:unsupported access protocol".into());
    }
    deadline
        .observe(server_time_ms, expires_at_ms, started)
        .map_err(|e| format!("terminal:{e}"))?;
    let observed_server_ms = server_time_ms.saturating_add(boot_ms().saturating_sub(started));
    let config = local_config(config, observed_server_ms, expires_at_ms)
        .map_err(|e| format!("terminal:{e}"))?;
    *auth.config.write().unwrap() = Some(config.clone());
    executor.set_computer_generation(generation);
    executor.apply_config(Some(config)).await;
    auth.gate
        .store(deadline.gate_until(lease_timeout_ms), Ordering::Release);
    executor.resume();
    if !executor.is_open() {
        return Err("terminal:executor configuration is not ready".into());
    }
    *resume = Some(resume_token);
    status.capabilities = capabilities(&options.shell, executor);
    send(
        socket,
        AgentDaemonClientMessage::Ready {
            generation,
            store_id: executor.store_id(),
            capabilities: status.capabilities.clone(),
        },
    )
    .await?;
    status.state = "online".into();
    status.generation = Some(generation);
    status.expires_at_ms = expires_at_ms;
    status.write(dir)?;
    eprintln!(
        "executor {} online (generation {generation}); Ctrl-C to stop",
        executor_id
    );
    let mut heartbeats = tokio::time::interval(Duration::from_millis(heartbeat_interval_ms));
    let mut watchdog = tokio::time::interval(Duration::from_millis(100));
    heartbeats.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut streams = JoinSet::new();
    let mut current_server_ms = observed_server_ms;
    let mut server_observed_at = boot_ms();
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => {
                auth.gate.store(0, Ordering::Release);
                executor.pause();
                let goodbye = send(socket, AgentDaemonClientMessage::Goodbye { generation });
                let shutdown = executor.shutdown(ExecutorCloseReason::Shutdown, "daemon stopped");
                let _ = tokio::join!(goodbye, shutdown);
                let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
                return Ok(());
            }
            _ = watchdog.tick() => {
                if deadline.expired() {
                    auth.gate.store(0, Ordering::Release);
                    executor.pause();
                    let goodbye = send(socket, AgentDaemonClientMessage::Goodbye { generation });
                    let shutdown = executor.shutdown(ExecutorCloseReason::AuthExpired, "access URL expired");
                    let _ = tokio::join!(goodbye, shutdown);
                    return Ok(());
                }
                if boot_ms() >= auth.gate.load(Ordering::Acquire) {
                    executor.pause();
                    return Err("management heartbeat lease expired".into());
                }
            }
            result = receive(socket) => match result? {
                AgentDaemonServerMessage::Snapshot { generation: incoming, server_time_ms, expires_at_ms, config } => {
                    if incoming != generation { return Err("terminal:unexpected connection generation".into()); }
                    deadline.observe(server_time_ms, expires_at_ms, boot_ms()).map_err(|e| format!("terminal:{e}"))?;
                    let config = local_config(config, server_time_ms, expires_at_ms).map_err(|e| format!("terminal:{e}"))?;
                    if auth.config.read().unwrap().as_ref().is_some_and(|previous| previous.revision != config.revision) {
                        executor.invalidate_computer();
                    }
                    *auth.config.write().unwrap() = Some(config.clone());
                    executor.apply_config(Some(config)).await;
                    current_server_ms = server_time_ms; server_observed_at = boot_ms();
                    auth.gate.store(deadline.gate_until(lease_timeout_ms), Ordering::Release);
                    status.write(dir)?;
                }
                AgentDaemonServerMessage::OpenStream { generation: incoming, stream_id, ticket, expires_at_ms, profile_id, session, session_token_sha256 } => {
                    let server_now = current_server_ms.saturating_add(boot_ms().saturating_sub(server_observed_at));
                    if incoming != generation || expires_at_ms <= server_now || streams.len() >= 64
                        || !crate::access::valid_executor_id(&stream_id) || !executor.is_open()
                        || deadline.expired() { continue; }
                    let mut endpoint = options.access.endpoint.clone();
                    endpoint.set_path(&format!("{AGENT_DAEMON_DATA_PATH}/{stream_id}"));
                    streams.spawn(serve_data(endpoint, ticket, profile_id, session_token_sha256, session,
                        server_now, executor.clone(), auth.clone()));
                }
                AgentDaemonServerMessage::Closing { reason } => {
                    auth.gate.store(0, Ordering::Release);
                    executor.pause();
                    match reason.as_str() {
                        "heartbeat_timeout" | "disconnected" | "authorization_unavailable" =>
                            return Err("management connection temporarily unavailable".into()),
                        "expired" => return Err("terminal:access URL expired".into()),
                        "revoked" | "revoked_or_replaced" => return Err("terminal:access URL revoked".into()),
                        _ => return Err("terminal:access closed by Zone".into()),
                    }
                }
                _ => return Err("terminal:unexpected management message".into()),
            },
            _ = heartbeats.tick() => {
                let current = capabilities(&options.shell, executor);
                if current != status.capabilities {
                    status.capabilities = current;
                    send(socket, AgentDaemonClientMessage::Ready {
                        generation, store_id: executor.store_id(), capabilities: status.capabilities.clone(),
                    }).await?;
                    status.write(dir)?;
                } else {
                    send(socket, AgentDaemonClientMessage::Heartbeat { generation }).await?;
                }
            },
            Some(_) = streams.join_next(), if !streams.is_empty() => {},
        }
    }
}
