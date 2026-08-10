//! OpenDAN Dispatch Runner Adapter — the receive boundary for external
//! structured delegation (`agent.delegate/v1`) via the Task Dispatch Center.
//!
//! TaskMgr 2.0 push model: the dispatcher creates the single public Task up
//! front, then actively calls this runner's `offer_task` (validate + reserve
//! capacity, NO side effects) and `activate_task` (start execution exactly
//! once per delivery). There is no claim loop and no second target-side
//! task; the executor drives the dispatcher-created task directly.
//!
//! Activation contract: `activate_task` ACKs only after the executor start
//! path has completed and the journal entry is durably marked. A failed
//! start returns an RPC error without consuming the delivery, so the
//! dispatcher keeps the record in `Activating` and replays the same
//! delivery id — the ACK therefore proves execution actually started.
//!
//! Idempotency: the `delivery_id -> reservation/activation` journal lives in
//! the agent's own persistent store (an fsync'd JSON file under the agent
//! root, mutated under a process-wide lock), so offer/activate replays after
//! a crash return the original decision and never start a second execution.
//! An unreadable journal refuses to serve (fail-loud) instead of silently
//! restarting empty, which would break the replay contract.
//!
//! Trust model: offers reach this runner over the zone-internal push
//! channel. The dispatcher hands the instance a per-lease `delivery_token`
//! in the attach/renew responses; every offer/activate call must present
//! it. Combined with the dispatcher-stamped auth envelope (identity fields
//! written from the *verified* caller token, never the payload), this is
//! the v1 target-side evidence chain: the runner trusts the envelope
//! because only the real dispatcher can present the token it minted for
//! this lease.
//!
//! Deployment: the runner kapi is served on the agent app's declared
//! service port (`$OPENDAN_SERVICE_PORT`, default 4060) bound to
//! `0.0.0.0`, which node_daemon publishes with `-p {port}:{port}` for
//! containerized agents. The endpoint reported through `attach_instance`
//! is `http://127.0.0.1:{port}` from the *dispatcher's* (host) point of
//! view — reachable both for the native same-host deployment and for the
//! containerized one via the published port. v1 explicitly assumes the
//! dispatcher and the agent share one node; a multi-node split must route
//! through cyfs-gateway and resolve the endpoint from service discovery
//! instead of trusting instance self-reports.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, ActivateTaskReq, ActivateTaskResp, AgentDelegateDispatchInput,
    AttachInstanceReq, DetachInstanceReq, DispatchRejectReason, OfferTaskReq, OfferTaskResp,
    RenewInstanceReq, RunnerFunctionDescriptor, TargetRegistration, TaskRunnerHandler,
    TaskRunnerServerHandler, AGENT_DELEGATE_OPERATION_V1, OPENDAN_SERVICE_PORT,
};
use buckyos_http_server::{HttpServer, Runner};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::agent::AIAgent;

const BINDINGS_FILE: &str = "dispatch_bindings.json";
/// In-flight reservation capacity reported to the dispatcher. A single
/// AgentRuntime instance is the normal deployment; 4 matches the design
/// doc's pilot registration.
const DISPATCH_CAPACITY: u32 = 4;
const REGISTER_RETRY_SECS: u64 = 30;
const RUNNER_KAPI_PATH: &str = "/kapi/opendan-task-runner";
/// node_daemon injects the published agent service port into the container
/// env under this (legacy) name; the native deployment may leave it unset.
const SERVICE_PORT_ENV: &str = "OPENDAN_SERVICE_PORT";
/// Journal retention for activated (terminal for this adapter) deliveries.
const ACTIVATED_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
/// A reservation the dispatcher never activated is considered abandoned
/// after this long and stops counting against capacity. Must comfortably
/// exceed the dispatcher's max activate backoff (300s).
const STALE_RESERVATION_MS: i64 = 15 * 60 * 1000;

// ---------------------------------------------------------------------------
// persistent delivery journal
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryEntry {
    task_id: String,
    reservation_token: String,
    /// True only after the executor start path completed successfully once.
    #[serde(default)]
    activated: bool,
    updated_at_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BindingFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    deliveries: HashMap<String, DeliveryEntry>,
}

/// File-backed `delivery_id -> reservation/activation` journal. All
/// mutations run under one async lock (offer handling is low-frequency),
/// and every write goes through write-tmp + fsync + rename + dir-fsync so
/// a post-crash replay always sees the last acknowledged decision.
#[derive(Debug)]
pub struct DispatchBindingStore {
    path: PathBuf,
    state: Mutex<BindingFile>,
}

impl DispatchBindingStore {
    /// Open the journal. A missing file starts empty (first run); an
    /// unreadable or unparsable file is a hard error — starting empty would
    /// silently void the idempotency contract, so the caller must refuse to
    /// register the runner instead.
    pub fn open(agent_root: &std::path::Path) -> Result<Self> {
        let path = agent_root.join(BINDINGS_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<BindingFile>(&bytes).with_context(|| {
                format!(
                    "dispatch bindings journal {} is corrupt; refusing to serve deliveries \
                     from an empty state (move the file aside to explicitly reset)",
                    path.display()
                )
            })?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => BindingFile::default(),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("read dispatch bindings journal {}", path.display())
                })
            }
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    async fn get(&self, delivery_id: &str) -> Option<DeliveryEntry> {
        let state = self.state.lock().await;
        state.deliveries.get(delivery_id).cloned()
    }

    /// Reservations that still count against capacity: not yet activated
    /// and not stale-abandoned.
    async fn open_reservations(&self, now_ms: i64) -> usize {
        let state = self.state.lock().await;
        state
            .deliveries
            .values()
            .filter(|entry| !entry.activated && now_ms - entry.updated_at_ms < STALE_RESERVATION_MS)
            .count()
    }

    async fn put(&self, delivery_id: &str, entry: DeliveryEntry) -> Result<()> {
        let mut state = self.state.lock().await;
        let now_ms = entry.updated_at_ms;
        Self::prune_locked(&mut state, now_ms);
        state.version = 2;
        state.deliveries.insert(delivery_id.to_string(), entry);
        Self::persist_locked(&state, &self.path)
    }

    /// Drop entries the dispatcher can no longer act on: activated ones
    /// past the retention window (its redelivery budget is minutes, not
    /// hours) and reservations abandoned without an activate.
    fn prune_locked(state: &mut BindingFile, now_ms: i64) {
        state.deliveries.retain(|_, entry| {
            let age = now_ms - entry.updated_at_ms;
            if entry.activated {
                age < ACTIVATED_RETENTION_MS
            } else {
                age < STALE_RESERVATION_MS
            }
        });
    }

    fn persist_locked(state: &BindingFile, path: &std::path::Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(state)?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = std::fs::File::create(&tmp)
                .with_context(|| format!("create dispatch bindings tmp {}", tmp.display()))?;
            file.write_all(&bytes)
                .with_context(|| format!("write dispatch bindings tmp {}", tmp.display()))?;
            file.sync_all()
                .with_context(|| format!("fsync dispatch bindings tmp {}", tmp.display()))?;
        }
        std::fs::rename(&tmp, path)
            .with_context(|| format!("commit dispatch bindings {}", path.display()))?;
        // The rename itself must survive a crash: fsync the parent dir.
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)
                .and_then(|dir| dir.sync_all())
                .with_context(|| format!("fsync dispatch bindings dir {}", parent.display()))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// runner handler (offer/activate push protocol)
// ---------------------------------------------------------------------------

/// The executor entry the handler drives on activate. Split out from
/// `AIAgent` so the offer/activate protocol is unit-testable.
#[async_trait]
pub trait DispatchTaskStarter: Send + Sync {
    /// Idempotently start (or resume) execution of a dispatcher-bound task.
    /// Returning `Ok` means the task has durably left the handoff state:
    /// a session drives it, or it was terminally failed with the reason
    /// written back. Errors are retryable — the dispatcher replays the
    /// delivery.
    async fn start_dispatch_task(&self, task_id: &str) -> Result<()>;
}

/// Production starter: drives the agent's idempotent executor entry.
struct AgentStarter(Arc<AIAgent>);

#[async_trait]
impl DispatchTaskStarter for AgentStarter {
    async fn start_dispatch_task(&self, task_id: &str) -> Result<()> {
        self.0.clone().process_accepted_dispatch_task(task_id).await
    }
}

struct AgentRunnerHandler {
    starter: Arc<dyn DispatchTaskStarter>,
    bindings: Arc<DispatchBindingStore>,
    agent_name: String,
    target_id: String,
    instance_id: String,
    /// Per-lease token minted by the dispatcher (attach/renew responses).
    /// Every push call must present it; `None` (not attached yet) rejects.
    delivery_token: RwLock<Option<String>>,
    /// Serializes reservation decisions and activation admission so
    /// capacity checks, journal writes and the in-flight set never
    /// interleave. Deliberately NOT held across the executor start await.
    accept_state: Mutex<AcceptState>,
}

#[derive(Default)]
struct AcceptState {
    /// Deliveries whose synchronous start is currently executing. A replay
    /// arriving meanwhile is told to retry instead of double-starting.
    activating: HashSet<String>,
}

impl AgentRunnerHandler {
    async fn set_delivery_token(&self, token: String) {
        let mut guard = self.delivery_token.write().await;
        *guard = Some(token);
    }

    /// Push-channel authentication: the caller must present the exact
    /// delivery token of the current lease. After a dispatcher restart its
    /// freshly derived token mismatches until our next renew picks the new
    /// one up (≤ a third of the lease) — those calls fail closed and the
    /// dispatcher's backoff absorbs the gap.
    async fn check_delivery_token(&self, ctx: &kRPC::RPCContext) -> kRPC::Result<()> {
        let presented = ctx
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let guard = self.delivery_token.read().await;
        match (guard.as_deref(), presented) {
            (Some(expected), Some(got)) if expected == got => Ok(()),
            (None, _) => Err(kRPC::RPCErrors::NoPermission(
                "runner instance is not attached yet".to_string(),
            )),
            _ => Err(kRPC::RPCErrors::NoPermission(
                "invalid delivery token".to_string(),
            )),
        }
    }

    /// The runner's own business validation — the dispatcher's ACL already
    /// ran, but the target-side authorization is never skipped.
    fn validate_offer(&self, req: &OfferTaskReq) -> Option<OfferTaskResp> {
        if req.schema_id != AGENT_DELEGATE_OPERATION_V1 {
            return Some(OfferTaskResp::Rejected {
                stable_reason: DispatchRejectReason::UnsupportedOperation,
                detail: Some(format!("schema {} not supported", req.schema_id)),
            });
        }
        if req.target_id != self.target_id {
            return Some(OfferTaskResp::Rejected {
                stable_reason: DispatchRejectReason::PreconditionFailed,
                detail: Some(format!(
                    "offer targets {}, this instance serves {}",
                    req.target_id, self.target_id
                )),
            });
        }
        if req.auth.on_behalf_of.trim().is_empty() {
            return Some(OfferTaskResp::Rejected {
                stable_reason: DispatchRejectReason::AuthDenied,
                detail: Some("auth envelope carries no business user".to_string()),
            });
        }
        let input: std::result::Result<AgentDelegateDispatchInput, _> =
            serde_json::from_value(req.input.clone());
        match input {
            Ok(input) if input.purpose.trim().is_empty() => Some(OfferTaskResp::Rejected {
                stable_reason: DispatchRejectReason::InvalidInput,
                detail: Some("purpose must not be empty".to_string()),
            }),
            Ok(_) => None,
            Err(err) => Some(OfferTaskResp::Rejected {
                stable_reason: DispatchRejectReason::SchemaMismatch,
                detail: Some(format!("invalid agent.delegate/v1 input: {err}")),
            }),
        }
    }
}

#[async_trait]
impl TaskRunnerHandler for AgentRunnerHandler {
    async fn handle_offer_task(
        &self,
        req: OfferTaskReq,
        ctx: kRPC::RPCContext,
    ) -> kRPC::Result<OfferTaskResp> {
        self.check_delivery_token(&ctx).await?;
        let _guard = self.accept_state.lock().await;
        // Idempotent replay returns the original decision without a second
        // reservation.
        if let Some(entry) = self.bindings.get(&req.delivery_id).await {
            return Ok(OfferTaskResp::OfferAccepted {
                app_instance_id: self.instance_id.clone(),
                reservation_token: entry.reservation_token,
            });
        }
        if let Some(reject) = self.validate_offer(&req) {
            return Ok(reject);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        if self.bindings.open_reservations(now_ms).await >= DISPATCH_CAPACITY as usize {
            return Ok(OfferTaskResp::Busy {
                retry_after_ms: Some(30_000),
            });
        }
        // Reserve only — business side effects wait for activate.
        let reservation_token = format!("res-{}", uuid::Uuid::new_v4().simple());
        self.bindings
            .put(
                &req.delivery_id,
                DeliveryEntry {
                    task_id: req.task_id.clone(),
                    reservation_token: reservation_token.clone(),
                    activated: false,
                    updated_at_ms: now_ms,
                },
            )
            .await
            .map_err(|err| kRPC::RPCErrors::ReasonError(format!("persist reservation: {err}")))?;
        info!(
            "opendan.dispatch_adapter[{}]: reserved delivery {} for task {}",
            self.agent_name, req.delivery_id, req.task_id
        );
        Ok(OfferTaskResp::OfferAccepted {
            app_instance_id: self.instance_id.clone(),
            reservation_token,
        })
    }

    async fn handle_activate_task(
        &self,
        req: ActivateTaskReq,
        ctx: kRPC::RPCContext,
    ) -> kRPC::Result<ActivateTaskResp> {
        self.check_delivery_token(&ctx).await?;
        // Admission under the lock: journal check + in-flight registration.
        {
            let mut state = self.accept_state.lock().await;
            let Some(entry) = self.bindings.get(&req.delivery_id).await else {
                return Err(kRPC::RPCErrors::ReasonError(format!(
                    "unknown delivery {}",
                    req.delivery_id
                )));
            };
            if entry.reservation_token != req.reservation_token || entry.task_id != req.task_id {
                return Err(kRPC::RPCErrors::ReasonError(format!(
                    "reservation mismatch for delivery {}",
                    req.delivery_id
                )));
            }
            // Exactly-once per (task_id, runner_epoch, delivery_id): a
            // completed activation replay is acknowledged without a second
            // start.
            if entry.activated {
                return Ok(ActivateTaskResp { activated: true });
            }
            if !state.activating.insert(req.delivery_id.clone()) {
                // A concurrent replay is mid-start; let the dispatcher's
                // retry find the settled outcome.
                return Err(kRPC::RPCErrors::ReasonError(format!(
                    "activation of delivery {} is in progress",
                    req.delivery_id
                )));
            }
        }

        // Synchronous start OUTSIDE the lock: the ACK must prove execution
        // began, and a failure must leave the delivery consumable again.
        let start_result = self.starter.start_dispatch_task(&req.task_id).await;

        let mut state = self.accept_state.lock().await;
        state.activating.remove(&req.delivery_id);
        drop(state);
        match start_result {
            Ok(()) => {
                let entry = DeliveryEntry {
                    task_id: req.task_id.clone(),
                    reservation_token: req.reservation_token.clone(),
                    activated: true,
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                };
                self.bindings
                    .put(&req.delivery_id, entry)
                    .await
                    .map_err(|err| {
                        kRPC::RPCErrors::ReasonError(format!("persist activation: {err}"))
                    })?;
                info!(
                    "opendan.dispatch_adapter[{}]: activated delivery {} (task {}, epoch {})",
                    self.agent_name, req.delivery_id, req.task_id, req.runner_epoch
                );
                Ok(ActivateTaskResp { activated: true })
            }
            Err(err) => {
                warn!(
                    "opendan.dispatch_adapter[{}]: start execution for task {} failed \
                     (delivery {} stays replayable): {err:#}",
                    self.agent_name, req.task_id, req.delivery_id
                );
                Err(kRPC::RPCErrors::ReasonError(format!(
                    "start execution failed: {err}"
                )))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// agent wiring
// ---------------------------------------------------------------------------

impl AIAgent {
    /// Stable Target id: the agent DID when configured, the agent name
    /// otherwise. Must match `request.target_agent_id` stamped on accepted
    /// tasks.
    pub fn dispatch_target_id(&self) -> String {
        let did = self.config.toml.identity.agent_did.trim();
        if did.is_empty() {
            self.agent_name.clone()
        } else {
            did.to_string()
        }
    }

    /// App id under which this agent runs its own tasks (runner scan key).
    pub fn own_task_app_id(&self) -> String {
        match get_buckyos_api_runtime() {
            Ok(runtime) => runtime.get_app_id(),
            Err(_) => self.agent_id(),
        }
    }

    /// The port the runner kapi serves on: the agent app's declared service
    /// port. node_daemon publishes exactly this port for containerized
    /// agents, so a random port would be unreachable from the host.
    fn runner_service_port(&self) -> u16 {
        std::env::var(SERVICE_PORT_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<u16>().ok())
            .filter(|port| *port != 0)
            .unwrap_or(OPENDAN_SERVICE_PORT)
    }

    /// Register this agent as a dispatch runner and keep the instance lease
    /// alive (register → serve offer/activate → attach → renew → detach)
    /// until shutdown. `None` when the executor is disabled or the local
    /// stores are absent — IM/session paths never depend on it.
    pub fn spawn_dispatch_target(self: Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.toml.runtime.task_executor.enabled {
            return None;
        }
        let bindings = match DispatchBindingStore::open(self.config.layout.root.as_path()) {
            Ok(store) => Arc::new(store),
            Err(err) => {
                // Fail-loud: an unreadable journal means replayed deliveries
                // could double-execute. Stay unregistered so the dispatcher
                // routes nothing here until an operator intervenes.
                error!(
                    "opendan.dispatch_adapter[{}]: open binding store failed; NOT registering \
                     as a dispatch runner: {err:#}",
                    self.agent_name
                );
                return None;
            }
        };
        Some(tokio::spawn(async move {
            self.run_dispatch_target(bindings).await;
        }))
    }

    async fn run_dispatch_target(self: Arc<Self>, bindings: Arc<DispatchBindingStore>) {
        let target_id = self.dispatch_target_id();
        let instance_id = format!("{}-inst", self.agent_name);
        let registration = TargetRegistration {
            target_id: target_id.clone(),
            // Owner identity is stamped server-side from our verified token.
            owner_user_id: String::new(),
            owner_app_id: String::new(),
            functions: vec![RunnerFunctionDescriptor::new(AGENT_DELEGATE_OPERATION_V1)],
            // Any zone user may delegate to an agent; the delegation itself
            // is the product's normal entry path, and the dispatcher stamps
            // the verified caller into the auth envelope we re-check at
            // offer time. Tightening to an approval gate was evaluated and
            // deliberately rejected for agent.delegate/v1 (it would put a
            // manual release in front of every ordinary agent request).
            auth_policy: buckyos_api::DispatchAuthPolicy::ZoneUsers,
            approval_policy: buckyos_api::DispatchApprovalPolicy::Never,
            delivery_policy: buckyos_api::DeliveryPolicy::default(),
            max_concurrency: DISPATCH_CAPACITY,
            enabled: true,
            registration_revision: 0,
        };

        let handler = Arc::new(AgentRunnerHandler {
            starter: Arc::new(AgentStarter(self.clone())),
            bindings,
            agent_name: self.agent_name.clone(),
            target_id: target_id.clone(),
            instance_id: instance_id.clone(),
            delivery_token: RwLock::new(None),
            accept_state: Mutex::new(AcceptState::default()),
        });

        // Serve the runner kapi on the declared service port, bound to
        // 0.0.0.0 (Runner::new binds UNSPECIFIED) so the container DNAT
        // path works. This is the process's single public HTTP surface —
        // future web faces mount alongside on the same Runner.
        let port = self.runner_service_port();
        let endpoint = format!("http://127.0.0.1:{port}{RUNNER_KAPI_PATH}");
        let runner_server = OpendanTaskRunnerServer {
            rpc_handler: TaskRunnerServerHandler::new(AgentRunnerHandlerRef {
                inner: handler.clone(),
            }),
        };
        let http_runner = Runner::new(port);
        if let Err(err) =
            http_runner.add_http_server(RUNNER_KAPI_PATH.to_string(), Arc::new(runner_server))
        {
            error!(
                "opendan.dispatch_adapter[{}]: mount runner server failed: {err:?}",
                self.agent_name
            );
            return;
        }
        let agent_name_for_server = self.agent_name.clone();
        tokio::spawn(async move {
            // Bind failure (port taken by another process) is fatal for the
            // dispatch path: without the server no offer can ever land.
            if let Err(err) = http_runner.run().await {
                error!(
                    "opendan.dispatch_adapter[{}]: runner http server on port {} exited: {err:?} \
                     (dispatch deliveries cannot reach this agent)",
                    agent_name_for_server, port
                );
            }
        });

        // Registration must land before the instance lease; retry quietly —
        // the dispatcher may still be coming up.
        loop {
            let result = match self.runtime.task_dispatcher_client().await {
                Ok(dispatcher) => dispatcher.register_target(registration.clone()).await,
                Err(err) => Err(err),
            };
            match result {
                Ok(_) => {
                    info!(
                        "opendan.dispatch_adapter[{}]: registered dispatch runner {} (endpoint {})",
                        self.agent_name, target_id, endpoint
                    );
                    break;
                }
                Err(err) => {
                    warn!(
                        "opendan.dispatch_adapter[{}]: register target {} failed: {err}; retrying in {}s",
                        self.agent_name, target_id, REGISTER_RETRY_SECS
                    );
                }
            }
            tokio::select! {
                _ = self.pump_shutdown.notified() => return,
                _ = tokio::time::sleep(std::time::Duration::from_secs(REGISTER_RETRY_SECS)) => {}
            }
        }

        // Instance lease loop: attach, renew at a third of the lease, on a
        // stale lease re-attach (which fences the previous epoch). Attach
        // and renew responses carry the delivery token the dispatcher will
        // present on pushes; the handler rejects pushes without it.
        let mut lease_epoch: Option<u64> = None;
        let mut lease_expires_at: u64 = 0;
        loop {
            let dispatcher = match self.runtime.task_dispatcher_client().await {
                Ok(dispatcher) => dispatcher,
                Err(err) => {
                    warn!(
                        "opendan.dispatch_adapter[{}]: dispatcher unavailable: {err}",
                        self.agent_name
                    );
                    tokio::select! {
                        _ = self.pump_shutdown.notified() => return,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(REGISTER_RETRY_SECS)) => continue,
                    }
                }
            };
            match lease_epoch {
                None => {
                    match dispatcher
                        .attach_instance(AttachInstanceReq {
                            target_id: target_id.clone(),
                            instance_id: instance_id.clone(),
                            endpoint: endpoint.clone(),
                            capacity: DISPATCH_CAPACITY,
                            available_capacity: None,
                            lease_ms: None,
                        })
                        .await
                    {
                        Ok(attached) => {
                            info!(
                                "opendan.dispatch_adapter[{}]: attached instance {} (lease epoch {})",
                                self.agent_name, instance_id, attached.lease_epoch
                            );
                            lease_epoch = Some(attached.lease_epoch);
                            lease_expires_at = attached.lease_expires_at;
                            if attached.delivery_token.is_empty() {
                                warn!(
                                    "opendan.dispatch_adapter[{}]: dispatcher returned no delivery token; \
                                     pushes will be rejected until it does",
                                    self.agent_name
                                );
                            } else {
                                handler.set_delivery_token(attached.delivery_token).await;
                            }
                        }
                        Err(err) => {
                            warn!(
                                "opendan.dispatch_adapter[{}]: attach instance failed: {err}",
                                self.agent_name
                            );
                        }
                    }
                }
                Some(epoch) => {
                    match dispatcher
                        .renew_instance(RenewInstanceReq {
                            target_id: target_id.clone(),
                            instance_id: instance_id.clone(),
                            lease_epoch: epoch,
                            available_capacity: None,
                            lease_ms: None,
                        })
                        .await
                    {
                        Ok(renewed) => {
                            lease_expires_at = renewed.lease_expires_at;
                            // Re-adopt the token every renew: it heals the
                            // mismatch window after a dispatcher restart.
                            if !renewed.delivery_token.is_empty() {
                                handler.set_delivery_token(renewed.delivery_token).await;
                            }
                        }
                        Err(err) => {
                            warn!(
                                "opendan.dispatch_adapter[{}]: renew lease failed ({err}); re-attaching",
                                self.agent_name
                            );
                            lease_epoch = None;
                        }
                    }
                }
            }

            let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
            let wait_ms = if lease_epoch.is_some() && lease_expires_at > now_ms {
                ((lease_expires_at - now_ms) / 3).clamp(5_000, 60_000)
            } else {
                REGISTER_RETRY_SECS * 1000
            };
            tokio::select! {
                _ = self.pump_shutdown.notified() => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(wait_ms)) => {}
            }
        }

        // Best-effort detach on shutdown.
        if let Some(epoch) = lease_epoch {
            if let Ok(dispatcher) = self.runtime.task_dispatcher_client().await {
                let _ = dispatcher
                    .detach_instance(DetachInstanceReq {
                        target_id: target_id.clone(),
                        instance_id: instance_id.clone(),
                        lease_epoch: epoch,
                    })
                    .await;
            }
        }
    }
}

/// Arc-wrapper so the RPC dispatch layer can own the shared handler.
struct AgentRunnerHandlerRef {
    inner: Arc<AgentRunnerHandler>,
}

#[async_trait]
impl TaskRunnerHandler for AgentRunnerHandlerRef {
    async fn handle_offer_task(
        &self,
        req: OfferTaskReq,
        ctx: kRPC::RPCContext,
    ) -> kRPC::Result<OfferTaskResp> {
        self.inner.handle_offer_task(req, ctx).await
    }

    async fn handle_activate_task(
        &self,
        req: ActivateTaskReq,
        ctx: kRPC::RPCContext,
    ) -> kRPC::Result<ActivateTaskResp> {
        self.inner.handle_activate_task(req, ctx).await
    }
}

struct OpendanTaskRunnerServer {
    rpc_handler: TaskRunnerServerHandler<AgentRunnerHandlerRef>,
}

#[async_trait]
impl HttpServer for OpendanTaskRunnerServer {
    async fn serve_request(
        &self,
        req: http::Request<
            http_body_util::combinators::BoxBody<bytes::Bytes, buckyos_http_server::ServerError>,
        >,
        info: buckyos_http_server::StreamInfo,
    ) -> buckyos_http_server::ServerResult<
        http::Response<
            http_body_util::combinators::BoxBody<bytes::Bytes, buckyos_http_server::ServerError>,
        >,
    > {
        if *req.method() == http::Method::POST {
            return buckyos_http_server::serve_http_by_rpc_handler(req, info, &self.rpc_handler)
                .await;
        }
        Err(buckyos_http_server::server_err!(
            buckyos_http_server::ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "opendan-task-runner".to_string()
    }

    fn http_version(&self) -> http::Version {
        http::Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use buckyos_api::DispatchAuthEnvelope;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TOKEN: &str = "tok-test";

    struct MockStarter {
        starts: AtomicUsize,
        fail_first: AtomicUsize,
        delay_ms: u64,
    }

    impl MockStarter {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                starts: AtomicUsize::new(0),
                fail_first: AtomicUsize::new(0),
                delay_ms: 0,
            })
        }

        fn failing(times: usize) -> Arc<Self> {
            Arc::new(Self {
                starts: AtomicUsize::new(0),
                fail_first: AtomicUsize::new(times),
                delay_ms: 0,
            })
        }

        fn slow(delay_ms: u64) -> Arc<Self> {
            Arc::new(Self {
                starts: AtomicUsize::new(0),
                fail_first: AtomicUsize::new(0),
                delay_ms,
            })
        }
    }

    #[async_trait]
    impl DispatchTaskStarter for MockStarter {
        async fn start_dispatch_task(&self, _task_id: &str) -> Result<()> {
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            self.starts.fetch_add(1, Ordering::SeqCst);
            let remaining = self.fail_first.load(Ordering::SeqCst);
            if remaining > 0 {
                self.fail_first.store(remaining - 1, Ordering::SeqCst);
                return Err(anyhow!("simulated start failure"));
            }
            Ok(())
        }
    }

    fn handler_with(
        dir: &std::path::Path,
        starter: Arc<dyn DispatchTaskStarter>,
    ) -> Arc<AgentRunnerHandler> {
        let bindings = Arc::new(DispatchBindingStore::open(dir).expect("open store"));
        let handler = Arc::new(AgentRunnerHandler {
            starter,
            bindings,
            agent_name: "test-agent".to_string(),
            target_id: "did:agent:test".to_string(),
            instance_id: "test-agent-inst".to_string(),
            delivery_token: RwLock::new(Some(TOKEN.to_string())),
            accept_state: Mutex::new(AcceptState::default()),
        });
        handler
    }

    fn ctx_with_token(token: Option<&str>) -> kRPC::RPCContext {
        let mut ctx = kRPC::RPCContext::default();
        ctx.token = token.map(str::to_string);
        ctx
    }

    fn offer_req(delivery_id: &str, task_id: &str) -> OfferTaskReq {
        OfferTaskReq {
            delivery_id: delivery_id.to_string(),
            task_id: task_id.to_string(),
            schema_id: AGENT_DELEGATE_OPERATION_V1.to_string(),
            schema_version: 1,
            target_id: "did:agent:test".to_string(),
            input: json!({"purpose": "do the thing"}),
            input_digest: "digest".to_string(),
            auth: DispatchAuthEnvelope {
                requested_by_user: "alice".to_string(),
                requested_by_app: "control-panel".to_string(),
                on_behalf_of: "alice".to_string(),
                zone_trusted_caller: false,
                workflow_ref: None,
                input_digest: "digest".to_string(),
                created_at: 1,
                expires_at: None,
            },
            lease_epoch: 1,
            deadline_at: i64::MAX as u64,
        }
    }

    async fn accept_offer(handler: &AgentRunnerHandler, delivery_id: &str, task_id: &str) -> String {
        match handler
            .handle_offer_task(offer_req(delivery_id, task_id), ctx_with_token(Some(TOKEN)))
            .await
            .expect("offer")
        {
            OfferTaskResp::OfferAccepted {
                reservation_token, ..
            } => reservation_token,
            other => panic!("expected OfferAccepted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn offer_is_idempotent_per_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let handler = handler_with(dir.path(), MockStarter::new());
        let token1 = accept_offer(&handler, "d-1", "t-1").await;
        let token2 = accept_offer(&handler, "d-1", "t-1").await;
        assert_eq!(token1, token2);
    }

    #[tokio::test]
    async fn offer_rejects_wrong_or_missing_delivery_token() {
        let dir = tempfile::tempdir().unwrap();
        let handler = handler_with(dir.path(), MockStarter::new());
        for token in [None, Some("wrong")] {
            let err = handler
                .handle_offer_task(offer_req("d-1", "t-1"), ctx_with_token(token))
                .await
                .expect_err("must reject");
            assert!(matches!(err, kRPC::RPCErrors::NoPermission(_)), "{err:?}");
        }
        // Not attached yet (no token stored) rejects even a real-looking one.
        {
            let mut guard = handler.delivery_token.write().await;
            *guard = None;
        }
        let err = handler
            .handle_offer_task(offer_req("d-1", "t-1"), ctx_with_token(Some(TOKEN)))
            .await
            .expect_err("must reject before attach");
        assert!(matches!(err, kRPC::RPCErrors::NoPermission(_)), "{err:?}");
    }

    #[tokio::test]
    async fn offer_honors_capacity_with_busy() {
        let dir = tempfile::tempdir().unwrap();
        let handler = handler_with(dir.path(), MockStarter::new());
        for i in 0..DISPATCH_CAPACITY {
            accept_offer(&handler, &format!("d-{i}"), &format!("t-{i}")).await;
        }
        let resp = handler
            .handle_offer_task(offer_req("d-over", "t-over"), ctx_with_token(Some(TOKEN)))
            .await
            .expect("offer");
        assert!(matches!(resp, OfferTaskResp::Busy { .. }), "{resp:?}");
    }

    #[tokio::test]
    async fn activate_acks_only_after_successful_start() {
        let dir = tempfile::tempdir().unwrap();
        let starter = MockStarter::failing(1);
        let handler = handler_with(dir.path(), starter.clone());
        let reservation = accept_offer(&handler, "d-1", "t-1").await;
        let req = ActivateTaskReq {
            delivery_id: "d-1".to_string(),
            task_id: "t-1".to_string(),
            runner_epoch: 1,
            reservation_token: reservation,
        };
        // First start fails → no ACK, delivery stays replayable.
        let err = handler
            .handle_activate_task(req.clone(), ctx_with_token(Some(TOKEN)))
            .await
            .expect_err("failed start must not ACK");
        assert!(err.to_string().contains("start execution failed"), "{err}");
        // Replay succeeds and starts execution a second time.
        let resp = handler
            .handle_activate_task(req.clone(), ctx_with_token(Some(TOKEN)))
            .await
            .expect("replay activate");
        assert!(resp.activated);
        assert_eq!(starter.starts.load(Ordering::SeqCst), 2);
        // Further replays ACK without another start (exactly-once).
        let resp = handler
            .handle_activate_task(req, ctx_with_token(Some(TOKEN)))
            .await
            .expect("idempotent activate");
        assert!(resp.activated);
        assert_eq!(starter.starts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn activate_rejects_unknown_or_mismatched_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let handler = handler_with(dir.path(), MockStarter::new());
        let reservation = accept_offer(&handler, "d-1", "t-1").await;
        let unknown = handler
            .handle_activate_task(
                ActivateTaskReq {
                    delivery_id: "d-unknown".to_string(),
                    task_id: "t-1".to_string(),
                    runner_epoch: 1,
                    reservation_token: reservation.clone(),
                },
                ctx_with_token(Some(TOKEN)),
            )
            .await
            .expect_err("unknown delivery");
        assert!(unknown.to_string().contains("unknown delivery"));
        let mismatch = handler
            .handle_activate_task(
                ActivateTaskReq {
                    delivery_id: "d-1".to_string(),
                    task_id: "t-1".to_string(),
                    runner_epoch: 1,
                    reservation_token: "res-bogus".to_string(),
                },
                ctx_with_token(Some(TOKEN)),
            )
            .await
            .expect_err("mismatched reservation");
        assert!(mismatch.to_string().contains("reservation mismatch"));
    }

    #[tokio::test]
    async fn concurrent_activate_replay_does_not_double_start() {
        let dir = tempfile::tempdir().unwrap();
        let starter = MockStarter::slow(200);
        let handler = handler_with(dir.path(), starter.clone());
        let reservation = accept_offer(&handler, "d-1", "t-1").await;
        let req = ActivateTaskReq {
            delivery_id: "d-1".to_string(),
            task_id: "t-1".to_string(),
            runner_epoch: 1,
            reservation_token: reservation,
        };
        let first = {
            let handler = handler.clone();
            let req = req.clone();
            tokio::spawn(async move {
                handler
                    .handle_activate_task(req, ctx_with_token(Some(TOKEN)))
                    .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let racing = handler
            .handle_activate_task(req, ctx_with_token(Some(TOKEN)))
            .await
            .expect_err("in-progress replay must be told to retry");
        assert!(racing.to_string().contains("in progress"), "{racing}");
        let first = first.await.expect("join").expect("first activate");
        assert!(first.activated);
        assert_eq!(starter.starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn journal_survives_reopen_and_replays_decisions() {
        let dir = tempfile::tempdir().unwrap();
        let starter = MockStarter::new();
        let reservation = {
            let handler = handler_with(dir.path(), starter.clone());
            let reservation = accept_offer(&handler, "d-1", "t-1").await;
            let resp = handler
                .handle_activate_task(
                    ActivateTaskReq {
                        delivery_id: "d-1".to_string(),
                        task_id: "t-1".to_string(),
                        runner_epoch: 1,
                        reservation_token: reservation.clone(),
                    },
                    ctx_with_token(Some(TOKEN)),
                )
                .await
                .expect("activate");
            assert!(resp.activated);
            reservation
        };
        // "Crash" + reopen: the activated decision must replay without a
        // second execution start.
        let handler = handler_with(dir.path(), starter.clone());
        let replayed = accept_offer(&handler, "d-1", "t-1").await;
        assert_eq!(replayed, reservation);
        let resp = handler
            .handle_activate_task(
                ActivateTaskReq {
                    delivery_id: "d-1".to_string(),
                    task_id: "t-1".to_string(),
                    runner_epoch: 1,
                    reservation_token: reservation,
                },
                ctx_with_token(Some(TOKEN)),
            )
            .await
            .expect("replayed activate");
        assert!(resp.activated);
        assert_eq!(starter.starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn corrupt_journal_fails_loud() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(BINDINGS_FILE), b"{ not json").unwrap();
        let err = DispatchBindingStore::open(dir.path()).expect_err("corrupt journal must error");
        assert!(err.to_string().contains("corrupt"), "{err:#}");
    }

    #[tokio::test]
    async fn journal_prunes_terminal_and_stale_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = DispatchBindingStore::open(dir.path()).expect("open");
        let now = chrono::Utc::now().timestamp_millis();
        store
            .put(
                "d-old-activated",
                DeliveryEntry {
                    task_id: "t-1".to_string(),
                    reservation_token: "r-1".to_string(),
                    activated: true,
                    updated_at_ms: now - ACTIVATED_RETENTION_MS - 1,
                },
            )
            .await
            .unwrap();
        store
            .put(
                "d-stale-reservation",
                DeliveryEntry {
                    task_id: "t-2".to_string(),
                    reservation_token: "r-2".to_string(),
                    activated: false,
                    updated_at_ms: now - STALE_RESERVATION_MS - 1,
                },
            )
            .await
            .unwrap();
        // Stale reservations stop counting against capacity immediately.
        assert_eq!(store.open_reservations(now).await, 0);
        // The next put physically prunes both.
        store
            .put(
                "d-fresh",
                DeliveryEntry {
                    task_id: "t-3".to_string(),
                    reservation_token: "r-3".to_string(),
                    activated: false,
                    updated_at_ms: now,
                },
            )
            .await
            .unwrap();
        assert!(store.get("d-old-activated").await.is_none());
        assert!(store.get("d-stale-reservation").await.is_none());
        assert!(store.get("d-fresh").await.is_some());
        assert_eq!(store.open_reservations(now).await, 1);
    }
}
