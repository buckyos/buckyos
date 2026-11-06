//! In-process runner for `FunctionType::Native` thunks (TODO D1 / D16).
//!
//! `schedule_thunk` only admits the execution into a bounded queue and
//! returns; a spawned task waits for its lane slot, runs the native
//! implementation under deadline / cancellation, stores the result object and
//! pushes a [`ThunkExecutionResult`] into the result channel that the run
//! driver consumes. The runner also keeps the thunk task of the TaskMgr tree
//! up to date and records active executions for restart cleanup.

use crate::dispatcher::ThunkDispatcher;
use crate::error::{WorkflowError, WorkflowResult};
use crate::function_registry::{FunctionRegistry, RegisteredFunction};
use crate::native::{
    CancelToken, EnvironmentProvider, ExecutionLane, ExecutionLimits, NativeContext, NativeIo,
};
use crate::persistence::{ActiveExecutionRecord, ExecutionLedger};
use crate::task_tracker::{ThunkTaskPhase, WorkflowTaskTracker};
use async_trait::async_trait;
use buckyos_api::{
    error_codes, FunctionParamType, ThunkExecution, ThunkExecutionError, ThunkExecutionResult,
    ThunkObject,
};
use log::{debug, info, warn};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};

pub type ResultSender = mpsc::Sender<ThunkExecutionResult>;

pub struct LocalThunkRunner {
    inner: Arc<RunnerInner>,
}

struct RunnerInner {
    registry: Arc<FunctionRegistry>,
    io: Arc<dyn NativeIo>,
    env: Arc<dyn EnvironmentProvider>,
    tracker: Arc<dyn WorkflowTaskTracker>,
    ledger: Arc<dyn ExecutionLedger>,
    results: ResultSender,
    lanes: HashMap<ExecutionLane, Arc<Semaphore>>,
    active: Mutex<HashMap<String, Arc<CancelToken>>>,
    queued: AtomicUsize,
    temp_root: PathBuf,
    epoch: u64,
    limits: ExecutionLimits,
}

impl LocalThunkRunner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Arc<FunctionRegistry>,
        io: Arc<dyn NativeIo>,
        env: Arc<dyn EnvironmentProvider>,
        tracker: Arc<dyn WorkflowTaskTracker>,
        ledger: Arc<dyn ExecutionLedger>,
        results: ResultSender,
        temp_root: PathBuf,
        epoch: u64,
        limits: ExecutionLimits,
    ) -> Self {
        let lanes = ExecutionLane::ALL
            .iter()
            .map(|lane| (*lane, Arc::new(Semaphore::new(lane.default_slots()))))
            .collect();
        Self {
            inner: Arc::new(RunnerInner {
                registry,
                io,
                env,
                tracker,
                ledger,
                results,
                lanes,
                active: Mutex::new(HashMap::new()),
                queued: AtomicUsize::new(0),
                temp_root,
                epoch,
                limits,
            }),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.inner.epoch
    }

    pub fn active_count(&self) -> usize {
        self.inner.active.lock().unwrap().len()
    }

    pub fn queued_count(&self) -> usize {
        self.inner.queued.load(Ordering::SeqCst)
    }

    /// Fails executions a previous process instance left behind and removes
    /// their temp dirs. Only this runner's own ledger rows are touched.
    pub async fn cleanup_stale(&self) -> WorkflowResult<Vec<ActiveExecutionRecord>> {
        let stale = self.inner.ledger.stale_executions(self.inner.epoch).await?;
        for record in &stale {
            let error = ThunkExecutionError::new(
                error_codes::INTERRUPTED,
                format!(
                    "workflow service restarted (epoch {} -> {}) while the execution was running",
                    record.epoch, self.inner.epoch
                ),
            )
            .with_details(json!({
                "previous_epoch": record.epoch,
                "current_epoch": self.inner.epoch,
            }));
            if let Err(err) = self
                .inner
                .tracker
                .update_thunk(
                    &record.run_id,
                    &record.execution_id,
                    ThunkTaskPhase::Failed(error),
                )
                .await
            {
                warn!(
                    "workflow runner: mark interrupted thunk task failed execution_id={} err={}",
                    record.execution_id, err
                );
            }
            let temp_dir = PathBuf::from(&record.temp_dir);
            if temp_dir.starts_with(&self.inner.temp_root) {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            }
            self.inner.ledger.record_finished(&record.execution_id).await?;
            info!(
                "workflow runner: interrupted execution cleaned execution_id={} run_id={}",
                record.execution_id, record.run_id
            );
        }
        Ok(stale)
    }
}

#[async_trait]
impl ThunkDispatcher for LocalThunkRunner {
    async fn schedule_thunk(
        &self,
        execution: &ThunkExecution,
        thunk: &ThunkObject,
    ) -> WorkflowResult<()> {
        let fun_id = thunk.fun_id.to_string();
        let function = self.inner.registry.get(&fun_id).ok_or_else(|| {
            WorkflowError::Execution(
                ThunkExecutionError::new(
                    error_codes::FUNCTION_NOT_FOUND,
                    format!("function `{fun_id}` is not registered on this runner"),
                )
                .with_details(json!({"fun_id": fun_id})),
            )
        })?;
        if function.native.is_none() {
            return Err(WorkflowError::Execution(
                ThunkExecutionError::new(
                    error_codes::FUNCTION_NOT_FOUND,
                    format!(
                        "function `{}@{}` has no native implementation on this runner",
                        function.registration.name, function.registration.version
                    ),
                )
                .with_details(json!({"fun_id": fun_id})),
            ));
        }
        let queued = self.inner.queued.fetch_add(1, Ordering::SeqCst);
        if queued >= self.inner.limits.max_queue {
            self.inner.queued.fetch_sub(1, Ordering::SeqCst);
            return Err(WorkflowError::Execution(
                ThunkExecutionError::new(
                    error_codes::QUEUE_FULL,
                    format!("runner queue is full ({} executions)", self.inner.limits.max_queue),
                )
                .retryable(),
            ));
        }
        let cancel = CancelToken::new();
        self.inner
            .active
            .lock()
            .unwrap()
            .insert(execution.execution_id.clone(), cancel.clone());
        let _ = self
            .inner
            .tracker
            .update_thunk(
                &execution.run_id,
                &execution.execution_id,
                ThunkTaskPhase::Queued,
            )
            .await;
        let inner = self.inner.clone();
        let execution = execution.clone();
        let thunk = thunk.clone();
        tokio::spawn(async move {
            inner.run(execution, thunk, function, cancel).await;
        });
        Ok(())
    }

    async fn cancel_thunk(&self, execution_id: &str) -> WorkflowResult<()> {
        if let Some(token) = self.inner.active.lock().unwrap().get(execution_id).cloned() {
            token.cancel();
        }
        Ok(())
    }
}

impl RunnerInner {
    async fn run(
        self: Arc<Self>,
        execution: ThunkExecution,
        thunk: ThunkObject,
        function: Arc<RegisteredFunction>,
        cancel: Arc<CancelToken>,
    ) {
        let queued_at = Instant::now();
        let deadline = execution
            .deadline_at
            .map(|at| {
                let now = chrono::Utc::now().timestamp();
                Instant::now() + Duration::from_secs(at.saturating_sub(now).max(0) as u64)
            });
        let native = function.native.clone().expect("checked at schedule time");
        let lane = native.lane();
        let semaphore = self.lanes[&lane].clone();
        let queue_wait = Duration::from_secs(self.limits.max_queue_wait_secs);
        let wait_limit = deadline
            .map(|d| d.saturating_duration_since(Instant::now()).min(queue_wait))
            .unwrap_or(queue_wait);
        let permit = tokio::select! {
            permit = semaphore.acquire_owned() => permit.ok(),
            _ = cancel.cancelled() => None,
            _ = tokio::time::sleep(wait_limit) => None,
        };
        self.queued.fetch_sub(1, Ordering::SeqCst);
        let Some(_permit) = permit else {
            let code = if cancel.is_cancelled() {
                error_codes::CANCELLED
            } else {
                error_codes::TIMEOUT
            };
            let error = ThunkExecutionError::new(
                code,
                format!("execution left the queue before a `{lane:?}` slot was available"),
            )
            .with_details(json!({"lane": lane, "phase": "queued"}));
            self.finish(&execution, ThunkExecutionResult::failed(&execution, error), None)
                .await;
            return;
        };
        let queued_ms = queued_at.elapsed().as_millis() as u64;
        let temp_dir = self.temp_root.join(sanitize(&execution.execution_id));
        if let Err(err) = tokio::fs::create_dir_all(&temp_dir).await {
            let error = ThunkExecutionError::new(
                error_codes::INTERNAL,
                format!("create temp dir {}: {err}", temp_dir.display()),
            );
            self.finish(&execution, ThunkExecutionResult::failed(&execution, error), None)
                .await;
            return;
        }
        let _ = self
            .ledger
            .record_started(&ActiveExecutionRecord {
                execution_id: execution.execution_id.clone(),
                run_id: execution.run_id.clone(),
                node_id: execution.node_id.clone(),
                epoch: self.epoch,
                temp_dir: temp_dir.to_string_lossy().into_owned(),
                task_id: None,
                started_at: chrono::Utc::now().timestamp(),
            })
            .await;
        let _ = self
            .tracker
            .update_thunk(
                &execution.run_id,
                &execution.execution_id,
                ThunkTaskPhase::Running,
            )
            .await;
        let started = Instant::now();
        let env = self.env.current();
        let fingerprint = native.fingerprint(&env);
        let cacheable_prefix = execution
            .cache_key
            .as_ref()
            .map(|key| format!("media/{key}"))
            .unwrap_or_else(|| format!("exec/{}", sanitize(&execution.execution_id)));
        let ctx = NativeContext {
            execution: execution.clone(),
            temp_dir: temp_dir.clone(),
            cancel: cancel.clone(),
            deadline,
            env: env.clone(),
            io: self.io.clone(),
            limits: self.limits.clone(),
            artifact_prefix: cacheable_prefix,
            tool_inputs: Default::default(),
        };
        let params = Value::Object(thunk.params.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        let outcome = async {
            if !execution.fingerprint.is_empty() && execution.fingerprint != fingerprint {
                return Err(ThunkExecutionError::new(
                    error_codes::ENVIRONMENT_CHANGED,
                    "tool environment changed between scheduling and execution",
                )
                .with_details(json!({
                    "expected": execution.fingerprint,
                    "actual": fingerprint,
                }))
                .retryable());
            }
            ctx.require_tools(native.tools())?;
            function
                .object
                .validate_params(&params)
                .map_err(|err| ThunkExecutionError::new(error_codes::INVALID_PARAMS, err))?;
            for (name, kind) in &function.object.params_type {
                let Some(value) = params.get(name) else {
                    continue;
                };
                match kind {
                    FunctionParamType::CheckByRunner(_) | FunctionParamType::ObjId(_) => {
                        let id = value.as_str().ok_or_else(|| {
                            ThunkExecutionError::new(
                                error_codes::INVALID_PARAMS,
                                format!("param `{name}` must be an object id string"),
                            )
                        })?;
                        if self.io.content_size(id).await?.is_none() {
                            return Err(ThunkExecutionError::new(
                                error_codes::INPUT_UNAVAILABLE,
                                format!("param `{name}` references unavailable content `{id}`"),
                            )
                            .with_details(json!({"param": name, "content_id": id})));
                        }
                    }
                    FunctionParamType::Fixed(_) => {}
                }
            }
            let remaining = ctx.remaining().unwrap_or(Duration::from_secs(self.limits.max_exec_secs));
            let invoke = native.invoke(&ctx, &params);
            let outcome = tokio::select! {
                outcome = invoke => outcome?,
                _ = cancel.cancelled() => {
                    return Err(ThunkExecutionError::new(error_codes::CANCELLED, "execution cancelled"));
                }
                _ = tokio::time::sleep(remaining) => {
                    cancel.cancel();
                    return Err(ThunkExecutionError::new(error_codes::TIMEOUT, "execution deadline passed")
                        .with_details(json!({"deadline_at": execution.deadline_at})));
                }
            };
            function
                .object
                .validate_result(&outcome.result)
                .map_err(|err| ThunkExecutionError::new(error_codes::INVALID_RESULT, err))?;
            let result_obj_id = self.io.put_object("wfresult", &outcome.result).await?;
            Ok((outcome, result_obj_id))
        }
        .await;
        ctx.release_tool_inputs().await;
        let duration_ms = started.elapsed().as_millis() as u64;
        let result = match outcome {
            Ok((outcome, result_obj_id)) => {
                let mut result = ThunkExecutionResult::success(&execution, outcome.result);
                result.result_obj_id = ndn_lib::ObjId::new(&result_obj_id).ok();
                result.fingerprint = fingerprint;
                result.artifact_refs = outcome.artifact_refs;
                result.cacheable = outcome.cacheable;
                result.metrics = json!({
                    "duration_ms": duration_ms,
                    "queued_ms": queued_ms,
                    "lane": lane,
                    "native": outcome.metrics,
                });
                result
            }
            Err(error) => {
                let mut result = ThunkExecutionResult::failed(&execution, error);
                result.fingerprint = fingerprint;
                result.metrics = json!({
                    "duration_ms": duration_ms,
                    "queued_ms": queued_ms,
                    "lane": lane,
                });
                result
            }
        };
        self.finish(&execution, result, Some(temp_dir)).await;
    }

    async fn finish(
        &self,
        execution: &ThunkExecution,
        result: ThunkExecutionResult,
        temp_dir: Option<PathBuf>,
    ) {
        if let Some(dir) = temp_dir {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
        let _ = self.ledger.record_finished(&execution.execution_id).await;
        self.active
            .lock()
            .unwrap()
            .remove(&execution.execution_id);
        let phase = match (&result.error, result.status) {
            (Some(error), buckyos_api::ThunkExecutionStatus::Cancelled) => {
                ThunkTaskPhase::Cancelled(error.clone())
            }
            (Some(error), _) => ThunkTaskPhase::Failed(error.clone()),
            (None, _) => ThunkTaskPhase::Succeeded(json!({
                "result_obj_id": result.result_obj_id,
                "metrics": result.metrics,
            })),
        };
        if let Err(err) = self
            .tracker
            .update_thunk(&execution.run_id, &execution.execution_id, phase)
            .await
        {
            debug!(
                "workflow runner: thunk task update failed execution_id={} err={}",
                execution.execution_id, err
            );
        }
        if self.results.send(result).await.is_err() {
            warn!(
                "workflow runner: result channel closed, dropping receipt execution_id={}",
                execution.execution_id
            );
        }
    }
}

fn sanitize(execution_id: &str) -> String {
    execution_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}
