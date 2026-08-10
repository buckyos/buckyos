//! Dispatcher 2.0 protocol tests: saga recovery, deterministic delivery,
//! offer/bind/activate fencing, approval gate and cancel convergence.

use super::dispatch_db::DispatchDb;
use super::service::{RunnerCaller, TaskDispatcherService};
use crate::server::tests::setup_service;
use crate::server::TaskManagerService;
use async_trait::async_trait;
use buckyos_api::*;
use kRPC::{RPCContext, RPCErrors, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Scripted runner transport: pops one response per call and journals every
/// request so tests can assert idempotent replays.
#[derive(Default)]
struct MockRunnerCaller {
    responses: Mutex<VecDeque<Result<Value>>>,
    calls: Mutex<Vec<(String, String, Value)>>,
    tokens: Mutex<Vec<Option<String>>>,
}

impl MockRunnerCaller {
    fn push_offer_accepted(&self, instance: &str, token: &str) {
        self.responses.lock().unwrap().push_back(Ok(json!({
            "kind": "OfferAccepted",
            "app_instance_id": instance,
            "reservation_token": token,
        })));
    }

    fn push_busy(&self, retry_after_ms: Option<u64>) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Ok(json!({"kind": "Busy", "retry_after_ms": retry_after_ms})));
    }

    fn push_rejected(&self, reason: &str) {
        self.responses.lock().unwrap().push_back(Ok(json!({
            "kind": "Rejected",
            "stable_reason": reason,
        })));
    }

    fn push_activated(&self) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Ok(json!({"activated": true})));
    }

    fn push_transport_error(&self) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Err(RPCErrors::ReasonError("connect refused".into())));
    }

    fn calls(&self) -> Vec<(String, String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    fn tokens(&self) -> Vec<Option<String>> {
        self.tokens.lock().unwrap().clone()
    }
}

#[async_trait]
impl RunnerCaller for MockRunnerCaller {
    async fn call(
        &self,
        endpoint: &str,
        method: &str,
        params: Value,
        _timeout_ms: u64,
        auth_token: Option<String>,
    ) -> Result<Value> {
        self.calls
            .lock()
            .unwrap()
            .push((endpoint.to_string(), method.to_string(), params));
        self.tokens.lock().unwrap().push(auth_token);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(RPCErrors::ReasonError("no scripted response".into())))
    }
}

struct TestEnv {
    task_core: TaskManagerService,
    dispatcher: TaskDispatcherService,
    caller: Arc<MockRunnerCaller>,
    _tmp: tempfile::TempDir,
    _tmp2: tempfile::TempDir,
}

async fn setup_env() -> TestEnv {
    let (task_core, tmp) = setup_service().await;
    let tmp2 = tempfile::tempdir().unwrap();
    let db_path = tmp2.path().join("dispatch.db");
    let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
    let db = DispatchDb::open(&conn, RdbBackend::Sqlite, None).await.unwrap();
    let caller = Arc::new(MockRunnerCaller::default());
    let dispatcher = TaskDispatcherService::new(
        Arc::new(db),
        task_core.clone(),
        crate::server::tests::static_verifier(),
        caller.clone(),
        None,
    );
    TestEnv {
        task_core,
        dispatcher,
        caller,
        _tmp: tmp,
        _tmp2: tmp2,
    }
}

fn service_ctx(user: &str, app: &str) -> RPCContext {
    crate::server::tests::service_ctx_pub(user, app)
}

fn user_ctx(user: &str, app: &str) -> RPCContext {
    crate::server::tests::user_ctx_pub(user, app)
}

fn test_registration(target_id: &str, approval: DispatchApprovalPolicy) -> TargetRegistration {
    TargetRegistration {
        target_id: target_id.to_string(),
        owner_user_id: String::new(),
        owner_app_id: String::new(),
        functions: vec![RunnerFunctionDescriptor::new(RAW_TASK_SCHEMA_ID)],
        auth_policy: DispatchAuthPolicy::ZoneUsers,
        approval_policy: approval,
        delivery_policy: DeliveryPolicy {
            max_attempts: 3,
            ..Default::default()
        },
        max_concurrency: 4,
        enabled: true,
        registration_revision: 0,
    }
}

async fn register_and_attach(env: &TestEnv, target_id: &str) {
    env.dispatcher
        .handle_register_target(
            RegisterTargetReq {
                registration: test_registration(target_id, DispatchApprovalPolicy::Never),
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    env.dispatcher
        .handle_attach_instance(
            AttachInstanceReq {
                target_id: target_id.to_string(),
                instance_id: "inst-1".into(),
                endpoint: "http://127.0.0.1:39321/kapi/runner".into(),
                capacity: 4,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
}

fn dispatch_req(key: &str) -> DispatchTaskReq {
    DispatchTaskReq {
        target_id: Some("target-1".into()),
        schema_id: RAW_TASK_SCHEMA_ID.into(),
        schema_version: None,
        name: Some("dispatched job".into()),
        input: json!({"work": key}),
        idempotency_key: key.to_string(),
        priority: None,
        expires_at: None,
        on_behalf_of: None,
        workflow_ref: None,
        parent_task_id: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_happy_path_offer_bind_activate() {
    let env = setup_env().await;
    register_and_attach(&env, "target-1").await;

    let result = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("job-1"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    // The public task exists immediately with a stable id.
    assert!(result.task_id.starts_with("t-"));
    assert_eq!(result.status, DispatchStatus::Queued);
    assert_eq!(result.task.phase, TaskPhase::Promised);
    assert_eq!(result.task.creator.user_id, "alice");
    assert_eq!(
        result.task.origin_ref.as_ref().unwrap().kind,
        TASK_DISPATCHER_SERVICE_NAME
    );

    env.caller.push_offer_accepted("inst-1", "res-1");
    env.caller.push_activated();
    env.dispatcher.evaluate_once(false).await;

    let record = env
        .dispatcher
        .db()
        .get_record(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Accepted);
    assert_eq!(record.attempt_count, 1);

    // The same public task is now bound and Accepted, epoch 1.
    let task = env
        .task_core
        .trusted_get_task(&result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.phase, TaskPhase::Accepted);
    assert_eq!(task.runner_epoch, 1);
    match &task.executor {
        TaskExecutor::App {
            target_id,
            app_id,
            app_instance_id,
        } => {
            assert_eq!(target_id.as_deref(), Some("target-1"));
            assert_eq!(app_id, "runner-app");
            assert_eq!(app_instance_id.as_deref(), Some("inst-1"));
        }
        other => panic!("unexpected executor {:?}", other),
    }

    // Exactly one offer + one activate, carrying the delivery id and epoch.
    let calls = env.caller.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, "offer_task");
    assert_eq!(
        calls[0].2["delivery_id"],
        json!(format!("{}#1", result.dispatch_id))
    );
    assert_eq!(calls[1].1, "activate_task");
    assert_eq!(calls[1].2["runner_epoch"], json!(1));
    assert_eq!(calls[1].2["reservation_token"], json!("res-1"));

    // The delivery journal is complete.
    let attempt = env
        .dispatcher
        .db()
        .latest_attempt(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempt.outcome, Some(DeliveryOutcome::Activated));
    assert_eq!(attempt.runner_epoch, Some(1));
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_idempotent_replay_returns_same_task() {
    let env = setup_env().await;
    register_and_attach(&env, "target-1").await;

    let first = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("dup"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    let replay = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("dup"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    assert_eq!(first.dispatch_id, replay.dispatch_id);
    assert_eq!(first.task_id, replay.task_id);

    // Same key + different input is a conflict.
    let mut conflicting = dispatch_req("dup");
    conflicting.input = json!({"work": "other"});
    let err = env
        .dispatcher
        .handle_dispatch_task(conflicting, user_ctx("alice", "app-a"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains(DISPATCH_ERR_IDEMPOTENCY_CONFLICT));
}

#[tokio::test(flavor = "current_thread")]
async fn offline_target_waits_then_delivers_on_attach() {
    let env = setup_env().await;
    // Registered but no instance attached.
    env.dispatcher
        .handle_register_target(
            RegisterTargetReq {
                registration: test_registration("target-1", DispatchApprovalPolicy::Never),
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();

    let result = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("wait-job"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    env.dispatcher.evaluate_once(false).await;

    let record = env
        .dispatcher
        .db()
        .get_record(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::WaitingForTarget);
    let task = env
        .task_core
        .trusted_get_task(&result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.phase, TaskPhase::Promised);
    assert_eq!(
        task.wait_reason.as_ref().unwrap().code.as_deref(),
        Some("target_offline")
    );

    // Instance comes online -> delivery proceeds.
    env.dispatcher
        .handle_attach_instance(
            AttachInstanceReq {
                target_id: "target-1".into(),
                instance_id: "inst-1".into(),
                endpoint: "http://127.0.0.1:39321/kapi/runner".into(),
                capacity: 1,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    env.caller.push_offer_accepted("inst-1", "res-1");
    env.caller.push_activated();
    env.dispatcher.evaluate_once(false).await;

    let record = env
        .dispatcher
        .db()
        .get_record(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Accepted);
}

#[tokio::test(flavor = "current_thread")]
async fn busy_requeues_with_backoff_rejected_fails_task() {
    let env = setup_env().await;
    register_and_attach(&env, "target-1").await;

    // Busy -> requeued with a future ready_at.
    let busy = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("busy-job"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    env.caller.push_busy(Some(60_000));
    env.dispatcher.evaluate_once(false).await;
    let record = env
        .dispatcher
        .db()
        .get_record(&busy.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Queued);
    assert!(record.ready_at > crate::task_store::now_ms() + 30_000);
    let task = env
        .task_core
        .trusted_get_task(&busy.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.wait_reason.as_ref().unwrap().kind,
        TaskWaitReasonKind::Capacity
    );

    // Stable rejection -> record Rejected, public task Terminal/Failed.
    let rejected = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("rejected-job"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    env.caller.push_rejected("auth_denied");
    env.dispatcher.evaluate_once(false).await;
    let record = env
        .dispatcher
        .db()
        .get_record(&rejected.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Rejected);
    assert_eq!(record.reject_reason, Some(DispatchRejectReason::AuthDenied));
    let task = env
        .task_core
        .trusted_get_task(&rejected.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.phase, TaskPhase::Terminal);
    assert_eq!(task.outcome, Some(TaskOutcome::Failed));
    assert_eq!(task.error.as_ref().unwrap().code, "auth_denied");
}

#[tokio::test(flavor = "current_thread")]
async fn transport_error_requeues_and_attempt_budget_expires() {
    let env = setup_env().await;
    register_and_attach(&env, "target-1").await;
    let result = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("flaky"), user_ctx("alice", "app-a"))
        .await
        .unwrap();

    // Exhaust the 3-attempt budget with transport errors, forcing ready_at
    // back to now between rounds.
    for _ in 0..3 {
        let record = env
            .dispatcher
            .db()
            .get_record(&result.dispatch_id)
            .await
            .unwrap()
            .unwrap();
        let mut update = super::dispatch_db::RecordStateUpdate::to_status(record.status);
        update.ready_at = Some(0);
        env.dispatcher
            .db()
            .update_record_state(&result.dispatch_id, record.status, update)
            .await
            .unwrap();
        env.caller.push_transport_error();
        env.dispatcher.evaluate_once(false).await;
    }
    // One more evaluation: budget exhausted -> Expired.
    let record = env
        .dispatcher
        .db()
        .get_record(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    let mut update = super::dispatch_db::RecordStateUpdate::to_status(record.status);
    update.ready_at = Some(0);
    env.dispatcher
        .db()
        .update_record_state(&result.dispatch_id, record.status, update)
        .await
        .unwrap();
    env.dispatcher.evaluate_once(false).await;

    let record = env
        .dispatcher
        .db()
        .get_record(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Expired);
    assert_eq!(record.attempt_count, 3);
    let task = env
        .task_core
        .trusted_get_task(&result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.outcome, Some(TaskOutcome::Failed));
}

#[tokio::test(flavor = "current_thread")]
async fn approval_gate_holds_then_releases_or_denies() {
    let env = setup_env().await;
    env.dispatcher
        .handle_register_target(
            RegisterTargetReq {
                registration: test_registration(
                    "target-1",
                    DispatchApprovalPolicy::InteractiveCallers,
                ),
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    env.dispatcher
        .handle_attach_instance(
            AttachInstanceReq {
                target_id: "target-1".into(),
                instance_id: "inst-1".into(),
                endpoint: "http://127.0.0.1:39321/kapi/runner".into(),
                capacity: 2,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();

    // Interactive caller is held at the gate; no offer happens.
    let held = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("gated"), user_ctx("bob", "app-b"))
        .await
        .unwrap();
    assert_eq!(held.status, DispatchStatus::PendingApproval);
    env.dispatcher.evaluate_once(false).await;
    assert!(env.caller.calls().is_empty(), "gated record must not offer");
    let task = env
        .task_core
        .trusted_get_task(&held.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        task.wait_reason.as_ref().unwrap().kind,
        TaskWaitReasonKind::Authorization
    );

    // A zone-trusted caller passes straight through the gate.
    let passed = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("trusted"), service_ctx("svc2", "svc-app"))
        .await
        .unwrap();
    assert_eq!(passed.status, DispatchStatus::Queued);

    // Interactive non-sudo cannot approve.
    let err = env
        .dispatcher
        .handle_approve_dispatch(
            ApproveDispatchReq {
                dispatch_id: held.dispatch_id.clone(),
                decision: ApprovalDecision::Approved,
                note: None,
            },
            user_ctx("bob", "app-b"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RPCErrors::NoPermission(_)));

    // Admin approves -> queued -> delivered; creator stays bob.
    env.dispatcher
        .handle_approve_dispatch(
            ApproveDispatchReq {
                dispatch_id: held.dispatch_id.clone(),
                decision: ApprovalDecision::Approved,
                note: Some("ok".into()),
            },
            service_ctx("admin", "admin-app"),
        )
        .await
        .unwrap();
    env.caller.push_offer_accepted("inst-1", "res-a");
    env.caller.push_activated();
    env.caller.push_offer_accepted("inst-1", "res-b");
    env.caller.push_activated();
    env.dispatcher.evaluate_once(false).await;
    let record = env
        .dispatcher
        .db()
        .get_record(&held.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Accepted);
    assert_eq!(record.approval.as_ref().unwrap().decided_by_user, "admin");
    let task = env
        .task_core
        .trusted_get_task(&held.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.creator.user_id, "bob");

    // Denial path terminates the public task with the stable error.
    let denied = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("denied"), user_ctx("bob", "app-b"))
        .await
        .unwrap();
    env.dispatcher
        .handle_approve_dispatch(
            ApproveDispatchReq {
                dispatch_id: denied.dispatch_id.clone(),
                decision: ApprovalDecision::Denied,
                note: None,
            },
            service_ctx("admin", "admin-app"),
        )
        .await
        .unwrap();
    let task = env
        .task_core
        .trusted_get_task(&denied.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.outcome, Some(TaskOutcome::Failed));
    assert_eq!(
        task.error.as_ref().unwrap().code,
        DISPATCH_ERR_APPROVAL_DENIED
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_converges_through_task_control_protocol() {
    let env = setup_env().await;
    // No instances: the record stays queued and cancellable.
    env.dispatcher
        .handle_register_target(
            RegisterTargetReq {
                registration: test_registration("target-1", DispatchApprovalPolicy::Never),
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    let result = env
        .dispatcher
        .handle_dispatch_task(dispatch_req("cancel-me"), user_ctx("alice", "app-a"))
        .await
        .unwrap();

    // The generic UI cancels via the TaskMgr control protocol; TaskMgr only
    // records the request (task has a dispatcher origin).
    let control = env
        .task_core
        .handle_request_control(
            RequestControlReq {
                task_id: result.task_id.clone(),
                action: TaskControlAction::Cancel,
                request_id: "req-1".into(),
                recursive: false,
                expected_revision: None,
            },
            user_ctx("alice", "app-a"),
        )
        .await
        .unwrap();
    let RequestControlResult::Task { task } = control else {
        panic!("expected task result")
    };
    assert_eq!(
        task.phase,
        TaskPhase::Promised,
        "cancel is pending, not applied"
    );
    assert!(task.pending_control.is_some());

    // The dispatcher sweep atomically revokes the queue entry and confirms.
    env.dispatcher.evaluate_once(true).await;
    let record = env
        .dispatcher
        .db()
        .get_record(&result.dispatch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Canceled);
    let task = env
        .task_core
        .trusted_get_task(&result.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.phase, TaskPhase::Terminal);
    assert_eq!(task.outcome, Some(TaskOutcome::Canceled));
}

#[tokio::test(flavor = "current_thread")]
async fn creating_task_saga_recovers_after_crash() {
    let env = setup_env().await;
    register_and_attach(&env, "target-1").await;

    // Simulate a crash right after the record persisted but before the task
    // create ACK: insert a bare CreatingTask record.
    let now = crate::task_store::now_ms();
    let record = DispatchRecord {
        dispatch_id: "dsp-crash".into(),
        requested_target_id: Some("target-1".into()),
        target_id: "target-1".into(),
        target_selection: TargetSelection::Explicit,
        schema_id: RAW_TASK_SCHEMA_ID.into(),
        schema_version: 1,
        registration_revision: 1,
        delivery_policy: DeliveryPolicy::default(),
        status: DispatchStatus::CreatingTask,
        task_id: None,
        input: json!({"work": "crashed"}),
        auth: DispatchAuthEnvelope {
            requested_by_user: "alice".into(),
            requested_by_app: "app-a".into(),
            on_behalf_of: "alice".into(),
            zone_trusted_caller: false,
            workflow_ref: None,
            input_digest: compute_input_digest(&json!({"work": "crashed"})),
            created_at: now,
            expires_at: None,
        },
        priority: 0,
        ready_at: now,
        attempt_count: 0,
        reject_reason: None,
        approval: None,
        message: None,
        expires_at: None,
        created_at: now,
        updated_at: now,
    };
    env.dispatcher
        .db()
        .insert_record(&record, "crash-key")
        .await
        .unwrap();

    // Recovery completes the saga: task created with the derived idempotency
    // key, record queued, then delivered.
    env.caller.push_offer_accepted("inst-1", "res-1");
    env.caller.push_activated();
    env.dispatcher.evaluate_once(false).await;

    let record = env
        .dispatcher
        .db()
        .get_record("dsp-crash")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, DispatchStatus::Accepted);
    let task_id = record.task_id.clone().unwrap();
    let task = env
        .task_core
        .trusted_get_task(&task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.idempotency_key, "dsp:dsp-crash");
    assert_eq!(task.creator.user_id, "alice");

    // Re-running recovery replays the same task (no duplicates).
    let before_calls = env.caller.calls().len();
    env.dispatcher.evaluate_once(false).await;
    assert_eq!(env.caller.calls().len(), before_calls);
}

#[tokio::test(flavor = "current_thread")]
async fn stale_lease_instance_cannot_renew() {
    let env = setup_env().await;
    register_and_attach(&env, "target-1").await;
    // Re-attach bumps the lease epoch; the old epoch is fenced.
    let second = env
        .dispatcher
        .handle_attach_instance(
            AttachInstanceReq {
                target_id: "target-1".into(),
                instance_id: "inst-1".into(),
                endpoint: "http://127.0.0.1:39321/kapi/runner".into(),
                capacity: 4,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    assert_eq!(second.lease_epoch, 2);
    let err = env
        .dispatcher
        .handle_renew_instance(
            RenewInstanceReq {
                target_id: "target-1".into(),
                instance_id: "inst-1".into(),
                lease_epoch: 1,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap_err();
    assert!(is_stale_instance_err(&err));
}

#[tokio::test(flavor = "current_thread")]
async fn attach_rejects_non_local_endpoints() {
    let env = setup_env().await;
    env.dispatcher
        .handle_register_target(
            RegisterTargetReq {
                registration: test_registration("target-1", DispatchApprovalPolicy::Never),
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    for endpoint in [
        "http://evil.example.com/kapi/runner",
        "http://10.0.0.7:4060/kapi/runner",
        "ftp://127.0.0.1/kapi/runner",
        "not a url",
    ] {
        let err = env
            .dispatcher
            .handle_attach_instance(
                AttachInstanceReq {
                    target_id: "target-1".into(),
                    instance_id: "inst-evil".into(),
                    endpoint: endpoint.into(),
                    capacity: 4,
                    available_capacity: None,
                    lease_ms: None,
                },
                service_ctx("svc", "runner-app"),
            )
            .await
            .expect_err("non-local endpoint must be rejected");
        assert!(
            matches!(err, RPCErrors::ParseRequestError(_)),
            "{endpoint}: {err:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pushes_present_the_lease_delivery_token() {
    let env = setup_env().await;
    env.dispatcher
        .handle_register_target(
            RegisterTargetReq {
                registration: test_registration("target-1", DispatchApprovalPolicy::Never),
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    let attached = env
        .dispatcher
        .handle_attach_instance(
            AttachInstanceReq {
                target_id: "target-1".into(),
                instance_id: "inst-1".into(),
                endpoint: "http://127.0.0.1:39321/kapi/runner".into(),
                capacity: 4,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    assert!(attached.delivery_token.starts_with("dlv-"));
    // Renewing the same lease hands back the same token; a re-attach
    // (new lease epoch) rotates it.
    let renewed = env
        .dispatcher
        .handle_renew_instance(
            RenewInstanceReq {
                target_id: "target-1".into(),
                instance_id: "inst-1".into(),
                lease_epoch: attached.lease_epoch,
                available_capacity: None,
                lease_ms: None,
            },
            service_ctx("svc", "runner-app"),
        )
        .await
        .unwrap();
    assert_eq!(renewed.delivery_token, attached.delivery_token);

    env.dispatcher
        .handle_dispatch_task(dispatch_req("job-token"), user_ctx("alice", "app-a"))
        .await
        .unwrap();
    env.caller.push_offer_accepted("inst-1", "res-1");
    env.caller.push_activated();
    env.dispatcher.evaluate_once(false).await;

    // Both offer and activate presented the lease token — and never a
    // kernel session token.
    let tokens = env.caller.tokens();
    assert_eq!(tokens.len(), 2);
    for token in tokens {
        assert_eq!(token.as_deref(), Some(attached.delivery_token.as_str()));
    }
}
