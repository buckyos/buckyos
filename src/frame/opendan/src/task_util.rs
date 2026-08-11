//! TaskMgr 2.0 runner-side helpers for OpenDAN's own tasks.
//!
//! OpenDAN is the bound App runner of every `agent.delegate` task it
//! executes (bound by the dispatcher, or self-bound at create time). All
//! state writes carry the runner fencing envelope; the mutable
//! `AgentDelegateTaskData` projection lives in the task's `progress`
//! snapshot, the immutable request in `input`, the one-shot outcome in
//! `result`.

use anyhow::{anyhow, Result};
use buckyos_api::{
    task_mgr_error_code, AckControlReq, CommitResultReq, FailTaskReq, ReportProgressReq,
    ReportRunningReq, ReportStartedReq, ReportWaitingReq, RunnerWriteEnvelope, Task,
    TaskControlAction, TaskError, TaskExecutor, TaskManagerClient, TaskPhase, TaskWaitReason,
    TASK_ERR_INVALID_PHASE, TASK_ERR_REVISION_CONFLICT,
};
use serde_json::Value;

/// Fencing envelope for the current snapshot.
pub fn runner_envelope(task: &Task) -> RunnerWriteEnvelope {
    let app_instance_id = match &task.executor {
        TaskExecutor::App {
            app_instance_id, ..
        } => app_instance_id.clone(),
        _ => None,
    };
    RunnerWriteEnvelope {
        task_id: task.task_id.clone(),
        app_instance_id,
        runner_epoch: task.runner_epoch,
        expected_revision: task.revision,
    }
}

/// The task-data payload as the executor sees it: committed result wins,
/// then the mutable progress projection, then the immutable input.
pub fn task_payload(task: &Task) -> Value {
    task.result
        .clone()
        .or_else(|| task.progress.clone())
        .unwrap_or_else(|| task.input.clone())
}

/// Drive the task into Running from wherever it legally can start
/// (Accepted → report_started, Waiting → report_running). Running/Paused
/// pass through unchanged.
pub async fn ensure_running(task_mgr: &TaskManagerClient, task: Task) -> Result<Task> {
    let result = match task.phase {
        TaskPhase::Accepted => {
            task_mgr
                .report_started(ReportStartedReq {
                    envelope: runner_envelope(&task),
                })
                .await
        }
        TaskPhase::Waiting => {
            task_mgr
                .report_running(ReportRunningReq {
                    envelope: runner_envelope(&task),
                })
                .await
        }
        _ => return Ok(task),
    };
    match result {
        Ok(task) => Ok(task),
        // A concurrent transition already moved it; re-read and accept.
        Err(err)
            if matches!(
                task_mgr_error_code(&err),
                Some(TASK_ERR_REVISION_CONFLICT) | Some(TASK_ERR_INVALID_PHASE)
            ) =>
        {
            task_mgr
                .get_task(&task.task_id)
                .await
                .map_err(|err| anyhow!("reload task after phase race: {err}"))
        }
        Err(err) => Err(anyhow!("start task {}: {err}", task.task_id)),
    }
}

/// Write the mutable projection + display message. Retries once on a
/// revision race.
pub async fn report_progress_value(
    task_mgr: &TaskManagerClient,
    task: Task,
    progress: Option<Value>,
    message: Option<String>,
) -> Result<Task> {
    let attempt = task_mgr
        .report_progress(ReportProgressReq {
            envelope: runner_envelope(&task),
            progress: progress.clone(),
            message: message.clone(),
        })
        .await;
    match attempt {
        Ok(task) => Ok(task),
        Err(err) if task_mgr_error_code(&err) == Some(TASK_ERR_REVISION_CONFLICT) => {
            let fresh = task_mgr
                .get_task(&task.task_id)
                .await
                .map_err(|err| anyhow!("reload task for progress retry: {err}"))?;
            task_mgr
                .report_progress(ReportProgressReq {
                    envelope: runner_envelope(&fresh),
                    progress,
                    message,
                })
                .await
                .map_err(|err| anyhow!("report progress: {err}"))
        }
        Err(err) => Err(anyhow!("report progress: {err}")),
    }
}

pub async fn report_waiting_reason(
    task_mgr: &TaskManagerClient,
    task: Task,
    reason: TaskWaitReason,
) -> Result<Task> {
    task_mgr
        .report_waiting(ReportWaitingReq {
            envelope: runner_envelope(&task),
            reason,
        })
        .await
        .map_err(|err| anyhow!("report waiting: {err}"))
}

pub async fn commit_task_result(
    task_mgr: &TaskManagerClient,
    task: Task,
    result: Value,
) -> Result<Task> {
    task_mgr
        .commit_result(CommitResultReq {
            task_id: task.task_id.clone(),
            result,
            app_instance_id: runner_envelope(&task).app_instance_id,
            runner_epoch: Some(task.runner_epoch),
            expected_revision: task.revision,
        })
        .await
        .map_err(|err| anyhow!("commit result: {err}"))
}

pub async fn fail_own_task(
    task_mgr: &TaskManagerClient,
    task: Task,
    code: &str,
    message: impl Into<String>,
    detail: Option<Value>,
) -> Result<Task> {
    task_mgr
        .fail_task(FailTaskReq {
            envelope: runner_envelope(&task),
            error: TaskError {
                code: code.to_string(),
                message: message.into(),
                detail,
            },
        })
        .await
        .map_err(|err| anyhow!("fail task: {err}"))
}

/// Acknowledge the pending control request on one of our tasks.
pub async fn ack_pending_control(
    task_mgr: &TaskManagerClient,
    task: &Task,
    applied: bool,
    reject_reason: Option<String>,
) -> Result<Task> {
    let Some(pending) = task.pending_control.as_ref() else {
        return Ok(task.clone());
    };
    task_mgr
        .ack_control(AckControlReq {
            envelope: runner_envelope(task),
            request_id: pending.request_id.clone(),
            applied,
            reject_reason,
        })
        .await
        .map_err(|err| anyhow!("ack control: {err}"))
}

pub fn pending_control_action(task: &Task) -> Option<TaskControlAction> {
    task.pending_control.as_ref().map(|request| request.action)
}

/// Recursive JSON merge-patch (object keys merge, everything else replaces)
/// — the 1.x server-side `data` merge semantics, now applied client-side
/// before writing the progress projection or the one-shot result.
pub fn merge_json(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                if value.is_null() {
                    target_map.remove(key);
                } else if let Some(existing) = target_map.get_mut(key) {
                    merge_json(existing, value);
                } else {
                    target_map.insert(key.clone(), value.clone());
                }
            }
        }
        (target_value, patch_value) => {
            *target_value = patch_value.clone();
        }
    }
}

/// Close one of our own (creator == runner) tasks as Canceled: record the
/// control request, then acknowledge it as the runner. Terminal races count
/// as success.
pub async fn cancel_own_task(task_mgr: &TaskManagerClient, task: &Task) -> Result<()> {
    let request_id = format!("cancel-{}", task.task_id);
    let requested = task_mgr
        .request_control(buckyos_api::RequestControlReq {
            task_id: task.task_id.clone(),
            action: TaskControlAction::Cancel,
            request_id: request_id.clone(),
            recursive: false,
            expected_revision: None,
        })
        .await;
    let task = match requested {
        Ok(buckyos_api::RequestControlResult::Task { task }) => task,
        Ok(_) => return Ok(()),
        Err(err) => {
            return match task_mgr_error_code(&err) {
                Some(buckyos_api::TASK_ERR_ALREADY_COMPLETED) => Ok(()),
                _ => Err(anyhow!("request cancel: {err}")),
            }
        }
    };
    if task.phase.is_terminal() {
        return Ok(());
    }
    let _ = ack_pending_control(task_mgr, &task, true, None).await;
    Ok(())
}
