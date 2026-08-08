//! Task Dispatch Center service: assignment (not FCFS claiming), offer
//! lease / expiry timers, startup recovery and the independent RPC surface
//! on `/kapi/task-dispatcher`.
//!
//! Concurrency model: every mutation that could enable an assignment calls
//! `evaluate_target` directly (serialized per target); a single background
//! maintenance loop handles the three deadline kinds (offer lease, request
//! expiry, instance lease) plus a low-frequency sweep as the lost-wakeup
//! backstop. KEvent is acceleration only — every notification path and the
//! sweep converge on the same store reads.

use async_trait::async_trait;
use buckyos_api::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use kRPC::{RPCContext, RPCErrors, Result};
use log::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use uuid::Uuid;

use super::dispatch_db::{DispatchDb, RecordFilter, TargetRow};
use crate::server::{RequestContext, SessionTokenVerifier};

/// Instance lease TTL handed out by attach/renew. Instances renew at ttl/3
/// (the SDK does this automatically).
pub const INSTANCE_LEASE_TTL_MS: u64 = 60_000;
/// Upper bound for the maintenance sleep — also the low-frequency sweep
/// cadence that survives lost notifications and timer drift.
const MAINTENANCE_MAX_SLEEP: Duration = Duration::from_secs(60);
const MAINTENANCE_MIN_SLEEP: Duration = Duration::from_millis(200);
/// Max records considered per evaluation round (per target).
const EVAL_BATCH: u32 = 64;
const DEFAULT_LIST_LIMIT: u32 = 100;

const EXPIRE_DETAIL_REQUEST: &str = "request_expired";
const EXPIRE_DETAIL_DELIVERY_EXHAUSTED: &str = "delivery_exhausted";

#[derive(Clone)]
pub struct TaskDispatcherService {
    inner: Arc<DispatcherInner>,
}

struct DispatcherInner {
    db: DispatchDb,
    kevent_client: KEventClient,
    token_verifier: Arc<dyn SessionTokenVerifier>,
    /// Serializes evaluation per target so concurrent triggers can't
    /// double-assign against stale capacity reads.
    eval_locks: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Round-robin cursors (in-memory; restart reset is fine).
    rr_cursor: StdMutex<HashMap<String, usize>>,
    maintenance_notify: tokio::sync::Notify,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn reason(msg: impl Into<String>) -> RPCErrors {
    RPCErrors::ReasonError(msg.into())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => {
            db_err.is_unique_violation()
                || db_err.message().to_ascii_uppercase().contains("UNIQUE")
        }
        _ => false,
    }
}

impl TaskDispatcherService {
    pub fn new(
        db: DispatchDb,
        kevent_client: KEventClient,
        token_verifier: Arc<dyn SessionTokenVerifier>,
    ) -> Self {
        Self {
            inner: Arc::new(DispatcherInner {
                db,
                kevent_client,
                token_verifier,
                eval_locks: tokio::sync::Mutex::new(HashMap::new()),
                rr_cursor: StdMutex::new(HashMap::new()),
                maintenance_notify: tokio::sync::Notify::new(),
            }),
        }
    }

    fn db(&self) -> &DispatchDb {
        &self.inner.db
    }

    // ------------------------------------------------------------------
    // authentication / authorization
    // ------------------------------------------------------------------

    /// Fail closed: no token / bad signature => NoPermission. Identity is
    /// never taken from the request payload. Same trust grading as TaskMgr:
    /// verify-hub issued tokens are interactive sessions, every other
    /// verifiable signer (owner/device key) is zone-trusted. Holding a
    /// TaskMgr permission grants nothing here — this is a separate path
    /// with its own checks.
    async fn authenticate(&self, ctx: &RPCContext) -> Result<RequestContext> {
        let token = ctx
            .token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RPCErrors::NoPermission("task-dispatcher requires a session token".to_string())
            })?;
        let verified = self.inner.token_verifier.verify(token).await?;
        let (user_id, app_id) = verified.get_subs()?;
        if user_id.trim().is_empty() {
            return Err(RPCErrors::InvalidToken(
                "session token has an empty subject".to_string(),
            ));
        }
        let zone_trusted = verified.iss.as_deref() != Some(VERIFY_HUB_UNIQUE_ID);
        Ok(RequestContext {
            user_id,
            app_id,
            zone_trusted,
            sudo: verified.sudo,
        })
    }

    fn require_zone_trusted(request_ctx: &RequestContext, what: &str) -> Result<()> {
        if !request_ctx.zone_trusted {
            return Err(RPCErrors::NoPermission(format!(
                "{} requires a zone-trusted caller",
                what
            )));
        }
        Ok(())
    }

    /// The manual-release admin grade (doc §7.1): zone-trusted callers and
    /// sudo-elevated interactive sessions. Deliberately ONE predicate for
    /// both uses — who is exempt from `InteractiveCallers` holds, and who
    /// may approve/deny. Target ownership and being the submitter grant
    /// nothing here (a registrant must not be able to release itself).
    fn is_approval_admin(request_ctx: &RequestContext) -> bool {
        request_ctx.zone_trusted || request_ctx.sudo
    }

    fn require_approval_admin(request_ctx: &RequestContext, what: &str) -> Result<()> {
        if !Self::is_approval_admin(request_ctx) {
            return Err(RPCErrors::NoPermission(format!(
                "{} requires a zone-trusted caller or a sudo session",
                what
            )));
        }
        Ok(())
    }

    /// Target-side operations are only valid for the registered owner
    /// identity (a zone-trusted caller whose token matches the owner).
    fn require_target_owner(request_ctx: &RequestContext, target: &TargetRow) -> Result<()> {
        Self::require_zone_trusted(request_ctx, "target-side dispatcher access")?;
        if target.owner_user_id != request_ctx.user_id
            || target.owner_app_id != request_ctx.app_id
        {
            return Err(RPCErrors::NoPermission(format!(
                "caller {}/{} is not the owner of target {}",
                request_ctx.user_id, request_ctx.app_id, target.registration.target_id
            )));
        }
        Ok(())
    }

    fn can_view_record(request_ctx: &RequestContext, record: &DispatchRecord) -> bool {
        if request_ctx.zone_trusted {
            return true;
        }
        (record.auth.requested_by_user == request_ctx.user_id
            && record.auth.requested_by_app == request_ctx.app_id)
            || record.auth.on_behalf_of == request_ctx.user_id
    }

    async fn load_target(&self, target_id: &str) -> Result<Option<TargetRow>> {
        self.db()
            .get_target(target_id)
            .await
            .map_err(|e| reason(e.to_string()))
    }

    async fn load_record(&self, dispatch_id: &str) -> Result<DispatchRecord> {
        self.db()
            .get_record(dispatch_id)
            .await
            .map_err(|e| reason(e.to_string()))?
            .ok_or_else(|| reason(format!("dispatch {} not found", dispatch_id)))
    }

    /// Validate that `(instance_id, lease_epoch)` is the *current* lease of
    /// an online instance of `target_id`. Stale epochs (paused-and-resumed
    /// instances, replayed connections) and expired leases are refused.
    async fn validate_instance(
        &self,
        target_id: &str,
        instance_id: &str,
        lease_epoch: u64,
        now: i64,
    ) -> Result<TargetInstance> {
        let instance = self
            .db()
            .get_instance(target_id, instance_id)
            .await
            .map_err(|e| reason(e.to_string()))?
            .ok_or_else(|| {
                reason(format!(
                    "{}: instance {} of target {} is not attached",
                    DISPATCH_ERR_STALE_INSTANCE, instance_id, target_id
                ))
            })?;
        if instance.lease_epoch != lease_epoch {
            return Err(reason(format!(
                "{}: lease epoch {} does not match current epoch {}",
                DISPATCH_ERR_STALE_INSTANCE, lease_epoch, instance.lease_epoch
            )));
        }
        if (instance.lease_expires_at as i64) <= now {
            return Err(reason(format!(
                "{}: instance lease expired",
                DISPATCH_ERR_STALE_INSTANCE
            )));
        }
        Ok(instance)
    }

    // ------------------------------------------------------------------
    // events (audit row + kevent acceleration)
    // ------------------------------------------------------------------

    async fn emit_transition(
        &self,
        dispatch_id: &str,
        target_id: &str,
        from: &str,
        to: &str,
        instance_id: Option<&str>,
        detail: Option<&str>,
        target_task_id: Option<i64>,
        ts: i64,
    ) {
        if let Err(err) = self
            .db()
            .insert_event(dispatch_id, ts, from, to, instance_id, detail)
            .await
        {
            warn!(
                "task_dispatcher.emit_transition: audit insert failed for {}: {}",
                dispatch_id, err
            );
        }
        // Payload carries ids only — never the input (doc §6.1).
        let mut payload = json!({
            "dispatch_id": dispatch_id,
            "target_id": target_id,
            "from": from,
            "to": to,
            "ts": ts,
        });
        if let Some(task_id) = target_task_id {
            payload["target_task_id"] = json!(task_id);
        }
        if let Some(detail) = detail {
            payload["detail"] = json!(detail);
        }
        let event_id = task_dispatcher_record_event_id(dispatch_id);
        if let Err(err) = self
            .inner
            .kevent_client
            .pub_event(event_id.as_str(), payload)
            .await
        {
            warn!(
                "task_dispatcher.emit_transition: kevent publish failed for {}: {}",
                event_id, err
            );
        }
    }

    /// Admin-UI wake-up hint for a newly held record. Acceleration only:
    /// the authoritative queue is `list_dispatches(status=PendingApproval)`,
    /// and a lost hint degrades to polling without losing backlog.
    async fn notify_approvals_channel(&self, target_id: &str, dispatch_id: &str, ts: i64) {
        let event_id = task_dispatcher_approvals_event_id();
        let payload = json!({
            "dispatch_id": dispatch_id,
            "target_id": target_id,
            "to": "PendingApproval",
            "ts": ts,
        });
        if let Err(err) = self
            .inner
            .kevent_client
            .pub_event(event_id.as_str(), payload)
            .await
        {
            warn!(
                "task_dispatcher.notify_approvals_channel: publish {} failed: {}",
                event_id, err
            );
        }
    }

    async fn notify_target_channel(&self, target_id: &str, dispatch_id: &str, ts: i64) {
        let target_key = task_dispatcher_target_key(target_id);
        let event_id = task_dispatcher_target_event_id(target_key.as_str());
        let payload = json!({
            "dispatch_id": dispatch_id,
            "target_id": target_id,
            "to": "Offered",
            "ts": ts,
        });
        if let Err(err) = self
            .inner
            .kevent_client
            .pub_event(event_id.as_str(), payload)
            .await
        {
            warn!(
                "task_dispatcher.notify_target_channel: publish {} failed: {}",
                event_id, err
            );
        }
    }

    // ------------------------------------------------------------------
    // assignment: the only path that turns waiting records into offers
    // ------------------------------------------------------------------

    async fn eval_lock(&self, target_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.inner.eval_locks.lock().await;
        locks
            .entry(target_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Centralized instance assignment (doc §5.2). Never FCFS: instances
    /// only ever see records already assigned to them; `claim_next` is a
    /// transport, not a selection point.
    pub(crate) async fn evaluate_target(&self, target_id: &str, now: i64) {
        let lock = self.eval_lock(target_id).await;
        let _guard = lock.lock().await;

        let target = match self.db().get_target(target_id).await {
            Ok(Some(target)) => target,
            Ok(None) => return,
            Err(err) => {
                warn!(
                    "task_dispatcher.evaluate_target({}): load target failed: {}",
                    target_id, err
                );
                return;
            }
        };
        if !target.enabled {
            // Disabled targets keep their waiting records; nothing is
            // assigned until re-enabled.
            let _ = self.db().mark_queued_as_waiting(target_id, now).await;
            return;
        }

        let instances = match self.db().list_active_instances(target_id, now).await {
            Ok(instances) => instances,
            Err(err) => {
                warn!(
                    "task_dispatcher.evaluate_target({}): list instances failed: {}",
                    target_id, err
                );
                return;
            }
        };
        if instances.is_empty() {
            let _ = self.db().mark_queued_as_waiting(target_id, now).await;
            return;
        }

        let offered_total = match self.db().count_offered(target_id).await {
            Ok(count) => count,
            Err(err) => {
                warn!(
                    "task_dispatcher.evaluate_target({}): count offered failed: {}",
                    target_id, err
                );
                return;
            }
        };
        let max_concurrency = target.registration.max_concurrency.max(1);
        let mut target_budget = max_concurrency.saturating_sub(offered_total);

        // Remaining per-instance budget: the instance's self-reported free
        // capacity minus offers already parked on it.
        let mut instance_budget: Vec<(String, u32)> = Vec::with_capacity(instances.len());
        for instance in &instances {
            let parked = self
                .db()
                .count_offered_for_instance(target_id, instance.instance_id.as_str())
                .await
                .unwrap_or(0);
            let budget = instance
                .available_capacity
                .min(instance.capacity.max(1))
                .saturating_sub(parked);
            instance_budget.push((instance.instance_id.clone(), budget));
        }

        let due = match self.db().list_assignable(target_id, EVAL_BATCH).await {
            Ok(due) => due,
            Err(err) => {
                warn!(
                    "task_dispatcher.evaluate_target({}): list assignable failed: {}",
                    target_id, err
                );
                return;
            }
        };

        let offer_lease_ms = target.registration.delivery_policy.offer_lease_ms.max(1_000);
        let selection = target.registration.delivery_policy.instance_selection;
        let mut assigned_any = false;

        for record in due {
            if target_budget == 0 {
                break;
            }
            let Some(slot) = pick_instance(
                &self.inner.rr_cursor,
                target_id,
                selection,
                &mut instance_budget,
            ) else {
                break;
            };
            let lease_expires_at = now + offer_lease_ms as i64;
            match self
                .db()
                .mark_offered(
                    record.dispatch_id.as_str(),
                    slot.as_str(),
                    lease_expires_at,
                    now,
                )
                .await
            {
                Ok(true) => {
                    target_budget -= 1;
                    assigned_any = true;
                    self.emit_transition(
                        record.dispatch_id.as_str(),
                        target_id,
                        record.status.to_string().as_str(),
                        "Offered",
                        Some(slot.as_str()),
                        None,
                        None,
                        now,
                    )
                    .await;
                    self.notify_target_channel(target_id, record.dispatch_id.as_str(), now)
                        .await;
                }
                Ok(false) => {
                    // Lost a race (canceled/expired concurrently) — give the
                    // budget back to the instance.
                    if let Some(entry) = instance_budget.iter_mut().find(|(id, _)| *id == slot) {
                        entry.1 += 1;
                    }
                }
                Err(err) => {
                    warn!(
                        "task_dispatcher.evaluate_target({}): mark_offered {} failed: {}",
                        target_id, record.dispatch_id, err
                    );
                }
            }
        }

        // Whatever could not be assigned this round waits explicitly.
        let _ = self.db().mark_queued_as_waiting(target_id, now).await;

        if assigned_any {
            // New offer leases became the nearest deadline.
            self.inner.maintenance_notify.notify_one();
        }
    }

    // ------------------------------------------------------------------
    // maintenance: deadlines + startup recovery + low-frequency sweep
    // ------------------------------------------------------------------

    /// One maintenance round at time `now`:
    /// expired instance leases -> drop; request `expires_at` -> Expired;
    /// expired offer leases -> requeue (IdempotentAccept, delivery budget
    /// permitting) or Uncertain (no contract); then re-evaluate every target
    /// with open handoffs.
    pub(crate) async fn run_maintenance_once(&self, now: i64) {
        match self.db().delete_expired_instances(now).await {
            Ok(expired) => {
                for (target_id, instance_id) in expired {
                    info!(
                        "task_dispatcher.maintenance: instance {}/{} lease expired",
                        target_id, instance_id
                    );
                }
            }
            Err(err) => warn!("task_dispatcher.maintenance: expire instances failed: {}", err),
        }

        // Request deadline first: a record that is both offer-lease-expired
        // and request-expired must end up Expired, not requeued.
        match self.db().list_request_expired(now).await {
            Ok(records) => {
                for record in records {
                    match self
                        .db()
                        .mark_expired(record.dispatch_id.as_str(), EXPIRE_DETAIL_REQUEST, now)
                        .await
                    {
                        Ok(true) => {
                            self.emit_transition(
                                record.dispatch_id.as_str(),
                                record.target_id.as_str(),
                                record.status.to_string().as_str(),
                                "Expired",
                                record.offer_instance_id.as_deref(),
                                Some(EXPIRE_DETAIL_REQUEST),
                                None,
                                now,
                            )
                            .await;
                        }
                        Ok(false) => {}
                        Err(err) => warn!(
                            "task_dispatcher.maintenance: expire {} failed: {}",
                            record.dispatch_id, err
                        ),
                    }
                }
            }
            Err(err) => warn!(
                "task_dispatcher.maintenance: list request-expired failed: {}",
                err
            ),
        }

        match self.db().list_expired_offers(now).await {
            Ok(records) => {
                for record in records {
                    self.recover_expired_offer(&record, now).await;
                }
            }
            Err(err) => warn!(
                "task_dispatcher.maintenance: list expired offers failed: {}",
                err
            ),
        }

        match self.db().list_targets_with_open_records().await {
            Ok(targets) => {
                for target_id in targets {
                    self.evaluate_target(target_id.as_str(), now).await;
                }
            }
            Err(err) => warn!(
                "task_dispatcher.maintenance: list open targets failed: {}",
                err
            ),
        }
    }

    /// Offer-lease recovery is handoff-protocol recovery, never a business
    /// retry: it reuses the same dispatch_id and envelope, and it is gated
    /// on the target's idempotency contract (doc §3.4 / §12).
    async fn recover_expired_offer(&self, record: &DispatchRecord, now: i64) {
        let contract = match self.load_target(record.target_id.as_str()).await {
            Ok(Some(target)) => target.registration.idempotency_contract,
            _ => IdempotencyContract::IdempotentAccept,
        };
        let max_deliveries = match self.load_target(record.target_id.as_str()).await {
            Ok(Some(target)) => target.registration.delivery_policy.max_offer_deliveries,
            _ => 10,
        };

        match contract {
            IdempotencyContract::IdempotentAccept => {
                if record.offer_delivery_count >= max_deliveries {
                    if let Ok(true) = self
                        .db()
                        .mark_expired(
                            record.dispatch_id.as_str(),
                            EXPIRE_DETAIL_DELIVERY_EXHAUSTED,
                            now,
                        )
                        .await
                    {
                        self.emit_transition(
                            record.dispatch_id.as_str(),
                            record.target_id.as_str(),
                            "Offered",
                            "Expired",
                            record.offer_instance_id.as_deref(),
                            Some(EXPIRE_DETAIL_DELIVERY_EXHAUSTED),
                            None,
                            now,
                        )
                        .await;
                    }
                } else if let Ok(true) = self
                    .db()
                    .requeue_expired_offer(record.dispatch_id.as_str(), now)
                    .await
                {
                    self.emit_transition(
                        record.dispatch_id.as_str(),
                        record.target_id.as_str(),
                        "Offered",
                        "Queued",
                        record.offer_instance_id.as_deref(),
                        Some("offer_lease_expired"),
                        None,
                        now,
                    )
                    .await;
                }
            }
            IdempotencyContract::None => {
                if let Ok(true) = self
                    .db()
                    .mark_uncertain_expired(record.dispatch_id.as_str(), now)
                    .await
                {
                    self.emit_transition(
                        record.dispatch_id.as_str(),
                        record.target_id.as_str(),
                        "Offered",
                        "Uncertain",
                        record.offer_instance_id.as_deref(),
                        Some("offer_lease_expired_no_idempotency_contract"),
                        None,
                        now,
                    )
                    .await;
                }
            }
        }
    }

    /// Startup recovery scan (doc §6.4): rebuild deadlines, recycle expired
    /// offers, expire overdue requests, re-evaluate open targets.
    pub async fn startup_recovery(&self) {
        self.run_maintenance_once(now_ms()).await;
    }

    /// Background deadline/backstop loop. Wakes on the earliest known
    /// deadline (offer lease / request expiry / instance lease), on
    /// mutation notifications, and at least every MAINTENANCE_MAX_SLEEP.
    pub fn spawn_maintenance_loop(&self) -> tokio::task::JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                let sleep_for = match service.db().next_deadline_ms().await {
                    Ok(Some(deadline)) => {
                        let delta = deadline.saturating_sub(now_ms());
                        Duration::from_millis(delta.max(0) as u64)
                            .clamp(MAINTENANCE_MIN_SLEEP, MAINTENANCE_MAX_SLEEP)
                    }
                    Ok(None) => MAINTENANCE_MAX_SLEEP,
                    Err(err) => {
                        warn!("task_dispatcher.maintenance: deadline query failed: {}", err);
                        MAINTENANCE_MAX_SLEEP
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(sleep_for) => {}
                    _ = service.inner.maintenance_notify.notified() => {}
                }
                service.run_maintenance_once(now_ms()).await;
            }
        })
    }
}

/// Pick an instance with remaining budget. RoundRobin rotates a per-target
/// cursor; LeastLoaded picks the largest remaining budget. Consumes one
/// budget unit from the chosen slot.
fn pick_instance(
    cursors: &StdMutex<HashMap<String, usize>>,
    target_id: &str,
    selection: InstanceSelection,
    budgets: &mut [(String, u32)],
) -> Option<String> {
    if budgets.iter().all(|(_, budget)| *budget == 0) {
        return None;
    }
    let index = match selection {
        InstanceSelection::RoundRobin => {
            let mut cursors = cursors.lock().unwrap();
            let cursor = cursors.entry(target_id.to_string()).or_insert(0);
            let len = budgets.len();
            let mut chosen = None;
            for step in 0..len {
                let candidate = (*cursor + step) % len;
                if budgets[candidate].1 > 0 {
                    chosen = Some(candidate);
                    *cursor = (candidate + 1) % len;
                    break;
                }
            }
            chosen?
        }
        InstanceSelection::LeastLoaded => {
            let (index, _) = budgets
                .iter()
                .enumerate()
                .max_by_key(|(_, (_, budget))| *budget)?;
            if budgets[index].1 == 0 {
                return None;
            }
            index
        }
    };
    budgets[index].1 -= 1;
    Some(budgets[index].0.clone())
}

// ---------------------------------------------------------------------------
// TaskDispatcherHandler
// ---------------------------------------------------------------------------

#[async_trait]
impl TaskDispatcherHandler for TaskDispatcherService {
    async fn handle_dispatch(
        &self,
        params: DispatchRequestParams,
        ctx: RPCContext,
    ) -> Result<DispatchSubmitResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let now = now_ms();

        let operation = params.operation.trim().to_string();
        if operation.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "operation is required".to_string(),
            ));
        }
        let idempotency_key = params.idempotency_key.trim().to_string();
        if idempotency_key.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "idempotency_key is required".to_string(),
            ));
        }

        // on_behalf_of: normal callers can only act as themselves;
        // zone-trusted callers may carry an already-authenticated business
        // user. Never an identity-laundering hop: only the *direct* caller's
        // trust grade counts.
        let requested_obo = params
            .on_behalf_of
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let on_behalf_of = if request_ctx.zone_trusted {
            requested_obo.unwrap_or(request_ctx.user_id.as_str()).to_string()
        } else {
            if let Some(requested) = requested_obo {
                if requested != request_ctx.user_id {
                    return Err(RPCErrors::NoPermission(format!(
                        "caller {} cannot dispatch on behalf of {}",
                        request_ctx.user_id, requested
                    )));
                }
            }
            request_ctx.user_id.clone()
        };

        let requested_target_id = params
            .target_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let input_digest = compute_input_digest(&params.input);

        // Idempotent replay: same key must describe the same immutable
        // request; then the existing record is returned without re-resolving
        // the route (target stickiness).
        let replay_check = |existing: &DispatchRecord| -> Result<DispatchSubmitResult> {
            let same = existing.operation == operation
                && existing.requested_target_id == requested_target_id
                && existing.auth.input_digest == input_digest
                && existing.auth.on_behalf_of == on_behalf_of;
            if !same {
                return Err(reason(format!(
                    "{}: idempotency_key {} was used with a different request",
                    DISPATCH_ERR_IDEMPOTENCY_CONFLICT, idempotency_key
                )));
            }
            Ok(DispatchSubmitResult {
                dispatch_id: existing.dispatch_id.clone(),
                target_id: existing.target_id.clone(),
                target_selection: existing.target_selection.clone(),
                status: existing.status,
            })
        };

        if let Some(existing) = self
            .db()
            .find_record_by_idempotency_key(
                request_ctx.user_id.as_str(),
                request_ctx.app_id.as_str(),
                idempotency_key.as_str(),
            )
            .await
            .map_err(|e| reason(e.to_string()))?
        {
            return replay_check(&existing);
        }

        // Deadline sanity applies to NEW records only — a replay whose
        // expires_at has since passed must still answer with the existing
        // (by now Expired) record above.
        if let Some(expires_at) = params.expires_at {
            if expires_at as i64 <= now {
                return Err(RPCErrors::ParseRequestError(
                    "expires_at is already in the past".to_string(),
                ));
            }
        }

        // Resolve and freeze the target (doc §5: resolve_target).
        let (target_row, target_selection) = match requested_target_id.as_deref() {
            Some(target_id) => {
                let target = self.load_target(target_id).await?.ok_or_else(|| {
                    reason(format!(
                        "{}: target {} is not registered",
                        DISPATCH_ERR_TARGET_NOT_REGISTERED, target_id
                    ))
                })?;
                (target, TargetSelection::Explicit)
            }
            None => {
                let route = self
                    .db()
                    .get_route(operation.as_str())
                    .await
                    .map_err(|e| reason(e.to_string()))?
                    .filter(|route| route.enabled)
                    .ok_or_else(|| {
                        reason(format!(
                            "{}: no enabled default route for operation {}",
                            DISPATCH_ERR_DEFAULT_TARGET_NOT_CONFIGURED, operation
                        ))
                    })?;
                let target = self
                    .load_target(route.default_target_id.as_str())
                    .await?
                    .ok_or_else(|| {
                        reason(format!(
                            "{}: default route target {} is not registered",
                            DISPATCH_ERR_TARGET_NOT_REGISTERED, route.default_target_id
                        ))
                    })?;
                (
                    target,
                    TargetSelection::DefaultRoute {
                        route_revision: route.revision,
                    },
                )
            }
        };
        if !target_row.enabled {
            return Err(reason(format!(
                "{}: target {} is disabled",
                DISPATCH_ERR_TARGET_DISABLED, target_row.registration.target_id
            )));
        }
        if !target_row.registration.supports_operation(operation.as_str()) {
            return Err(reason(format!(
                "{}: target {} does not support operation {}",
                DISPATCH_ERR_UNSUPPORTED_OPERATION,
                target_row.registration.target_id,
                operation
            )));
        }
        match target_row.registration.auth_policy {
            DispatchAuthPolicy::ZoneUsers => {}
            DispatchAuthPolicy::ZoneTrustedOnly => {
                Self::require_zone_trusted(&request_ctx, "dispatch to this target")?;
            }
        }

        // Approval gate (doc §7.1): auth_policy answered "may this caller
        // submit"; this answers "is the submission held for a human". Judged
        // on the DIRECT caller's token grade only — same principle as
        // on_behalf_of. The record is persisted either way; a held record is
        // never evaluated and never reaches the target.
        let held_for_approval = match target_row.registration.approval_policy {
            DispatchApprovalPolicy::Never => false,
            DispatchApprovalPolicy::InteractiveCallers => !Self::is_approval_admin(&request_ctx),
            DispatchApprovalPolicy::AllCallers => true,
        };
        let initial_status = if held_for_approval {
            DispatchStatus::PendingApproval
        } else {
            DispatchStatus::Queued
        };

        let dispatch_id = format!("dsp-{}", Uuid::new_v4());
        let record = DispatchRecord {
            dispatch_id: dispatch_id.clone(),
            requested_target_id: requested_target_id.clone(),
            target_id: target_row.registration.target_id.clone(),
            target_selection: target_selection.clone(),
            operation: operation.clone(),
            status: initial_status,
            input: params.input.clone(),
            auth: DispatchAuthEnvelope {
                requested_by_user: request_ctx.user_id.clone(),
                requested_by_app: request_ctx.app_id.clone(),
                on_behalf_of: on_behalf_of.clone(),
                zone_trusted_caller: request_ctx.zone_trusted,
                workflow_ref: params.workflow_ref.clone(),
                input_digest: input_digest.clone(),
                created_at: now as u64,
                expires_at: params.expires_at,
            },
            offer_instance_id: None,
            offer_lease_expires_at: None,
            offer_delivery_count: 0,
            target_task_id: None,
            reject_reason: None,
            approval: None,
            message: None,
            created_at: now as u64,
            updated_at: now as u64,
        };

        if let Err(err) = self
            .db()
            .insert_record(&record, idempotency_key.as_str())
            .await
        {
            if is_unique_violation(&err) {
                // Concurrent replay of the same key: fall back to the
                // already-inserted record.
                if let Some(existing) = self
                    .db()
                    .find_record_by_idempotency_key(
                        request_ctx.user_id.as_str(),
                        request_ctx.app_id.as_str(),
                        idempotency_key.as_str(),
                    )
                    .await
                    .map_err(|e| reason(e.to_string()))?
                {
                    return replay_check(&existing);
                }
            }
            return Err(reason(err.to_string()));
        }
        info!(
            "task_dispatcher.dispatch: {} op={} target={} selection={:?} status={} by={}/{} obo={}",
            dispatch_id,
            operation,
            record.target_id,
            target_selection,
            initial_status,
            request_ctx.user_id,
            request_ctx.app_id,
            record.auth.on_behalf_of
        );
        self.emit_transition(
            dispatch_id.as_str(),
            record.target_id.as_str(),
            "",
            initial_status.to_string().as_str(),
            None,
            None,
            None,
            now,
        )
        .await;

        if held_for_approval {
            // The gate sits BEFORE assignment: no evaluation, no offer, no
            // concurrency budget, no delivery count. Only the admin hint.
            self.notify_approvals_channel(record.target_id.as_str(), dispatch_id.as_str(), now)
                .await;
        } else {
            self.evaluate_target(record.target_id.as_str(), now).await;
        }
        if record.auth.expires_at.is_some() {
            self.inner.maintenance_notify.notify_one();
        }

        let current = self.load_record(dispatch_id.as_str()).await?;
        Ok(DispatchSubmitResult {
            dispatch_id: current.dispatch_id,
            target_id: current.target_id,
            target_selection: current.target_selection,
            status: current.status,
        })
    }

    async fn handle_get_dispatch(
        &self,
        req: GetDispatchReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord> {
        let request_ctx = self.authenticate(&ctx).await?;
        let record = self.load_record(req.dispatch_id.as_str()).await?;
        if !Self::can_view_record(&request_ctx, &record) {
            return Err(RPCErrors::NoPermission(
                "No permission to read dispatch record".to_string(),
            ));
        }
        Ok(record)
    }

    async fn handle_list_dispatches(
        &self,
        req: ListDispatchesReq,
        ctx: RPCContext,
    ) -> Result<Vec<DispatchRecord>> {
        let request_ctx = self.authenticate(&ctx).await?;
        let filter = RecordFilter {
            target_id: req.target_id,
            operation: req.operation,
            status: req.status,
            requested_by_user: req.requested_by_user,
            requested_by_app: req.requested_by_app,
            on_behalf_of: req.on_behalf_of,
            since_ms: req.since_ms.map(|v| v as i64),
            until_ms: req.until_ms.map(|v| v as i64),
            limit: req.limit.unwrap_or(DEFAULT_LIST_LIMIT).min(1_000),
        };
        let records = self
            .db()
            .list_records(&filter)
            .await
            .map_err(|e| reason(e.to_string()))?;
        Ok(records
            .into_iter()
            .filter(|record| Self::can_view_record(&request_ctx, record))
            .collect())
    }

    async fn handle_cancel_dispatch(
        &self,
        req: CancelDispatchReq,
        ctx: RPCContext,
    ) -> Result<CancelDispatchResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let record = self.load_record(req.dispatch_id.as_str()).await?;
        let allowed = request_ctx.zone_trusted
            || (record.auth.requested_by_user == request_ctx.user_id
                && record.auth.requested_by_app == request_ctx.app_id)
            || record.auth.on_behalf_of == request_ctx.user_id;
        if !allowed {
            return Err(RPCErrors::NoPermission(
                "No permission to cancel dispatch".to_string(),
            ));
        }

        match record.status {
            DispatchStatus::Accepted => {
                return Err(reason(format!(
                    "{}: dispatch {} is already accepted; cancel the business task via the target's own interface (target_task_id={:?})",
                    DISPATCH_ERR_ALREADY_ACCEPTED, record.dispatch_id, record.target_task_id
                )));
            }
            DispatchStatus::Uncertain => {
                return Err(reason(format!(
                    "{}: dispatch {} is Uncertain; use resolve_uncertain",
                    DISPATCH_ERR_UNCERTAIN_REQUIRES_RESOLVE, record.dispatch_id
                )));
            }
            DispatchStatus::Rejected | DispatchStatus::Expired | DispatchStatus::Canceled => {
                return Ok(CancelDispatchResult {
                    status: record.status,
                });
            }
            // PendingApproval: the submitter may withdraw a record that is
            // still waiting for a manual release.
            DispatchStatus::PendingApproval
            | DispatchStatus::Queued
            | DispatchStatus::WaitingForTarget
            | DispatchStatus::Offered => {}
        }

        let now = now_ms();
        let changed = self
            .db()
            .mark_canceled(record.dispatch_id.as_str(), now)
            .await
            .map_err(|e| reason(e.to_string()))?;
        let current = self.load_record(record.dispatch_id.as_str()).await?;
        if changed {
            self.emit_transition(
                record.dispatch_id.as_str(),
                record.target_id.as_str(),
                record.status.to_string().as_str(),
                "Canceled",
                record.offer_instance_id.as_deref(),
                Some("canceled_by_caller"),
                None,
                now,
            )
            .await;
            return Ok(CancelDispatchResult {
                status: DispatchStatus::Canceled,
            });
        }
        // Raced with accept/expiry — report where the record actually ended.
        if current.status == DispatchStatus::Accepted {
            return Err(reason(format!(
                "{}: dispatch {} was accepted concurrently",
                DISPATCH_ERR_ALREADY_ACCEPTED, current.dispatch_id
            )));
        }
        Ok(CancelDispatchResult {
            status: current.status,
        })
    }

    async fn handle_register_target(
        &self,
        req: RegisterTargetReq,
        ctx: RPCContext,
    ) -> Result<TargetRegistration> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "register_target")?;

        let mut registration = req.registration;
        registration.target_id = registration.target_id.trim().to_string();
        if registration.target_id.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "target_id is required".to_string(),
            ));
        }
        if registration.operations.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "operations must not be empty (closed, versioned list)".to_string(),
            ));
        }
        for descriptor in &registration.operations {
            let operation = descriptor.operation.trim();
            if operation.is_empty() || !operation.contains('/') {
                return Err(RPCErrors::ParseRequestError(format!(
                    "operation '{}' must carry a major version (e.g. name/v1)",
                    descriptor.operation
                )));
            }
        }
        if registration.max_concurrency == 0 {
            return Err(RPCErrors::ParseRequestError(
                "max_concurrency must be >= 1".to_string(),
            ));
        }

        // Owner binding comes from the verified token — payload claims are
        // overwritten. Updates must come from the same owner.
        if let Some(existing) = self.load_target(registration.target_id.as_str()).await? {
            if existing.owner_user_id != request_ctx.user_id
                || existing.owner_app_id != request_ctx.app_id
            {
                return Err(RPCErrors::NoPermission(format!(
                    "target {} is owned by {}/{}",
                    registration.target_id, existing.owner_user_id, existing.owner_app_id
                )));
            }
        }
        registration.owner_user_id = request_ctx.user_id.clone();
        registration.owner_app_id = request_ctx.app_id.clone();

        let now = now_ms();
        self.db()
            .upsert_target(&registration, now)
            .await
            .map_err(|e| reason(e.to_string()))?;
        info!(
            "task_dispatcher.register_target: {} owner={}/{} operations={:?} enabled={}",
            registration.target_id,
            registration.owner_user_id,
            registration.owner_app_id,
            registration
                .operations
                .iter()
                .map(|descriptor| descriptor.operation.as_str())
                .collect::<Vec<_>>(),
            registration.enabled
        );
        self.evaluate_target(registration.target_id.as_str(), now)
            .await;
        Ok(registration)
    }

    async fn handle_disable_target(&self, req: DisableTargetReq, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        let target = self
            .load_target(req.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", req.target_id)))?;
        Self::require_target_owner(&request_ctx, &target)?;
        self.db()
            .set_target_enabled(req.target_id.as_str(), false, now_ms())
            .await
            .map_err(|e| reason(e.to_string()))?;
        info!("task_dispatcher.disable_target: {}", req.target_id);
        Ok(())
    }

    async fn handle_attach_instance(
        &self,
        req: AttachInstanceReq,
        ctx: RPCContext,
    ) -> Result<AttachInstanceResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let target = self
            .load_target(req.target_id.as_str())
            .await?
            .ok_or_else(|| {
                reason(format!(
                    "{}: target {} is not registered",
                    DISPATCH_ERR_TARGET_NOT_REGISTERED, req.target_id
                ))
            })?;
        Self::require_target_owner(&request_ctx, &target)?;

        let now = now_ms();
        let lease_epoch = self
            .db()
            .next_lease_epoch(req.target_id.as_str())
            .await
            .map_err(|e| reason(e.to_string()))?
            .ok_or_else(|| reason(format!("target {} not found", req.target_id)))?;
        let capacity = req.capacity.max(1);
        let instance = TargetInstance {
            target_id: req.target_id.clone(),
            instance_id: format!("ins-{}", Uuid::new_v4()),
            lease_epoch: lease_epoch.max(0) as u64,
            lease_expires_at: (now + INSTANCE_LEASE_TTL_MS as i64) as u64,
            capacity,
            available_capacity: capacity,
        };
        self.db()
            .insert_instance(&instance, now)
            .await
            .map_err(|e| reason(e.to_string()))?;
        info!(
            "task_dispatcher.attach_instance: target={} instance={} epoch={}",
            instance.target_id, instance.instance_id, instance.lease_epoch
        );

        self.evaluate_target(req.target_id.as_str(), now).await;
        self.inner.maintenance_notify.notify_one();

        Ok(AttachInstanceResult {
            instance_id: instance.instance_id,
            lease_epoch: instance.lease_epoch,
            lease_ttl_ms: INSTANCE_LEASE_TTL_MS,
            target_key: task_dispatcher_target_key(req.target_id.as_str()),
        })
    }

    async fn handle_renew_instance(
        &self,
        req: RenewInstanceReq,
        ctx: RPCContext,
    ) -> Result<RenewInstanceResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let target = self
            .load_target(req.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", req.target_id)))?;
        Self::require_target_owner(&request_ctx, &target)?;

        let now = now_ms();
        let renewed = self
            .db()
            .renew_instance(
                req.target_id.as_str(),
                req.instance_id.as_str(),
                req.lease_epoch,
                now + INSTANCE_LEASE_TTL_MS as i64,
                req.available_capacity,
                now,
            )
            .await
            .map_err(|e| reason(e.to_string()))?;
        if !renewed {
            return Err(reason(format!(
                "{}: instance {} (epoch {}) is not the current lease of target {}",
                DISPATCH_ERR_STALE_INSTANCE, req.instance_id, req.lease_epoch, req.target_id
            )));
        }

        // Fresh capacity may unblock waiting records.
        self.evaluate_target(req.target_id.as_str(), now).await;

        let has_due = self
            .db()
            .has_due_for_instance(req.target_id.as_str(), req.instance_id.as_str(), now)
            .await
            .unwrap_or(false);
        Ok(RenewInstanceResult {
            lease_ttl_ms: INSTANCE_LEASE_TTL_MS,
            has_due,
        })
    }

    async fn handle_detach_instance(&self, req: DetachInstanceReq, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        let target = self
            .load_target(req.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", req.target_id)))?;
        Self::require_target_owner(&request_ctx, &target)?;

        let now = now_ms();
        let Some(instance) = self
            .db()
            .get_instance(req.target_id.as_str(), req.instance_id.as_str())
            .await
            .map_err(|e| reason(e.to_string()))?
        else {
            return Ok(()); // already gone — idempotent
        };
        if instance.lease_epoch != req.lease_epoch {
            // A stale epoch cannot force-detach the current instance.
            return Ok(());
        }

        self.db()
            .delete_instance(req.target_id.as_str(), req.instance_id.as_str())
            .await
            .map_err(|e| reason(e.to_string()))?;
        info!(
            "task_dispatcher.detach_instance: target={} instance={}",
            req.target_id, req.instance_id
        );

        // Recycle the instance's in-flight offers right away — still gated
        // on the idempotency contract, exactly like lease-expiry recovery.
        let contract = target.registration.idempotency_contract;
        let max_deliveries = target.registration.delivery_policy.max_offer_deliveries;
        let offers = self
            .db()
            .list_offered_for_instance(req.target_id.as_str(), req.instance_id.as_str())
            .await
            .map_err(|e| reason(e.to_string()))?;
        for record in offers {
            match contract {
                IdempotencyContract::IdempotentAccept => {
                    if record.offer_delivery_count >= max_deliveries {
                        if let Ok(true) = self
                            .db()
                            .mark_expired(
                                record.dispatch_id.as_str(),
                                EXPIRE_DETAIL_DELIVERY_EXHAUSTED,
                                now,
                            )
                            .await
                        {
                            self.emit_transition(
                                record.dispatch_id.as_str(),
                                record.target_id.as_str(),
                                "Offered",
                                "Expired",
                                Some(req.instance_id.as_str()),
                                Some(EXPIRE_DETAIL_DELIVERY_EXHAUSTED),
                                None,
                                now,
                            )
                            .await;
                        }
                    } else if let Ok(true) = self
                        .db()
                        .requeue_offer_for_instance(
                            record.dispatch_id.as_str(),
                            req.instance_id.as_str(),
                            now,
                        )
                        .await
                    {
                        self.emit_transition(
                            record.dispatch_id.as_str(),
                            record.target_id.as_str(),
                            "Offered",
                            "Queued",
                            Some(req.instance_id.as_str()),
                            Some("instance_detached"),
                            None,
                            now,
                        )
                        .await;
                    }
                }
                IdempotencyContract::None => {
                    if let Ok(true) = self
                        .db()
                        .mark_uncertain_for_instance(
                            record.dispatch_id.as_str(),
                            req.instance_id.as_str(),
                            now,
                        )
                        .await
                    {
                        self.emit_transition(
                            record.dispatch_id.as_str(),
                            record.target_id.as_str(),
                            "Offered",
                            "Uncertain",
                            Some(req.instance_id.as_str()),
                            Some("instance_detached_no_idempotency_contract"),
                            None,
                            now,
                        )
                        .await;
                    }
                }
            }
        }

        self.evaluate_target(req.target_id.as_str(), now).await;
        Ok(())
    }

    async fn handle_claim_next(
        &self,
        req: ClaimNextReq,
        ctx: RPCContext,
    ) -> Result<Vec<DispatchRecord>> {
        let request_ctx = self.authenticate(&ctx).await?;
        let target = self
            .load_target(req.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", req.target_id)))?;
        Self::require_target_owner(&request_ctx, &target)?;

        let now = now_ms();
        self.validate_instance(
            req.target_id.as_str(),
            req.instance_id.as_str(),
            req.lease_epoch,
            now,
        )
        .await?;

        // claim_next is a transport and an evaluation trigger — never a
        // cross-instance selection point.
        self.evaluate_target(req.target_id.as_str(), now).await;
        self.db()
            .claim_for_instance(
                req.target_id.as_str(),
                req.instance_id.as_str(),
                now,
                req.max,
            )
            .await
            .map_err(|e| reason(e.to_string()))
    }

    async fn handle_accept_dispatch(
        &self,
        req: AcceptDispatchReq,
        ctx: RPCContext,
    ) -> Result<AcceptDispatchResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        let record = self.load_record(req.dispatch_id.as_str()).await?;
        let target = self
            .load_target(record.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", record.target_id)))?;
        Self::require_target_owner(&request_ctx, &target)?;

        let now = now_ms();
        self.validate_instance(
            record.target_id.as_str(),
            req.instance_id.as_str(),
            req.lease_epoch,
            now,
        )
        .await?;

        // Two passes cover accept racing with cancel/expiry: the second
        // read reports the final state.
        for _ in 0..2 {
            let current = self.load_record(req.dispatch_id.as_str()).await?;
            match current.status {
                DispatchStatus::Accepted => {
                    return if current.target_task_id == Some(req.target_task_id) {
                        // ACK-loss replay: same dispatch_id, same task.
                        Ok(AcceptDispatchResult {
                            accepted: true,
                            status: DispatchStatus::Accepted,
                            message: None,
                        })
                    } else {
                        Err(reason(format!(
                            "dispatch {} already accepted with target_task_id {:?}, got {}",
                            current.dispatch_id, current.target_task_id, req.target_task_id
                        )))
                    };
                }
                DispatchStatus::Canceled | DispatchStatus::Expired | DispatchStatus::Rejected => {
                    // The target must cancel the local task it just created.
                    return Ok(AcceptDispatchResult {
                        accepted: false,
                        status: current.status,
                        message: current.message.clone(),
                    });
                }
                DispatchStatus::PendingApproval => {
                    // The approval gate sits before assignment: an unreleased
                    // record must never be receivable — not even by the
                    // target owner, not through any late-accept path. (The
                    // store-level guard excludes it too; this is the explicit
                    // protocol answer.)
                    return Err(reason(format!(
                        "{}: dispatch {} awaits manual approval; targets cannot accept it",
                        DISPATCH_ERR_PENDING_APPROVAL, current.dispatch_id
                    )));
                }
                DispatchStatus::Offered
                | DispatchStatus::Queued
                | DispatchStatus::WaitingForTarget
                | DispatchStatus::Uncertain => {
                    // Late accepts from Queued/WaitingForTarget (lease
                    // expired, then requeued) and Uncertain (the target is
                    // the authoritative business resolver) converge here —
                    // idempotent accept keeps redelivery loops closed.
                    let changed = self
                        .db()
                        .mark_accepted(req.dispatch_id.as_str(), req.target_task_id, now)
                        .await
                        .map_err(|e| reason(e.to_string()))?;
                    if changed {
                        info!(
                            "task_dispatcher.accept: {} -> Accepted(task {}) by {}",
                            req.dispatch_id, req.target_task_id, req.instance_id
                        );
                        self.emit_transition(
                            req.dispatch_id.as_str(),
                            record.target_id.as_str(),
                            current.status.to_string().as_str(),
                            "Accepted",
                            Some(req.instance_id.as_str()),
                            None,
                            Some(req.target_task_id),
                            now,
                        )
                        .await;
                        // Accepted frees target concurrency budget.
                        self.evaluate_target(record.target_id.as_str(), now).await;
                        return Ok(AcceptDispatchResult {
                            accepted: true,
                            status: DispatchStatus::Accepted,
                            message: None,
                        });
                    }
                    // Raced — loop once more to report the winner.
                }
            }
        }
        let current = self.load_record(req.dispatch_id.as_str()).await?;
        Ok(AcceptDispatchResult {
            accepted: current.status == DispatchStatus::Accepted
                && current.target_task_id == Some(req.target_task_id),
            status: current.status,
            message: current.message,
        })
    }

    async fn handle_reject_dispatch(&self, req: RejectDispatchReq, ctx: RPCContext) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        let record = self.load_record(req.dispatch_id.as_str()).await?;
        let target = self
            .load_target(record.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", record.target_id)))?;
        Self::require_target_owner(&request_ctx, &target)?;

        let now = now_ms();
        self.validate_instance(
            record.target_id.as_str(),
            req.instance_id.as_str(),
            req.lease_epoch,
            now,
        )
        .await?;

        match record.status {
            DispatchStatus::Rejected => return Ok(()), // idempotent
            DispatchStatus::Canceled | DispatchStatus::Expired => return Ok(()),
            DispatchStatus::Accepted => {
                return Err(reason(format!(
                    "{}: dispatch {} is already accepted",
                    DISPATCH_ERR_ALREADY_ACCEPTED, record.dispatch_id
                )));
            }
            DispatchStatus::Uncertain => {
                return Err(reason(format!(
                    "{}: dispatch {} is Uncertain; use resolve_uncertain",
                    DISPATCH_ERR_UNCERTAIN_REQUIRES_RESOLVE, record.dispatch_id
                )));
            }
            DispatchStatus::PendingApproval => {
                // Unreleased records are invisible to the receive path;
                // reject is a receive-path verb (deny_dispatch is the
                // admin-side refusal).
                return Err(reason(format!(
                    "{}: dispatch {} awaits manual approval; targets cannot reject it",
                    DISPATCH_ERR_PENDING_APPROVAL, record.dispatch_id
                )));
            }
            DispatchStatus::Queued | DispatchStatus::WaitingForTarget | DispatchStatus::Offered => {}
        }

        let changed = self
            .db()
            .mark_rejected(
                req.dispatch_id.as_str(),
                req.reason,
                req.detail.clone(),
                now,
            )
            .await
            .map_err(|e| reason(e.to_string()))?;
        if changed {
            info!(
                "task_dispatcher.reject: {} -> Rejected({}) by {}",
                req.dispatch_id, req.reason, req.instance_id
            );
            self.emit_transition(
                req.dispatch_id.as_str(),
                record.target_id.as_str(),
                record.status.to_string().as_str(),
                "Rejected",
                Some(req.instance_id.as_str()),
                Some(req.reason.as_str()),
                None,
                now,
            )
            .await;
            self.evaluate_target(record.target_id.as_str(), now).await;
        }
        Ok(())
    }

    async fn handle_approve_dispatch(
        &self,
        req: ApproveDispatchReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord> {
        let request_ctx = self.authenticate(&ctx).await?;
        // Same predicate as the InteractiveCallers exemption — deliberately.
        // Neither target ownership nor being the submitter grants approval
        // rights (a registrant must not release its own gate).
        Self::require_approval_admin(&request_ctx, "approve_dispatch")?;

        let record = self.load_record(req.dispatch_id.as_str()).await?;
        if record.status != DispatchStatus::PendingApproval {
            // Idempotent on an already-released record; anything else is a
            // hard error (approve never revives terminal records).
            let already_approved = record
                .approval
                .as_ref()
                .map(|approval| approval.decision == ApprovalDecision::Approved)
                .unwrap_or(false);
            if already_approved {
                return Ok(record);
            }
            return Err(reason(format!(
                "{}: dispatch {} is {} (approve requires PendingApproval)",
                DISPATCH_ERR_NOT_PENDING_APPROVAL, record.dispatch_id, record.status
            )));
        }

        let now = now_ms();
        let approval = DispatchApproval {
            decision: ApprovalDecision::Approved,
            decided_by_user: request_ctx.user_id.clone(),
            decided_by_app: request_ctx.app_id.clone(),
            decided_at: now as u64,
            note: req.note.clone(),
        };
        let changed = self
            .db()
            .approve_pending(req.dispatch_id.as_str(), &approval, now)
            .await
            .map_err(|e| reason(e.to_string()))?;
        if changed {
            info!(
                "task_dispatcher.approve: {} released by {}/{}",
                req.dispatch_id, request_ctx.user_id, request_ctx.app_id
            );
            self.emit_transition(
                req.dispatch_id.as_str(),
                record.target_id.as_str(),
                "PendingApproval",
                "Queued",
                None,
                Some("approval_granted"),
                None,
                now,
            )
            .await;
            // Release means normal centralized assignment — nothing manual
            // about instance selection, target or envelope.
            self.evaluate_target(record.target_id.as_str(), now).await;
        }
        // !changed: lost a race against cancel/expiry — report the actual
        // final state (an approved replay returns above on the next call).
        self.load_record(req.dispatch_id.as_str()).await
    }

    async fn handle_deny_dispatch(
        &self,
        req: DenyDispatchReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_approval_admin(&request_ctx, "deny_dispatch")?;

        let record = self.load_record(req.dispatch_id.as_str()).await?;
        if record.status != DispatchStatus::PendingApproval {
            let already_denied = record.status == DispatchStatus::Rejected
                && record
                    .approval
                    .as_ref()
                    .map(|approval| approval.decision == ApprovalDecision::Denied)
                    .unwrap_or(false);
            if already_denied {
                return Ok(record);
            }
            return Err(reason(format!(
                "{}: dispatch {} is {} (deny requires PendingApproval)",
                DISPATCH_ERR_NOT_PENDING_APPROVAL, record.dispatch_id, record.status
            )));
        }

        let now = now_ms();
        let approval = DispatchApproval {
            decision: ApprovalDecision::Denied,
            decided_by_user: request_ctx.user_id.clone(),
            decided_by_app: request_ctx.app_id.clone(),
            decided_at: now as u64,
            note: req.note.clone(),
        };
        let changed = self
            .db()
            .deny_pending(req.dispatch_id.as_str(), &approval, now)
            .await
            .map_err(|e| reason(e.to_string()))?;
        if changed {
            info!(
                "task_dispatcher.deny: {} refused by {}/{}",
                req.dispatch_id, request_ctx.user_id, request_ctx.app_id
            );
            self.emit_transition(
                req.dispatch_id.as_str(),
                record.target_id.as_str(),
                "PendingApproval",
                "Rejected",
                None,
                Some(DispatchRejectReason::ApprovalDenied.as_str()),
                None,
                now,
            )
            .await;
        }
        self.load_record(req.dispatch_id.as_str()).await
    }

    async fn handle_resolve_uncertain(
        &self,
        req: ResolveUncertainReq,
        ctx: RPCContext,
    ) -> Result<DispatchRecord> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "resolve_uncertain")?;

        let record = self.load_record(req.dispatch_id.as_str()).await?;
        let now = now_ms();
        let (task_binding, to_status, detail) = match &req.resolution {
            UncertainResolution::Accepted { target_task_id } => {
                (Some(*target_task_id), "Accepted", "resolved_as_accepted")
            }
            UncertainResolution::Canceled => (None, "Canceled", "resolved_as_canceled"),
        };

        if record.status != DispatchStatus::Uncertain {
            // Idempotent replay of the same resolution is fine.
            if record.status == DispatchStatus::Accepted && record.target_task_id == task_binding {
                return Ok(record);
            }
            if record.status == DispatchStatus::Canceled && task_binding.is_none() {
                return Ok(record);
            }
            return Err(reason(format!(
                "dispatch {} is {} (resolve_uncertain requires Uncertain)",
                record.dispatch_id, record.status
            )));
        }

        let changed = self
            .db()
            .resolve_uncertain(req.dispatch_id.as_str(), task_binding, now)
            .await
            .map_err(|e| reason(e.to_string()))?;
        if changed {
            self.emit_transition(
                req.dispatch_id.as_str(),
                record.target_id.as_str(),
                "Uncertain",
                to_status,
                None,
                Some(detail),
                task_binding,
                now,
            )
            .await;
        }
        self.load_record(req.dispatch_id.as_str()).await
    }

    async fn handle_list_targets(&self, ctx: RPCContext) -> Result<Vec<TargetRegistration>> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "list_targets")?;
        let targets = self
            .db()
            .list_targets()
            .await
            .map_err(|e| reason(e.to_string()))?;
        Ok(targets.into_iter().map(|row| row.registration).collect())
    }

    async fn handle_get_target(
        &self,
        req: GetTargetReq,
        ctx: RPCContext,
    ) -> Result<TargetRegistration> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "get_target")?;
        let target = self
            .load_target(req.target_id.as_str())
            .await?
            .ok_or_else(|| reason(format!("target {} not found", req.target_id)))?;
        Ok(target.registration)
    }

    async fn handle_set_operation_route(
        &self,
        req: SetOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<OperationRoute> {
        let request_ctx = self.authenticate(&ctx).await?;
        // Route config is system-admin surface. Zone-trusted is the coarsest
        // gate available today; a finer "system config admin" capability can
        // tighten this without protocol changes. Target owners get no route
        // rights from registration alone (verified in tests).
        Self::require_zone_trusted(&request_ctx, "set_operation_route")?;

        let operation = req.operation.trim().to_string();
        if operation.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "operation is required".to_string(),
            ));
        }
        let target = self
            .load_target(req.default_target_id.as_str())
            .await?
            .ok_or_else(|| {
                reason(format!(
                    "{}: target {} is not registered",
                    DISPATCH_ERR_TARGET_NOT_REGISTERED, req.default_target_id
                ))
            })?;
        if !target.enabled {
            return Err(reason(format!(
                "{}: target {} is disabled",
                DISPATCH_ERR_TARGET_DISABLED, req.default_target_id
            )));
        }
        if !target.registration.supports_operation(operation.as_str()) {
            return Err(reason(format!(
                "{}: target {} does not declare operation {}",
                DISPATCH_ERR_UNSUPPORTED_OPERATION, req.default_target_id, operation
            )));
        }

        let route = self
            .db()
            .upsert_route(operation.as_str(), req.default_target_id.as_str(), now_ms())
            .await
            .map_err(|e| reason(e.to_string()))?;
        info!(
            "task_dispatcher.set_operation_route: {} -> {} (revision {})",
            route.operation, route.default_target_id, route.revision
        );
        Ok(route)
    }

    async fn handle_disable_operation_route(
        &self,
        req: DisableOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "disable_operation_route")?;
        let changed = self
            .db()
            .set_route_enabled(req.operation.as_str(), false, now_ms())
            .await
            .map_err(|e| reason(e.to_string()))?;
        if !changed {
            return Err(reason(format!(
                "operation route {} not found",
                req.operation
            )));
        }
        Ok(())
    }

    async fn handle_get_operation_route(
        &self,
        req: GetOperationRouteReq,
        ctx: RPCContext,
    ) -> Result<OperationRouteView> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "get_operation_route")?;
        let route = self
            .db()
            .get_route(req.operation.as_str())
            .await
            .map_err(|e| reason(e.to_string()))?
            .ok_or_else(|| reason(format!("operation route {} not found", req.operation)))?;
        let target_valid = self.route_target_valid(&route).await;
        Ok(OperationRouteView {
            route,
            target_valid,
        })
    }

    async fn handle_list_operation_routes(
        &self,
        ctx: RPCContext,
    ) -> Result<Vec<OperationRouteView>> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "list_operation_routes")?;
        let routes = self
            .db()
            .list_routes()
            .await
            .map_err(|e| reason(e.to_string()))?;
        let mut views = Vec::with_capacity(routes.len());
        for route in routes {
            let target_valid = self.route_target_valid(&route).await;
            views.push(OperationRouteView {
                route,
                target_valid,
            });
        }
        Ok(views)
    }
}

impl TaskDispatcherService {
    #[cfg(test)]
    pub(crate) fn kevent_client_for_tests(&self) -> KEventClient {
        self.inner.kevent_client.clone()
    }

    #[cfg(test)]
    pub(crate) fn db_for_tests(&self) -> &DispatchDb {
        &self.inner.db
    }

    async fn route_target_valid(&self, route: &OperationRoute) -> bool {
        match self.db().get_target(route.default_target_id.as_str()).await {
            Ok(Some(target)) => {
                target.enabled
                    && target
                        .registration
                        .supports_operation(route.operation.as_str())
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP mount
// ---------------------------------------------------------------------------

pub struct TaskDispatcherHttpServer<T: TaskDispatcherHandler> {
    rpc_handler: TaskDispatcherServerHandler<T>,
}

impl<T: TaskDispatcherHandler> TaskDispatcherHttpServer<T> {
    pub fn new(handler: T) -> Self {
        Self {
            rpc_handler: TaskDispatcherServerHandler::new(handler),
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

/// Open the dispatcher store, run startup recovery, spawn the maintenance
/// loop and return the mountable service. TaskMgr must keep working when
/// this fails — the caller logs and continues.
pub async fn start_task_dispatcher(
    token_verifier: Arc<dyn SessionTokenVerifier>,
) -> Result<TaskDispatcherService> {
    let db = DispatchDb::open_from_service_spec()
        .await
        .map_err(RPCErrors::ReasonError)?;
    info!("task-dispatcher database initialized");

    let kevent_client = get_buckyos_api_runtime()
        .map_err(|err| reason(format!("api runtime unavailable: {}", err)))?
        .get_kevent_client()
        .await?;

    let service = TaskDispatcherService::new(db, kevent_client, token_verifier);
    service.startup_recovery().await;
    service.spawn_maintenance_loop();
    Ok(service)
}
