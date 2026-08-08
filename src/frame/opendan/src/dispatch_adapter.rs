//! OpenDAN Dispatch Target Adapter — the receive boundary for external
//! structured delegation (`agent.delegate/v1`) via the Task Dispatch Center.
//!
//! Design: `doc/opendan/Agent Task Executor.md` §5/§6/§8.1 and
//! `doc/task_mgr/task_dispatch_center.md` §9.
//!
//! The adapter only handles the handoff boundary: register the agent as a
//! Target, attach a TargetInstance, validate offers, idempotently create the
//! OpenDAN-owned `agent.delegate` task, accept. Execution stays with the
//! Agent Task Executor / WorkSession machinery; recovery of accepted tasks
//! stays owner-only.
//!
//! Idempotency contract (`IdempotentAccept`): the `dispatch_id -> task_id`
//! binding lives in the agent's own persistent store (a fsync'd JSON file
//! under the agent root, mutated under a process-wide lock). The order is
//! crash-safe:
//!   1. persist `dispatch_id -> pending` (intent)
//!   2. create the TaskMgr task (its `data.request.dispatch_id` names us)
//!   3. persist `dispatch_id -> task_id`
//! A crash between 2 and 3 is healed on redelivery by scanning our own
//! tasks for `request.dispatch_id` before creating anything new.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, AgentDelegateDispatchInput, AgentDelegateProgress,
    AgentDelegateTaskData, AgentDelegateTaskRequest, DispatchOfferDecision, DispatchOfferHandler,
    DispatchRecord, DispatchRejectReason, OperationDescriptor, TargetInstanceConfig,
    TargetRegistration, TaskDispatcherClient, TaskFilter, TaskManagerClient,
    AGENT_DELEGATE_OPERATION_V1,
};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::agent::AIAgent;
use crate::agent_task_executor::TASK_TYPE_AGENT_DELEGATE;

const BINDINGS_FILE: &str = "dispatch_bindings.json";
/// In-flight offer capacity reported to the dispatcher. A single
/// AgentRuntime instance is the normal deployment; 4 matches the design
/// doc's pilot registration.
const DISPATCH_CAPACITY: u32 = 4;
const REGISTER_RETRY_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// persistent dispatch_id -> task_id bindings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BindingEntry {
    /// `None` = intent persisted, task creation possibly in flight
    /// (crash window); healed via owner-task scan on redelivery.
    task_id: Option<i64>,
    updated_at_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BindingFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    bindings: HashMap<String, BindingEntry>,
}

/// File-backed `dispatch_id -> task_id` store. All mutations run under one
/// async lock (offer handling is low-frequency), and every write goes
/// through write-tmp + rename.
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

    async fn get(&self, dispatch_id: &str) -> Option<Option<i64>> {
        let state = self.state.lock().await;
        state
            .bindings
            .get(dispatch_id)
            .map(|entry| entry.task_id)
    }

    async fn put(&self, dispatch_id: &str, task_id: Option<i64>) -> Result<()> {
        let mut state = self.state.lock().await;
        state.version = 1;
        state.bindings.insert(
            dispatch_id.to_string(),
            BindingEntry {
                task_id,
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
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
// offer handler
// ---------------------------------------------------------------------------

struct AgentDispatchHandler {
    agent: Arc<AIAgent>,
    bindings: Arc<DispatchBindingStore>,
    target_id: String,
    /// Serializes offer handling: the binding check + task creation must not
    /// interleave for the same (or different) dispatch ids.
    accept_lock: Mutex<()>,
}

impl AgentDispatchHandler {
    fn task_mgr(&self) -> Result<Arc<TaskManagerClient>> {
        self.agent
            .runtime
            .task_mgr
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("task manager unavailable"))
    }

    /// Own-task scan for the crash window between "task created" and
    /// "binding persisted". Owner-only (our app id), never a cross-owner
    /// inbox: it just recognizes tasks we created for this dispatch before.
    async fn find_own_task_for_dispatch(&self, dispatch_id: &str) -> Result<Option<i64>> {
        let task_mgr = self.task_mgr()?;
        let app_id = self.agent.own_task_app_id();
        let tasks = task_mgr
            .list_tasks(Some(TaskFilter {
                app_id: Some(app_id),
                task_type: Some(TASK_TYPE_AGENT_DELEGATE.to_string()),
                ..Default::default()
            }))
            .await
            .map_err(|err| anyhow!("list own delegate tasks: {err}"))?;
        Ok(tasks
            .into_iter()
            .find(|task| {
                task.data
                    .get("request")
                    .and_then(|request| request.get("dispatch_id"))
                    .and_then(serde_json::Value::as_str)
                    == Some(dispatch_id)
            })
            .map(|task| task.id))
    }

    /// Validate + idempotently create the OpenDAN-owned task for one offer.
    async fn accept_offer(&self, record: &DispatchRecord) -> Result<DispatchOfferDecision> {
        // 1. Operation / envelope validation. The dispatcher's ACL already
        //    ran; this is the target's own business authorization and it is
        //    never skipped.
        if record.operation != AGENT_DELEGATE_OPERATION_V1 {
            return Ok(DispatchOfferDecision::Reject {
                reason: DispatchRejectReason::UnsupportedOperation,
                detail: Some(format!("operation {} not supported", record.operation)),
            });
        }
        if record.target_id != self.target_id {
            return Ok(DispatchOfferDecision::Reject {
                reason: DispatchRejectReason::PreconditionFailed,
                detail: Some(format!(
                    "offer targets {}, this instance serves {}",
                    record.target_id, self.target_id
                )),
            });
        }
        let business_user = record.auth.on_behalf_of.trim().to_string();
        if business_user.is_empty() {
            return Ok(DispatchOfferDecision::Reject {
                reason: DispatchRejectReason::AuthDenied,
                detail: Some("auth envelope carries no business user".to_string()),
            });
        }
        let input: AgentDelegateDispatchInput = match serde_json::from_value(record.input.clone())
        {
            Ok(input) => input,
            Err(err) => {
                return Ok(DispatchOfferDecision::Reject {
                    reason: DispatchRejectReason::SchemaMismatch,
                    detail: Some(format!("invalid agent.delegate/v1 input: {err}")),
                });
            }
        };
        if input.purpose.trim().is_empty() {
            return Ok(DispatchOfferDecision::Reject {
                reason: DispatchRejectReason::InvalidInput,
                detail: Some("purpose must not be empty".to_string()),
            });
        }

        // 2. Idempotent accept: same dispatch_id must always yield the same
        //    task id, at most one task.
        let _guard = self.accept_lock.lock().await;
        if let Some(Some(task_id)) = self.bindings.get(record.dispatch_id.as_str()).await {
            return Ok(DispatchOfferDecision::Accept {
                target_task_id: task_id,
            });
        }
        // Pending intent (or first sight): heal the create-then-crash window
        // from our own tasks before creating anything.
        if let Some(task_id) = self
            .find_own_task_for_dispatch(record.dispatch_id.as_str())
            .await?
        {
            self.bindings
                .put(record.dispatch_id.as_str(), Some(task_id))
                .await?;
            return Ok(DispatchOfferDecision::Accept {
                target_task_id: task_id,
            });
        }

        // 3. Create the OpenDAN-owned task. Owner = verified business user +
        //    our own app id; the requester never becomes the task owner.
        self.bindings.put(record.dispatch_id.as_str(), None).await?;
        let task_mgr = self.task_mgr()?;
        let task_name = input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("agent.delegate")
            .to_string();
        let data = AgentDelegateTaskData {
            request: AgentDelegateTaskRequest {
                version: 1,
                source: Some("task-dispatcher".to_string()),
                dispatch_id: Some(record.dispatch_id.clone()),
                target_agent_id: Some(self.target_id.clone()),
                title: input.title.clone(),
                purpose: Some(input.purpose.clone()),
                requester_agent_id: None,
                owner_session_id: input
                    .owner_session_ref
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                input: input.input.clone(),
                context_refs: input.context_refs.clone(),
                workspace_hints: input.workspace_hints.clone(),
                constraints: input.constraints.clone(),
                reason_messages: Vec::new(),
                trigger: None,
            },
            progress: Some(AgentDelegateProgress {
                execution: Some(serde_json::json!({ "status": "accepted" })),
                one_line_status: Some("Accepted by OpenDAN".to_string()),
                updated_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            }),
            result: None,
            route: None,
            blocker: None,
            human_input: None,
            error: None,
        };
        let task = task_mgr
            .create_task(
                task_name.as_str(),
                TASK_TYPE_AGENT_DELEGATE,
                Some(serde_json::to_value(&data)?),
                business_user.as_str(),
                self.agent.own_task_app_id().as_str(),
                None,
            )
            .await
            .map_err(|err| anyhow!("create delegate task for {}: {err}", record.dispatch_id))?;
        self.bindings
            .put(record.dispatch_id.as_str(), Some(task.id))
            .await?;
        info!(
            "opendan.dispatch_adapter[{}]: dispatch {} -> task {} (user {})",
            self.agent.agent_name, record.dispatch_id, task.id, business_user
        );
        Ok(DispatchOfferDecision::Accept {
            target_task_id: task.id,
        })
    }
}

#[async_trait]
impl DispatchOfferHandler for AgentDispatchHandler {
    async fn handle_offer(&self, record: &DispatchRecord) -> DispatchOfferDecision {
        match self.accept_offer(record).await {
            Ok(decision) => decision,
            Err(err) => {
                // Transient local failure (task-mgr unreachable, disk):
                // leave the offer to lease-expiry redelivery.
                warn!(
                    "opendan.dispatch_adapter[{}]: offer {} deferred: {err:#}",
                    self.agent.agent_name, record.dispatch_id
                );
                DispatchOfferDecision::Defer
            }
        }
    }

    async fn on_accepted(&self, record: &DispatchRecord, target_task_id: i64) {
        // Execution starts only after accept succeeded (receive contract
        // §6.3.4): hand the owned task to the executor path.
        if let Err(err) = self
            .agent
            .clone()
            .process_accepted_dispatch_task(target_task_id)
            .await
        {
            warn!(
                "opendan.dispatch_adapter[{}]: start execution for task {} (dispatch {}) failed: {err:#}",
                self.agent.agent_name, target_task_id, record.dispatch_id
            );
        }
    }

    async fn on_dispatch_gone(&self, record: &DispatchRecord, target_task_id: i64) {
        // The dispatch was canceled/expired while our accept was in flight:
        // the local task must not run.
        warn!(
            "opendan.dispatch_adapter[{}]: dispatch {} gone, canceling local task {}",
            self.agent.agent_name, record.dispatch_id, target_task_id
        );
        match self.task_mgr() {
            Ok(task_mgr) => {
                if let Err(err) = task_mgr.cancel_task(target_task_id, false).await {
                    error!(
                        "opendan.dispatch_adapter[{}]: cancel local task {} failed: {err}",
                        self.agent.agent_name, target_task_id
                    );
                }
            }
            Err(err) => error!(
                "opendan.dispatch_adapter[{}]: cancel local task {} impossible: {err}",
                self.agent.agent_name, target_task_id
            ),
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

    /// App id under which this agent records its own tasks (owner scan key).
    pub fn own_task_app_id(&self) -> String {
        match get_buckyos_api_runtime() {
            Ok(runtime) => runtime.get_app_id(),
            Err(_) => self.agent_id(),
        }
    }

    /// Register this agent as a Dispatch Target and run the TargetInstance
    /// loop (attach → kevent → claim/accept → renew → detach) until
    /// shutdown. `None` when the executor is disabled or the dispatcher /
    /// task-mgr handles are absent — IM/session paths never depend on it.
    pub fn spawn_dispatch_target(self: Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.toml.runtime.task_executor.enabled {
            return None;
        }
        let Some(dispatcher) = self.runtime.task_dispatcher.as_ref().cloned() else {
            info!(
                "opendan.dispatch_adapter[{}]: task-dispatcher not configured; external delegation disabled",
                self.agent_name
            );
            return None;
        };
        if self.runtime.task_mgr.is_none() {
            warn!(
                "opendan.dispatch_adapter[{}]: task-mgr unavailable; not registering dispatch target",
                self.agent_name
            );
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
            self.run_dispatch_target(dispatcher, bindings).await;
        }))
    }

    async fn run_dispatch_target(
        self: Arc<Self>,
        dispatcher: Arc<TaskDispatcherClient>,
        bindings: Arc<DispatchBindingStore>,
    ) {
        let target_id = self.dispatch_target_id();
        let registration = TargetRegistration {
            target_id: target_id.clone(),
            // Owner identity is stamped server-side from our verified token.
            owner_user_id: String::new(),
            owner_app_id: String::new(),
            operations: vec![OperationDescriptor {
                operation: AGENT_DELEGATE_OPERATION_V1.to_string(),
                input_schema_ref: None,
            }],
            auth_policy: buckyos_api::DispatchAuthPolicy::ZoneUsers,
            // agent.delegate/v1 carries no manual gate: delegation to an
            // agent is the normal entry path, not a privileged operation.
            approval_policy: buckyos_api::DispatchApprovalPolicy::Never,
            idempotency_contract: buckyos_api::IdempotencyContract::IdempotentAccept,
            delivery_policy: buckyos_api::DeliveryPolicy::default(),
            max_concurrency: DISPATCH_CAPACITY,
            enabled: true,
        };

        // Registration must land before the instance loop; retry quietly —
        // the dispatcher may still be coming up.
        loop {
            match dispatcher.register_target(registration.clone()).await {
                Ok(_) => {
                    info!(
                        "opendan.dispatch_adapter[{}]: registered dispatch target {}",
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

        let handler: Arc<dyn DispatchOfferHandler> = Arc::new(AgentDispatchHandler {
            agent: self.clone(),
            bindings,
            target_id: target_id.clone(),
            accept_lock: Mutex::new(()),
        });
        let kevent = self
            .runtime
            .kevent_client
            .as_ref()
            .map(|client| client.as_ref().clone());
        let result = buckyos_api::run_target_instance(
            dispatcher,
            TargetInstanceConfig::new(target_id.clone(), DISPATCH_CAPACITY),
            kevent,
            self.pump_shutdown.clone(),
            handler,
        )
        .await;
        if let Err(err) = result {
            error!(
                "opendan.dispatch_adapter[{}]: target instance loop for {} exited: {err}",
                self.agent_name, target_id
            );
        }
    }
}
