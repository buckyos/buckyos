//! OpenDAN Dispatch Runner Adapter — the receive boundary for external
//! structured delegation (`agent.delegate/v1`) via the Task Dispatch Center.
//!
//! TaskMgr 2.0 push model: the dispatcher creates the single public Task up
//! front, then actively calls this runner's `offer_task` (validate + reserve
//! capacity, NO side effects) and `activate_task` (start execution exactly
//! once per delivery). There is no claim loop and no second target-side
//! task; the executor drives the dispatcher-created task directly.
//!
//! Idempotency: the `delivery_id -> reservation/activation` journal lives in
//! the agent's own persistent store (a fsync'd JSON file under the agent
//! root, mutated under a process-wide lock), so offer/activate replays after
//! a crash return the original decision and never start a second execution.
//!
//! Deployment note: the runner endpoint is a loopback kRPC server the
//! adapter starts itself and reports through `attach_instance`. That is
//! reachable in the single-OOD deployment (dispatcher and OpenDAN on one
//! node); a multi-node split needs a routable app endpoint instead.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, ActivateTaskReq, ActivateTaskResp, AgentDelegateDispatchInput,
    AttachInstanceReq, DetachInstanceReq, DispatchRejectReason, OfferTaskReq, OfferTaskResp,
    RenewInstanceReq, RunnerFunctionDescriptor, TargetRegistration, TaskRunnerHandler,
    TaskRunnerServerHandler, AGENT_DELEGATE_OPERATION_V1,
};
use buckyos_http_server::{HttpServer, Runner};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::agent::AIAgent;

const BINDINGS_FILE: &str = "dispatch_bindings.json";
/// In-flight reservation capacity reported to the dispatcher. A single
/// AgentRuntime instance is the normal deployment; 4 matches the design
/// doc's pilot registration.
const DISPATCH_CAPACITY: u32 = 4;
const REGISTER_RETRY_SECS: u64 = 30;
const RUNNER_KAPI_PATH: &str = "/kapi/opendan-task-runner";

// ---------------------------------------------------------------------------
// persistent delivery journal
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryEntry {
    task_id: String,
    reservation_token: String,
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
/// and every write goes through write-tmp + rename.
pub struct DispatchBindingStore {
    path: PathBuf,
    state: Mutex<BindingFile>,
}

impl DispatchBindingStore {
    pub fn open(agent_root: &std::path::Path) -> Result<Self> {
        let path = agent_root.join(BINDINGS_FILE);
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<BindingFile>(&bytes).unwrap_or_else(|err| {
                warn!(
                    "opendan.dispatch_adapter: bindings file {} unreadable ({}); starting empty",
                    path.display(),
                    err
                );
                BindingFile::default()
            }),
            Err(_) => BindingFile::default(),
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

    async fn open_reservations(&self) -> usize {
        let state = self.state.lock().await;
        state
            .deliveries
            .values()
            .filter(|entry| !entry.activated)
            .count()
    }

    async fn put(&self, delivery_id: &str, entry: DeliveryEntry) -> Result<()> {
        let mut state = self.state.lock().await;
        state.version = 2;
        state.deliveries.insert(delivery_id.to_string(), entry);
        let bytes = serde_json::to_vec_pretty(&*state)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)
            .with_context(|| format!("write dispatch bindings tmp {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("commit dispatch bindings {}", self.path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// runner handler (offer/activate push protocol)
// ---------------------------------------------------------------------------

struct AgentRunnerHandler {
    agent: Arc<AIAgent>,
    bindings: Arc<DispatchBindingStore>,
    target_id: String,
    instance_id: String,
    /// Serializes reservation decisions so capacity checks and journal
    /// writes never interleave.
    accept_lock: Mutex<()>,
}

impl AgentRunnerHandler {
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
        _ctx: kRPC::RPCContext,
    ) -> kRPC::Result<OfferTaskResp> {
        let _guard = self.accept_lock.lock().await;
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
        if self.bindings.open_reservations().await >= DISPATCH_CAPACITY as usize {
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
                    updated_at_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await
            .map_err(|err| kRPC::RPCErrors::ReasonError(format!("persist reservation: {err}")))?;
        info!(
            "opendan.dispatch_adapter[{}]: reserved delivery {} for task {}",
            self.agent.agent_name, req.delivery_id, req.task_id
        );
        Ok(OfferTaskResp::OfferAccepted {
            app_instance_id: self.instance_id.clone(),
            reservation_token,
        })
    }

    async fn handle_activate_task(
        &self,
        req: ActivateTaskReq,
        _ctx: kRPC::RPCContext,
    ) -> kRPC::Result<ActivateTaskResp> {
        let _guard = self.accept_lock.lock().await;
        let Some(mut entry) = self.bindings.get(&req.delivery_id).await else {
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
        // Exactly-once activation per (task_id, runner_epoch, delivery_id):
        // replays are acknowledged without a second execution start.
        if !entry.activated {
            entry.activated = true;
            entry.updated_at_ms = chrono::Utc::now().timestamp_millis();
            self.bindings
                .put(&req.delivery_id, entry)
                .await
                .map_err(|err| {
                    kRPC::RPCErrors::ReasonError(format!("persist activation: {err}"))
                })?;
            let agent = self.agent.clone();
            let task_id = req.task_id.clone();
            let agent_name = agent.agent_name.clone();
            tokio::spawn(async move {
                if let Err(err) = agent.process_accepted_dispatch_task(&task_id).await {
                    warn!(
                        "opendan.dispatch_adapter[{}]: start execution for task {} failed: {err:#}",
                        agent_name, task_id
                    );
                }
            });
            info!(
                "opendan.dispatch_adapter[{}]: activated delivery {} (task {}, epoch {})",
                self.agent.agent_name, req.delivery_id, req.task_id, req.runner_epoch
            );
        }
        Ok(ActivateTaskResp { activated: true })
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
                error!(
                    "opendan.dispatch_adapter[{}]: open binding store failed: {err:#}",
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
            auth_policy: buckyos_api::DispatchAuthPolicy::ZoneUsers,
            // agent.delegate/v1 carries no manual gate: delegation to an
            // agent is the normal entry path, not a privileged operation.
            approval_policy: buckyos_api::DispatchApprovalPolicy::Never,
            delivery_policy: buckyos_api::DeliveryPolicy::default(),
            max_concurrency: DISPATCH_CAPACITY,
            enabled: true,
            registration_revision: 0,
        };

        // Loopback runner endpoint the dispatcher pushes offers to.
        let handler = Arc::new(AgentRunnerHandler {
            agent: self.clone(),
            bindings,
            target_id: target_id.clone(),
            instance_id: instance_id.clone(),
            accept_lock: Mutex::new(()),
        });
        let port = match std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
        {
            Ok(port) => port,
            Err(err) => {
                error!(
                    "opendan.dispatch_adapter[{}]: allocate runner port failed: {err}",
                    self.agent_name
                );
                return;
            }
        };
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
        tokio::spawn(async move {
            if let Err(err) = http_runner.run().await {
                error!("opendan.dispatch_adapter: runner http server exited: {err:?}");
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
                        "opendan.dispatch_adapter[{}]: registered dispatch runner {}",
                        self.agent_name, target_id
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
        // stale lease re-attach (which fences the previous epoch).
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
