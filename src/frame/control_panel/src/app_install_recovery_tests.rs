use super::*;
use crate::app_install_engine::*;
use async_trait::async_trait;
use buckyos_api::*;
use kRPC::{RPCContext, RPCErrors};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[path = "../../../test/install_plan_fixture.rs"]
mod fixture;

#[derive(Default)]
struct SchedulerState {
    record: Mutex<Option<InstallPlanExecutionRecord>>,
    unavailable: AtomicBool,
    lose_response: AtomicBool,
    retries: AtomicUsize,
    cancels: AtomicUsize,
}

struct TestScheduler(Arc<SchedulerState>);

fn transport_error() -> RPCErrors {
    RPCErrors::ReasonError("connection lost".into())
}

#[async_trait]
impl SchedulerHandler for TestScheduler {
    async fn handle_submit_install_plan(
        &self,
        plan: InstallPlan,
        _: RPCContext,
    ) -> kRPC::Result<InstallPlanExecutionRecord> {
        if self.0.unavailable.load(Ordering::SeqCst) {
            return Err(transport_error());
        }
        let mut current = self.0.record.lock().unwrap();
        let record = current
            .get_or_insert_with(|| {
                let mut record = InstallPlanExecutionRecord::new(plan);
                record.state = InstallPlanExecutionState::Completed;
                record.commit_point = InstallPlanCommitPoint::NodeConfigPublished;
                record
            })
            .clone();
        if self.0.lose_response.load(Ordering::SeqCst) {
            return Err(transport_error());
        }
        Ok(record)
    }
    async fn handle_get_install_plan_status(
        &self,
        _: InstallPlanExecutionKey,
        _: RPCContext,
    ) -> kRPC::Result<InstallPlanExecutionRecord> {
        if self.0.unavailable.load(Ordering::SeqCst) {
            return Err(transport_error());
        }
        self.0
            .record
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(transport_error)
    }
    async fn handle_retry_install_plan(
        &self,
        _: InstallPlanExecutionKey,
        _: RPCContext,
    ) -> kRPC::Result<InstallPlanExecutionRecord> {
        self.0.retries.fetch_add(1, Ordering::SeqCst);
        let mut current = self.0.record.lock().unwrap();
        let record = current.as_mut().unwrap();
        record.state = InstallPlanExecutionState::Completed;
        record.commit_point = InstallPlanCommitPoint::NodeConfigPublished;
        record.error = None;
        Ok(record.clone())
    }
    async fn handle_cancel_install_plan(
        &self,
        plan: InstallPlan,
        _: RPCContext,
    ) -> kRPC::Result<InstallPlanExecutionRecord> {
        self.0.cancels.fetch_add(1, Ordering::SeqCst);
        let mut current = self.0.record.lock().unwrap();
        if current
            .as_ref()
            .is_some_and(|record| record.commit_point.is_committed())
        {
            return Err(RPCErrors::ReasonError(
                "desired state already committed".into(),
            ));
        }
        let record = current.get_or_insert_with(|| InstallPlanExecutionRecord::new(plan));
        record.state = InstallPlanExecutionState::Canceled;
        Ok(record.clone())
    }
    async fn handle_run_thunk(
        &self,
        _: SchedulerRunThunkRequest,
        _: RPCContext,
    ) -> kRPC::Result<SchedulerRunThunkResponse> {
        unreachable!()
    }
    async fn handle_refresh_rbac(
        &self,
        _: RPCContext,
    ) -> kRPC::Result<SchedulerRefreshRbacResponse> {
        unreachable!()
    }
    async fn handle_mutate_shortcut(
        &self,
        _: SchedulerShortcutMutationPlan,
        _: RPCContext,
    ) -> kRPC::Result<SchedulerShortcutMutationRecord> {
        unreachable!()
    }
}

fn client(state: &Arc<SchedulerState>) -> SchedulerClient {
    SchedulerClient::new_in_process(Box::new(TestScheduler(state.clone())))
}

#[tokio::test]
async fn lost_submit_response_recovers_the_committed_execution() {
    let state = Arc::new(SchedulerState::default());
    state.lose_response.store(true, Ordering::SeqCst);
    let plan = fixture::plan();
    let record = reconcile_execution(&client(&state), &plan, InstallStage::Prepare, true)
        .await
        .unwrap();
    assert_eq!(record.state, InstallPlanExecutionState::Completed);
    assert_eq!(record.key, InstallPlanExecutionKey::from_plan(&plan));
    assert_eq!(state.retries.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retry_preserves_the_original_plan_before_and_after_commit() {
    for commit_point in [
        InstallPlanCommitPoint::Claimed,
        InstallPlanCommitPoint::DesiredStateCommitted,
    ] {
        let state = Arc::new(SchedulerState::default());
        let plan = fixture::plan();
        let mut record = InstallPlanExecutionRecord::new(plan.clone());
        record.state = InstallPlanExecutionState::Failed;
        record.commit_point = commit_point;
        record.error = Some(stage_error(
            InstallStage::Deploy,
            InstallErrorCode::ActivationFailed,
            true,
            "schedule failed",
        ));
        *state.record.lock().unwrap() = Some(record);
        let completed = reconcile_execution(&client(&state), &plan, InstallStage::Prepare, false)
            .await
            .unwrap();
        assert_eq!(completed.plan, plan);
        assert_eq!(completed.state, InstallPlanExecutionState::Completed);
        assert_eq!(state.retries.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn superseded_or_mismatched_execution_is_not_retried() {
    let plan = fixture::plan();
    for mismatched in [false, true] {
        let state = Arc::new(SchedulerState::default());
        let mut record = InstallPlanExecutionRecord::new(plan.clone());
        record.state = InstallPlanExecutionState::Failed;
        record.commit_point = InstallPlanCommitPoint::DesiredStateCommitted;
        record.error = Some(stage_error(
            InstallStage::Deploy,
            InstallErrorCode::PlanNotApplicable,
            false,
            "uninstalled",
        ));
        if mismatched {
            record.key.task_id = "other-task".into();
        }
        *state.record.lock().unwrap() = Some(record);
        let error = reconcile_execution(&client(&state), &plan, InstallStage::Prepare, false)
            .await
            .unwrap_err();
        assert!(!error.retryable);
        assert_eq!(state.retries.load(Ordering::SeqCst), 0);
    }
}

struct MemoryStore(Mutex<InstallTaskView>);

impl MemoryStore {
    fn new() -> Arc<Self> {
        let plan = fixture::plan();
        let data = AppInstallTaskData {
            schema_version: APP_INSTALL_SCHEMA_VERSION,
            request: AppInstallTaskRequest {
                source: InstallSource::identifier(plan.app.did.to_string(), None),
                creator_user_id: "alice".into(),
                creator_app_id: "control-panel".into(),
                owner_user_id: "alice".into(),
                idempotency_key: "request-1".into(),
                submitted_plan: None,
                approved_plan_fingerprint: None,
                policy: InstallPolicy::Normal,
                options: None,
            },
            state: InstallTransactionState {
                stage: Some(InstallStage::Prepare),
                plan: Some(plan),
                completed_stages: vec![
                    InstallStage::Resolve,
                    InstallStage::Inspect,
                    InstallStage::Acquire,
                    InstallStage::Verify,
                ],
                ..Default::default()
            },
        };
        Arc::new(Self(Mutex::new(InstallTaskView {
            id: "install-1".into(),
            planning_task_id: "install-1".into(),
            parent_id: None,
            root_id: "install-1".into(),
            task_type: TASK_DATA_TYPE_APP_INSTALL.into(),
            status: InstallTaskStatus::Running,
            user_id: "alice".into(),
            app_id: "control-panel".into(),
            data: serde_json::to_value(data).unwrap(),
            phase: TaskPhase::Running,
            outcome: None,
            wait_reason: None,
            display: None,
            updated_at: 1,
        })))
    }
}

#[async_trait]
impl InstallTaskStore for MemoryStore {
    async fn load(&self, _: &str) -> Result<InstallTaskView, InstallError> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn write_data(&self, _: &str, data: Value) -> Result<(), InstallError> {
        let mut view = self.0.lock().unwrap();
        assert!(!view.status.is_terminal());
        view.data = data;
        Ok(())
    }
    async fn set_status(
        &self,
        _: &str,
        status: InstallTaskStatus,
        _: Option<f32>,
        _: Option<String>,
    ) -> Result<(), InstallError> {
        self.0.lock().unwrap().status = status;
        Ok(())
    }
    async fn list_active(&self) -> Result<Vec<InstallTaskView>, InstallError> {
        Ok(vec![self.0.lock().unwrap().clone()])
    }
    async fn create_install_task(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: Value,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<String, InstallError> {
        unreachable!()
    }
}

struct RecoveryDriver {
    scheduler: Arc<SchedulerState>,
    released: AtomicUsize,
    prepare_gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}

#[async_trait]
impl InstallStageDriver for RecoveryDriver {
    async fn resolve(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<ResolveOutcome, InstallError> {
        panic!("must preserve completed stages")
    }
    async fn inspect(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<Option<InstallInspection>, InstallError> {
        unreachable!()
    }
    async fn acquire(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<InstallPlanStatus, InstallError> {
        unreachable!()
    }
    async fn verify(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<VerificationReport, InstallError> {
        unreachable!()
    }
    async fn prepare(
        &self,
        _: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError> {
        if let Some((entered, resume)) = &self.prepare_gate {
            entered.notify_one();
            resume.notified().await;
        }
        let plan = data.state.plan.as_ref().unwrap();
        reconcile_execution(&client(&self.scheduler), plan, InstallStage::Prepare, true).await?;
        Ok(PreparedDeployment {
            app_instance_id: plan.app_instance_id.clone(),
            task_id: plan.task_id.clone(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            submitted_at: 1,
        })
    }
    async fn deploy(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        Ok(())
    }
    async fn activate(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<InstallTaskResult, InstallError> {
        Ok(InstallTaskResult {
            install_record_key: None,
            proof_id: None,
            instance_node_id: None,
            completed_at: Some(2),
        })
    }
    async fn rollback_deploy(
        &self,
        _: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        client(&self.scheduler)
            .cancel_install_plan(data.state.plan.clone().unwrap())
            .await
            .map_err(|error| {
                stage_error(
                    InstallStage::Prepare,
                    InstallErrorCode::Conflict,
                    true,
                    error.to_string(),
                )
            })?;
        Ok(())
    }
    async fn release_staging(
        &self,
        _: &InstallTaskView,
        _: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        self.released.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn unavailable_scheduler_keeps_task_recoverable_across_runner_restart() {
    let scheduler = Arc::new(SchedulerState::default());
    scheduler.unavailable.store(true, Ordering::SeqCst);
    let driver = Arc::new(RecoveryDriver {
        scheduler: scheduler.clone(),
        released: AtomicUsize::new(0),
        prepare_gate: None,
    });
    let store = MemoryStore::new();
    let engine = InstallEngine::new(store.clone(), driver.clone());
    assert!(engine.run_task("install-1").await.unwrap_err().retryable);
    let failed = store.load("install-1").await.unwrap();
    assert_eq!(failed.status, InstallTaskStatus::Running);
    let data: AppInstallTaskData = serde_json::from_value(failed.data).unwrap();
    assert_eq!(data.state.stage, Some(InstallStage::Prepare));
    assert!(data.state.last_error.is_some());
    assert_eq!(driver.released.load(Ordering::SeqCst), 0);
    assert_eq!(
        engine
            .retry("install-1", "alice", false, "control-panel", "retry-1")
            .await
            .unwrap(),
        "install-1"
    );
    scheduler.unavailable.store(false, Ordering::SeqCst);
    let restarted = InstallEngine::new(store.clone(), driver.clone());
    assert_eq!(
        restarted.run_task("install-1").await.unwrap(),
        RunOutcome::Completed
    );
    assert_eq!(
        store.load("install-1").await.unwrap().status,
        InstallTaskStatus::Completed
    );
    assert_eq!(driver.released.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn prepare_cancellation_uses_scheduler_commit_point() {
    for committed in [false, true] {
        let scheduler = Arc::new(SchedulerState::default());
        let mut record = InstallPlanExecutionRecord::new(fixture::plan());
        record.commit_point = if committed {
            InstallPlanCommitPoint::DesiredStateCommitted
        } else {
            InstallPlanCommitPoint::Claimed
        };
        *scheduler.record.lock().unwrap() = Some(record);
        let driver = Arc::new(RecoveryDriver {
            scheduler: scheduler.clone(),
            released: AtomicUsize::new(0),
            prepare_gate: None,
        });
        let store = MemoryStore::new();
        let engine = InstallEngine::new(store.clone(), driver);
        let result = engine
            .cancel("install-1", "alice", "control-panel", false)
            .await;
        assert_eq!(result.is_err(), committed);
        assert_eq!(scheduler.cancels.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.load("install-1").await.unwrap().status,
            if committed {
                InstallTaskStatus::Running
            } else {
                InstallTaskStatus::Canceled
            }
        );
    }
}

#[tokio::test]
async fn cancel_during_prepare_cannot_be_overwritten_by_late_runner() {
    let scheduler = Arc::new(SchedulerState::default());
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let driver = Arc::new(RecoveryDriver {
        scheduler: scheduler.clone(),
        released: AtomicUsize::new(0),
        prepare_gate: Some((entered.clone(), resume.clone())),
    });
    let store = MemoryStore::new();
    let engine = Arc::new(InstallEngine::new(store.clone(), driver));
    let runner = engine.clone();
    let running = tokio::spawn(async move { runner.run_task("install-1").await });
    entered.notified().await;
    engine
        .cancel("install-1", "alice", "control-panel", false)
        .await
        .unwrap();
    resume.notify_one();
    assert_eq!(
        running.await.unwrap().unwrap_err().code,
        InstallErrorCode::Canceled
    );
    assert_eq!(
        store.load("install-1").await.unwrap().status,
        InstallTaskStatus::Canceled
    );
    assert_eq!(
        scheduler.record.lock().unwrap().as_ref().unwrap().state,
        InstallPlanExecutionState::Canceled
    );
}

#[test]
fn confirmed_commit_stops_offering_cancellation() {
    let store = MemoryStore::new();
    let view = store.0.lock().unwrap().clone();
    let mut data: AppInstallTaskData = serde_json::from_value(view.data).unwrap();
    for committed in [false, true] {
        if committed {
            data.state.mark_stage_completed(InstallStage::Prepare);
        }
        let snapshot = build_install_status_snapshot(
            "install-1".into(),
            TaskPhase::Running,
            None,
            None,
            "alice",
            &data.request.source,
            data.request.policy,
            &data.state,
            None,
            1,
        );
        assert_eq!(
            snapshot
                .available_actions
                .contains(&InstallUserAction::Cancel),
            !committed
        );
    }
}
