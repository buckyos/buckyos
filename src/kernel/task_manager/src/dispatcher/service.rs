//! Task Dispatch Center 2.0 service (doc §11).
//!
//! A deterministic state machine over durable state: dispatch records are
//! accepted with a frozen plan, the single public Task is created up front
//! through the Task Core trusted control plane, and delivery advances by
//! write-ahead DeliveryAttempts + push RPCs (`offer_task` / `activate_task`)
//! to registered runner instances. Crash recovery replays from the persisted
//! record/attempt state; KEvent is acceleration only.

use super::dispatch_db::{DispatchDb, RecordStateUpdate};
use crate::server::{RequestContext, SessionTokenVerifier, TaskManagerService};
use crate::task_store::now_ms;
use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::*;
use name_lib::DID;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

const EVALUATE_TICK: Duration = Duration::from_secs(2);
/// Cancel/expiry sweep cadence, in evaluation ticks.
const SWEEP_EVERY_TICKS: u64 = 8;
const MAX_BATCH_PER_TICK: u32 = 32;
const DEFAULT_INSTANCE_LEASE_MS: u64 = 60_000;
const MAX_INSTANCE_LEASE_MS: u64 = 10 * 60_000;

fn dispatch_err(code: &str, detail: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(format!("{}: {}", code, detail))
}

/// Transport used for the dispatcher's push RPCs to runner endpoints.
/// Injected so tests can run the full protocol without HTTP.
/// `auth_token` is the per-lease delivery token minted for the instance —
/// NEVER the dispatcher's own kernel session token, which must not leave
/// this process (a runner-supplied endpoint is not a trusted sink).
#[async_trait]
pub trait RunnerCaller: Send + Sync {
    async fn call(
        &self,
        endpoint: &str,
        method: &str,
        params: Value,
        timeout_ms: u64,
        auth_token: Option<String>,
    ) -> Result<Value>;
}

/// Runner endpoints are instance self-reports, so constrain where the
/// dispatcher will ever connect: plain http(s) to the local host only.
/// This matches the v1 same-node deployment (native process or container
/// published via `-p port:port`); relaxing it for multi-node zones must
/// come with endpoints resolved from service discovery, not self-reports.
pub fn validate_runner_endpoint(endpoint: &str) -> Result<()> {
    let parsed = url::Url::parse(endpoint).map_err(|err| {
        RPCErrors::ParseRequestError(format!("invalid runner endpoint {}: {}", endpoint, err))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(RPCErrors::ParseRequestError(format!(
            "runner endpoint {} must be http(s)",
            endpoint
        )));
    }
    let loopback = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if !loopback {
        return Err(RPCErrors::ParseRequestError(format!(
            "runner endpoint {} must resolve to the local host (v1 same-node dispatch)",
            endpoint
        )));
    }
    Ok(())
}

/// Production caller: a fresh kRPC client per call, authenticated with the
/// per-lease delivery token.
pub struct KrpcRunnerCaller;

#[async_trait]
impl RunnerCaller for KrpcRunnerCaller {
    async fn call(
        &self,
        endpoint: &str,
        method: &str,
        params: Value,
        timeout_ms: u64,
        auth_token: Option<String>,
    ) -> Result<Value> {
        // Defense in depth: attach already validates, but endpoints persisted
        // by older builds or other writers must not widen the egress.
        validate_runner_endpoint(endpoint)?;
        let client = kRPC::new_with_timeout_secs(endpoint, auth_token, (timeout_ms / 1000).max(1));
        client.call(method, params).await
    }
}

/// The dispatcher's own actor identity for Task Core writes and audits.
fn dispatcher_actor() -> ActorRef {
    ActorRef::from_auth_target(
        TASK_DISPATCHER_SERVICE_NAME,
        &AuthTarget::system(SystemServiceId::parse(TASK_DISPATCHER_SERVICE_NAME).unwrap()),
    )
}

#[derive(Clone)]
pub struct TaskDispatcherService {
    db: Arc<DispatchDb>,
    task_core: TaskManagerService,
    token_verifier: Arc<dyn SessionTokenVerifier>,
    runner_caller: Arc<dyn RunnerCaller>,
    kevent_client: Option<KEventClient>,
    wakeup: Arc<Notify>,
    /// Process-local secret the per-lease delivery tokens are derived from.
    /// Deliberately NOT persisted: a dispatcher restart rotates every token,
    /// and runners re-adopt the fresh one from their next renew response
    /// (offer/activate calls in the gap fail closed and back off).
    delivery_secret: Arc<String>,
}

impl TaskDispatcherService {
    pub fn new(
        db: Arc<DispatchDb>,
        task_core: TaskManagerService,
        token_verifier: Arc<dyn SessionTokenVerifier>,
        runner_caller: Arc<dyn RunnerCaller>,
        kevent_client: Option<KEventClient>,
    ) -> Self {
        TaskDispatcherService {
            db,
            task_core,
            token_verifier,
            runner_caller,
            kevent_client,
            wakeup: Arc::new(Notify::new()),
            delivery_secret: Arc::new(format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn db(&self) -> Arc<DispatchDb> {
        self.db.clone()
    }

    /// The runner surface (register/attach/renew/detach) authenticates App
    /// identities, not the zone-trusted kernel grade. Ownership is the full
    /// AppInstance identity `(owner_user_id, app_id)`, so installations of
    /// the same App product for different users cannot mutate each other's
    /// targets. Zone-trusted kernel identities retain an admin override.
    async fn require_target_owner(
        &self,
        request_ctx: &RequestContext,
        target_id: &str,
        what: &str,
    ) -> Result<TargetRegistration> {
        let registration = self
            .db
            .get_registration(target_id)
            .await?
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_TARGET_NOT_REGISTERED, target_id))?;
        if !request_ctx.zone_trusted
            && (registration.owner_user_id != request_ctx.user_id
                || registration.owner_app_id != request_ctx.app_id)
        {
            return Err(RPCErrors::NoPermission(format!(
                "{} must come from registered owner {}@{}",
                what, registration.owner_app_id, registration.owner_user_id
            )));
        }
        Ok(registration)
    }

    /// A stable Agent DID is implemented by the AppInstance named in its
    /// AgentServiceBinding. This check lets the currently bound runtime
    /// recover a registration left by an obsolete AppId without allowing an
    /// unrelated App to take it over.
    async fn request_matches_agent_binding(
        &self,
        request_ctx: &RequestContext,
        target_id: &str,
    ) -> Result<bool> {
        let Some(raw_app_instance_id) = request_ctx.app_instance_id.as_deref() else {
            return Ok(false);
        };
        let app_instance_id = match raw_app_instance_id.parse::<AppInstanceId>() {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let agent_did = match DID::from_str(target_id) {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let agent_id = match AgentId::from_agent_did(&agent_did) {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let key = agent_spec_key(&request_ctx.user_id, &agent_id);
        let value = client.get(&key).await.map_err(|error| {
            RPCErrors::ReasonError(format!("load AgentSpec {key} failed: {error}"))
        })?;
        let spec: AgentSpec = serde_json::from_str(&value.value).map_err(|error| {
            RPCErrors::ReasonError(format!("decode AgentSpec {key} failed: {error}"))
        })?;
        spec.validate()
            .map_err(|error| RPCErrors::ReasonError(format!("invalid AgentSpec {key}: {error}")))?;
        Ok(spec.agent_did == agent_did
            && spec.agent_id == agent_id
            && spec.binding.references_runtime(&app_instance_id))
    }

    /// Deterministic per-lease push credential. Scoped to
    /// `(target, instance, lease_epoch)` so a leaked token neither works
    /// for another instance nor survives a re-attach.
    fn delivery_token_for(&self, target_id: &str, instance_id: &str, lease_epoch: u64) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.delivery_secret.as_bytes());
        hasher.update(b"\n");
        hasher.update(target_id.as_bytes());
        hasher.update(b"\n");
        hasher.update(instance_id.as_bytes());
        hasher.update(b"\n");
        hasher.update(lease_epoch.to_le_bytes());
        format!("dlv-{}", hex::encode(hasher.finalize()))
    }

    async fn authenticate(&self, ctx: &RPCContext) -> Result<RequestContext> {
        let token = ctx
            .token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RPCErrors::NoPermission("task-dispatcher requires a session token".to_string())
            })?;
        let verified = self.token_verifier.verify(token).await?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session)?;
        let user_id = verified
            .sub
            .clone()
            .ok_or_else(|| RPCErrors::InvalidToken("session token has no subject".to_string()))?;
        let app_id = claims.target.canonical_key();
        let executor_app_id = claims.target.appid_claim().to_string();
        let authorization_id = claims.target.authorization_key();
        let app_instance_id = match &claims.target {
            AuthTarget::App { app_instance_id } => Some(app_instance_id.to_string()),
            AuthTarget::System { .. } => None,
        };
        if user_id.trim().is_empty() || app_id.trim().is_empty() {
            return Err(RPCErrors::InvalidToken(
                "session token has an empty subject or app id".to_string(),
            ));
        }
        let zone_trusted = crate::server::token_is_zone_trusted(&verified);
        Ok(RequestContext {
            user_id,
            app_id,
            executor_app_id,
            app_instance_id,
            authorization_id,
            zone_trusted,
            sudo: verified.sudo,
        })
    }

    fn require_zone_trusted(request_ctx: &RequestContext, what: &str) -> Result<()> {
        if !request_ctx.zone_trusted {
            return Err(RPCErrors::NoPermission(format!(
                "{} requires a zone-trusted service identity",
                what
            )));
        }
        Ok(())
    }

    /// Approval-release admin grade: zone-trusted services or elevated
    /// (sudo) interactive sessions.
    fn can_decide_approval(request_ctx: &RequestContext) -> bool {
        request_ctx.zone_trusted || request_ctx.sudo
    }

    fn notify_evaluate(&self) {
        self.wakeup.notify_one();
    }

    async fn publish_record_event(&self, record: &DispatchRecord) {
        let Some(client) = self.kevent_client.as_ref() else {
            return;
        };
        let payload = json!({
            "dispatch_id": record.dispatch_id,
            "task_id": record.task_id,
            "status": record.status,
            "target_id": record.target_id,
            "updated_at": record.updated_at,
        });
        let path = task_dispatcher_record_event_id(&record.dispatch_id);
        if let Err(err) = client.pub_event(&path, payload).await {
            debug!("dispatcher event publish failed: {} {}", path, err);
        }
        if record.status == DispatchStatus::PendingApproval {
            let _ = client
                .pub_event(
                    &task_dispatcher_approvals_event_id(),
                    json!({"dispatch_id": record.dispatch_id}),
                )
                .await;
        }
    }

    // -----------------------------------------------------------------
    // Saga step 1: accept + freeze + create the public Task
    // -----------------------------------------------------------------

    /// Complete the CreatingTask leg: idempotently create the promised task,
    /// backfill task_id, then move to PendingApproval or Queued.
    async fn complete_creating_task(&self, record: &DispatchRecord) -> Result<DispatchRecord> {
        let task = self
            .task_core
            .trusted_create_promised_task(
                CreatePromisedTaskReq {
                    name: record
                        .message
                        .clone()
                        .filter(|m| !m.trim().is_empty())
                        .unwrap_or_else(|| record.schema_id.clone()),
                    schema_id: record.schema_id.clone(),
                    schema_version: Some(record.schema_version),
                    input: record.input.clone(),
                    creator: ActorRef {
                        user_id: record.auth.requested_by_user.clone(),
                        app_id: record.auth.requested_by_app.clone(),
                        app_instance_id: None,
                    },
                    expected_input_digest: Some(record.auth.input_digest.clone()),
                    origin_ref: Some(TaskOriginRef {
                        kind: TASK_DISPATCHER_SERVICE_NAME.to_string(),
                        id: record.dispatch_id.clone(),
                    }),
                    parent_id: record.auth.parent_task_id.clone(),
                    child_control_policy: None,
                    policy_preset: None,
                    permission_boundary: false,
                    storage_domain: record.auth.storage_domain,
                    // Deterministically derived from the dispatch id so
                    // replays after a crash find the same task.
                    idempotency_key: format!("dsp:{}", record.dispatch_id),
                    wait_reason: Some(TaskWaitReason::with_code(
                        TaskWaitReasonKind::Dispatch,
                        "dispatch_pending",
                    )),
                    message: None,
                },
                &dispatcher_actor(),
            )
            .await?;

        let needs_approval = record.approval.is_none() && self.approval_required(record).await?;
        let next_status = if needs_approval {
            DispatchStatus::PendingApproval
        } else {
            DispatchStatus::Queued
        };
        let mut update = RecordStateUpdate::to_status(next_status);
        update.task_id = Some(task.task_id.clone());
        self.db
            .update_record_state(&record.dispatch_id, DispatchStatus::CreatingTask, update)
            .await?;
        let updated = self
            .db
            .get_record(&record.dispatch_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record vanished".into()))?;
        if needs_approval {
            // Surface the gate on the public task.
            let _ = self
                .task_core
                .trusted_set_promise_wait(
                    SetPromiseWaitReq {
                        task_id: task.task_id.clone(),
                        wait_reason: TaskWaitReason::with_code(
                            TaskWaitReasonKind::Authorization,
                            "dispatch_approval",
                        ),
                        expected_revision: task.revision,
                    },
                    &dispatcher_actor(),
                )
                .await;
        }
        self.publish_record_event(&updated).await;
        Ok(updated)
    }

    async fn approval_required(&self, record: &DispatchRecord) -> Result<bool> {
        let registration = self
            .db
            .get_registration(&record.target_id)
            .await?
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_TARGET_NOT_REGISTERED, &record.target_id))?;
        Ok(match registration.approval_policy {
            DispatchApprovalPolicy::Never => false,
            DispatchApprovalPolicy::InteractiveCallers => !record.auth.zone_trusted_caller,
            DispatchApprovalPolicy::AllCallers => true,
        })
    }

    // -----------------------------------------------------------------
    // Evaluation loop
    // -----------------------------------------------------------------

    pub fn spawn_maintenance_loop(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut ticks: u64 = 0;
            loop {
                tokio::select! {
                    _ = service.wakeup.notified() => {}
                    _ = tokio::time::sleep(EVALUATE_TICK) => {}
                }
                ticks = ticks.wrapping_add(1);
                let deep_sweep = ticks % SWEEP_EVERY_TICKS == 0;
                service.evaluate_once(deep_sweep).await;
            }
        })
    }

    /// One deterministic evaluation round. `deep_sweep` additionally runs
    /// the cancel/expiry reconciliation against the Task Core snapshot.
    pub async fn evaluate_once(&self, deep_sweep: bool) {
        if let Err(err) = self.resume_creating_tasks().await {
            warn!("dispatcher.resume_creating_tasks failed: {}", err);
        }
        if let Err(err) = self.resume_inflight().await {
            warn!("dispatcher.resume_inflight failed: {}", err);
        }
        if let Err(err) = self.process_due_queue().await {
            warn!("dispatcher.process_due_queue failed: {}", err);
        }
        if deep_sweep {
            if let Err(err) = self.sweep_open_records().await {
                warn!("dispatcher.sweep_open_records failed: {}", err);
            }
        }
    }

    pub async fn startup_recovery(&self) {
        info!("dispatcher.startup_recovery");
        // First finish any CreatingTask saga whose Task Core insert committed
        // before the dispatcher record captured its task_id. Otherwise that
        // live task would look orphaned during StorageDomain recovery.
        self.evaluate_once(true).await;
        let live_task_ids = match self.db.list_task_ids().await {
            Ok(task_ids) => task_ids,
            Err(err) => {
                warn!(
                    "dispatcher: durable state is unreadable; treating restored User task bindings as lost: {}",
                    err
                );
                Default::default()
            }
        };
        match self
            .task_core
            .recover_lost_user_tasks_by_origin(TASK_DISPATCHER_SERVICE_NAME, &live_task_ids)
            .await
        {
            Ok(count) if count > 0 => {
                warn!(
                    "dispatcher: failed {} restored User tasks with runner_lost",
                    count
                )
            }
            Ok(_) => {}
            Err(err) => warn!("dispatcher: User task restore recovery failed: {}", err),
        }
    }

    async fn resume_creating_tasks(&self) -> Result<()> {
        let records = self
            .db
            .list_records_in_status(&[DispatchStatus::CreatingTask])
            .await?;
        for record in records {
            if let Err(err) = self.complete_creating_task(&record).await {
                warn!(
                    "dispatcher: creating-task saga for {} still incomplete: {}",
                    record.dispatch_id, err
                );
            }
        }
        Ok(())
    }

    /// Queue engine: freeze-ordered records → pick instance → write-ahead
    /// attempt → offer → bind → activate.
    async fn process_due_queue(&self) -> Result<()> {
        let now = now_ms();
        let due = self.db.list_due_assignable(now, MAX_BATCH_PER_TICK).await?;
        for record in due {
            if let Err(err) = self.advance_assignable(record).await {
                warn!("dispatcher.advance failed: {}", err);
            }
        }
        Ok(())
    }

    async fn advance_assignable(&self, record: DispatchRecord) -> Result<()> {
        let now = now_ms();
        // Handoff deadline.
        if let Some(expires_at) = record.expires_at {
            if now >= expires_at {
                return self
                    .finish_record_failure(
                        &record,
                        record.status,
                        DispatchStatus::Expired,
                        None,
                        "dispatch handoff deadline exceeded",
                    )
                    .await;
            }
        }
        // Attempt budget.
        if record.attempt_count >= record.delivery_policy.max_attempts {
            return self
                .finish_record_failure(
                    &record,
                    record.status,
                    DispatchStatus::Expired,
                    None,
                    DISPATCH_ERR_DELIVERY_EXHAUSTED,
                )
                .await;
        }

        let instances = self.db.live_instances(&record.target_id, now).await?;
        let available: Vec<&TargetInstance> = instances
            .iter()
            .filter(|i| i.available_capacity > 0)
            .collect();
        if available.is_empty() {
            if record.status != DispatchStatus::WaitingForTarget {
                let code = if instances.is_empty() {
                    "target_offline"
                } else {
                    "runner_busy"
                };
                self.db
                    .update_record_state(
                        &record.dispatch_id,
                        record.status,
                        RecordStateUpdate::to_status(DispatchStatus::WaitingForTarget),
                    )
                    .await?;
                self.project_promise_wait(&record, code).await;
            }
            return Ok(());
        }

        // Deterministic instance selection (doc §11.2).
        let chosen = match record.delivery_policy.instance_selection {
            InstanceSelection::RoundRobin => {
                let cursor = self.db.get_cursor(&record.target_id).await?;
                let next = match cursor {
                    Some(cursor) => available
                        .iter()
                        .find(|i| i.instance_id.as_str() > cursor.as_str())
                        .or_else(|| available.first())
                        .copied(),
                    None => available.first().copied(),
                };
                next
            }
            InstanceSelection::LeastLoaded => available
                .iter()
                .max_by(|a, b| {
                    a.available_capacity
                        .cmp(&b.available_capacity)
                        // Stable tie-breaker: lowest instance id wins.
                        .then_with(|| b.instance_id.cmp(&a.instance_id))
                })
                .copied(),
        };
        let Some(instance) = chosen else {
            return Ok(());
        };
        if record.delivery_policy.instance_selection == InstanceSelection::RoundRobin {
            self.db
                .set_cursor(&record.target_id, &instance.instance_id)
                .await?;
        }

        // Write-ahead attempt, then transition to Offering.
        let attempt_no = record.attempt_count + 1;
        let attempt = DeliveryAttempt {
            dispatch_id: record.dispatch_id.clone(),
            attempt_no,
            delivery_id: delivery_id_for(&record.dispatch_id, attempt_no),
            lease_epoch: instance.lease_epoch,
            target_id: record.target_id.clone(),
            instance_id: instance.instance_id.clone(),
            endpoint: instance.endpoint.clone(),
            stage: DeliveryStage::Offer,
            outcome: None,
            outcome_detail: None,
            reservation_token: None,
            runner_epoch: None,
            deadline_at: now + record.delivery_policy.rpc_timeout_ms,
            created_at: now,
            updated_at: now,
        };
        self.db.insert_attempt(&attempt).await?;
        let mut update = RecordStateUpdate::to_status(DispatchStatus::Offering);
        update.attempt_count = Some(attempt_no);
        let moved = self
            .db
            .update_record_state(&record.dispatch_id, record.status, update)
            .await?;
        if !moved {
            // Lost the CAS to a concurrent transition (e.g. cancel sweep).
            return Ok(());
        }
        let record = self
            .db
            .get_record(&record.dispatch_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record vanished".into()))?;
        self.run_offer(&record, &attempt).await
    }

    async fn run_offer(&self, record: &DispatchRecord, attempt: &DeliveryAttempt) -> Result<()> {
        let Some(task_id) = record.task_id.clone() else {
            return Err(RPCErrors::ReasonError(
                "offering a record without a task id".into(),
            ));
        };
        let registration = self
            .db
            .get_registration(&record.target_id)
            .await?
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_TARGET_NOT_REGISTERED, &record.target_id))?;
        let function = registration
            .function_for(&record.schema_id, record.schema_version)
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_UNSUPPORTED_SCHEMA, &record.schema_id))?;

        let offer_req = OfferTaskReq {
            delivery_id: attempt.delivery_id.clone(),
            task_id: task_id.clone(),
            schema_id: record.schema_id.clone(),
            schema_version: record.schema_version,
            target_id: record.target_id.clone(),
            input: record.input.clone(),
            input_digest: record.auth.input_digest.clone(),
            auth: record.auth.clone(),
            lease_epoch: attempt.lease_epoch,
            deadline_at: attempt.deadline_at,
        };
        let delivery_token = self.delivery_token_for(
            &attempt.target_id,
            &attempt.instance_id,
            attempt.lease_epoch,
        );
        let offer_result = self
            .runner_caller
            .call(
                &attempt.endpoint,
                &function.offer_method,
                serde_json::to_value(&offer_req).unwrap_or(Value::Null),
                record.delivery_policy.rpc_timeout_ms,
                Some(delivery_token),
            )
            .await;

        match offer_result {
            Ok(value) => match OfferTaskResp::from_json(value) {
                Ok(OfferTaskResp::OfferAccepted {
                    app_instance_id,
                    reservation_token,
                }) => {
                    self.db
                        .update_attempt(
                            &attempt.dispatch_id,
                            attempt.attempt_no,
                            DeliveryStage::Offer,
                            Some(DeliveryOutcome::OfferAccepted),
                            None,
                            Some(reservation_token.clone()),
                            None,
                        )
                        .await?;
                    self.db
                        .update_record_state(
                            &record.dispatch_id,
                            DispatchStatus::Offering,
                            RecordStateUpdate::to_status(DispatchStatus::Binding),
                        )
                        .await?;
                    self.run_bind_and_activate(
                        record,
                        attempt,
                        &registration,
                        &app_instance_id,
                        &reservation_token,
                    )
                    .await
                }
                Ok(OfferTaskResp::Busy { retry_after_ms }) => {
                    self.db
                        .update_attempt(
                            &attempt.dispatch_id,
                            attempt.attempt_no,
                            DeliveryStage::Offer,
                            Some(DeliveryOutcome::Busy),
                            None,
                            None,
                            None,
                        )
                        .await?;
                    let delay = retry_after_ms.unwrap_or_else(|| {
                        record
                            .delivery_policy
                            .backoff_ms(&record.dispatch_id, attempt.attempt_no)
                    });
                    let mut update = RecordStateUpdate::to_status(DispatchStatus::Queued);
                    update.ready_at = Some(now_ms() + delay);
                    self.db
                        .update_record_state(&record.dispatch_id, DispatchStatus::Offering, update)
                        .await?;
                    self.project_promise_wait(record, "runner_busy").await;
                    Ok(())
                }
                Ok(OfferTaskResp::Rejected {
                    stable_reason,
                    detail,
                }) => {
                    self.db
                        .update_attempt(
                            &attempt.dispatch_id,
                            attempt.attempt_no,
                            DeliveryStage::Offer,
                            Some(DeliveryOutcome::Rejected),
                            detail.clone(),
                            None,
                            None,
                        )
                        .await?;
                    self.finish_record_failure(
                        record,
                        DispatchStatus::Offering,
                        DispatchStatus::Rejected,
                        Some(stable_reason),
                        detail.unwrap_or_else(|| DISPATCH_ERR_RUNNER_REJECTED.to_string()),
                    )
                    .await
                }
                Err(parse_err) => {
                    self.record_offer_transport_failure(record, attempt, parse_err.to_string())
                        .await
                }
            },
            Err(err) => {
                self.record_offer_transport_failure(record, attempt, err.to_string())
                    .await
            }
        }
    }

    async fn record_offer_transport_failure(
        &self,
        record: &DispatchRecord,
        attempt: &DeliveryAttempt,
        detail: String,
    ) -> Result<()> {
        self.db
            .update_attempt(
                &attempt.dispatch_id,
                attempt.attempt_no,
                DeliveryStage::Offer,
                Some(DeliveryOutcome::TransportError),
                Some(detail),
                None,
                None,
            )
            .await?;
        let delay = record
            .delivery_policy
            .backoff_ms(&record.dispatch_id, attempt.attempt_no);
        let mut update = RecordStateUpdate::to_status(DispatchStatus::Queued);
        update.ready_at = Some(now_ms() + delay);
        self.db
            .update_record_state(&record.dispatch_id, DispatchStatus::Offering, update)
            .await?;
        self.project_promise_wait(record, "offer_retry").await;
        Ok(())
    }

    async fn run_bind_and_activate(
        &self,
        record: &DispatchRecord,
        attempt: &DeliveryAttempt,
        registration: &TargetRegistration,
        app_instance_id: &str,
        reservation_token: &str,
    ) -> Result<()> {
        let Some(task_id) = record.task_id.clone() else {
            return Err(RPCErrors::ReasonError("binding without a task id".into()));
        };
        let task = self
            .task_core
            .trusted_get_task(&task_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("task {} vanished", task_id)))?;

        // Idempotent bind: if the executor is already this instance the
        // earlier bind won; otherwise CAS-bind now.
        let runner_epoch = match &task.executor {
            TaskExecutor::App {
                app_instance_id: bound,
                ..
            } if bound.as_deref() == Some(app_instance_id) => task.runner_epoch,
            _ => {
                let runner_app_id = AuthTarget::from_canonical_key(&registration.owner_app_id)
                    .map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "invalid target owner identity {}: {error}",
                            registration.owner_app_id
                        ))
                    })?
                    .appid_claim()
                    .to_string();
                let bound_task = self
                    .task_core
                    .trusted_bind_app_executor(
                        BindAppExecutorReq {
                            task_id: task_id.clone(),
                            target_id: Some(record.target_id.clone()),
                            app_id: runner_app_id,
                            app_instance_id: app_instance_id.to_string(),
                            delivery_id: Some(attempt.delivery_id.clone()),
                            expected_revision: task.revision,
                        },
                        &dispatcher_actor(),
                    )
                    .await?;
                bound_task.runner_epoch
            }
        };
        self.db
            .update_attempt(
                &attempt.dispatch_id,
                attempt.attempt_no,
                DeliveryStage::Activate,
                None,
                None,
                None,
                Some(runner_epoch),
            )
            .await?;
        self.db
            .update_record_state(
                &record.dispatch_id,
                DispatchStatus::Binding,
                RecordStateUpdate::to_status(DispatchStatus::Activating),
            )
            .await?;
        self.run_activate(
            record,
            attempt,
            registration,
            runner_epoch,
            reservation_token,
        )
        .await
    }

    async fn run_activate(
        &self,
        record: &DispatchRecord,
        attempt: &DeliveryAttempt,
        registration: &TargetRegistration,
        runner_epoch: u64,
        reservation_token: &str,
    ) -> Result<()> {
        let Some(task_id) = record.task_id.clone() else {
            return Err(RPCErrors::ReasonError(
                "activating without a task id".into(),
            ));
        };
        let function = registration
            .function_for(&record.schema_id, record.schema_version)
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_UNSUPPORTED_SCHEMA, &record.schema_id))?;
        let activate_req = ActivateTaskReq {
            delivery_id: attempt.delivery_id.clone(),
            task_id: task_id.clone(),
            runner_epoch,
            reservation_token: reservation_token.to_string(),
        };
        let delivery_token = self.delivery_token_for(
            &attempt.target_id,
            &attempt.instance_id,
            attempt.lease_epoch,
        );
        let activate_result = self
            .runner_caller
            .call(
                &attempt.endpoint,
                &function.activate_method,
                serde_json::to_value(&activate_req).unwrap_or(Value::Null),
                record.delivery_policy.rpc_timeout_ms,
                Some(delivery_token),
            )
            .await;

        let activated = match activate_result {
            Ok(_) => true,
            Err(err) => {
                // A task already Running under this epoch is authoritative
                // proof of activation (doc §11.4).
                let task = self.task_core.trusted_get_task(&task_id).await?;
                match task {
                    Some(task)
                        if task.runner_epoch == runner_epoch
                            && matches!(
                                task.phase,
                                TaskPhase::Running | TaskPhase::Waiting | TaskPhase::Terminal
                            ) =>
                    {
                        true
                    }
                    _ => {
                        debug!(
                            "dispatcher: activate {} not confirmed yet: {}",
                            attempt.delivery_id, err
                        );
                        false
                    }
                }
            }
        };
        if !activated {
            // Keep Activating: the tick loop replays the same delivery id.
            return Ok(());
        }
        self.db
            .update_attempt(
                &attempt.dispatch_id,
                attempt.attempt_no,
                DeliveryStage::Activate,
                Some(DeliveryOutcome::Activated),
                None,
                None,
                Some(runner_epoch),
            )
            .await?;
        self.db
            .update_record_state(
                &record.dispatch_id,
                DispatchStatus::Activating,
                RecordStateUpdate::to_status(DispatchStatus::Accepted),
            )
            .await?;
        if let Some(updated) = self.db.get_record(&record.dispatch_id).await? {
            self.publish_record_event(&updated).await;
        }
        info!(
            "dispatcher: {} accepted (task {} instance {})",
            record.dispatch_id, task_id, attempt.instance_id
        );
        Ok(())
    }

    /// Replay in-flight attempts after a crash or transport uncertainty.
    async fn resume_inflight(&self) -> Result<()> {
        let records = self
            .db
            .list_records_in_status(&[
                DispatchStatus::Offering,
                DispatchStatus::Binding,
                DispatchStatus::Activating,
            ])
            .await?;
        for record in records {
            let Some(attempt) = self.db.latest_attempt(&record.dispatch_id).await? else {
                // No journaled attempt: fall back to the queue.
                self.db
                    .update_record_state(
                        &record.dispatch_id,
                        record.status,
                        RecordStateUpdate::to_status(DispatchStatus::Queued),
                    )
                    .await?;
                continue;
            };
            let now = now_ms();
            // The instance lease decides whether the same delivery id may be
            // replayed or the attempt is fenced and a new one is due.
            let instance = self
                .db
                .get_instance(&attempt.target_id, &attempt.instance_id)
                .await?;
            let lease_alive = instance
                .as_ref()
                .map(|i| i.lease_epoch == attempt.lease_epoch && i.lease_expires_at > now)
                .unwrap_or(false);

            match record.status {
                DispatchStatus::Offering => {
                    if lease_alive {
                        if let Err(err) = self.run_offer(&record, &attempt).await {
                            warn!("dispatcher: offer replay failed: {}", err);
                        }
                    } else {
                        // Old lease fenced -> journal and requeue.
                        self.db
                            .update_attempt(
                                &attempt.dispatch_id,
                                attempt.attempt_no,
                                attempt.stage,
                                Some(DeliveryOutcome::Superseded),
                                Some("instance lease lost".into()),
                                None,
                                None,
                            )
                            .await?;
                        let mut update = RecordStateUpdate::to_status(DispatchStatus::Queued);
                        update.ready_at = Some(now);
                        self.db
                            .update_record_state(&record.dispatch_id, record.status, update)
                            .await?;
                    }
                }
                DispatchStatus::Binding | DispatchStatus::Activating => {
                    let registration = match self.db.get_registration(&record.target_id).await? {
                        Some(r) => r,
                        None => continue,
                    };
                    let reservation = attempt.reservation_token.clone().unwrap_or_default();
                    if record.status == DispatchStatus::Binding {
                        if let Err(err) = self
                            .run_bind_and_activate(
                                &record,
                                &attempt,
                                &registration,
                                &attempt.instance_id,
                                &reservation,
                            )
                            .await
                        {
                            warn!("dispatcher: bind replay failed: {}", err);
                        }
                    } else if let Some(runner_epoch) = attempt.runner_epoch {
                        if let Err(err) = self
                            .run_activate(
                                &record,
                                &attempt,
                                &registration,
                                runner_epoch,
                                &reservation,
                            )
                            .await
                        {
                            warn!("dispatcher: activate replay failed: {}", err);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Cancel/expiry reconciliation against the authoritative Task Core
    /// snapshot (doc §11.4): the queue owner confirms pending cancels for
    /// still-promised tasks.
    async fn sweep_open_records(&self) -> Result<()> {
        let records = self.db.list_open_records().await?;
        let now = now_ms();
        for record in records {
            // Expired handoff deadlines fail even while waiting.
            if let Some(expires_at) = record.expires_at {
                if now >= expires_at && record.status.is_assignable() {
                    let _ = self
                        .finish_record_failure(
                            &record,
                            record.status,
                            DispatchStatus::Expired,
                            None,
                            "dispatch handoff deadline exceeded",
                        )
                        .await;
                    continue;
                }
            }
            let Some(task_id) = record.task_id.as_deref() else {
                continue;
            };
            let Some(task) = self.task_core.trusted_get_task(task_id).await? else {
                continue;
            };
            let cancel_requested = task
                .pending_control
                .as_ref()
                .map(|c| c.action == TaskControlAction::Cancel)
                .unwrap_or(false);
            if record.status.owns_promised_task() && cancel_requested {
                let moved = self
                    .db
                    .update_record_state(
                        &record.dispatch_id,
                        record.status,
                        RecordStateUpdate::to_status(DispatchStatus::Canceled),
                    )
                    .await?;
                if moved {
                    let _ = self
                        .task_core
                        .trusted_cancel_promised_task(
                            CancelPromisedTaskReq {
                                task_id: task_id.to_string(),
                                expected_revision: task.revision,
                            },
                            &dispatcher_actor(),
                        )
                        .await;
                    if let Some(updated) = self.db.get_record(&record.dispatch_id).await? {
                        self.publish_record_event(&updated).await;
                    }
                }
            } else if record.status.owns_promised_task() && task.phase.is_terminal() {
                // Task closed elsewhere (e.g. direct cancel): converge the
                // record.
                let _ = self
                    .db
                    .update_record_state(
                        &record.dispatch_id,
                        record.status,
                        RecordStateUpdate::to_status(DispatchStatus::Canceled),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// Terminal delivery failure: close the record and converge the public
    /// task via the promise-failure command.
    async fn finish_record_failure(
        &self,
        record: &DispatchRecord,
        expected_status: DispatchStatus,
        final_status: DispatchStatus,
        reject_reason: Option<DispatchRejectReason>,
        detail: impl Into<String>,
    ) -> Result<()> {
        let detail = detail.into();
        let mut update = RecordStateUpdate::to_status(final_status);
        update.reject_reason = reject_reason;
        update.message = Some(detail.clone());
        let moved = self
            .db
            .update_record_state(&record.dispatch_id, expected_status, update)
            .await?;
        if !moved {
            return Ok(());
        }
        if let Some(task_id) = record.task_id.as_deref() {
            if let Ok(Some(task)) = self.task_core.trusted_get_task(task_id).await {
                if !task.phase.is_terminal() {
                    let _ = self
                        .task_core
                        .trusted_finish_promise_failure(
                            FinishPromiseFailureReq {
                                task_id: task_id.to_string(),
                                error: TaskError::new(
                                    reject_reason
                                        .map(|r| r.to_string())
                                        .unwrap_or_else(|| "dispatch_failed".to_string()),
                                    detail.clone(),
                                ),
                                expected_revision: task.revision,
                            },
                            &dispatcher_actor(),
                        )
                        .await;
                }
            }
        }
        if let Some(updated) = self.db.get_record(&record.dispatch_id).await? {
            self.publish_record_event(&updated).await;
        }
        Ok(())
    }

    /// Project a queue condition onto the public Promised task (doc §11.6).
    async fn project_promise_wait(&self, record: &DispatchRecord, code: &str) {
        let Some(task_id) = record.task_id.as_deref() else {
            return;
        };
        let Ok(Some(task)) = self.task_core.trusted_get_task(task_id).await else {
            return;
        };
        if task.phase != TaskPhase::Promised {
            return;
        }
        let kind = match code {
            "runner_busy" => TaskWaitReasonKind::Capacity,
            _ => TaskWaitReasonKind::Dispatch,
        };
        if task
            .wait_reason
            .as_ref()
            .map(|r| r.code.as_deref() == Some(code))
            .unwrap_or(false)
        {
            return;
        }
        let _ = self
            .task_core
            .trusted_set_promise_wait(
                SetPromiseWaitReq {
                    task_id: task_id.to_string(),
                    wait_reason: TaskWaitReason::with_code(kind, code),
                    expected_revision: task.revision,
                },
                &dispatcher_actor(),
            )
            .await;
    }
}

// ---------------------------------------------------------------------------
// RPC handlers
// ---------------------------------------------------------------------------

#[async_trait]
impl TaskDispatcherHandler for TaskDispatcherService {
    async fn handle_dispatch_task(
        &self,
        req: DispatchTaskReq,
        ctx: RPCContext,
    ) -> Result<DispatchTaskResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        if req.idempotency_key.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "idempotency_key is required".into(),
            ));
        }

        // Idempotent replay.
        if let Some(existing) = self
            .db
            .get_record_by_idempotency(
                &request_ctx.user_id,
                &request_ctx.app_id,
                &req.idempotency_key,
            )
            .await?
        {
            let existing_digest = compute_input_digest(&req.input);
            if existing.auth.input_digest != existing_digest
                || existing.schema_id != req.schema_id
                || existing.auth.parent_task_id != req.parent_task_id
                || existing.auth.storage_domain != req.storage_domain
            {
                return Err(dispatch_err(
                    DISPATCH_ERR_IDEMPOTENCY_CONFLICT,
                    &req.idempotency_key,
                ));
            }
            let existing = if existing.status == DispatchStatus::CreatingTask {
                self.complete_creating_task(&existing).await?
            } else {
                existing
            };
            let task_id = existing
                .task_id
                .clone()
                .ok_or_else(|| RPCErrors::ReasonError("dispatch record has no task yet".into()))?;
            let task = self
                .task_core
                .trusted_get_task(&task_id)
                .await?
                .ok_or_else(|| RPCErrors::ReasonError("dispatched task vanished".into()))?;
            return Ok(DispatchTaskResult {
                task_id,
                dispatch_id: existing.dispatch_id.clone(),
                status: existing.status,
                task,
            });
        }

        // Synchronous route resolution + freeze; failures here reject the
        // call without creating a task (doc §11.3).
        let (target_id, target_selection) = match req.target_id.as_deref() {
            Some(explicit) => (explicit.to_string(), TargetSelection::Explicit),
            None => {
                let route = self
                    .db
                    .get_operation_route(&req.schema_id)
                    .await?
                    .filter(|r| r.enabled)
                    .ok_or_else(|| {
                        dispatch_err(DISPATCH_ERR_DEFAULT_TARGET_NOT_CONFIGURED, &req.schema_id)
                    })?;
                (
                    route.default_target_id.clone(),
                    TargetSelection::DefaultRoute {
                        route_revision: route.revision,
                    },
                )
            }
        };
        let registration = self
            .db
            .get_registration(&target_id)
            .await?
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_TARGET_NOT_REGISTERED, &target_id))?;
        if !registration.enabled {
            return Err(dispatch_err(DISPATCH_ERR_TARGET_DISABLED, &target_id));
        }
        if registration.auth_policy == DispatchAuthPolicy::ZoneTrustedOnly
            && !request_ctx.zone_trusted
        {
            return Err(RPCErrors::NoPermission(
                "this target only accepts zone-trusted dispatchers".into(),
            ));
        }
        // Freeze the schema revision from the Task Core registry.
        let schema = self
            .task_core
            .trusted_get_schema(&req.schema_id, req.schema_version)
            .await?;
        if registration
            .function_for(&schema.schema_id, schema.schema_version)
            .is_none()
        {
            return Err(dispatch_err(
                DISPATCH_ERR_UNSUPPORTED_SCHEMA,
                format!("{}@{}", schema.schema_id, schema.schema_version),
            ));
        }
        // on_behalf_of is a trusted-caller delegation claim.
        let on_behalf_of = match req.on_behalf_of.as_deref() {
            Some(user) if request_ctx.zone_trusted => user.to_string(),
            Some(user) if user == request_ctx.user_id => user.to_string(),
            Some(_) => {
                return Err(RPCErrors::NoPermission(
                    "on_behalf_of delegation requires a zone-trusted caller".into(),
                ))
            }
            None => request_ctx.user_id.clone(),
        };

        let now = now_ms();
        let dispatch_id = format!("dsp-{}", uuid::Uuid::new_v4().simple());
        let record = DispatchRecord {
            dispatch_id: dispatch_id.clone(),
            requested_target_id: req.target_id.clone(),
            target_id: target_id.clone(),
            target_selection,
            schema_id: schema.schema_id.clone(),
            schema_version: schema.schema_version,
            registration_revision: registration.registration_revision,
            delivery_policy: registration.delivery_policy.clone(),
            status: DispatchStatus::CreatingTask,
            task_id: None,
            input: req.input.clone(),
            auth: DispatchAuthEnvelope {
                requested_by_user: request_ctx.user_id.clone(),
                requested_by_app: request_ctx.app_id.clone(),
                on_behalf_of,
                zone_trusted_caller: request_ctx.zone_trusted,
                workflow_ref: req.workflow_ref.clone(),
                parent_task_id: req.parent_task_id.clone(),
                storage_domain: req.storage_domain,
                input_digest: compute_input_digest(&req.input),
                created_at: now,
                expires_at: req.expires_at,
            },
            priority: req.priority.unwrap_or(0),
            ready_at: now,
            attempt_count: 0,
            reject_reason: None,
            approval: None,
            message: req.name.clone(),
            expires_at: req.expires_at,
            created_at: now,
            updated_at: now,
        };
        let record = match self.db.insert_record(&record, &req.idempotency_key).await? {
            Some(existing) => existing,
            None => record,
        };
        let record = self.complete_creating_task(&record).await?;
        let task_id = record
            .task_id
            .clone()
            .ok_or_else(|| RPCErrors::ReasonError("task backfill failed".into()))?;
        let task = self
            .task_core
            .trusted_get_task(&task_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatched task vanished".into()))?;
        self.notify_evaluate();
        Ok(DispatchTaskResult {
            task_id,
            dispatch_id: record.dispatch_id.clone(),
            status: record.status,
            task,
        })
    }

    async fn handle_get_dispatch(
        &self,
        req: GetDispatchReq,
        ctx: RPCContext,
    ) -> Result<GetDispatchResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let record = match (req.dispatch_id.as_deref(), req.task_id.as_deref()) {
            (Some(dispatch_id), _) => self.db.get_record(dispatch_id).await?,
            (None, Some(task_id)) => self.db.get_record_by_task(task_id).await?,
            (None, None) => {
                return Err(RPCErrors::ParseRequestError(
                    "dispatch_id or task_id is required".into(),
                ))
            }
        };
        let record = record
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record not found".to_string()))?;
        if !request_ctx.zone_trusted
            && !(record.auth.requested_by_user == request_ctx.user_id
                && record.auth.requested_by_app == request_ctx.app_id)
        {
            return Err(RPCErrors::NoPermission(
                "only the submitter or a zone-trusted service may view a dispatch".into(),
            ));
        }
        Ok(GetDispatchResult { record })
    }

    async fn handle_list_dispatches(
        &self,
        req: ListDispatchesReq,
        ctx: RPCContext,
    ) -> Result<ListDispatchesResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let records = self
            .db
            .list_records(
                req.status,
                req.target_id.as_deref(),
                req.schema_id.as_deref(),
                req.limit.unwrap_or(100),
            )
            .await?;
        let records = if request_ctx.zone_trusted || request_ctx.sudo {
            records
        } else {
            records
                .into_iter()
                .filter(|r| {
                    r.auth.requested_by_user == request_ctx.user_id
                        && r.auth.requested_by_app == request_ctx.app_id
                })
                .collect()
        };
        Ok(ListDispatchesResult {
            records,
            next_cursor: None,
        })
    }

    async fn handle_cancel_dispatch(
        &self,
        req: CancelDispatchReq,
        ctx: RPCContext,
    ) -> Result<GetDispatchResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let record = self
            .db
            .get_record(&req.dispatch_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record not found".into()))?;
        let allowed = request_ctx.zone_trusted
            || (record.auth.requested_by_user == request_ctx.user_id
                && record.auth.requested_by_app == request_ctx.app_id);
        if !allowed {
            return Err(RPCErrors::NoPermission(
                "only the submitter or a zone-trusted service may cancel a dispatch".into(),
            ));
        }
        if record.status.is_terminal() {
            return Err(dispatch_err(DISPATCH_ERR_ALREADY_TERMINAL, &record.status));
        }
        if !record.status.owns_promised_task() {
            // Already bound: cancellation is the ordinary task control
            // protocol (doc §11.6).
            return Err(RPCErrors::ReasonError(
                "task already bound; cancel via task-manager request_control".into(),
            ));
        }
        let moved = self
            .db
            .update_record_state(
                &record.dispatch_id,
                record.status,
                RecordStateUpdate::to_status(DispatchStatus::Canceled),
            )
            .await?;
        if moved {
            if let Some(task_id) = record.task_id.as_deref() {
                if let Ok(Some(task)) = self.task_core.trusted_get_task(task_id).await {
                    if !task.phase.is_terminal() {
                        let _ = self
                            .task_core
                            .trusted_cancel_promised_task(
                                CancelPromisedTaskReq {
                                    task_id: task_id.to_string(),
                                    expected_revision: task.revision,
                                },
                                &dispatcher_actor(),
                            )
                            .await;
                    }
                }
            }
        }
        let record = self
            .db
            .get_record(&req.dispatch_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record vanished".into()))?;
        self.publish_record_event(&record).await;
        Ok(GetDispatchResult { record })
    }

    async fn handle_approve_dispatch(
        &self,
        req: ApproveDispatchReq,
        ctx: RPCContext,
    ) -> Result<GetDispatchResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        if !Self::can_decide_approval(&request_ctx) {
            return Err(RPCErrors::NoPermission(
                "approving a dispatch requires a zone-trusted or sudo session".into(),
            ));
        }
        let record = self
            .db
            .get_record(&req.dispatch_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record not found".into()))?;
        if record.status != DispatchStatus::PendingApproval {
            return Err(dispatch_err(
                DISPATCH_ERR_NOT_PENDING_APPROVAL,
                &record.status,
            ));
        }
        let approval = DispatchApproval {
            decision: req.decision,
            decided_by_user: request_ctx.user_id.clone(),
            decided_by_app: request_ctx.app_id.clone(),
            decided_at: now_ms(),
            note: req.note.clone(),
        };
        match req.decision {
            ApprovalDecision::Approved => {
                let mut update = RecordStateUpdate::to_status(DispatchStatus::Queued);
                update.approval = Some(approval);
                update.ready_at = Some(now_ms());
                self.db
                    .update_record_state(
                        &record.dispatch_id,
                        DispatchStatus::PendingApproval,
                        update,
                    )
                    .await?;
                // Clear the authorization wait on the public task.
                if let Some(task_id) = record.task_id.as_deref() {
                    if let Ok(Some(task)) = self.task_core.trusted_get_task(task_id).await {
                        if task.phase == TaskPhase::Promised {
                            let _ = self
                                .task_core
                                .trusted_set_promise_wait(
                                    SetPromiseWaitReq {
                                        task_id: task_id.to_string(),
                                        wait_reason: TaskWaitReason::with_code(
                                            TaskWaitReasonKind::Dispatch,
                                            "dispatch_pending",
                                        ),
                                        expected_revision: task.revision,
                                    },
                                    &dispatcher_actor(),
                                )
                                .await;
                        }
                    }
                }
                self.notify_evaluate();
            }
            ApprovalDecision::Denied => {
                let mut update = RecordStateUpdate::to_status(DispatchStatus::Rejected);
                update.approval = Some(approval);
                update.reject_reason = Some(DispatchRejectReason::ApprovalDenied);
                update.message = Some(DISPATCH_ERR_APPROVAL_DENIED.to_string());
                let moved = self
                    .db
                    .update_record_state(
                        &record.dispatch_id,
                        DispatchStatus::PendingApproval,
                        update,
                    )
                    .await?;
                if moved {
                    if let Some(task_id) = record.task_id.as_deref() {
                        if let Ok(Some(task)) = self.task_core.trusted_get_task(task_id).await {
                            if !task.phase.is_terminal() {
                                let _ = self
                                    .task_core
                                    .trusted_finish_promise_failure(
                                        FinishPromiseFailureReq {
                                            task_id: task_id.to_string(),
                                            error: TaskError::new(
                                                DISPATCH_ERR_APPROVAL_DENIED,
                                                "dispatch approval denied",
                                            ),
                                            expected_revision: task.revision,
                                        },
                                        &dispatcher_actor(),
                                    )
                                    .await;
                            }
                        }
                    }
                }
            }
        }
        let record = self
            .db
            .get_record(&req.dispatch_id)
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("dispatch record vanished".into()))?;
        self.publish_record_event(&record).await;
        Ok(GetDispatchResult { record })
    }

    async fn handle_register_target(
        &self,
        req: RegisterTargetReq,
        ctx: RPCContext,
    ) -> Result<TargetRegistration> {
        let request_ctx = self.authenticate(&ctx).await?;
        let mut registration = req.registration;
        if registration.target_id.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError("target_id is required".into()));
        }
        if registration.functions.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "a registration needs at least one runner function".into(),
            ));
        }
        // Existing registrations are AppInstance-owned. A new AppInstance
        // may replace stale ownership only when the AgentSpec explicitly
        // binds this stable Agent DID to it.
        let existing = self.db.get_registration(&registration.target_id).await?;
        let mut binding_authorized_takeover = false;
        if let Some(existing) = existing.as_ref() {
            let same_owner = existing.owner_user_id == request_ctx.user_id
                && existing.owner_app_id == request_ctx.app_id;
            if !request_ctx.zone_trusted && !same_owner {
                binding_authorized_takeover = self
                    .request_matches_agent_binding(&request_ctx, &registration.target_id)
                    .await
                    .unwrap_or_else(|error| {
                        warn!(
                            "cannot authorize AgentServiceBinding takeover for {}: {}",
                            registration.target_id, error
                        );
                        false
                    });
                if !binding_authorized_takeover {
                    return Err(RPCErrors::NoPermission(format!(
                        "target {} is owned by app instance {}@{}",
                        registration.target_id, existing.owner_app_id, existing.owner_user_id
                    )));
                }
            }
        }
        // Owner identity comes from the verified token, never the payload.
        // An admin (zone-trusted) update of someone else's registration
        // keeps the original owner — admin edits must not hijack the
        // target away from its runner.
        match existing {
            Some(existing)
                if request_ctx.zone_trusted
                    && (existing.owner_user_id != request_ctx.user_id
                        || existing.owner_app_id != request_ctx.app_id) =>
            {
                registration.owner_user_id = existing.owner_user_id;
                registration.owner_app_id = existing.owner_app_id;
            }
            _ => {
                registration.owner_user_id = request_ctx.user_id.clone();
                registration.owner_app_id = request_ctx.app_id.clone();
            }
        }
        if binding_authorized_takeover {
            info!(
                "AgentServiceBinding transferred target {} to app instance {}@{}",
                registration.target_id, registration.owner_app_id, registration.owner_user_id
            );
        }
        let stored = self.db.upsert_registration(registration).await?;
        self.notify_evaluate();
        Ok(stored)
    }

    async fn handle_disable_target(&self, req: DisableTargetReq, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "disable_target")?;
        let changed = self
            .db
            .set_registration_enabled(&req.target_id, false)
            .await?;
        if !changed {
            return Err(dispatch_err(
                DISPATCH_ERR_TARGET_NOT_REGISTERED,
                &req.target_id,
            ));
        }
        Ok(())
    }

    async fn handle_attach_instance(
        &self,
        req: AttachInstanceReq,
        ctx: RPCContext,
    ) -> Result<AttachInstanceResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_target_owner(&request_ctx, &req.target_id, "attach_instance")
            .await?;
        if req.endpoint.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "instance endpoint is required".into(),
            ));
        }
        validate_runner_endpoint(&req.endpoint)?;
        let lease_epoch = self.db.next_lease_epoch(&req.target_id).await?;
        let lease_ms = req
            .lease_ms
            .unwrap_or(DEFAULT_INSTANCE_LEASE_MS)
            .min(MAX_INSTANCE_LEASE_MS);
        let now = now_ms();
        let instance = TargetInstance {
            target_id: req.target_id.clone(),
            instance_id: req.instance_id.clone(),
            endpoint: req.endpoint.clone(),
            lease_epoch,
            lease_expires_at: now + lease_ms,
            capacity: req.capacity,
            available_capacity: req.available_capacity.unwrap_or(req.capacity),
        };
        self.db.upsert_instance(&instance).await?;
        self.notify_evaluate();
        Ok(AttachInstanceResult {
            lease_epoch,
            lease_expires_at: instance.lease_expires_at,
            target_key: task_dispatcher_target_key(&req.target_id),
            delivery_token: self.delivery_token_for(&req.target_id, &req.instance_id, lease_epoch),
        })
    }

    async fn handle_renew_instance(
        &self,
        req: RenewInstanceReq,
        ctx: RPCContext,
    ) -> Result<RenewInstanceResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_target_owner(&request_ctx, &req.target_id, "renew_instance")
            .await?;
        let lease_ms = req
            .lease_ms
            .unwrap_or(DEFAULT_INSTANCE_LEASE_MS)
            .min(MAX_INSTANCE_LEASE_MS);
        let lease_expires_at = now_ms() + lease_ms;
        let renewed = self
            .db
            .update_instance_lease(
                &req.target_id,
                &req.instance_id,
                req.lease_epoch,
                lease_expires_at,
                req.available_capacity,
            )
            .await?;
        if !renewed {
            return Err(dispatch_err(
                DISPATCH_ERR_STALE_INSTANCE,
                format!("{}/{}", req.target_id, req.instance_id),
            ));
        }
        self.notify_evaluate();
        Ok(RenewInstanceResult {
            lease_epoch: req.lease_epoch,
            lease_expires_at,
            delivery_token: self.delivery_token_for(
                &req.target_id,
                &req.instance_id,
                req.lease_epoch,
            ),
        })
    }

    async fn handle_detach_instance(&self, req: DetachInstanceReq, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_target_owner(&request_ctx, &req.target_id, "detach_instance")
            .await?;
        let instance = self
            .db
            .get_instance(&req.target_id, &req.instance_id)
            .await?;
        if let Some(instance) = instance {
            if instance.lease_epoch != req.lease_epoch {
                return Err(dispatch_err(
                    DISPATCH_ERR_STALE_INSTANCE,
                    format!("{}/{}", req.target_id, req.instance_id),
                ));
            }
            self.db
                .remove_instance(&req.target_id, &req.instance_id)
                .await?;
        }
        Ok(())
    }

    async fn handle_get_target(
        &self,
        req: GetTargetReq,
        ctx: RPCContext,
    ) -> Result<GetTargetResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "get_target")?;
        let registration = self
            .db
            .get_registration(&req.target_id)
            .await?
            .ok_or_else(|| dispatch_err(DISPATCH_ERR_TARGET_NOT_REGISTERED, &req.target_id))?;
        let instances = self.db.live_instances(&req.target_id, now_ms()).await?;
        Ok(GetTargetResult {
            registration,
            instances,
        })
    }

    async fn handle_list_targets(&self, ctx: RPCContext) -> Result<ListTargetsResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "list_targets")?;
        Ok(ListTargetsResult {
            targets: self.db.list_registrations().await?,
        })
    }

    async fn handle_set_operation_route(
        &self,
        req: SetOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<OperationRoute> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "set_operation_route")?;
        // The default backend must exist and support the schema — a target
        // cannot become a default by merely registering (doc §11.5).
        let registration = self
            .db
            .get_registration(&req.default_target_id)
            .await?
            .ok_or_else(|| {
                dispatch_err(DISPATCH_ERR_TARGET_NOT_REGISTERED, &req.default_target_id)
            })?;
        if !registration.supports_schema(&req.schema_id) {
            return Err(dispatch_err(
                DISPATCH_ERR_UNSUPPORTED_SCHEMA,
                &req.schema_id,
            ));
        }
        self.db
            .set_operation_route(&req.schema_id, &req.default_target_id)
            .await
    }

    async fn handle_disable_operation_route(
        &self,
        req: DisableOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "disable_operation_route")?;
        self.db.disable_operation_route(&req.schema_id).await
    }

    async fn handle_get_operation_route(
        &self,
        req: GetOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<GetOperationRouteResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "get_operation_route")?;
        Ok(GetOperationRouteResult {
            route: self.db.get_operation_route(&req.schema_id).await?,
        })
    }

    async fn handle_list_operation_routes(
        &self,
        ctx: RPCContext,
    ) -> Result<ListOperationRoutesResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "list_operation_routes")?;
        Ok(ListOperationRoutesResult {
            routes: self.db.list_operation_routes().await?,
        })
    }
}

// ---------------------------------------------------------------------------
// HTTP wiring
// ---------------------------------------------------------------------------

pub struct TaskDispatcherHttpServer<T: TaskDispatcherHandler> {
    rpc_handler: buckyos_api::TaskDispatcherServerHandler<T>,
}

impl<T: TaskDispatcherHandler> TaskDispatcherHttpServer<T> {
    pub fn new(handler: T) -> Self {
        Self {
            rpc_handler: buckyos_api::TaskDispatcherServerHandler::new(handler),
        }
    }
}

#[async_trait]
impl<T: TaskDispatcherHandler + 'static> HttpServer for TaskDispatcherHttpServer<T> {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, &self.rpc_handler).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "task-dispatcher-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_task_dispatcher(
    token_verifier: Arc<dyn SessionTokenVerifier>,
    task_core: TaskManagerService,
) -> Result<TaskDispatcherService> {
    let db = DispatchDb::open_from_service_spec()
        .await
        .map_err(RPCErrors::ReasonError)?;
    let kevent_client = match get_buckyos_api_runtime() {
        Ok(runtime) => runtime.get_kevent_client().await.ok(),
        Err(_) => None,
    };
    let service = TaskDispatcherService::new(
        Arc::new(db),
        task_core,
        token_verifier,
        Arc::new(KrpcRunnerCaller),
        kevent_client,
    );
    service.startup_recovery().await;
    let service_arc = Arc::new(service.clone());
    service_arc.spawn_maintenance_loop();
    Ok(service)
}
