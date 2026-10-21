//! Node Executor: device-local execution of authorized Command Stream
//! operations (doc/arch/node_exector.md). Embedded by node_daemon (form A);
//! the same crate is meant for the standalone/embedded Agent Daemon.

mod auth;
mod fsutil;
mod ops;
mod spawn;
mod store;
mod stream;
pub mod worker;

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buckyos_api::{ExecutorCloseReason, ExecutorClosing, ExecutorConfig, ExecutorOp, ExecutorLimits};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

pub use auth::{ApplyResult, Grant, RbacChecker, TokenVerifier, VerifiedSession};
pub use spawn::{default_shell, WorkerLauncher};
pub use store::{Actor, StoreOpenError};

use auth::AuthState;
use ops::{CancelReason, OpsManager};
use store::Store;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct ExecutorHost {
    /// Node name used in `obj://node/<node>/executor/<profile>`.
    pub node_id: String,
    pub node_did: String,
    /// Private ledger directory, e.g. `$BUCKYOS_ROOT/local/node_daemon/executor`.
    pub store_dir: PathBuf,
    pub worker: WorkerLauncher,
    pub shell: PathBuf,
    pub verifier: Arc<dyn TokenVerifier>,
    pub rbac: Arc<dyn RbacChecker>,
}

pub(crate) struct StreamEntry {
    pub session: VerifiedSession,
    pub grant: Grant,
    pub close: watch::Sender<Option<ExecutorClosing>>,
}

pub(crate) struct ExecutorInner {
    pub node_id: String,
    pub node_did: String,
    pub shell: PathBuf,
    pub verifier: Arc<dyn TokenVerifier>,
    pub rbac: Arc<dyn RbacChecker>,
    pub store: Arc<Store>,
    pub ops: Arc<OpsManager>,
    pub auth: AuthState,
    pub host_caps: BTreeSet<ExecutorOp>,
    pub server_epoch: String,
    pub streams: Mutex<HashMap<u64, StreamEntry>>,
    pub next_stream: AtomicU64,
    pub launcher: WorkerLauncher,
    expired_handled: AtomicBool,
    reconcile_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct NodeExecutor {
    inner: Arc<ExecutorInner>,
}

/// Operations the host can actually perform. Form A needs a root daemon on
/// Linux or macOS to switch to the profile's OS identity; otherwise only the
/// control operations remain and the rest return Capability.
fn host_capabilities() -> BTreeSet<ExecutorOp> {
    let caps: BTreeSet<ExecutorOp> = [
        ExecutorOp::OpenStream,
        ExecutorOp::Describe,
        ExecutorOp::Query,
        ExecutorOp::Cancel,
    ]
    .into_iter()
    .collect();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if unsafe { libc::geteuid() } == 0 {
        let mut caps = caps;
        caps.extend([
            ExecutorOp::Exec,
            ExecutorOp::Read,
            ExecutorOp::Write,
            ExecutorOp::Edit,
        ]);
        return caps;
    }
    caps
}

impl NodeExecutor {
    /// Open the ledger (single writer) and start the lease watchdog and GC.
    /// Must be called inside a tokio runtime.
    pub fn open(host: ExecutorHost) -> Result<Self, StoreOpenError> {
        let store = Arc::new(Store::open(&host.store_dir, &host.node_did)?);
        let server_epoch = format!("ep-{}", uuid::Uuid::new_v4().simple());
        let ops = Arc::new(OpsManager::new(
            store.clone(),
            host.worker.clone(),
            host.shell.clone(),
            server_epoch.clone(),
        ));
        let inner = Arc::new(ExecutorInner {
            node_id: host.node_id,
            node_did: host.node_did,
            shell: host.shell,
            verifier: host.verifier,
            rbac: host.rbac,
            store,
            ops,
            auth: AuthState::default(),
            host_caps: host_capabilities(),
            server_epoch,
            streams: Mutex::new(HashMap::new()),
            next_stream: AtomicU64::new(1),
            launcher: host.worker,
            expired_handled: AtomicBool::new(true),
            reconcile_lock: tokio::sync::Mutex::new(()),
        });
        let watchdog = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(250));
            loop {
                tick.tick().await;
                let Some(inner) = watchdog.upgrade() else { break };
                inner.check_expiry().await;
            }
        });
        let gc = Arc::downgrade(&inner);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let Some(inner) = gc.upgrade() else { break };
                let retention = inner
                    .auth
                    .current()
                    .map(|c| c.limits.result_retention_secs)
                    .unwrap_or_else(|| ExecutorLimits::default().result_retention_secs);
                let store = inner.store.clone();
                let _ = tokio::task::spawn_blocking(move || store.gc(now_ms(), retention)).await;
            }
        });
        Ok(Self { inner })
    }

    pub fn store_id(&self) -> String {
        self.inner.store.store_id().to_string()
    }

    pub fn server_epoch(&self) -> String {
        self.inner.server_epoch.clone()
    }

    pub fn host_capabilities(&self) -> BTreeSet<ExecutorOp> {
        self.inner.host_caps.clone()
    }

    /// Deliver the latest `nodes/<node>/config.executor` (None when absent).
    pub async fn apply_config(&self, config: Option<ExecutorConfig>) -> ApplyResult {
        let result = match config {
            None => {
                if self.inner.auth.clear() {
                    ApplyResult::Replaced
                } else {
                    ApplyResult::Ignored
                }
            }
            Some(config) => self.inner.auth.apply(config, now_ms()),
        };
        if result == ApplyResult::Replaced {
            self.inner.reconcile().await;
        }
        result
    }

    /// Device ACL the gateway should enforce now: the snapshot's revision and
    /// allowed devices, or None when there is no valid snapshot.
    pub fn gateway_acl(&self) -> Option<(String, Vec<String>)> {
        self.inner
            .auth
            .current()
            .map(|c| (c.revision, c.allowed_devices))
    }

    pub fn is_open(&self) -> bool {
        self.inner.auth.current().is_some()
    }

    /// Serve one Command Stream (a gateway-forwarded TCP connection or an
    /// in-process stream for form C).
    pub async fn serve<S>(&self, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        stream::serve_stream(self.inner.clone(), stream).await
    }

    /// Accept loop on a loopback address; only the local gateway (or a local
    /// process, which still has to log in) can reach it.
    pub async fn listen(&self, addr: SocketAddr) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let gate = Arc::new(tokio::sync::Semaphore::new(64));
        loop {
            let (socket, _) = listener.accept().await?;
            let Ok(permit) = gate.clone().try_acquire_owned() else {
                drop(socket);
                continue;
            };
            let me = self.clone();
            tokio::spawn(async move {
                let _ = socket.set_nodelay(true);
                me.serve(socket).await;
                drop(permit);
            });
        }
    }
}

impl ExecutorInner {
    pub(crate) fn close_stream(&self, id: u64, reason: ExecutorCloseReason, message: &str) {
        if let Some(entry) = self.streams.lock().unwrap().get(&id) {
            let _ = entry.close.send(Some(ExecutorClosing {
                reason,
                message: message.to_string(),
            }));
        }
    }

    async fn revoke_all(&self, reason: ExecutorCloseReason, cancel: CancelReason, message: &str) {
        let ids: Vec<u64> = self.streams.lock().unwrap().keys().copied().collect();
        for id in ids {
            self.close_stream(id, reason, message);
        }
        self.ops.cancel_where(cancel, |_| true).await;
    }

    async fn check_expiry(&self) {
        if self.auth.has_snapshot() && self.auth.current().is_none() {
            if !self.expired_handled.swap(true, Ordering::SeqCst) {
                let _lock = self.reconcile_lock.lock().await;
                self.revoke_all(
                    ExecutorCloseReason::AuthExpired,
                    CancelReason::AuthExpired,
                    "authorization snapshot expired",
                )
                .await;
            }
        } else if self.auth.current().is_some() {
            self.expired_handled.store(false, Ordering::SeqCst);
        }
    }

    /// Apply a replaced snapshot: block and close what it no longer grants,
    /// then cancel active operations outside it.
    async fn reconcile(&self) {
        let _lock = self.reconcile_lock.lock().await;
        let Some(config) = self.auth.current() else {
            self.revoke_all(
                ExecutorCloseReason::Disabled,
                CancelReason::Revoked,
                "executor is disabled or has no valid authorization",
            )
            .await;
            return;
        };
        self.expired_handled.store(false, Ordering::SeqCst);
        let streams: Vec<(u64, VerifiedSession, Grant)> = self
            .streams
            .lock()
            .unwrap()
            .iter()
            .map(|(id, e)| (*id, e.session.clone(), e.grant.clone()))
            .collect();
        for (id, session, old) in streams {
            let new = match auth::map_login(&session, &config, &old.profile_id, &self.host_caps) {
                Ok(mut g) => {
                    auth::apply_rbac(self.rbac.as_ref(), &self.node_id, &session, &mut g).await;
                    Some(g)
                }
                Err(_) => None,
            };
            match new {
                Some(g)
                    if g.same_target(&old)
                        && g.caps.contains(&ExecutorOp::OpenStream)
                        && g.caps.is_superset(&old.caps) =>
                {
                    if let Some(entry) = self.streams.lock().unwrap().get_mut(&id) {
                        entry.grant = g;
                    }
                }
                _ => self.close_stream(id, ExecutorCloseReason::Revoked, "authorization changed"),
            }
        }
        let host_caps = self.host_caps.clone();
        self.ops
            .cancel_where(CancelReason::Revoked, |op| {
                let agent = &op.identity.actor.agent_did;
                let Some(profile) = config.profiles.get(&op.identity.profile_id) else {
                    return true;
                };
                let still_bound = config
                    .principal_bindings
                    .get(agent)
                    .map(|b| b.app_instance_id.to_string() == op.identity.actor.app_instance_id)
                    .unwrap_or(false);
                let granted = config.granted_ops(&op.identity.profile_id, agent);
                !(still_bound
                    && profile.run_as == op.profile.run_as
                    && profile.workdir_roots == op.profile.workdir_roots
                    && profile.env == op.profile.env
                    && granted.contains(&op.kind.op())
                    && host_caps.contains(&op.kind.op()))
            })
            .await;
    }
}
