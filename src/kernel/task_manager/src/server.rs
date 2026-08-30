//! TaskMgr 2.0 Task Core service: command-style state writes, control
//! requests, tree policy and permission computation (doc §12/§13).
//!
//! Layering: `task_store` owns the transactional invariants (CAS, one-shot
//! result, epoch fencing), `acl` owns policy computation, and this module
//! owns authentication, per-command authorization, protocol validation and
//! post-commit KEvent fan-out.

use crate::acl::{
    compute_permission, summarize_task, trim_task, Principal, TaskPermission,
    SYSTEM_ROLE_ZONE_TRUSTED,
};
use crate::json_schema::validate_json_schema;
use crate::task_store::{now_ms, CreateTaskArgs, MutationOutcome, TaskStore};
use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, Runner, ServerError, ServerErrorCode,
    ServerResult, StreamInfo,
};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Recursive control requests refuse to walk unbounded trees (doc §15.8).
const MAX_RECURSIVE_CONTROL_NODES: usize = 512;

// The built-in schema ids (`RAW_TASK_SCHEMA_ID`, `HUMAN_APPROVAL_SCHEMA_ID`,
// ...) and their definitions live in buckyos-api next to the `TaskDataType`
// catalog they were migrated from; this module seeds and enforces them.

/// The caller identity resolved from the *verified* session token.
///
/// `zone_trusted` marks Verify Hub sessions that preserve a device/service principal.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub user_id: String,
    /// Kind-aware `AuthTarget::canonical_key()` used for ActorRef identity.
    pub app_id: String,
    /// Bare app/service id used for executor and schema bindings.
    pub executor_app_id: String,
    pub app_instance_id: Option<String>,
    pub authorization_id: String,
    pub zone_trusted: bool,
    pub sudo: bool,
}

impl RequestContext {
    pub fn principal(&self) -> Principal {
        let mut roles = Vec::new();
        if self.zone_trusted {
            roles.push(SYSTEM_ROLE_ZONE_TRUSTED.to_string());
        }
        Principal {
            user_id: self.user_id.clone(),
            app_id: self.app_id.clone(),
            executor_app_id: self.executor_app_id.clone(),
            authorization_id: self.authorization_id.clone(),
            roles,
            sudo: self.sudo,
            view_gate: Default::default(),
        }
    }

    pub fn actor_ref(&self) -> ActorRef {
        ActorRef {
            user_id: self.user_id.clone(),
            app_id: self.app_id.clone(),
            app_instance_id: self.app_instance_id.clone(),
        }
    }
}

/// Verifies the raw session token of a request. Production delegates to
/// buckyos-api's standard trusted-session-token verifier; tests inject a
/// verifier with a fixed key.
#[async_trait]
pub trait SessionTokenVerifier: Send + Sync {
    async fn verify(&self, token: &str) -> Result<RPCSessionToken>;
}

pub struct RuntimeSessionTokenVerifier;

pub(crate) fn token_is_zone_trusted(token: &RPCSessionToken) -> bool {
    matches!(
        validate_verify_hub_token_claims(token, TokenUse::Session)
            .map(|claims| claims.principal_kind),
        Ok(TokenPrincipalKind::Device | TokenPrincipalKind::System)
    )
}

#[async_trait]
impl SessionTokenVerifier for RuntimeSessionTokenVerifier {
    async fn verify(&self, token: &str) -> Result<RPCSessionToken> {
        get_buckyos_api_runtime()?
            .verify_trusted_session_token(token)
            .await
    }
}

#[derive(Clone)]
pub struct TaskManagerService {
    store: Arc<TaskStore>,
    kevent_client: KEventClient,
    token_verifier: Arc<dyn SessionTokenVerifier>,
}

impl TaskManagerService {
    pub fn new(
        store: Arc<TaskStore>,
        kevent_client: KEventClient,
        token_verifier: Arc<dyn SessionTokenVerifier>,
    ) -> Self {
        TaskManagerService {
            store,
            kevent_client,
            token_verifier,
        }
    }

    #[cfg(test)]
    pub(crate) fn store(&self) -> Arc<TaskStore> {
        self.store.clone()
    }

    /// Seed the first-party schema catalog so a clean zone can create the
    /// production tasks (workflow, opendan, aicc, app install, ...) before any
    /// service has published a contract of its own.
    ///
    /// Per-entry failures are logged and skipped rather than aborting startup:
    /// one bad contract must not take the whole Task Core offline, and the
    /// affected schema simply degrades to `task_schema_not_found` on create.
    pub async fn ensure_builtin_schemas(&self) -> Result<()> {
        for definition in builtin_task_schemas() {
            if let Err(err) = self.store.register_schema(&definition).await {
                error!(
                    "task-manager: seeding built-in schema {} failed: {:?}",
                    definition.schema_id, err
                );
            }
        }
        Ok(())
    }

    /// Resolve the caller identity from the request's session token.
    /// Fail closed: no token / bad signature => NoPermission. Identity is
    /// never taken from the request payload.
    async fn authenticate(&self, ctx: &RPCContext) -> Result<RequestContext> {
        let token = ctx
            .token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                RPCErrors::NoPermission("task-manager requires a session token".to_string())
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
        if user_id.trim().is_empty() {
            return Err(RPCErrors::InvalidToken(
                "session token has an empty subject".to_string(),
            ));
        }
        if app_id.trim().is_empty() {
            return Err(RPCErrors::InvalidToken(
                "session token has an empty app id".to_string(),
            ));
        }
        let zone_trusted = token_is_zone_trusted(&verified);
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
            return Err(task_mgr_error(
                TASK_ERR_PERMISSION_DENIED,
                format!("{} requires a zone-trusted service identity", what),
            ));
        }
        Ok(())
    }

    async fn publish_outcome(&self, outcome: &MutationOutcome) {
        let event = &outcome.event;
        let payload = json!({
            "event_id": event.event_id,
            "task_id": event.task_id,
            "root_id": event.root_id,
            "revision": event.task_revision,
            "event_type": event.event_type,
            "actor": event.actor,
            "phase": outcome.task.phase,
            "outcome": outcome.task.outcome,
            "wait_reason": outcome.task.wait_reason.as_ref().map(|r| &r.kind),
            "pending_control": outcome.task.pending_control.as_ref().map(|c| c.action),
            "created_at": event.created_at,
        });
        let task_path = task_mgr_task_event_path(&event.task_id);
        if let Err(err) = self
            .kevent_client
            .pub_event(&task_path, payload.clone())
            .await
        {
            warn!(
                "task_mgr.publish event failed: path={} err={}",
                task_path, err
            );
        }
        if event.root_id != event.task_id {
            let tree_path = task_mgr_tree_event_path(&event.root_id);
            if let Err(err) = self.kevent_client.pub_event(&tree_path, payload).await {
                warn!(
                    "task_mgr.publish tree event failed: path={} err={}",
                    tree_path, err
                );
            }
        }
    }

    async fn load_task(&self, task_id: &str) -> Result<Task> {
        self.store
            .get_task(task_id)
            .await?
            .ok_or_else(|| task_mgr_error(TASK_ERR_NOT_FOUND, task_id))
    }

    /// Load + compute permission; absent ReadMeta uniformly reports
    /// `task_not_found` so callers cannot enumerate invisible tasks.
    async fn load_visible(
        &self,
        request_ctx: &RequestContext,
        task_id: &str,
    ) -> Result<(Task, TaskPermission)> {
        let task = self.load_task(task_id).await?;
        let principal = request_ctx.principal();
        let permission = compute_permission(&self.store, &principal, &task).await?;
        if !permission.visible() {
            return Err(task_mgr_error(TASK_ERR_NOT_FOUND, task_id));
        }
        Ok((task, permission))
    }

    async fn require_action(
        &self,
        request_ctx: &RequestContext,
        task_id: &str,
        action: TaskAction,
    ) -> Result<(Task, TaskPermission)> {
        let (task, permission) = self.load_visible(request_ctx, task_id).await?;
        if !permission.allows(action) {
            return Err(task_mgr_error(
                TASK_ERR_PERMISSION_DENIED,
                format!("missing {:?} on task {}", action, task_id),
            ));
        }
        Ok((task, permission))
    }

    /// Resolve + validate the frozen schema for a create request.
    async fn resolve_schema(
        &self,
        schema_id: &str,
        schema_version: Option<u32>,
        input: &Value,
        executor_kind: TaskExecutorKind,
    ) -> Result<TaskSchemaDefinition> {
        let schema = self.store.get_schema(schema_id, schema_version).await?;
        if !schema.enabled {
            return Err(task_mgr_error(
                TASK_ERR_SCHEMA_NOT_FOUND,
                format!("schema {} is disabled", schema_id),
            ));
        }
        if !schema.allowed_executor_kinds.contains(&executor_kind) {
            return Err(task_mgr_error(
                TASK_ERR_INPUT_SCHEMA_MISMATCH,
                format!(
                    "schema {} does not allow executor kind {:?}",
                    schema_id, executor_kind
                ),
            ));
        }
        validate_json_schema(&schema.input_schema, input)
            .map_err(|violation| task_mgr_error(TASK_ERR_INPUT_SCHEMA_MISMATCH, violation))?;
        Ok(schema)
    }

    async fn resolve_storage_domain(
        &self,
        requested: Option<StorageDomain>,
        parent_id: Option<&str>,
        schema: &TaskSchemaDefinition,
        creator: &ActorRef,
    ) -> Result<StorageDomain> {
        if let Some(parent_id) = parent_id {
            let parent = self.load_task(parent_id).await?;
            if let Some(requested) = requested {
                if requested != parent.storage_domain {
                    return Err(task_mgr_error(
                        TASK_ERR_STORAGE_DOMAIN_CONFLICT,
                        format!(
                            "parent {} is in {}, requested {}",
                            parent_id, parent.storage_domain, requested
                        ),
                    ));
                }
            }
            return Ok(parent.storage_domain);
        }
        if let Some(requested) = requested {
            return Ok(requested);
        }
        if !schema.schema_id.trim().is_empty() {
            return Ok(schema.default_storage_domain);
        }
        Ok(if creator.app_id.starts_with("system:") {
            StorageDomain::System
        } else {
            StorageDomain::User
        })
    }

    fn verify_app_runner_write(
        request_ctx: &RequestContext,
        task: &Task,
        envelope: &RunnerWriteEnvelope,
    ) -> Result<()> {
        let TaskExecutor::App {
            app_id,
            app_instance_id,
            ..
        } = &task.executor
        else {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                "task has no App executor",
            ));
        };
        if *app_id != request_ctx.executor_app_id {
            return Err(task_mgr_error(
                TASK_ERR_PERMISSION_DENIED,
                format!(
                    "caller app {} is not the bound runner {}",
                    request_ctx.executor_app_id, app_id
                ),
            ));
        }
        if let Some(bound_instance) = app_instance_id {
            if envelope.app_instance_id.as_deref() != Some(bound_instance.as_str()) {
                return Err(task_mgr_error(
                    TASK_ERR_STALE_RUNNER_EPOCH,
                    format!(
                        "instance {:?} is not the bound instance {}",
                        envelope.app_instance_id, bound_instance
                    ),
                ));
            }
        }
        if envelope.runner_epoch != task.runner_epoch {
            return Err(task_mgr_error(
                TASK_ERR_STALE_RUNNER_EPOCH,
                format!(
                    "runner epoch {} != current {}",
                    envelope.runner_epoch, task.runner_epoch
                ),
            ));
        }
        if task.phase.is_terminal() {
            return Err(task_mgr_error(
                TASK_ERR_ALREADY_COMPLETED,
                "task already terminal",
            ));
        }
        Ok(())
    }

    /// Record (or replay/supersede) one control request on a single task.
    async fn request_control_single(
        &self,
        request_ctx: &RequestContext,
        task: &Task,
        action: TaskControlAction,
        request_id: &str,
        expected_revision: Option<u64>,
    ) -> Result<MutationOutcome> {
        // Idempotent replay of the same request id returns the current task.
        if let Some(pending) = &task.pending_control {
            if pending.request_id == request_id {
                return Ok(MutationOutcome {
                    task: task.clone(),
                    event: TaskEvent {
                        event_id: String::new(),
                        task_id: task.task_id.clone(),
                        root_id: task.root_id.clone(),
                        task_revision: task.revision,
                        event_type: TaskEventType::ControlRequested,
                        actor: Some(request_ctx.actor_ref()),
                        payload: json!({"replayed": true}),
                        created_at: now_ms(),
                    },
                });
            }
        }
        if task.phase.is_terminal() {
            return Err(task_mgr_error(
                TASK_ERR_ALREADY_COMPLETED,
                "task already terminal",
            ));
        }

        // Direct-close paths that need no runner acknowledgement (doc §6.2):
        // HumanSet cancel, and unbound tasks with no promise owner.
        if action == TaskControlAction::Cancel {
            match &task.executor {
                TaskExecutor::HumanSet => {
                    return self
                        .cancel_direct(request_ctx, task, expected_revision, "human_set")
                        .await;
                }
                TaskExecutor::Unbound if task.origin_ref.is_none() => {
                    return self
                        .cancel_direct(request_ctx, task, expected_revision, "unbound")
                        .await;
                }
                _ => {}
            }
        }

        match action {
            TaskControlAction::Pause => {
                if !matches!(task.control_profile.pause, ControlAvailability::Available) {
                    return Err(task_mgr_error(
                        TASK_ERR_CONTROL_NOT_AVAILABLE,
                        "runner does not currently support pause",
                    ));
                }
            }
            TaskControlAction::Resume => {
                if task.phase != TaskPhase::Paused {
                    return Err(task_mgr_error(
                        TASK_ERR_INVALID_PHASE,
                        "resume requires a Paused task",
                    ));
                }
                if !matches!(task.control_profile.resume, ControlAvailability::Available) {
                    return Err(task_mgr_error(
                        TASK_ERR_CONTROL_NOT_AVAILABLE,
                        "runner does not currently support resume",
                    ));
                }
            }
            TaskControlAction::Cancel => {
                if matches!(
                    task.control_profile.cancel,
                    CancelCapability::Unavailable { .. }
                ) && matches!(task.executor, TaskExecutor::App { .. })
                {
                    return Err(task_mgr_error(
                        TASK_ERR_CONTROL_NOT_AVAILABLE,
                        "runner does not currently support cancel",
                    ));
                }
            }
        }

        // Cancel atomically supersedes an unfinished pause/resume; any other
        // overlap is rejected, and a pending cancel is never overridden.
        let (event_type, payload) = match &task.pending_control {
            Some(pending) if pending.action == TaskControlAction::Cancel => {
                return Err(task_mgr_error(
                    TASK_ERR_CONTROL_ALREADY_PENDING,
                    "a cancel request is already pending",
                ));
            }
            Some(pending) if action == TaskControlAction::Cancel => (
                TaskEventType::ControlSuperseded,
                json!({
                    "superseded_request_id": pending.request_id,
                    "superseded_action": pending.action,
                    "action": action,
                    "request_id": request_id,
                }),
            ),
            Some(pending) => {
                return Err(task_mgr_error(
                    TASK_ERR_CONTROL_ALREADY_PENDING,
                    format!("pending {} request {}", pending.action, pending.request_id),
                ));
            }
            None => (
                TaskEventType::ControlRequested,
                json!({"action": action, "request_id": request_id}),
            ),
        };

        let request = TaskControlRequest {
            request_id: request_id.to_string(),
            action,
            requested_by: request_ctx.actor_ref(),
            requested_at: now_ms(),
        };
        let actor = request_ctx.actor_ref();
        self.store
            .mutate_task(
                &task.task_id,
                Some(&actor),
                event_type,
                payload,
                expected_revision,
                move |current| {
                    if current.phase.is_terminal() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    current.pending_control = Some(request);
                    Ok(())
                },
            )
            .await
    }

    // -----------------------------------------------------------------
    // Trusted in-process control plane (co-deployed dispatcher)
    // -----------------------------------------------------------------
    //
    // The Dispatch Center shares this process and TCB; it calls these
    // directly instead of looping a token through kRPC. The RPC-facing
    // handlers wrap the same implementations behind `require_zone_trusted`,
    // so remote trusted control planes get identical semantics.

    pub(crate) async fn trusted_create_promised_task(
        &self,
        req: CreatePromisedTaskReq,
        actor: &ActorRef,
    ) -> Result<Task> {
        if req.name.trim().is_empty() || req.idempotency_key.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "name and idempotency_key are required".into(),
            ));
        }
        if req.creator.user_id.trim().is_empty() || req.creator.app_id.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "delegated creator envelope is incomplete".into(),
            ));
        }
        if let Some(expected) = req.expected_input_digest.as_deref() {
            let actual = compute_task_input_digest(&req.input);
            if actual != expected {
                return Err(task_mgr_error(
                    TASK_ERR_INPUT_SCHEMA_MISMATCH,
                    "input digest does not match the control plane's frozen copy",
                ));
            }
        }
        let schema = self
            .resolve_schema(
                &req.schema_id,
                req.schema_version,
                &req.input,
                TaskExecutorKind::Unbound,
            )
            .await?;
        let storage_domain = self
            .resolve_storage_domain(
                req.storage_domain,
                req.parent_id.as_deref(),
                &schema,
                &req.creator,
            )
            .await?;
        let wait_reason = req
            .wait_reason
            .clone()
            .or_else(|| Some(TaskWaitReason::new(TaskWaitReasonKind::Dispatch)));
        let _ = actor;
        let outcome = self
            .store
            .create_task(CreateTaskArgs {
                task_id: None,
                name: req.name.clone(),
                schema_id: schema.schema_id.clone(),
                schema_version: schema.schema_version,
                input: req.input.clone(),
                creator: req.creator.clone(),
                storage_domain,
                idempotency_key: req.idempotency_key.clone(),
                origin_ref: req.origin_ref.clone(),
                parent_id: req.parent_id.clone(),
                child_control_policy: req.child_control_policy.unwrap_or_default(),
                policy_preset: req
                    .policy_preset
                    .clone()
                    .unwrap_or_else(|| TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1.to_string()),
                permission_boundary: req.permission_boundary,
                retry_of: None,
                supersedes: None,
                executor: TaskExecutor::Unbound,
                assignees: Vec::new(),
                phase: TaskPhase::Promised,
                wait_reason,
                message: req.message.clone(),
            })
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    pub(crate) async fn trusted_set_promise_wait(
        &self,
        req: SetPromiseWaitReq,
        actor: &ActorRef,
    ) -> Result<Task> {
        let reason = req.wait_reason.clone();
        let reason_kind = reason.kind;
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(actor),
                TaskEventType::WaitReasonChanged,
                json!({"kind": reason_kind}),
                Some(req.expected_revision),
                move |current| {
                    if current.phase != TaskPhase::Promised {
                        return Err(task_mgr_error(
                            TASK_ERR_INVALID_PHASE,
                            "set_promise_wait requires a Promised task",
                        ));
                    }
                    current.wait_reason = Some(reason);
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    pub(crate) async fn trusted_bind_app_executor(
        &self,
        req: BindAppExecutorReq,
        actor: &ActorRef,
    ) -> Result<Task> {
        let req_target = req.target_id.clone();
        let req_app = req.app_id.clone();
        let req_instance = req.app_instance_id.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(actor),
                TaskEventType::RunnerBound,
                json!({
                    "target_id": req.target_id,
                    "app_id": req.app_id,
                    "app_instance_id": req.app_instance_id,
                    "delivery_id": req.delivery_id,
                }),
                Some(req.expected_revision),
                move |current| {
                    match &current.executor {
                        TaskExecutor::Unbound => {
                            if current.phase != TaskPhase::Promised {
                                return Err(task_mgr_error(
                                    TASK_ERR_INVALID_PHASE,
                                    "bind requires a Promised/Unbound task",
                                ));
                            }
                        }
                        TaskExecutor::App {
                            target_id: bound_target,
                            app_instance_id: bound_instance,
                            ..
                        } => {
                            // Rebind within the same frozen logical target
                            // after a release (doc §10.1).
                            if bound_instance.is_some() {
                                return Err(task_mgr_error(
                                    TASK_ERR_INVALID_PHASE,
                                    "task already has a bound instance",
                                ));
                            }
                            if bound_target.as_deref() != req_target.as_deref() {
                                return Err(task_mgr_error(
                                    TASK_ERR_INVALID_PHASE,
                                    "logical target is frozen after the first bind",
                                ));
                            }
                        }
                        TaskExecutor::HumanSet => {
                            return Err(task_mgr_error(
                                TASK_ERR_INVALID_PHASE,
                                "cannot bind an App executor on a HumanSet task",
                            ));
                        }
                    }
                    if current.phase.is_terminal() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    current.executor = TaskExecutor::App {
                        target_id: req_target,
                        app_id: req_app,
                        app_instance_id: Some(req_instance),
                    };
                    current.runner_epoch += 1;
                    current.phase = TaskPhase::Accepted;
                    current.wait_reason = None;
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    pub(crate) async fn trusted_release_app_executor(
        &self,
        req: ReleaseAppExecutorReq,
        actor: &ActorRef,
    ) -> Result<Task> {
        let reason = req.reason.clone();
        let reason_kind = reason.kind;
        let expected_instance = req.expected_instance_id.clone();
        let expected_epoch = req.expected_runner_epoch;
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(actor),
                TaskEventType::RunnerReleased,
                json!({"instance_id": expected_instance, "reason": reason_kind}),
                Some(req.expected_revision),
                move |current| {
                    let TaskExecutor::App {
                        target_id,
                        app_id,
                        app_instance_id,
                    } = current.executor.clone()
                    else {
                        return Err(task_mgr_error(
                            TASK_ERR_INVALID_PHASE,
                            "task has no App executor",
                        ));
                    };
                    if current.phase.is_terminal() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    if app_instance_id.as_deref() != Some(req.expected_instance_id.as_str()) {
                        return Err(task_mgr_error(
                            TASK_ERR_STALE_RUNNER_EPOCH,
                            "bound instance does not match",
                        ));
                    }
                    if current.runner_epoch != expected_epoch {
                        return Err(task_mgr_error(
                            TASK_ERR_STALE_RUNNER_EPOCH,
                            "runner epoch does not match",
                        ));
                    }
                    current.executor = TaskExecutor::App {
                        target_id,
                        app_id,
                        app_instance_id: None,
                    };
                    current.runner_epoch += 1;
                    current.phase = TaskPhase::Waiting;
                    current.wait_reason = Some(req.reason.clone());
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    pub(crate) async fn trusted_finish_promise_failure(
        &self,
        req: FinishPromiseFailureReq,
        actor: &ActorRef,
    ) -> Result<Task> {
        let completed_by = actor.clone();
        let error = req.error.clone();
        let error_code = error.code.clone();
        let error_message = error.message.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(actor),
                TaskEventType::TaskFailed,
                json!({"code": error_code, "message": error_message, "path": "promise"}),
                Some(req.expected_revision),
                move |current| {
                    if current.executor.kind() != TaskExecutorKind::Unbound {
                        return Err(task_mgr_error(
                            TASK_ERR_INVALID_PHASE,
                            "finish_promise_failure requires an Unbound task",
                        ));
                    }
                    if current.phase.is_terminal() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    let now = now_ms();
                    current.error = Some(error);
                    current.outcome = Some(TaskOutcome::Failed);
                    current.phase = TaskPhase::Terminal;
                    current.pending_control = None;
                    current.wait_reason = None;
                    current.completed_by = Some(completed_by);
                    current.completed_at = Some(now);
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    pub(crate) async fn trusted_cancel_promised_task(
        &self,
        req: CancelPromisedTaskReq,
        actor: &ActorRef,
    ) -> Result<Task> {
        let completed_by = actor.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(actor),
                TaskEventType::TaskCanceled,
                json!({"cancel_mode": "interrupt", "path": "promise"}),
                Some(req.expected_revision),
                move |current| {
                    if current.executor.kind() != TaskExecutorKind::Unbound {
                        return Err(task_mgr_error(
                            TASK_ERR_INVALID_PHASE,
                            "cancel_promised_task requires an Unbound task",
                        ));
                    }
                    if current.phase.is_terminal() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    let now = now_ms();
                    current.phase = TaskPhase::Terminal;
                    current.outcome = Some(TaskOutcome::Canceled);
                    current.pending_control = None;
                    current.wait_reason = None;
                    current.completed_by = Some(completed_by);
                    current.completed_at = Some(now);
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    pub(crate) async fn trusted_get_task(&self, task_id: &str) -> Result<Option<Task>> {
        self.store.get_task(task_id).await
    }

    pub(crate) async fn trusted_get_schema(
        &self,
        schema_id: &str,
        schema_version: Option<u32>,
    ) -> Result<TaskSchemaDefinition> {
        self.store.get_schema(schema_id, schema_version).await
    }

    pub(crate) async fn recover_lost_user_tasks_by_origin(
        &self,
        origin_kind: &str,
        live_task_ids: &HashSet<TaskId>,
    ) -> Result<usize> {
        let tasks = self
            .store
            .list_nonterminal_by_origin(StorageDomain::User, origin_kind)
            .await?;
        let actor = ActorRef::new("system", format!("system:{}", TASK_MANAGER_SERVICE_NAME));
        let mut recovered = 0usize;
        for task in tasks {
            if live_task_ids.contains(&task.task_id) {
                continue;
            }
            let completed_by = actor.clone();
            let outcome = self
                .store
                .mutate_task(
                    &task.task_id,
                    Some(&actor),
                    TaskEventType::TaskFailed,
                    json!({"code": "runner_lost", "path": "storage_domain_recovery"}),
                    Some(task.revision),
                    move |current| {
                        if current.phase.is_terminal() {
                            return Ok(());
                        }
                        let now = now_ms();
                        current.phase = TaskPhase::Terminal;
                        current.outcome = Some(TaskOutcome::Failed);
                        current.error = Some(TaskError::new(
                            "runner_lost",
                            "dispatcher state or runner binding was lost during restore",
                        ));
                        current.pending_control = None;
                        current.wait_reason = None;
                        current.completed_by = Some(completed_by);
                        current.completed_at = Some(now);
                        Ok(())
                    },
                )
                .await?;
            self.publish_outcome(&outcome).await;
            recovered += 1;
        }
        Ok(recovered)
    }

    /// CAS-close a task without runner involvement. Guarantee level is
    /// interrupt: no side-effect rollback promise.
    async fn cancel_direct(
        &self,
        request_ctx: &RequestContext,
        task: &Task,
        expected_revision: Option<u64>,
        mode: &str,
    ) -> Result<MutationOutcome> {
        let actor = request_ctx.actor_ref();
        let completed_by = actor.clone();
        self.store
            .mutate_task(
                &task.task_id,
                Some(&actor),
                TaskEventType::TaskCanceled,
                json!({"cancel_mode": "interrupt", "path": mode}),
                expected_revision,
                move |current| {
                    if current.phase.is_terminal() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    if current.result.is_some() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "result already committed",
                        ));
                    }
                    let now = now_ms();
                    current.phase = TaskPhase::Terminal;
                    current.outcome = Some(TaskOutcome::Canceled);
                    current.pending_control = None;
                    current.wait_reason = None;
                    current.completed_by = Some(completed_by);
                    current.completed_at = Some(now);
                    Ok(())
                },
            )
            .await
    }
}

// ---------------------------------------------------------------------------
// Handler implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl TaskManagerHandler for TaskManagerService {
    async fn handle_create_task(&self, req: CreateTaskReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        if req.name.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError("task name is required".into()));
        }
        if req.idempotency_key.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "idempotency_key is required".into(),
            ));
        }

        // A plain create may only bind the authenticated caller itself or a
        // human set (doc §13.1).
        let (executor, assignees, phase, wait_reason) = match &req.executor {
            CreateTaskExecutor::SelfApp { app_instance_id } => (
                TaskExecutor::App {
                    target_id: None,
                    app_id: request_ctx.executor_app_id.clone(),
                    app_instance_id: app_instance_id.clone(),
                },
                Vec::new(),
                TaskPhase::Accepted,
                None,
            ),
            CreateTaskExecutor::HumanSet { assignees } => {
                let cleaned: Vec<String> = assignees
                    .iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if cleaned.is_empty() {
                    return Err(RPCErrors::ParseRequestError(
                        "a HumanSet task needs at least one assignee".into(),
                    ));
                }
                (
                    TaskExecutor::HumanSet,
                    cleaned,
                    TaskPhase::Waiting,
                    Some(TaskWaitReason::new(TaskWaitReasonKind::HumanInput)),
                )
            }
        };

        let schema = self
            .resolve_schema(
                &req.schema_id,
                req.schema_version,
                &req.input,
                executor.kind(),
            )
            .await?;
        let creator = request_ctx.actor_ref();
        let storage_domain = self
            .resolve_storage_domain(
                req.storage_domain,
                req.parent_id.as_deref(),
                &schema,
                &creator,
            )
            .await?;

        if let Some(parent_id) = req.parent_id.as_deref() {
            let (_parent, permission) = self.load_visible(&request_ctx, parent_id).await?;
            if !permission.allows(TaskAction::CreateChild) {
                return Err(task_mgr_error(
                    TASK_ERR_PERMISSION_DENIED,
                    format!("missing CreateChild on parent {}", parent_id),
                ));
            }
        }

        let outcome = self
            .store
            .create_task(CreateTaskArgs {
                task_id: None,
                name: req.name.clone(),
                schema_id: schema.schema_id.clone(),
                schema_version: schema.schema_version,
                input: req.input.clone(),
                creator,
                storage_domain,
                idempotency_key: req.idempotency_key.clone(),
                origin_ref: None,
                parent_id: req.parent_id.clone(),
                child_control_policy: req.child_control_policy.unwrap_or_default(),
                policy_preset: req
                    .policy_preset
                    .clone()
                    .unwrap_or_else(|| TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1.to_string()),
                permission_boundary: req.permission_boundary,
                retry_of: req.retry_of.clone(),
                supersedes: req.supersedes.clone(),
                executor,
                assignees,
                phase,
                wait_reason,
                message: req.message.clone(),
            })
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_create_delegated_task(
        &self,
        req: CreateDelegatedTaskReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "create_delegated_task")?;
        if req.name.trim().is_empty() || req.idempotency_key.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "name and idempotency_key are required".into(),
            ));
        }
        if req.creator.user_id.trim().is_empty() || req.creator.app_id.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "delegated creator envelope is incomplete".into(),
            ));
        }
        if let Some(task_id) = req.task_id.as_deref() {
            let suffix = task_id.strip_prefix("t-").unwrap_or_default();
            if suffix.len() != 32
                || !suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(RPCErrors::ParseRequestError(
                    "delegated task_id must use the canonical t-<32 lowercase hex> form".into(),
                ));
            }
        }
        let schema = self
            .resolve_schema(
                &req.schema_id,
                req.schema_version,
                &req.input,
                TaskExecutorKind::App,
            )
            .await?;
        let storage_domain = self
            .resolve_storage_domain(
                req.storage_domain,
                req.parent_id.as_deref(),
                &schema,
                &req.creator,
            )
            .await?;
        let outcome = self
            .store
            .create_task(CreateTaskArgs {
                task_id: req.task_id,
                name: req.name,
                schema_id: schema.schema_id,
                schema_version: schema.schema_version,
                input: req.input,
                creator: req.creator,
                storage_domain,
                idempotency_key: req.idempotency_key,
                origin_ref: None,
                parent_id: req.parent_id,
                child_control_policy: req.child_control_policy.unwrap_or_default(),
                policy_preset: req
                    .policy_preset
                    .unwrap_or_else(|| TASK_POLICY_PRESET_COLLABORATIVE_TREE_V1.to_string()),
                permission_boundary: req.permission_boundary,
                retry_of: req.retry_of,
                supersedes: req.supersedes,
                executor: TaskExecutor::App {
                    target_id: None,
                    app_id: request_ctx.executor_app_id,
                    app_instance_id: req.runner_app_instance_id,
                },
                assignees: Vec::new(),
                phase: TaskPhase::Accepted,
                wait_reason: None,
                message: req.message,
            })
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_get_task(&self, req: GetTaskReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let (task, permission) = self.load_visible(&request_ctx, &req.task_id).await?;
        Ok(trim_task(task, &permission))
    }

    async fn handle_list_tasks(
        &self,
        req: ListTasksReq,
        ctx: RPCContext,
    ) -> Result<TaskSummaryPage> {
        let request_ctx = self.authenticate(&ctx).await?;
        let principal = request_ctx.principal();
        let (tasks, next_cursor) = self.store.list_tasks(&req).await?;
        let mut summaries = Vec::new();
        for task in tasks {
            let permission = compute_permission(&self.store, &principal, &task).await?;
            if permission.visible() {
                summaries.push(summarize_task(&task));
            }
        }
        Ok(TaskSummaryPage {
            tasks: summaries,
            next_cursor,
        })
    }

    async fn handle_get_task_tree(
        &self,
        req: GetTaskTreeReq,
        ctx: RPCContext,
    ) -> Result<TaskSummaryPage> {
        let request_ctx = self.authenticate(&ctx).await?;
        let principal = request_ctx.principal();
        let limit = req.limit.unwrap_or(200).clamp(1, 500);
        let (tasks, next_cursor) = self
            .store
            .list_tree(&req.root_id, req.cursor.as_deref(), limit)
            .await?;
        let mut summaries = Vec::new();
        for task in tasks {
            let permission = compute_permission(&self.store, &principal, &task).await?;
            if permission.visible() {
                summaries.push(summarize_task(&task));
            }
        }
        Ok(TaskSummaryPage {
            tasks: summaries,
            next_cursor,
        })
    }

    async fn handle_get_subtasks(
        &self,
        req: GetSubtasksReq,
        ctx: RPCContext,
    ) -> Result<TaskSummaryPage> {
        let request_ctx = self.authenticate(&ctx).await?;
        let principal = request_ctx.principal();
        // The parent must at least be visible.
        self.load_visible(&request_ctx, &req.task_id).await?;
        let limit = req.limit.unwrap_or(100).clamp(1, 500);
        let (tasks, next_cursor) = self
            .store
            .list_children(&req.task_id, req.cursor.as_deref(), limit)
            .await?;
        let mut summaries = Vec::new();
        for task in tasks {
            let permission = compute_permission(&self.store, &principal, &task).await?;
            if permission.visible() {
                summaries.push(summarize_task(&task));
            }
        }
        Ok(TaskSummaryPage {
            tasks: summaries,
            next_cursor,
        })
    }

    async fn handle_archive_task(&self, req: ArchiveTaskReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let (task, _) = self
            .require_action(&request_ctx, &req.task_id, TaskAction::Archive)
            .await?;
        if !task.phase.is_terminal() {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                "only terminal tasks can be archived",
            ));
        }
        let actor = request_ctx.actor_ref();
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(&actor),
                TaskEventType::TaskArchived,
                json!({}),
                Some(req.expected_revision),
                |current| {
                    if current.archived_at.is_none() {
                        current.archived_at = Some(now_ms());
                    }
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_request_control(
        &self,
        req: RequestControlReq,
        ctx: RPCContext,
    ) -> Result<RequestControlResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        if req.request_id.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "request_id is required".into(),
            ));
        }
        let (task, _) = self
            .require_action(&request_ctx, &req.task_id, TaskAction::Control)
            .await?;

        if !req.recursive {
            let outcome = self
                .request_control_single(
                    &request_ctx,
                    &task,
                    req.action,
                    &req.request_id,
                    req.expected_revision,
                )
                .await?;
            self.publish_outcome(&outcome).await;
            return Ok(RequestControlResult::Task { task: outcome.task });
        }

        // Tree-level fan-out: per-edge control policy + per-task ACL, and
        // per-task requests only — no batch final-state write (doc §6.3).
        let principal = request_ctx.principal();
        let subtree = self
            .store
            .collect_subtree(&task, MAX_RECURSIVE_CONTROL_NODES)
            .await?;
        let mut result = BatchControlResult::default();
        for node in subtree {
            let is_request_root = node.task_id == task.task_id;
            if !is_request_root && !node.child_control_policy.follows(req.action) {
                result.skipped_by_policy.push(node.task_id.clone());
                continue;
            }
            if node.phase.is_terminal() {
                result.already_terminal.push(node.task_id.clone());
                continue;
            }
            let permission = compute_permission(&self.store, &principal, &node).await?;
            if !permission.allows(TaskAction::Control) {
                result.denied.push(node.task_id.clone());
                continue;
            }
            let request_id = if is_request_root {
                req.request_id.clone()
            } else {
                format!("{}:{}", req.request_id, node.task_id)
            };
            match self
                .request_control_single(&request_ctx, &node, req.action, &request_id, None)
                .await
            {
                Ok(outcome) => {
                    self.publish_outcome(&outcome).await;
                    result.requested.push(node.task_id.clone());
                }
                Err(err) => match task_mgr_error_code(&err) {
                    Some(TASK_ERR_ALREADY_COMPLETED) => {
                        result.already_terminal.push(node.task_id.clone())
                    }
                    Some(TASK_ERR_CONTROL_NOT_AVAILABLE)
                    | Some(TASK_ERR_CONTROL_ALREADY_PENDING)
                    | Some(TASK_ERR_INVALID_PHASE) => {
                        result.skipped_by_policy.push(node.task_id.clone())
                    }
                    Some(TASK_ERR_PERMISSION_DENIED) => result.denied.push(node.task_id.clone()),
                    _ => return Err(err),
                },
            }
        }
        Ok(RequestControlResult::Batch { result })
    }

    async fn handle_request_delegated_control(
        &self,
        req: RequestDelegatedControlReq,
        ctx: RPCContext,
    ) -> Result<RequestControlResult> {
        let service_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&service_ctx, "request_delegated_control")?;
        if req.request_id.trim().is_empty()
            || req.controller.user_id.trim().is_empty()
            || req.controller.app_id.trim().is_empty()
        {
            return Err(RPCErrors::ParseRequestError(
                "controller and request_id are required".into(),
            ));
        }
        let target = req.controller.auth_target().map_err(|err| {
            RPCErrors::ParseRequestError(format!("invalid controller identity: {err}"))
        })?;
        let authorization_id = target.authorization_key();
        let app_instance_id = match &target {
            AuthTarget::App { app_instance_id } => Some(app_instance_id.to_string()),
            AuthTarget::System { .. } => None,
        };
        let controller_ctx = RequestContext {
            user_id: req.controller.user_id.clone(),
            app_id: target.canonical_key(),
            executor_app_id: target.appid_claim().to_string(),
            app_instance_id,
            authorization_id,
            zone_trusted: false,
            sudo: false,
        };
        let (task, _) = self
            .require_action(&controller_ctx, &req.task_id, TaskAction::Control)
            .await?;
        let outcome = self
            .request_control_single(
                &controller_ctx,
                &task,
                req.action,
                &req.request_id,
                req.expected_revision,
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(RequestControlResult::Task { task: outcome.task })
    }

    async fn handle_update_assignees(
        &self,
        req: UpdateAssigneesReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_action(&request_ctx, &req.task_id, TaskAction::Reassign)
            .await?;
        let actor = request_ctx.actor_ref();
        let add: Vec<String> = req
            .add
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let remove: Vec<String> = req
            .remove
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let outcome = self
            .store
            .update_assignees(&req.task_id, &actor, &add, &remove, req.expected_revision)
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_grant_task_access(
        &self,
        req: GrantTaskAccessReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_action(&request_ctx, &req.task_id, TaskAction::Grant)
            .await?;
        if req.grant.actions.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "a grant needs at least one action".into(),
            ));
        }
        let actor = request_ctx.actor_ref();
        let (outcome, _grant) = self
            .store
            .insert_grant(&req.task_id, &actor, &req.grant, req.expected_revision)
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_revoke_task_access(
        &self,
        req: RevokeTaskAccessReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_action(&request_ctx, &req.task_id, TaskAction::Grant)
            .await?;
        let actor = request_ctx.actor_ref();
        let outcome = self
            .store
            .revoke_grant(&req.task_id, &actor, &req.grant_id, req.expected_revision)
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_list_task_access(
        &self,
        req: ListTaskAccessReq,
        ctx: RPCContext,
    ) -> Result<ListTaskAccessResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.require_action(&request_ctx, &req.task_id, TaskAction::Grant)
            .await?;
        let grants = self.store.all_grants_for_task(&req.task_id).await?;
        Ok(ListTaskAccessResult { grants })
    }

    async fn handle_report_started(&self, req: ReportStartedReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        if task.phase != TaskPhase::Accepted {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                format!("report_started requires Accepted, found {}", task.phase),
            ));
        }
        let actor = request_ctx.actor_ref();
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                TaskEventType::PhaseChanged,
                json!({"from": "Accepted", "to": "Running", "via": "report_started"}),
                Some(req.envelope.expected_revision),
                |current| {
                    current.phase = TaskPhase::Running;
                    current.wait_reason = None;
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_report_progress(
        &self,
        req: ReportProgressReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        let actor = request_ctx.actor_ref();
        let progress = req.progress.clone();
        let message = req.message.clone();
        let message_for_event = message.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                TaskEventType::ProgressReported,
                json!({"message": message_for_event}),
                Some(req.envelope.expected_revision),
                move |current| {
                    if let Some(progress) = progress {
                        current.progress = Some(progress);
                    }
                    if let Some(message) = message {
                        current.message = Some(message);
                    }
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_report_waiting(&self, req: ReportWaitingReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        if !matches!(task.phase, TaskPhase::Accepted | TaskPhase::Running) {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                format!(
                    "report_waiting requires Accepted/Running, found {}",
                    task.phase
                ),
            ));
        }
        let actor = request_ctx.actor_ref();
        let reason = req.reason.clone();
        let reason_kind = reason.kind;
        let from_phase = task.phase;
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                TaskEventType::WaitReasonChanged,
                json!({"from": from_phase.to_string(), "kind": reason_kind}),
                Some(req.envelope.expected_revision),
                move |current| {
                    current.phase = TaskPhase::Waiting;
                    current.wait_reason = Some(reason);
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_report_running(&self, req: ReportRunningReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        if !matches!(task.phase, TaskPhase::Waiting | TaskPhase::Accepted) {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                format!(
                    "report_running requires Waiting/Accepted, found {}",
                    task.phase
                ),
            ));
        }
        let actor = request_ctx.actor_ref();
        let from_phase = task.phase;
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                TaskEventType::PhaseChanged,
                json!({"from": from_phase.to_string(), "to": "Running", "via": "report_running"}),
                Some(req.envelope.expected_revision),
                |current| {
                    current.phase = TaskPhase::Running;
                    current.wait_reason = None;
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_update_control_profile(
        &self,
        req: UpdateControlProfileReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        let actor = request_ctx.actor_ref();
        let mut profile = req.profile.clone();
        profile.updated_at = now_ms();
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                TaskEventType::ControlProfileChanged,
                json!({}),
                Some(req.envelope.expected_revision),
                move |current| {
                    current.control_profile = profile;
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_ack_control(&self, req: AckControlReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        let Some(pending) = task.pending_control.clone() else {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                "no pending control request",
            ));
        };
        if pending.request_id != req.request_id {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                format!(
                    "pending request is {}, not {}",
                    pending.request_id, req.request_id
                ),
            ));
        }
        let actor = request_ctx.actor_ref();
        let applied = req.applied;
        let action = pending.action;
        let event_type = if applied {
            match action {
                TaskControlAction::Cancel => TaskEventType::TaskCanceled,
                _ => TaskEventType::ControlApplied,
            }
        } else {
            TaskEventType::ControlRejected
        };
        let cancel_mode = match task.control_profile.cancel {
            CancelCapability::Safe => "safe",
            _ => "interrupt",
        };
        let payload = if applied {
            json!({"action": action, "request_id": req.request_id, "cancel_mode": cancel_mode})
        } else {
            json!({"action": action, "request_id": req.request_id, "reject_reason": req.reject_reason})
        };
        let completed_by = actor.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                event_type,
                payload,
                Some(req.envelope.expected_revision),
                move |current| {
                    current.pending_control = None;
                    if applied {
                        match action {
                            TaskControlAction::Pause => {
                                current.phase = TaskPhase::Paused;
                            }
                            TaskControlAction::Resume => {
                                current.phase = TaskPhase::Running;
                                current.wait_reason = None;
                            }
                            TaskControlAction::Cancel => {
                                if current.result.is_some() {
                                    return Err(task_mgr_error(
                                        TASK_ERR_ALREADY_COMPLETED,
                                        "result already committed",
                                    ));
                                }
                                current.phase = TaskPhase::Terminal;
                                current.outcome = Some(TaskOutcome::Canceled);
                                current.wait_reason = None;
                                current.completed_by = Some(completed_by);
                                current.completed_at = Some(now_ms());
                            }
                        }
                    }
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_commit_result(&self, req: CommitResultReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.task_id).await?;

        // Identity check by executor mode (doc §9.3/§10).
        match &task.executor {
            TaskExecutor::App { .. } => {
                let runner_epoch = req.runner_epoch.ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "runner_epoch is required for App runner commits".into(),
                    )
                })?;
                let envelope = RunnerWriteEnvelope {
                    task_id: req.task_id.clone(),
                    app_instance_id: req.app_instance_id.clone(),
                    runner_epoch,
                    expected_revision: req.expected_revision,
                };
                Self::verify_app_runner_write(&request_ctx, &task, &envelope)?;
            }
            TaskExecutor::HumanSet => {
                let assignees = self.store.active_assignees(&req.task_id).await?;
                if !assignees.iter().any(|user| user == &request_ctx.user_id) {
                    return Err(task_mgr_error(
                        TASK_ERR_PERMISSION_DENIED,
                        format!("{} is not an active assignee", request_ctx.user_id),
                    ));
                }
                if task.phase.is_terminal() {
                    return Err(task_mgr_error(
                        TASK_ERR_ALREADY_COMPLETED,
                        "task already terminal",
                    ));
                }
            }
            TaskExecutor::Unbound => {
                return Err(task_mgr_error(
                    TASK_ERR_INVALID_PHASE,
                    "an unbound task has no committer",
                ));
            }
        }

        let schema = self
            .store
            .get_schema(&task.schema_id, Some(task.schema_version))
            .await?;
        validate_json_schema(&schema.output_schema, &req.result)
            .map_err(|violation| task_mgr_error(TASK_ERR_RESULT_SCHEMA_MISMATCH, violation))?;

        let actor = request_ctx.actor_ref();
        let completed_by = actor.clone();
        let result = req.result.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.task_id,
                Some(&actor),
                TaskEventType::ResultCommitted,
                json!({}),
                Some(req.expected_revision),
                move |current| {
                    if current.phase.is_terminal() || current.result.is_some() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "result already committed or task terminal",
                        ));
                    }
                    let now = now_ms();
                    current.result = Some(result);
                    current.outcome = Some(TaskOutcome::Succeeded);
                    current.phase = TaskPhase::Terminal;
                    current.pending_control = None;
                    current.wait_reason = None;
                    current.completed_by = Some(completed_by);
                    current.completed_at = Some(now);
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    async fn handle_fail_task(&self, req: FailTaskReq, ctx: RPCContext) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        let task = self.load_task(&req.envelope.task_id).await?;
        Self::verify_app_runner_write(&request_ctx, &task, &req.envelope)?;
        let actor = request_ctx.actor_ref();
        let completed_by = actor.clone();
        let error = req.error.clone();
        let error_code = error.code.clone();
        let error_message = error.message.clone();
        let outcome = self
            .store
            .mutate_task(
                &req.envelope.task_id,
                Some(&actor),
                TaskEventType::TaskFailed,
                json!({"code": error_code, "message": error_message}),
                Some(req.envelope.expected_revision),
                move |current| {
                    if current.phase.is_terminal() || current.result.is_some() {
                        return Err(task_mgr_error(
                            TASK_ERR_ALREADY_COMPLETED,
                            "task already terminal",
                        ));
                    }
                    let now = now_ms();
                    current.error = Some(error);
                    current.outcome = Some(TaskOutcome::Failed);
                    current.phase = TaskPhase::Terminal;
                    current.pending_control = None;
                    current.wait_reason = None;
                    current.completed_by = Some(completed_by);
                    current.completed_at = Some(now);
                    Ok(())
                },
            )
            .await?;
        self.publish_outcome(&outcome).await;
        Ok(outcome.task)
    }

    // --- Trusted promise/executor control plane (doc §13.4) ---

    async fn handle_create_promised_task(
        &self,
        req: CreatePromisedTaskReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "create_promised_task")?;
        self.trusted_create_promised_task(req, &request_ctx.actor_ref())
            .await
    }

    async fn handle_set_promise_wait(
        &self,
        req: SetPromiseWaitReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "set_promise_wait")?;
        self.trusted_set_promise_wait(req, &request_ctx.actor_ref())
            .await
    }

    async fn handle_bind_app_executor(
        &self,
        req: BindAppExecutorReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "bind_app_executor")?;
        self.trusted_bind_app_executor(req, &request_ctx.actor_ref())
            .await
    }

    async fn handle_release_app_executor(
        &self,
        req: ReleaseAppExecutorReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        // The frozen target's own control plane is zone-trusted; other
        // callers need an explicit Reassign grant.
        if !request_ctx.zone_trusted {
            self.require_action(&request_ctx, &req.task_id, TaskAction::Reassign)
                .await?;
        }
        self.trusted_release_app_executor(req, &request_ctx.actor_ref())
            .await
    }

    async fn handle_finish_promise_failure(
        &self,
        req: FinishPromiseFailureReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "finish_promise_failure")?;
        self.trusted_finish_promise_failure(req, &request_ctx.actor_ref())
            .await
    }

    async fn handle_cancel_promised_task(
        &self,
        req: CancelPromisedTaskReq,
        ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "cancel_promised_task")?;
        self.trusted_cancel_promised_task(req, &request_ctx.actor_ref())
            .await
    }

    // --- Schema registry ---

    async fn handle_register_task_schema(
        &self,
        req: RegisterTaskSchemaReq,
        ctx: RPCContext,
    ) -> Result<TaskSchemaDefinition> {
        let request_ctx = self.authenticate(&ctx).await?;
        let def = &req.definition;
        if def.schema_id.trim().is_empty() {
            return Err(RPCErrors::ParseRequestError("schema_id is required".into()));
        }
        if def.allowed_executor_kinds.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "allowed_executor_kinds must not be empty".into(),
            ));
        }
        // Apps publish their own schemas; publishing for another app needs a
        // zone-trusted identity.
        if !request_ctx.zone_trusted && def.publisher_app_id != request_ctx.executor_app_id {
            return Err(task_mgr_error(
                TASK_ERR_PERMISSION_DENIED,
                "publisher_app_id must match the caller app",
            ));
        }
        self.store.register_schema(def).await
    }

    async fn handle_get_task_schema(
        &self,
        req: GetTaskSchemaReq,
        ctx: RPCContext,
    ) -> Result<TaskSchemaDefinition> {
        self.authenticate(&ctx).await?;
        self.store
            .get_schema(&req.schema_id, req.schema_version)
            .await
    }

    async fn handle_list_task_schemas(
        &self,
        req: ListTaskSchemasReq,
        ctx: RPCContext,
    ) -> Result<ListTaskSchemasResult> {
        self.authenticate(&ctx).await?;
        let schemas = self
            .store
            .list_schemas(req.user_creatable_only, req.include_disabled)
            .await?;
        Ok(ListTaskSchemasResult { schemas })
    }

    async fn handle_set_task_schema_enabled(
        &self,
        req: SetTaskSchemaEnabledReq,
        ctx: RPCContext,
    ) -> Result<TaskSchemaDefinition> {
        let request_ctx = self.authenticate(&ctx).await?;
        Self::require_zone_trusted(&request_ctx, "set_task_schema_enabled")?;
        self.store
            .set_schema_enabled(&req.schema_id, req.schema_version, req.enabled)
            .await
    }

    // --- Events & notes ---

    async fn handle_list_task_events(
        &self,
        req: ListTaskEventsReq,
        ctx: RPCContext,
    ) -> Result<ListTaskEventsResult> {
        let request_ctx = self.authenticate(&ctx).await?;
        // Visibility is anchored on the referenced task (or tree root).
        let anchor = req
            .task_id
            .as_deref()
            .or(req.root_id.as_deref())
            .ok_or_else(|| RPCErrors::ParseRequestError("task_id or root_id is required".into()))?;
        self.load_visible(&request_ctx, anchor).await?;
        let events = self
            .store
            .list_events(
                req.task_id.as_deref(),
                req.root_id.as_deref(),
                req.after_event_id.as_deref(),
                req.limit.unwrap_or(100),
            )
            .await?;
        let next_cursor = events.last().map(|e| e.event_id.clone());
        Ok(ListTaskEventsResult {
            events,
            next_cursor,
        })
    }

    async fn handle_add_task_note(&self, req: AddTaskNoteReq, ctx: RPCContext) -> Result<TaskNote> {
        let request_ctx = self.authenticate(&ctx).await?;
        let content = req.content.trim();
        if content.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "note content is required".into(),
            ));
        }
        self.load_visible(&request_ctx, &req.task_id).await?;
        let now = now_ms();
        let note = TaskNote {
            id: 0,
            task_id: req.task_id.clone(),
            note_type: req
                .note_type
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("human")
                .to_string(),
            content: content.to_string(),
            data: req.data.clone().unwrap_or_else(|| json!({})),
            author_user_id: request_ctx.user_id.clone(),
            author_app_id: request_ctx.app_id.clone(),
            created_at: now,
            updated_at: now,
        };
        self.store.add_task_note(&note).await
    }

    async fn handle_list_task_notes(
        &self,
        req: ListTaskNotesReq,
        ctx: RPCContext,
    ) -> Result<Vec<TaskNote>> {
        let request_ctx = self.authenticate(&ctx).await?;
        self.load_visible(&request_ctx, &req.task_id).await?;
        self.store.list_task_notes(&req.task_id).await
    }
}

// ---------------------------------------------------------------------------
// HTTP wiring
// ---------------------------------------------------------------------------

pub struct TaskManagerHttpServer<T: TaskManagerHandler> {
    rpc_handler: buckyos_api::TaskManagerServerHandler<T>,
}

impl<T: TaskManagerHandler> TaskManagerHttpServer<T> {
    pub fn new(handler: T) -> Self {
        Self {
            rpc_handler: buckyos_api::TaskManagerServerHandler::new(handler),
        }
    }
}

#[async_trait]
impl<T: TaskManagerHandler + 'static> HttpServer for TaskManagerHttpServer<T> {
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
        "task-manager-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_task_manager_service() -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        TASK_MANAGER_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    if let Err(err) = runtime.login().await {
        error!("task manager service login to system failed! err:{:?}", err);
        return Err(RPCErrors::ReasonError(format!(
            "task manager login to system failed! err:{:?}",
            err
        )));
    }
    const TASK_MANAGER_SERVICE_MAIN_PORT: u16 = 3380;
    runtime
        .set_main_service_port(TASK_MANAGER_SERVICE_MAIN_PORT)
        .await;
    set_buckyos_api_runtime(runtime).map_err(|err| {
        RPCErrors::ReasonError(format!("register task manager runtime failed: {}", err))
    })?;

    let store = TaskStore::open_from_service_spec()
        .await
        .map_err(RPCErrors::ReasonError)?;
    info!("task-manager databases initialized (schema v8, User + System)");

    let kevent_client = get_buckyos_api_runtime()
        .map_err(|err| RPCErrors::ReasonError(format!("api runtime unavailable: {}", err)))?
        .get_kevent_client()
        .await?;
    let store = Arc::new(store);
    let handler = TaskManagerService::new(
        store.clone(),
        kevent_client,
        Arc::new(RuntimeSessionTokenVerifier),
    );
    handler.ensure_builtin_schemas().await?;
    let service_for_dispatcher = handler.clone();
    let service_for_dispatcher_failure = handler.clone();
    let server = TaskManagerHttpServer::new(handler);

    info!("start task manager 2.0 service...");
    let runner = Runner::new(TASK_MANAGER_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/task-manager".to_string(), Arc::new(server)) {
        error!("failed to add task manager http server: {:?}", err);
    }

    // Task Dispatch Center: same process/port, second kapi path, independent
    // store and authorization. The task service must keep working when the
    // dispatcher fails to start (missing rdb instance config on older
    // zones), so this is strictly best-effort.
    match crate::dispatcher::start_task_dispatcher(
        Arc::new(RuntimeSessionTokenVerifier),
        service_for_dispatcher,
    )
    .await
    {
        Ok(dispatcher_service) => {
            let dispatcher_server =
                crate::dispatcher::TaskDispatcherHttpServer::new(dispatcher_service);
            if let Err(err) = runner.add_http_server(
                "/kapi/task-dispatcher".to_string(),
                Arc::new(dispatcher_server),
            ) {
                error!("failed to add task dispatcher http server: {:?}", err);
            } else {
                info!("task dispatch center mounted at /kapi/task-dispatcher");
            }
        }
        Err(err) => {
            if let Err(recovery_err) = service_for_dispatcher_failure
                .recover_lost_user_tasks_by_origin(TASK_DISPATCHER_SERVICE_NAME, &HashSet::new())
                .await
            {
                warn!(
                    "could not converge restored User tasks after dispatcher startup failure: {}",
                    recovery_err
                );
            }
            warn!(
                "task dispatch center not started (task manager keeps running): {:?}",
                err
            );
        }
    }

    if let Err(err) = runner.run().await {
        error!("task manager runner exited with error: {:?}", err);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::task_store::TaskStore;
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use std::collections::HashMap;
    use tempfile::tempdir;

    // Fixed ed25519 test keypair (same material as the node_daemon tests).
    const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;
    const TEST_PUBLIC_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

    /// Verifies against the fixed test key: the real signature/exp path runs,
    /// only the trust-key lookup is replaced.
    struct StaticKeyVerifier {
        key: DecodingKey,
    }

    #[async_trait]
    impl SessionTokenVerifier for StaticKeyVerifier {
        async fn verify(&self, token: &str) -> Result<RPCSessionToken> {
            let mut parsed = RPCSessionToken::from_string(token)?;
            parsed.verify_by_key(&self.key)?;
            Ok(parsed)
        }
    }

    fn test_encoding_key() -> EncodingKey {
        EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap()
    }

    fn signed_token(
        user_id: &str,
        target: AuthTarget,
        principal_kind: TokenPrincipalKind,
    ) -> String {
        let now = buckyos_kit::buckyos_get_unix_timestamp();
        let mut session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(now + 3600),
            iss: Some(VERIFY_HUB_UNIQUE_ID.to_string()),
            jti: None,
            sub: Some(user_id.to_string()),
            appid: None,
            sudo: false,
            extra: HashMap::new(),
        };
        bind_token_principal_kind(&mut session_token, principal_kind);
        bind_token_target(&mut session_token, &target, TokenUse::Session).unwrap();
        session_token
            .generate_jwt(None, &test_encoding_key())
            .unwrap()
    }

    fn service_ctx(user_id: &str, service_id: &str) -> RPCContext {
        RPCContext {
            token: Some(signed_token(
                user_id,
                AuthTarget::system(SystemServiceId::parse(service_id).unwrap()),
                TokenPrincipalKind::System,
            )),
            ..Default::default()
        }
    }

    fn refreshed_service_ctx(user_id: &str, service_id: &str) -> RPCContext {
        service_ctx(user_id, service_id)
    }

    /// Interactive (verify-hub issued) session token: never zone-trusted.
    fn user_ctx(user_id: &str, app_id: &str) -> RPCContext {
        let target = if app_id == CONTROL_PANEL_SERVICE_NAME {
            AuthTarget::system(SystemServiceId::parse(app_id).unwrap())
        } else {
            AuthTarget::app(format!("{app_id}@{user_id}").parse().unwrap())
        };
        RPCContext {
            token: Some(signed_token(user_id, target, TokenPrincipalKind::User)),
            ..Default::default()
        }
    }

    // Shared with the dispatcher test module.
    pub(crate) fn service_ctx_pub(user_id: &str, app_id: &str) -> RPCContext {
        service_ctx(user_id, app_id)
    }

    pub(crate) fn user_ctx_pub(user_id: &str, app_id: &str) -> RPCContext {
        user_ctx(user_id, app_id)
    }

    pub(crate) fn static_verifier() -> Arc<dyn SessionTokenVerifier> {
        Arc::new(StaticKeyVerifier {
            key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
        })
    }

    pub(crate) async fn setup_service() -> (TaskManagerService, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let user_db_path = temp_dir.path().join("user.db");
        let system_db_path = temp_dir.path().join("system.db");
        let user_conn = format!("sqlite://{}?mode=rwc", user_db_path.to_str().unwrap());
        let system_conn = format!("sqlite://{}?mode=rwc", system_db_path.to_str().unwrap());
        let store = TaskStore::open_partitioned(
            &user_conn,
            RdbBackend::Sqlite,
            None,
            &system_conn,
            RdbBackend::Sqlite,
            None,
        )
        .await
        .unwrap();
        let verifier = StaticKeyVerifier {
            key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
        };
        let service = TaskManagerService::new(
            Arc::new(store),
            KEventClient::new_local(TASK_MANAGER_SERVICE_NAME),
            Arc::new(verifier),
        );
        service.ensure_builtin_schemas().await.unwrap();
        (service, temp_dir)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authentication_keeps_app_and_system_targets_distinct() {
        let (service, _tmp) = setup_service().await;
        let app_ctx = RPCContext {
            token: Some(signed_token(
                "alice",
                AuthTarget::app("control-panel@alice".parse().unwrap()),
                TokenPrincipalKind::User,
            )),
            ..Default::default()
        };
        let system_ctx = RPCContext {
            token: Some(signed_token(
                "alice",
                AuthTarget::system(SystemServiceId::parse("control-panel").unwrap()),
                TokenPrincipalKind::User,
            )),
            ..Default::default()
        };

        let app = service.authenticate(&app_ctx).await.unwrap();
        let system = service.authenticate(&system_ctx).await.unwrap();

        assert_eq!(app.app_id, "app:control-panel@alice");
        assert_eq!(system.app_id, "system:control-panel");
        assert_eq!(app.executor_app_id, "control-panel");
        assert_eq!(system.executor_app_id, "control-panel");
        assert_eq!(app.authorization_id, "app:control-panel");
        assert_eq!(system.authorization_id, "system:control-panel");
        assert_eq!(app.app_instance_id.as_deref(), Some("control-panel@alice"));
        assert_eq!(system.app_instance_id, None);
        assert_ne!(app.authorization_id, system.authorization_id);
        assert_ne!(app.actor_ref(), system.actor_ref());
    }

    fn raw_create_req(name: &str, key: &str) -> CreateTaskReq {
        CreateTaskReq {
            name: name.to_string(),
            schema_id: RAW_TASK_SCHEMA_ID.to_string(),
            schema_version: None,
            input: json!({"payload": name}),
            executor: CreateTaskExecutor::SelfApp {
                app_instance_id: None,
            },
            parent_id: None,
            child_control_policy: None,
            policy_preset: None,
            permission_boundary: false,
            storage_domain: None,
            idempotency_key: key.to_string(),
            retry_of: None,
            supersedes: None,
            message: None,
        }
    }

    fn envelope(task: &Task) -> RunnerWriteEnvelope {
        RunnerWriteEnvelope {
            task_id: task.task_id.clone(),
            app_instance_id: None,
            runner_epoch: task.runner_epoch,
            expected_revision: task.revision,
        }
    }

    /// A clean zone must be able to create every first-party production task
    /// without the owning service having published anything yet. Seeding
    /// swallows per-entry errors, so assert the rows actually landed.
    #[tokio::test(flavor = "current_thread")]
    async fn builtin_schemas_are_seeded_on_a_clean_store() {
        let (service, _tmp) = setup_service().await;
        let store = service.store();
        for definition in builtin_task_schemas() {
            let stored = store
                .get_schema(&definition.schema_id, None)
                .await
                .unwrap_or_else(|err| {
                    panic!(
                        "built-in schema {} not seeded: {:?}",
                        definition.schema_id, err
                    )
                });
            assert!(stored.enabled, "{} seeded disabled", definition.schema_id);
            assert_eq!(
                stored.allowed_executor_kinds,
                definition.allowed_executor_kinds
            );
        }

        // Re-running the bootstrap is a no-op, not an idempotency conflict.
        service.ensure_builtin_schemas().await.unwrap();

        // And a production schema is usable end to end on that clean store.
        let ctx = user_ctx("alice", "workflow");
        let task = service
            .handle_create_task(
                CreateTaskReq {
                    name: "run tree".into(),
                    schema_id: WORKFLOW_RUN_TREE_TASK_SCHEMA_ID.to_string(),
                    schema_version: None,
                    input: json!({"request": {"run_id": "r1"}}),
                    executor: CreateTaskExecutor::SelfApp {
                        app_instance_id: None,
                    },
                    parent_id: None,
                    child_control_policy: None,
                    policy_preset: None,
                    permission_boundary: false,
                    storage_domain: None,
                    idempotency_key: "wf-run-r1".into(),
                    retry_of: None,
                    supersedes: None,
                    message: None,
                },
                ctx,
            )
            .await
            .unwrap();
        assert_eq!(task.schema_id, WORKFLOW_RUN_TREE_TASK_SCHEMA_ID);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delegated_task_freezes_business_owner_and_runner_identity() {
        let (service, _tmp) = setup_service().await;
        let runner_ctx = refreshed_service_ctx("system", CONTROL_PANEL_SERVICE_NAME);
        let creator = ActorRef::from_auth_target(
            "alice",
            &AuthTarget::app("buckyos-tool@alice".parse().unwrap()),
        );
        let request = CreateDelegatedTaskReq {
            task_id: Some("t-0123456789abcdef0123456789abcdef".to_string()),
            name: "install demo".to_string(),
            schema_id: APP_INSTALL_TASK_SCHEMA_ID.to_string(),
            schema_version: None,
            input: json!({"request": "immutable"}),
            creator: creator.clone(),
            runner_app_instance_id: None,
            parent_id: None,
            child_control_policy: None,
            policy_preset: None,
            permission_boundary: false,
            storage_domain: Some(StorageDomain::System),
            idempotency_key: "alice-install-demo".to_string(),
            retry_of: None,
            supersedes: None,
            message: None,
        };
        let task = service
            .handle_create_delegated_task(request.clone(), runner_ctx.clone())
            .await
            .unwrap();
        assert_eq!(task.task_id, request.task_id.clone().unwrap());
        assert_eq!(task.creator, creator);
        assert!(matches!(
            task.executor,
            TaskExecutor::App { ref app_id, .. } if app_id == CONTROL_PANEL_SERVICE_NAME
        ));

        let owned = service
            .handle_get_task(
                GetTaskReq {
                    task_id: task.task_id.clone(),
                },
                user_ctx("alice", "buckyos-tool"),
            )
            .await
            .unwrap();
        assert_eq!(owned.task_id, task.task_id);
        assert!(service
            .handle_get_task(
                GetTaskReq {
                    task_id: task.task_id.clone(),
                },
                user_ctx("mallory", "buckyos-tool"),
            )
            .await
            .is_err());

        let replay = service
            .handle_create_delegated_task(request.clone(), runner_ctx.clone())
            .await
            .unwrap();
        assert_eq!(replay.task_id, task.task_id);
        let mut conflicting = request;
        conflicting.input = json!({"request": "different"});
        assert!(service
            .handle_create_delegated_task(conflicting, runner_ctx.clone())
            .await
            .is_err());

        let running = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: envelope(&task),
                },
                runner_ctx,
            )
            .await
            .unwrap();
        assert_eq!(running.phase, TaskPhase::Running);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canonical_delegated_creator_can_cancel_directly_and_via_service() {
        let (service, _tmp) = setup_service().await;
        let runner_ctx = refreshed_service_ctx("system", CONTROL_PANEL_SERVICE_NAME);
        let creator_target = AuthTarget::system(SystemServiceId::parse("buckycli").unwrap());
        let creator = ActorRef::from_auth_target("ood1", &creator_target);
        let request = CreateDelegatedTaskReq {
            task_id: Some("t-11111111111111111111111111111111".to_string()),
            name: "install direct cancel".to_string(),
            schema_id: APP_INSTALL_TASK_SCHEMA_ID.to_string(),
            schema_version: None,
            input: json!({"request": "direct"}),
            creator: creator.clone(),
            runner_app_instance_id: None,
            parent_id: None,
            child_control_policy: None,
            policy_preset: None,
            permission_boundary: false,
            storage_domain: Some(StorageDomain::System),
            idempotency_key: "direct-cancel".to_string(),
            retry_of: None,
            supersedes: None,
            message: None,
        };
        let direct_task = service
            .handle_create_delegated_task(request.clone(), runner_ctx.clone())
            .await
            .unwrap();
        let direct = service
            .handle_request_control(
                RequestControlReq {
                    task_id: direct_task.task_id,
                    action: TaskControlAction::Cancel,
                    request_id: "direct-cancel-request".to_string(),
                    recursive: false,
                    expected_revision: None,
                },
                service_ctx("ood1", "buckycli"),
            )
            .await
            .unwrap();
        let RequestControlResult::Task { task } = direct else {
            panic!("expected direct task control result")
        };
        assert_eq!(task.pending_control.unwrap().requested_by, creator);

        let delegated_request = CreateDelegatedTaskReq {
            task_id: Some("t-22222222222222222222222222222222".to_string()),
            idempotency_key: "delegated-cancel".to_string(),
            name: "install delegated cancel".to_string(),
            input: json!({"request": "delegated"}),
            ..request
        };
        let delegated_task = service
            .handle_create_delegated_task(delegated_request, runner_ctx.clone())
            .await
            .unwrap();
        let delegated = service
            .handle_request_delegated_control(
                RequestDelegatedControlReq {
                    controller: ActorRef::from_auth_target("ood1", &creator_target),
                    task_id: delegated_task.task_id,
                    action: TaskControlAction::Cancel,
                    request_id: "delegated-cancel-request".to_string(),
                    expected_revision: None,
                },
                runner_ctx,
            )
            .await
            .unwrap();
        let RequestControlResult::Task { task } = delegated else {
            panic!("expected delegated task control result")
        };
        assert_eq!(
            task.pending_control.unwrap().requested_by.app_id,
            "system:buckycli"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_self_app_task_and_run_to_success() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");

        let task = service
            .handle_create_task(raw_create_req("job1", "k1"), ctx.clone())
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Accepted);
        assert_eq!(task.runner_epoch, 1);
        assert_eq!(task.creator.user_id, "alice");
        assert!(task.task_id.starts_with("t-"));
        assert_eq!(task.root_id, task.task_id);

        let task = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: envelope(&task),
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Running);

        let task = service
            .handle_report_progress(
                ReportProgressReq {
                    envelope: envelope(&task),
                    progress: Some(json!({"percent": 50})),
                    message: Some("halfway".into()),
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(task.progress, Some(json!({"percent": 50})));

        let task = service
            .handle_commit_result(
                CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: json!({"ok": true}),
                    app_instance_id: None,
                    runner_epoch: Some(task.runner_epoch),
                    expected_revision: task.revision,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Terminal);
        assert_eq!(task.outcome, Some(TaskOutcome::Succeeded));
        assert_eq!(task.result, Some(json!({"ok": true})));
        assert_eq!(task.completed_by.as_ref().unwrap().user_id, "alice");

        // Result is one-shot; terminal is absorbing.
        let err = service
            .handle_commit_result(
                CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: json!({"ok": false}),
                    app_instance_id: None,
                    runner_epoch: Some(task.runner_epoch),
                    expected_revision: task.revision,
                },
                ctx.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_ALREADY_COMPLETED));

        // Events were journaled, one per revision.
        let events = service
            .handle_list_task_events(
                ListTaskEventsReq {
                    task_id: Some(task.task_id.clone()),
                    root_id: None,
                    after_event_id: None,
                    limit: None,
                },
                ctx,
            )
            .await
            .unwrap()
            .events;
        assert_eq!(events.len(), 4); // created, started, progress, committed
        assert_eq!(events[0].event_type, TaskEventType::TaskCreated);
        assert_eq!(
            events.last().unwrap().event_type,
            TaskEventType::ResultCommitted
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idempotent_create_replays_and_conflicts() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");

        let first = service
            .handle_create_task(raw_create_req("job", "same-key"), ctx.clone())
            .await
            .unwrap();
        let replay = service
            .handle_create_task(raw_create_req("job", "same-key"), ctx.clone())
            .await
            .unwrap();
        assert_eq!(first.task_id, replay.task_id);

        // Same key, different immutable input -> idempotency_conflict.
        let mut conflicting = raw_create_req("job", "same-key");
        conflicting.input = json!({"payload": "different"});
        let err = service
            .handle_create_task(conflicting, ctx)
            .await
            .unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_IDEMPOTENCY_CONFLICT)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_runner_epoch_and_revision_cas_are_fenced() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let task = service
            .handle_create_task(raw_create_req("fence", "kf"), ctx.clone())
            .await
            .unwrap();

        // Wrong epoch.
        let mut bad_epoch = envelope(&task);
        bad_epoch.runner_epoch = 99;
        let err = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: bad_epoch,
                },
                ctx.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_STALE_RUNNER_EPOCH));

        // Stale revision.
        let mut stale_rev = envelope(&task);
        stale_rev.expected_revision = task.revision + 5;
        let err = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: stale_rev,
                },
                ctx.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_REVISION_CONFLICT));

        // Another app cannot write runner state at all.
        let err = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: envelope(&task),
                },
                user_ctx("alice", "other-app"),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_PERMISSION_DENIED));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn human_set_concurrent_commit_single_winner() {
        let (service, _tmp) = setup_service().await;
        let creator = user_ctx("alice", "app-a");
        let mut req = raw_create_req("approval", "kh");
        req.executor = CreateTaskExecutor::HumanSet {
            assignees: vec!["bob".into(), "carol".into()],
        };
        let task = service.handle_create_task(req, creator).await.unwrap();
        assert_eq!(task.phase, TaskPhase::Waiting);
        assert_eq!(
            task.wait_reason.as_ref().unwrap().kind,
            TaskWaitReasonKind::HumanInput
        );
        assert_eq!(task.assignees.as_ref().unwrap().len(), 2);

        // Both assignees race a commit against the same revision.
        let bob = service.handle_commit_result(
            CommitResultReq {
                task_id: task.task_id.clone(),
                result: json!({"by": "bob"}),
                app_instance_id: None,
                runner_epoch: None,
                expected_revision: task.revision,
            },
            user_ctx("bob", "ui"),
        );
        let carol = service.handle_commit_result(
            CommitResultReq {
                task_id: task.task_id.clone(),
                result: json!({"by": "carol"}),
                app_instance_id: None,
                runner_epoch: None,
                expected_revision: task.revision,
            },
            user_ctx("carol", "ui"),
        );
        let (bob_result, carol_result) = tokio::join!(bob, carol);
        let winners = [bob_result.is_ok(), carol_result.is_ok()];
        assert_eq!(
            winners.iter().filter(|w| **w).count(),
            1,
            "exactly one commit wins"
        );

        // A non-assignee cannot commit.
        let final_task = service
            .store()
            .get_task(&task.task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(final_task.outcome, Some(TaskOutcome::Succeeded));
        let err = service
            .handle_commit_result(
                CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: json!({"by": "mallory"}),
                    app_instance_id: None,
                    runner_epoch: None,
                    expected_revision: final_task.revision,
                },
                user_ctx("mallory", "ui"),
            )
            .await
            .unwrap_err();
        assert!(task_mgr_error_code(&err).is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn assignee_update_races_commit_via_same_cas() {
        let (service, _tmp) = setup_service().await;
        let creator = user_ctx("alice", "app-a");
        let mut req = raw_create_req("handoff", "kr");
        req.executor = CreateTaskExecutor::HumanSet {
            assignees: vec!["bob".into()],
        };
        let task = service
            .handle_create_task(req, creator.clone())
            .await
            .unwrap();

        // bob hands the task to carol (bob keeps Reassign as assignee).
        let task = service
            .handle_update_assignees(
                UpdateAssigneesReq {
                    task_id: task.task_id.clone(),
                    add: vec!["carol".into()],
                    remove: vec!["bob".into()],
                    expected_revision: task.revision,
                },
                user_ctx("bob", "ui"),
            )
            .await
            .unwrap();
        assert_eq!(task.assignees.as_ref().unwrap(), &vec!["carol".to_string()]);

        // bob lost commit rights with the handoff.
        let err = service
            .handle_commit_result(
                CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: json!({}),
                    app_instance_id: None,
                    runner_epoch: None,
                    expected_revision: task.revision,
                },
                user_ctx("bob", "ui"),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_PERMISSION_DENIED));

        // Removing the last assignee is rejected.
        let err = service
            .handle_update_assignees(
                UpdateAssigneesReq {
                    task_id: task.task_id.clone(),
                    add: vec![],
                    remove: vec!["carol".into()],
                    expected_revision: task.revision,
                },
                user_ctx("carol", "ui"),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_INVALID_PHASE));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_protocol_pause_ack_cancel_supersede() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let task = service
            .handle_create_task(raw_create_req("ctl", "kc"), ctx.clone())
            .await
            .unwrap();
        let task = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: envelope(&task),
                },
                ctx.clone(),
            )
            .await
            .unwrap();

        // Runner declares pause support.
        let task = service
            .handle_update_control_profile(
                UpdateControlProfileReq {
                    envelope: envelope(&task),
                    profile: TaskControlProfile {
                        pause: ControlAvailability::Available,
                        resume: ControlAvailability::Available,
                        cancel: CancelCapability::Safe,
                        updated_at: 0,
                    },
                },
                ctx.clone(),
            )
            .await
            .unwrap();

        // Creator requests pause: recorded, not applied.
        let result = service
            .handle_request_control(
                RequestControlReq {
                    task_id: task.task_id.clone(),
                    action: TaskControlAction::Pause,
                    request_id: "req-pause".into(),
                    recursive: false,
                    expected_revision: None,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        let RequestControlResult::Task { task } = result else {
            panic!("expected single-task result")
        };
        assert_eq!(task.phase, TaskPhase::Running);
        assert_eq!(
            task.pending_control.as_ref().unwrap().action,
            TaskControlAction::Pause
        );

        // A second pause cannot stack; cancel supersedes.
        let err = service
            .handle_request_control(
                RequestControlReq {
                    task_id: task.task_id.clone(),
                    action: TaskControlAction::Pause,
                    request_id: "req-pause-2".into(),
                    recursive: false,
                    expected_revision: None,
                },
                ctx.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_CONTROL_ALREADY_PENDING)
        );
        let result = service
            .handle_request_control(
                RequestControlReq {
                    task_id: task.task_id.clone(),
                    action: TaskControlAction::Cancel,
                    request_id: "req-cancel".into(),
                    recursive: false,
                    expected_revision: None,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        let RequestControlResult::Task { task } = result else {
            panic!("expected single-task result")
        };
        assert_eq!(
            task.pending_control.as_ref().unwrap().action,
            TaskControlAction::Cancel
        );

        // Runner acks the cancel: task closes as Canceled.
        let task = service
            .handle_ack_control(
                AckControlReq {
                    envelope: envelope(&task),
                    request_id: "req-cancel".into(),
                    applied: true,
                    reject_reason: None,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Terminal);
        assert_eq!(task.outcome, Some(TaskOutcome::Canceled));
        assert!(task.pending_control.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recursive_control_respects_child_policy_and_acl() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let root = service
            .handle_create_task(raw_create_req("root", "kroot"), ctx.clone())
            .await
            .unwrap();

        // Child that follows cancel.
        let mut follow = raw_create_req("follow", "kfollow");
        follow.parent_id = Some(root.task_id.clone());
        let follow = service
            .handle_create_task(follow, ctx.clone())
            .await
            .unwrap();
        assert_eq!(follow.root_id, root.task_id);

        // Child that opted out of cancel propagation.
        let mut opt_out = raw_create_req("optout", "koptout");
        opt_out.parent_id = Some(root.task_id.clone());
        opt_out.child_control_policy = Some(ChildControlPolicy {
            follow_pause: true,
            follow_resume: true,
            follow_cancel: false,
        });
        let opt_out = service
            .handle_create_task(opt_out, ctx.clone())
            .await
            .unwrap();

        let result = service
            .handle_request_control(
                RequestControlReq {
                    task_id: root.task_id.clone(),
                    action: TaskControlAction::Cancel,
                    request_id: "req-tree".into(),
                    recursive: true,
                    expected_revision: None,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        let RequestControlResult::Batch { result } = result else {
            panic!("expected batch result")
        };
        assert!(result.requested.contains(&root.task_id));
        assert!(result.requested.contains(&follow.task_id));
        assert!(result.skipped_by_policy.contains(&opt_out.task_id));

        // App tasks got a pending request, not a forced state.
        let follow_now = service
            .store()
            .get_task(&follow.task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(follow_now.phase, TaskPhase::Accepted);
        assert_eq!(
            follow_now.pending_control.as_ref().unwrap().action,
            TaskControlAction::Cancel
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn acl_hides_foreign_tasks_and_supports_grants() {
        let (service, _tmp) = setup_service().await;
        let alice = user_ctx("alice", "app-a");
        let task = service
            .handle_create_task(raw_create_req("private", "kacl"), alice.clone())
            .await
            .unwrap();

        // A stranger sees task_not_found (not permission_denied).
        let err = service
            .handle_get_task(
                GetTaskReq {
                    task_id: task.task_id.clone(),
                },
                user_ctx("bob", "app-b"),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_NOT_FOUND));

        // Alice grants bob read access.
        let task = service
            .handle_grant_task_access(
                GrantTaskAccessReq {
                    task_id: task.task_id.clone(),
                    grant: TaskAclGrantSpec {
                        subject: TaskGrantSubject::User {
                            user_id: "bob".into(),
                        },
                        actions: vec![TaskAction::ReadMeta, TaskAction::ReadInput],
                        scope: TaskGrantScope::SelfOnly,
                        data_scope: TaskDataScope::Payload,
                    },
                    expected_revision: task.revision,
                },
                alice.clone(),
            )
            .await
            .unwrap();

        let seen = service
            .handle_get_task(
                GetTaskReq {
                    task_id: task.task_id.clone(),
                },
                user_ctx("bob", "app-b"),
            )
            .await
            .unwrap();
        assert_eq!(seen.input, json!({"payload": "private"}));
        // Payload scope hides the audit envelope; no ReadResult grant.
        assert_eq!(seen.idempotency_key, "");
        assert_eq!(seen.data_scope, Some(TaskDataScope::Payload));

        // Revoke closes the door again.
        let grants = service
            .handle_list_task_access(
                ListTaskAccessReq {
                    task_id: task.task_id.clone(),
                },
                alice.clone(),
            )
            .await
            .unwrap()
            .grants;
        let task = service
            .handle_revoke_task_access(
                RevokeTaskAccessReq {
                    task_id: task.task_id.clone(),
                    grant_id: grants[0].grant_id.clone(),
                    expected_revision: task.revision,
                },
                alice,
            )
            .await
            .unwrap();
        let err = service
            .handle_get_task(
                GetTaskReq {
                    task_id: task.task_id,
                },
                user_ctx("bob", "app-b"),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_NOT_FOUND));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn permission_boundary_blocks_root_creator() {
        let (service, _tmp) = setup_service().await;
        let alice = user_ctx("alice", "app-a");
        let root = service
            .handle_create_task(raw_create_req("tree-root", "kb-root"), alice.clone())
            .await
            .unwrap();

        // Bob is granted CreateChild on the root subtree so he can attach a
        // boundary-protected child.
        let _ = service
            .handle_grant_task_access(
                GrantTaskAccessReq {
                    task_id: root.task_id.clone(),
                    grant: TaskAclGrantSpec {
                        subject: TaskGrantSubject::User {
                            user_id: "bob".into(),
                        },
                        actions: vec![TaskAction::ReadMeta, TaskAction::CreateChild],
                        scope: TaskGrantScope::Subtree,
                        data_scope: TaskDataScope::MetaOnly,
                    },
                    expected_revision: root.revision,
                },
                alice.clone(),
            )
            .await
            .unwrap();

        let mut sealed = raw_create_req("sealed", "kb-child");
        sealed.parent_id = Some(root.task_id.clone());
        sealed.permission_boundary = true;
        let sealed = service
            .handle_create_task(sealed, user_ctx("bob", "app-b"))
            .await
            .unwrap();

        // Root creator alice can still see tree metadata as a participant,
        // but the boundary cuts her preset control/read-payload inheritance.
        let seen = service
            .handle_get_task(
                GetTaskReq {
                    task_id: sealed.task_id.clone(),
                },
                alice.clone(),
            )
            .await;
        match seen {
            Ok(task) => {
                assert_eq!(task.data_scope, Some(TaskDataScope::MetaOnly));
                assert_eq!(task.input, Value::Null);
            }
            Err(err) => {
                assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_NOT_FOUND));
            }
        }
        // And she cannot control it.
        let err = service
            .handle_request_control(
                RequestControlReq {
                    task_id: sealed.task_id.clone(),
                    action: TaskControlAction::Cancel,
                    request_id: "req-x".into(),
                    recursive: false,
                    expected_revision: None,
                },
                alice,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_PERMISSION_DENIED) | Some(TASK_ERR_NOT_FOUND)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn schema_validation_gates_input_and_result() {
        let (service, _tmp) = setup_service().await;
        let publisher = user_ctx("alice", "app-a");
        let _ = service
            .handle_register_task_schema(
                RegisterTaskSchemaReq {
                    definition: TaskSchemaDefinition {
                        schema_id: "test.op/v1".into(),
                        schema_version: 1,
                        input_schema: json!({"type": "object", "required": ["url"], "properties": {"url": {"type": "string"}}}),
                        output_schema: json!({"type": "object", "required": ["code"], "properties": {"code": {"type": "integer"}}}),
                        presentation_schema: None,
                        allowed_executor_kinds: vec![TaskExecutorKind::App],
                        user_creatable: true,
                        default_storage_domain: StorageDomain::User,
                        publisher_app_id: "app-a".into(),
                        enabled: true,
                        created_at: 0,
                    },
                },
                publisher.clone(),
            )
            .await
            .unwrap();

        // Bad input rejected at create.
        let mut bad = raw_create_req("typed", "kt1");
        bad.schema_id = "test.op/v1".into();
        bad.input = json!({"nope": 1});
        let err = service
            .handle_create_task(bad, publisher.clone())
            .await
            .unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_INPUT_SCHEMA_MISMATCH)
        );

        // Good input accepted; bad result rejected without changing the task.
        let mut good = raw_create_req("typed", "kt2");
        good.schema_id = "test.op/v1".into();
        good.input = json!({"url": "http://x"});
        let task = service
            .handle_create_task(good, publisher.clone())
            .await
            .unwrap();
        let err = service
            .handle_commit_result(
                CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: json!({"code": "not-an-int"}),
                    app_instance_id: None,
                    runner_epoch: Some(task.runner_epoch),
                    expected_revision: task.revision,
                },
                publisher.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_RESULT_SCHEMA_MISMATCH)
        );
        let unchanged = service
            .store()
            .get_task(&task.task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.revision, task.revision);
        assert!(unchanged.result.is_none());

        // HumanSet not allowed by this schema.
        let mut wrong_kind = raw_create_req("typed", "kt3");
        wrong_kind.schema_id = "test.op/v1".into();
        wrong_kind.input = json!({"url": "http://x"});
        wrong_kind.executor = CreateTaskExecutor::HumanSet {
            assignees: vec!["bob".into()],
        };
        let err = service
            .handle_create_task(wrong_kind, publisher)
            .await
            .unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_INPUT_SCHEMA_MISMATCH)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn trusted_promise_lifecycle_bind_release_fence() {
        let (service, _tmp) = setup_service().await;
        let dispatcher = service.clone();
        let actor = ActorRef::new("task-dispatcher", "task-dispatcher");

        // Interactive callers cannot mint promised tasks.
        let promised_req = CreatePromisedTaskReq {
            name: "promised".into(),
            schema_id: RAW_TASK_SCHEMA_ID.into(),
            schema_version: None,
            input: json!({"x": 1}),
            creator: ActorRef::new("alice", "app-a"),
            expected_input_digest: None,
            origin_ref: Some(TaskOriginRef {
                kind: "task-dispatcher".into(),
                id: "dsp-1".into(),
            }),
            parent_id: None,
            child_control_policy: None,
            policy_preset: None,
            permission_boundary: false,
            storage_domain: None,
            idempotency_key: "dsp:dsp-1".into(),
            wait_reason: None,
            message: None,
        };
        let err = service
            .handle_create_promised_task(promised_req.clone(), user_ctx("alice", "app-a"))
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_PERMISSION_DENIED));

        let task = dispatcher
            .trusted_create_promised_task(promised_req.clone(), &actor)
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Promised);
        assert_eq!(task.executor, TaskExecutor::Unbound);
        assert_eq!(task.creator.user_id, "alice");
        assert_eq!(task.runner_epoch, 0);

        // Idempotent replay returns the same task.
        let replay = dispatcher
            .trusted_create_promised_task(promised_req, &actor)
            .await
            .unwrap();
        assert_eq!(replay.task_id, task.task_id);

        // Bind -> Accepted, epoch 1.
        let task = dispatcher
            .trusted_bind_app_executor(
                BindAppExecutorReq {
                    task_id: task.task_id.clone(),
                    target_id: Some("target-1".into()),
                    app_id: "runner-app".into(),
                    app_instance_id: "inst-1".into(),
                    delivery_id: Some("dsp-1#1".into()),
                    expected_revision: task.revision,
                },
                &actor,
            )
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Accepted);
        assert_eq!(task.runner_epoch, 1);

        // Runner starts, then the instance is lost and released.
        let runner_ctx = user_ctx("svc", "runner-app");
        let task = service
            .handle_report_started(
                ReportStartedReq {
                    envelope: RunnerWriteEnvelope {
                        task_id: task.task_id.clone(),
                        app_instance_id: Some("inst-1".into()),
                        runner_epoch: 1,
                        expected_revision: task.revision,
                    },
                },
                runner_ctx.clone(),
            )
            .await
            .unwrap();
        let task = dispatcher
            .trusted_release_app_executor(
                ReleaseAppExecutorReq {
                    task_id: task.task_id.clone(),
                    expected_instance_id: "inst-1".into(),
                    expected_runner_epoch: 1,
                    reason: TaskWaitReason::with_code(
                        TaskWaitReasonKind::Capacity,
                        "instance_lost",
                    ),
                    expected_revision: task.revision,
                },
                &actor,
            )
            .await
            .unwrap();
        assert_eq!(task.phase, TaskPhase::Waiting);
        assert_eq!(task.runner_epoch, 2);
        assert!(matches!(
            &task.executor,
            TaskExecutor::App {
                app_instance_id: None,
                ..
            }
        ));

        // Old instance's late write is fenced.
        let err = service
            .handle_report_progress(
                ReportProgressReq {
                    envelope: RunnerWriteEnvelope {
                        task_id: task.task_id.clone(),
                        app_instance_id: Some("inst-1".into()),
                        runner_epoch: 1,
                        expected_revision: task.revision,
                    },
                    progress: Some(json!({"late": true})),
                    message: None,
                },
                runner_ctx.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_STALE_RUNNER_EPOCH));

        // Rebind on the same frozen target with a new instance.
        let task = dispatcher
            .trusted_bind_app_executor(
                BindAppExecutorReq {
                    task_id: task.task_id.clone(),
                    target_id: Some("target-1".into()),
                    app_id: "runner-app".into(),
                    app_instance_id: "inst-2".into(),
                    delivery_id: None,
                    expected_revision: task.revision,
                },
                &actor,
            )
            .await
            .unwrap();
        assert_eq!(task.runner_epoch, 3);
        assert_eq!(task.phase, TaskPhase::Accepted);

        // A different logical target is frozen out.
        let task_now = service
            .store()
            .get_task(&task.task_id)
            .await
            .unwrap()
            .unwrap();
        let released = dispatcher
            .trusted_release_app_executor(
                ReleaseAppExecutorReq {
                    task_id: task.task_id.clone(),
                    expected_instance_id: "inst-2".into(),
                    expected_runner_epoch: 3,
                    reason: TaskWaitReason::new(TaskWaitReasonKind::Capacity),
                    expected_revision: task_now.revision,
                },
                &actor,
            )
            .await
            .unwrap();
        let err = dispatcher
            .trusted_bind_app_executor(
                BindAppExecutorReq {
                    task_id: task.task_id.clone(),
                    target_id: Some("target-2".into()),
                    app_id: "runner-app".into(),
                    app_instance_id: "inst-9".into(),
                    delivery_id: None,
                    expected_revision: released.revision,
                },
                &actor,
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_INVALID_PHASE));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn human_set_cancel_closes_directly() {
        let (service, _tmp) = setup_service().await;
        let alice = user_ctx("alice", "app-a");
        let mut req = raw_create_req("cancelable", "khc");
        req.executor = CreateTaskExecutor::HumanSet {
            assignees: vec!["bob".into()],
        };
        let task = service
            .handle_create_task(req, alice.clone())
            .await
            .unwrap();

        let result = service
            .handle_request_control(
                RequestControlReq {
                    task_id: task.task_id.clone(),
                    action: TaskControlAction::Cancel,
                    request_id: "req-hc".into(),
                    recursive: false,
                    expected_revision: None,
                },
                alice,
            )
            .await
            .unwrap();
        let RequestControlResult::Task { task } = result else {
            panic!("expected task result")
        };
        assert_eq!(task.phase, TaskPhase::Terminal);
        assert_eq!(task.outcome, Some(TaskOutcome::Canceled));
    }

    /// `rbac::SYS_ENFORCE` is process-wide, so the tests that install a policy
    /// must not overlap each other.
    static ENFORCER_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

    /// Installs a policy shaped like a real zone's: the owner bound to
    /// `admin`, the Task Center's `control-panel` to `kernel`, and the agent
    /// that creates the tasks left in `agent`.
    async fn install_zone_like_enforcer() {
        let config = build_current_rbac_config(Some(
            "g, devtest, admin\ng, bob, users\ng, system:control-panel, kernel\ng, app:buckyos-jarvis, agent",
        ));
        rbac::create_enforcer(&config.model, &config.policy)
            .await
            .unwrap();
    }

    /// The Task Center runs as `control-panel`, so every relation in the
    /// preset — all keyed on the exact `{user_id, app_id}` — misses tasks the
    /// user's own agent created. RBAC's `obj://task/{user}` is what closes it.
    #[tokio::test(flavor = "current_thread")]
    async fn control_panel_sees_tasks_created_by_the_users_own_agent() {
        let _guard = ENFORCER_LOCK.get_or_init(Default::default).lock().await;
        install_zone_like_enforcer().await;
        let (service, _tmp) = setup_service().await;

        let agent = user_ctx("devtest", "buckyos-jarvis");
        let task = service
            .handle_create_task(raw_create_req("aicc:job-1", "k-aicc-1"), agent)
            .await
            .unwrap();

        // The owner, through the control surface.
        let page = service
            .handle_list_tasks(
                ListTasksReq::default(),
                user_ctx("devtest", "control-panel"),
            )
            .await
            .unwrap();
        assert!(
            page.tasks.iter().any(|t| t.task_id == task.task_id),
            "control-panel must list the owner's agent tasks"
        );

        // Full data scope: the Task Center renders progress and the scheduled
        // task payload straight off `input`.
        let detail = service
            .handle_get_task(
                GetTaskReq {
                    task_id: task.task_id.clone(),
                },
                user_ctx("devtest", "control-panel"),
            )
            .await
            .unwrap();
        assert_eq!(detail.data_scope, Some(TaskDataScope::Full));
        assert_eq!(detail.input, json!({"payload": "aicc:job-1"}));

        // Another user gets nothing, from the same control surface.
        let page = service
            .handle_list_tasks(ListTasksReq::default(), user_ctx("bob", "control-panel"))
            .await
            .unwrap();
        assert!(
            page.tasks.is_empty(),
            "the control surface must stay scoped to the requesting user"
        );

        // And an ordinary app of the same user still cannot cross app_id.
        let page = service
            .handle_list_tasks(
                ListTasksReq::default(),
                user_ctx("devtest", "buckyos-filebrowser"),
            )
            .await
            .unwrap();
        assert!(
            page.tasks.is_empty(),
            "a non-control-surface app must keep the doc §8.5 isolation"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn archive_requires_terminal_and_hides_from_default_list() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let task = service
            .handle_create_task(raw_create_req("arch", "ka"), ctx.clone())
            .await
            .unwrap();

        let err = service
            .handle_archive_task(
                ArchiveTaskReq {
                    task_id: task.task_id.clone(),
                    expected_revision: task.revision,
                },
                ctx.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(task_mgr_error_code(&err), Some(TASK_ERR_INVALID_PHASE));

        let task = service
            .handle_commit_result(
                CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: json!({}),
                    app_instance_id: None,
                    runner_epoch: Some(task.runner_epoch),
                    expected_revision: task.revision,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        let task = service
            .handle_archive_task(
                ArchiveTaskReq {
                    task_id: task.task_id.clone(),
                    expected_revision: task.revision,
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert!(task.archived_at.is_some());

        let page = service
            .handle_list_tasks(ListTasksReq::default(), ctx.clone())
            .await
            .unwrap();
        assert!(page.tasks.iter().all(|t| t.task_id != task.task_id));
        let page = service
            .handle_list_tasks(
                ListTasksReq {
                    include_archived: true,
                    ..Default::default()
                },
                ctx,
            )
            .await
            .unwrap();
        assert!(page.tasks.iter().any(|t| t.task_id == task.task_id));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_tasks_filters_by_creator_scoped_idempotency_key() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let first = service
            .handle_create_task(raw_create_req("first", "key-first"), ctx.clone())
            .await
            .unwrap();
        let second = service
            .handle_create_task(raw_create_req("second", "key-second"), ctx.clone())
            .await
            .unwrap();

        let page = service
            .handle_list_tasks(
                ListTasksReq {
                    creator_user_id: Some("alice".to_string()),
                    creator_app_id: Some("app:app-a@alice".to_string()),
                    idempotency_key: Some("key-second".to_string()),
                    include_archived: true,
                    ..Default::default()
                },
                ctx,
            )
            .await
            .unwrap();
        assert_eq!(page.tasks.len(), 1);
        assert_eq!(page.tasks[0].task_id, second.task_id);
        assert_ne!(page.tasks[0].task_id, first.task_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn storage_domain_is_frozen_inherited_and_cross_domain_parent_is_rejected() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");

        let mut root_req = raw_create_req("user-root", "domain-root");
        root_req.storage_domain = Some(StorageDomain::User);
        let root = service
            .handle_create_task(root_req.clone(), ctx.clone())
            .await
            .unwrap();
        assert_eq!(root.storage_domain, StorageDomain::User);

        let mut schema_default_req = raw_create_req("download", "domain-schema-default");
        schema_default_req.schema_id = DOWNLOAD_TASK_SCHEMA_ID.to_string();
        let schema_default = service
            .handle_create_task(schema_default_req, ctx.clone())
            .await
            .unwrap();
        assert_eq!(schema_default.storage_domain, StorageDomain::User);

        let mut child_req = raw_create_req("child", "domain-child");
        child_req.parent_id = Some(root.task_id.clone());
        let child = service
            .handle_create_task(child_req, ctx.clone())
            .await
            .unwrap();
        assert_eq!(child.storage_domain, StorageDomain::User);

        let mut conflict = raw_create_req("bad-child", "domain-bad-child");
        conflict.parent_id = Some(root.task_id.clone());
        conflict.storage_domain = Some(StorageDomain::System);
        let err = service
            .handle_create_task(conflict, ctx.clone())
            .await
            .unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_STORAGE_DOMAIN_CONFLICT)
        );

        root_req.storage_domain = Some(StorageDomain::System);
        let err = service.handle_create_task(root_req, ctx).await.unwrap_err();
        assert_eq!(
            task_mgr_error_code(&err),
            Some(TASK_ERR_IDEMPOTENCY_CONFLICT)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_tasks_merges_and_filters_storage_domains_with_composite_cursor() {
        let (service, _tmp) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let mut user_req = raw_create_req("user", "domain-list-user");
        user_req.storage_domain = Some(StorageDomain::User);
        let user = service
            .handle_create_task(user_req, ctx.clone())
            .await
            .unwrap();
        let system = service
            .handle_create_task(raw_create_req("system", "domain-list-system"), ctx.clone())
            .await
            .unwrap();
        assert_eq!(system.storage_domain, StorageDomain::System);

        let system_page = service
            .handle_list_tasks(
                ListTasksReq {
                    storage_domain: Some(StorageDomain::System),
                    include_archived: true,
                    ..Default::default()
                },
                ctx.clone(),
            )
            .await
            .unwrap();
        assert_eq!(system_page.tasks.len(), 1);
        assert_eq!(system_page.tasks[0].task_id, system.task_id);

        let mut cursor = None;
        let mut task_ids = Vec::new();
        loop {
            let page = service
                .handle_list_tasks(
                    ListTasksReq {
                        include_archived: true,
                        cursor: cursor.clone(),
                        limit: Some(1),
                        ..Default::default()
                    },
                    ctx.clone(),
                )
                .await
                .unwrap();
            task_ids.extend(page.tasks.into_iter().map(|task| task.task_id));
            let Some(next) = page.next_cursor else {
                break;
            };
            assert!(next.starts_with("v1|"));
            cursor = Some(next);
        }
        task_ids.sort();
        let mut expected = vec![user.task_id, system.task_id];
        expected.sort();
        assert_eq!(task_ids, expected);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deleting_local_store_preserves_user_tasks_and_drops_system_tasks() {
        let (service, temp_dir) = setup_service().await;
        let ctx = user_ctx("alice", "app-a");
        let mut user_req = raw_create_req("durable", "domain-durable");
        user_req.storage_domain = Some(StorageDomain::User);
        let user = service
            .handle_create_task(user_req, ctx.clone())
            .await
            .unwrap();
        let system = service
            .handle_create_task(raw_create_req("local", "domain-local"), ctx)
            .await
            .unwrap();
        drop(service);

        let user_db_path = temp_dir.path().join("user.db");
        let system_db_path = temp_dir.path().join("system.db");
        std::fs::remove_file(&system_db_path).unwrap();
        let user_conn = format!("sqlite://{}?mode=rwc", user_db_path.to_str().unwrap());
        let system_conn = format!("sqlite://{}?mode=rwc", system_db_path.to_str().unwrap());
        let store = TaskStore::open_partitioned(
            &user_conn,
            RdbBackend::Sqlite,
            None,
            &system_conn,
            RdbBackend::Sqlite,
            None,
        )
        .await
        .unwrap();
        assert!(store.get_task(&user.task_id).await.unwrap().is_some());
        assert!(store.get_task(&system.task_id).await.unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn partition_schema_version_mismatch_stops_open() {
        let (service, temp_dir) = setup_service().await;
        drop(service);
        let user_db_path = temp_dir.path().join("user.db");
        let system_db_path = temp_dir.path().join("system.db");
        let user_conn = format!("sqlite://{}?mode=rwc", user_db_path.to_str().unwrap());
        let system_conn = format!("sqlite://{}?mode=rwc", system_db_path.to_str().unwrap());
        let pool = sqlx::any::AnyPoolOptions::new()
            .connect(&system_conn)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE task RENAME TO task_v8")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE task_store_meta")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE task (task_id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let err = TaskStore::open_partitioned(
            &user_conn,
            RdbBackend::Sqlite,
            None,
            &system_conn,
            RdbBackend::Sqlite,
            None,
        )
        .await
        .err()
        .expect("mismatched System store version must fail startup");
        assert!(err.contains("User=8, System=7"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restored_user_dispatch_task_without_dispatch_record_fails_runner_lost() {
        let (service, _tmp) = setup_service().await;
        let actor = ActorRef::new("task-dispatcher", "system:task-dispatcher");
        let task = service
            .trusted_create_promised_task(
                CreatePromisedTaskReq {
                    name: "restored".into(),
                    schema_id: RAW_TASK_SCHEMA_ID.into(),
                    schema_version: None,
                    input: json!({"restored": true}),
                    creator: ActorRef::new("alice", "app:app-a@alice"),
                    expected_input_digest: None,
                    origin_ref: Some(TaskOriginRef {
                        kind: TASK_DISPATCHER_SERVICE_NAME.into(),
                        id: "lost-dispatch".into(),
                    }),
                    parent_id: None,
                    child_control_policy: None,
                    policy_preset: None,
                    permission_boundary: false,
                    storage_domain: Some(StorageDomain::User),
                    idempotency_key: "lost-dispatch".into(),
                    wait_reason: None,
                    message: None,
                },
                &actor,
            )
            .await
            .unwrap();

        let recovered = service
            .recover_lost_user_tasks_by_origin(TASK_DISPATCHER_SERVICE_NAME, &HashSet::new())
            .await
            .unwrap();
        assert_eq!(recovered, 1);
        let failed = service
            .trusted_get_task(&task.task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.phase, TaskPhase::Terminal);
        assert_eq!(failed.outcome, Some(TaskOutcome::Failed));
        assert_eq!(
            failed.error.as_ref().map(|error| error.code.as_str()),
            Some("runner_lost")
        );
        let events = service
            .store()
            .list_events(Some(&task.task_id), None, None, 100)
            .await
            .unwrap();
        assert_eq!(
            events.last().map(|event| event.event_type),
            Some(TaskEventType::TaskFailed)
        );
    }
}
